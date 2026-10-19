// Actual callback capture transport: publication, queue ordering and fences.
fn recall_audio_patch() -> Patch {
    let mut patch: Patch =
        serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"pitch","kind":"stock_lfo","params":{"rate":1.25}},
            {"id":"recall","kind":"recall_random","params":{"seed":13,"t_range":2,"t_rate":60}}],
        "cables":[{"from":"pitch","output":"sine","to":"recall","input":"register_cv"}],
        "output":"recall","output_port":"x1"}))
        .unwrap();
    patch.assign_missing_cable_ids().unwrap();
    patch
}
fn recall_audio_inspect(callback: &Callback) -> Result<serde_json::Value, String> {
    callback
        .performance
        .recall_random_capture("recall", "inspect", 0, 1)
}
fn recall_audio_complete(callback: &mut Callback) -> serde_json::Value {
    let receipt = callback
        .performance
        .recall_random_capture("recall", "begin", 2, 1)
        .unwrap();
    assert_eq!(receipt["queued"], true);
    assert_eq!(receipt["applied"], false);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 16384], 1));
    assert_eq!(
        recall_audio_inspect(callback).unwrap()["capture"]["status"],
        "recording"
    );
    callback
        .performance
        .recall_random_capture("recall", "stop", 0, 1)
        .unwrap();
    assert!(recall_audio_inspect(callback).is_err());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    let result = recall_audio_inspect(callback).unwrap();
    assert_eq!(result["capture"]["status"], "complete");
    let state: openpatch_core::RecallRandomCaptureState =
        serde_json::from_value(result["capture"].clone()).unwrap();
    state.scale.to_scale().validate().unwrap();
    result
}
#[test]
fn recall_capture_callback_publishes_real_weighted_result_and_blocks_pending_restart_or_cancel_receipts_without_heap(
) {
    let patch = recall_audio_patch();
    let (mut callback, _, _retired) = sequence_callback(&patch);
    assert!(recall_audio_inspect(&callback).is_err());
    let first = recall_audio_complete(&mut callback);
    let first_id = first["capture"]["capture_id"].as_u64().unwrap();
    callback
        .performance
        .recall_random_capture("recall", "begin", 4, 1)
        .unwrap();
    assert!(
        recall_audio_inspect(&callback)
            .unwrap_err()
            .contains("acknowledgment"),
        "queued restart must hide the old Complete result even at the same revision"
    );
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    let restarted = recall_audio_inspect(&callback).unwrap();
    assert_eq!(restarted["capture"]["status"], "recording");
    assert_eq!(restarted["capture"]["slot"], 4);
    assert!(restarted["capture"]["capture_id"].as_u64().unwrap() > first_id);
    callback
        .performance
        .recall_random_capture("recall", "cancel", 0, 1)
        .unwrap();
    assert!(recall_audio_inspect(&callback).is_err());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    let cancelled = recall_audio_inspect(&callback).unwrap();
    assert_eq!(cancelled["capture"]["status"], "cancelled");
    assert_eq!(cancelled["capture"]["scale"]["num_degrees"], 0);
    let completed = recall_audio_complete(&mut callback);
    callback
        .performance
        .recall_random_capture("recall", "cancel", 0, 1)
        .unwrap();
    assert!(recall_audio_inspect(&callback).is_err());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    let discarded = recall_audio_inspect(&callback).unwrap();
    assert_eq!(
        discarded["capture"]["capture_id"],
        completed["capture"]["capture_id"]
    );
    assert_eq!(discarded["capture"]["status"], "cancelled");
    assert_eq!(discarded["capture"]["scale"]["num_degrees"], 0);
}
#[test]
fn recall_capture_callback_preserves_begin_stop_restart_order_at_one_revision() {
    let (mut callback, _, _retired) = sequence_callback(&recall_audio_patch());
    let backend = callback.performance.clone();
    for (action, slot) in [("begin", 1), ("stop", 0), ("begin", 5)] {
        backend
            .recall_random_capture("recall", action, slot, 1)
            .unwrap();
    }
    assert!(recall_audio_inspect(&callback).is_err());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    let state = recall_audio_inspect(&callback).unwrap();
    assert_eq!(state["capture"]["status"], "recording");
    assert_eq!(state["capture"]["slot"], 5);
    assert_eq!(state["capture"]["capture_id"], 2);
    assert_eq!(backend.snapshot()["applied"], 3);
    assert_eq!(backend.snapshot()["rejected"], 0);
}
#[test]
fn recall_capture_rejected_latest_command_cannot_reexpose_an_old_complete_result() {
    let (mut callback, _, _retired) = sequence_callback(&recall_audio_patch());
    let completed = recall_audio_complete(&mut callback);
    let backend = callback.performance.clone();
    // STOP while already Complete is accepted by the bounded queue but rejected
    // by the callback. Later unrelated success must not clear its per-slot fence.
    backend
        .recall_random_capture("recall", "stop", 0, 1)
        .unwrap();
    backend.trigger("recall", ModuleAction::Reset).unwrap();
    assert!(recall_audio_inspect(&callback).is_err());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    assert!(recall_audio_inspect(&callback)
        .unwrap_err()
        .contains("rejected"));
    assert_eq!(backend.snapshot()["rejected"], 1);
    backend
        .recall_random_capture("recall", "begin", 3, 1)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    let next = recall_audio_inspect(&callback).unwrap();
    assert_eq!(next["capture"]["status"], "recording");
    assert!(
        next["capture"]["capture_id"].as_u64().unwrap()
            > completed["capture"]["capture_id"].as_u64().unwrap()
    );
    backend
        .recall_random_capture("recall", "begin", 0, 1)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    assert!(recall_audio_inspect(&callback)
        .unwrap_err()
        .contains("rejected"));
    backend
        .recall_random_capture("recall", "cancel", 0, 1)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    assert_eq!(
        recall_audio_inspect(&callback).unwrap()["capture"]["status"],
        "cancelled"
    );
}
#[test]
fn recall_capture_removal_recreation_and_revision_changes_fence_published_results() {
    let mut patch = recall_audio_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    let first = recall_audio_complete(&mut callback);
    let backend = callback.performance.clone();
    // Ordinary authored edits preserve recorder runtime but preserve its original
    // start revision too. Session must reject a commit against the newer document.
    patch.nodes[1].params.insert("t_bias".into(), 0.7);
    backend.sync_patch(&patch, 2);
    assert!(recall_audio_inspect(&callback).is_err());
    assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    let carried = backend
        .recall_random_capture("recall", "inspect", 0, 2)
        .unwrap();
    assert_eq!(carried["revision"], 2);
    assert_eq!(carried["capture"]["revision"], 1);
    assert_eq!(
        carried["capture"]["capture_id"],
        first["capture"]["capture_id"]
    );
    while retired.pop().is_ok() {}
    backend.retire_modules(&["recall".into()]);
    assert!(backend
        .recall_random_capture("recall", "inspect", 0, 2)
        .is_err());
    backend.sync_patch(&patch, 3);
    assert!(backend
        .recall_random_capture("recall", "inspect", 0, 3)
        .is_err());
    assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    assert!(
        backend
            .recall_random_capture("recall", "inspect", 0, 3)
            .is_err(),
        "recreated identity cannot inherit the removed instance's receipt"
    );
    backend
        .recall_random_capture("recall", "begin", 0, 3)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    let fresh = backend
        .recall_random_capture("recall", "inspect", 0, 3)
        .unwrap();
    assert_ne!(fresh["instance"], first["instance"]);
    assert_eq!(fresh["capture"]["status"], "recording");
    assert_eq!(fresh["capture"]["revision"], 3);
}
#[test]
fn recall_capture_bypass_and_suspended_execution_cannot_queue_runtime_commands() {
    let mut patch = recall_audio_patch();
    let (mut callback, mut updates, _retired) = sequence_callback(&patch);
    let backend = callback.performance.clone();
    for (id, action, slot, revision) in [
        ("missing", "begin", 0, 1),
        ("pitch", "begin", 0, 1),
        ("recall", "begin", 6, 1),
        ("recall", "begin", 0, 0),
    ] {
        assert!(backend
            .recall_random_capture(id, action, slot, revision)
            .is_err());
    }
    assert_eq!(backend.snapshot()["queued"], 0);
    patch.nodes[1].bypassed = true;
    backend.sync_patch(&patch, 2);
    assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    for action in ["begin", "stop", "cancel"] {
        assert!(backend
            .recall_random_capture("recall", action, 0, 2)
            .is_err());
    }
    backend.request_document_execution(2, ExecutionDisposition::SuspendedUnresolved);
    assert!(backend
        .recall_random_capture("recall", "begin", 0, 2)
        .is_err());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    assert_eq!(backend.snapshot()["queued"], 0);
}

#[test]
fn recall_capture_session_commit_cannot_use_previous_complete_while_restart_is_pending() {
    let patch = recall_audio_patch();
    let (mut callback, _, _retired) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    recall_audio_complete(&mut callback);
    let before = session.snapshot();
    session
        .execute(
            "capture_recall_random_scale",
            serde_json::json!({"id":"recall","action":"begin","slot":3,"expected_revision":1}),
        )
        .unwrap();
    let error = session
        .execute(
            "capture_recall_random_scale",
            serde_json::json!({"id":"recall","action":"commit","expected_revision":1}),
        )
        .unwrap_err();
    assert!(error.contains("acknowledgment"));
    assert_eq!(session.snapshot(), before);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    assert!(session
        .execute(
            "capture_recall_random_scale",
            serde_json::json!({"id":"recall","action":"commit"})
        )
        .is_err());
    assert_eq!(session.snapshot(), before);
    session
        .execute(
            "capture_recall_random_scale",
            serde_json::json!({"id":"recall","action":"cancel"}),
        )
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    assert!(session
        .execute(
            "capture_recall_random_scale",
            serde_json::json!({"id":"recall","action":"commit"})
        )
        .is_err());
    let complete = recall_audio_complete(&mut callback);
    session
        .execute(
            "capture_recall_random_scale",
            serde_json::json!({"id":"recall","action":"commit","expected_revision":1}),
        )
        .unwrap();
    let committed = session.snapshot();
    assert_eq!(session.revision(), 2);
    let captured: openpatch_core::RecallRandomCaptureState =
        serde_json::from_value(complete["capture"].clone()).unwrap();
    assert_eq!(
        committed["patch"]["nodes"][1]["content"]["scales"][2],
        serde_json::to_value(captured.scale.to_scale()).unwrap()
    );
    assert!(session
        .execute(
            "capture_recall_random_scale",
            serde_json::json!({"id":"recall","action":"commit"})
        )
        .is_err());
    assert_eq!(session.snapshot(), committed);
}

#[test]
fn recall_capture_expired_or_initialize_cancelled_commands_publish_rejection_instead_of_waiting_forever(
) {
    for cause in ["expired", "initialize"] {
        let patch = recall_audio_patch();
        let (mut callback, mut updates, _retired) = sequence_callback(&patch);
        let completed = recall_audio_complete(&mut callback);
        let backend = callback.performance.clone();
        backend
            .recall_random_capture("recall", "stop", 0, 1)
            .unwrap();
        if cause == "expired" {
            callback.frame_clock += 96001;
        } else {
            backend.initialize_modules(&["recall".into()]);
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
        let error = recall_audio_inspect(&callback).unwrap_err();
        assert!(
            error.contains("rejected"),
            "{cause} must acknowledge the discarded command, not leave it pending: {error}"
        );
        assert_eq!(
            backend.snapshot()[if cause == "expired" {
                "expired"
            } else {
                "rejected"
            }],
            1
        );
        if cause == "initialize" {
            assert!(updates.push(sequence_graph(&callback, &patch, 1)).is_ok());
            without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
            assert!(
                recall_audio_inspect(&callback).is_err(),
                "Initialize clears the old completed recorder receipt"
            );
        }
        backend
            .recall_random_capture("recall", "begin", 3, 1)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
        let restarted = recall_audio_inspect(&callback).unwrap();
        assert_eq!(restarted["capture"]["status"], "recording");
        assert!(
            restarted["capture"]["capture_id"].as_u64().unwrap()
                > completed["capture"]["capture_id"].as_u64().unwrap()
        );
    }
}

#[test]
fn recall_capture_seed_edit_before_callback_rejects_old_begin_and_accepts_fresh_revision() {
    let mut patch = recall_audio_patch();
    let (mut callback, mut updates, _retired) = sequence_callback(&patch);
    let backend = callback.performance.clone();
    backend
        .recall_random_capture("recall", "begin", 2, 1)
        .unwrap();
    patch.nodes[1].params.insert("seed".into(), 17.);
    backend.sync_patch(&patch, 2);
    assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    let status = backend.snapshot();
    assert_eq!(
        status["applied"], 0,
        "a queued old Begin cannot restart the freshly seeded source recorder"
    );
    assert_eq!(status["rejected"], 1);
    assert_eq!(status["pending"], 0);
    assert!(backend
        .recall_random_capture("recall", "inspect", 0, 2)
        .is_err());
    backend
        .recall_random_capture("recall", "begin", 4, 2)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    let fresh = backend
        .recall_random_capture("recall", "inspect", 0, 2)
        .unwrap();
    assert_eq!(fresh["capture"]["status"], "recording");
    assert_eq!(fresh["capture"]["revision"], 2);
    assert_eq!(fresh["capture"]["slot"], 4);
    assert_eq!(backend.snapshot()["applied"], 1);
}
