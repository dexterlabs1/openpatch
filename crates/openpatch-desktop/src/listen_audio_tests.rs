// Live worker/callback integration; these tests never open an audio device.
fn listen_audio_patch(connected: bool) -> Patch {
    let mut patch: Patch = serde_json::from_value(serde_json::json!({
        "version":2,"sample_rate":48000,"nodes":[
            {"id":"source","kind":"poly_constant","params":{"channels":2,"v1":4,"v2":6}},
            {"id":"listen","kind":"rackmcp_listen"}
        ],"cables":[],"output":"listen","output_port":"left"
    }))
    .unwrap();
    if connected {
        patch.cables.push(
            serde_json::from_value(serde_json::json!({
                "from":"source","to":"listen","input":"left"
            }))
            .unwrap(),
        );
    }
    patch.assign_missing_cable_ids().unwrap();
    patch
}

fn listen_pump_until(callback: &mut Callback, job: &str, terminal: bool) -> serde_json::Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    loop {
        without_heap_activity(|| callback.render(&mut [0.0_f32; 512], 1));
        let snapshot = callback.performance.inspect_listen_capture(job).unwrap();
        let finished = matches!(snapshot["state"].as_str(), Some("done" | "failed"));
        if (terminal && finished)
            || (!terminal
                && callback.performance.snapshot()["modules"]["listen"]["listen"]["capturing"]
                    == true)
        {
            return snapshot;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Listen did not progress: {snapshot}"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn listen_callback_publishes_real_source_cadence_and_bypass_freezes_observation() {
    let mut patch = listen_audio_patch(true);
    let (mut callback, mut updates, _retired) = sequence_callback(&patch);
    assert!(callback.performance.snapshot()["modules"]["listen"]["listen"].is_null());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 255], 1));
    let before = callback.performance.snapshot();
    assert_eq!(before["modules"]["listen"]["listen"]["frames_seen"], 255);
    assert_eq!(
        before["modules"]["listen"]["listen"]["left_connected"],
        false
    );
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let published = callback.performance.snapshot();
    assert_eq!(published["modules"]["listen"]["listen"]["frames_seen"], 256);
    assert_eq!(
        published["modules"]["listen"]["listen"]["left_connected"],
        true
    );
    assert_eq!(
        published["modules"]["listen"]["listen"]["right_connected"],
        false
    );
    assert_eq!(published["modules"]["listen"]["listen"]["capturing"], false);
    patch.nodes[1].bypassed = true;
    callback.performance.sync_patch(&patch, 2);
    assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 512], 1));
    assert_eq!(
        callback.performance.snapshot()["modules"]["listen"]["listen"],
        published["modules"]["listen"]["listen"]
    );
    let raw = read_raw_pair(
        &mut callback.graphs.active.engine,
        ("listen", "left"),
        ("listen", "right"),
    );
    assert_eq!(raw, [4.0, 0.0]);
}

#[test]
fn listen_live_worker_counts_silence_and_exports_polyphonic_sum_at_source_full_scale() {
    for connected in [false, true] {
        let patch = listen_audio_patch(connected);
        let (mut callback, _, _retired) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        assert!(callback
            .performance
            .start_listen_capture("listen", 0.1, false, workspace.path())
            .is_err());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        // Speaker mute has no effect on the raw module input capture.
        callback.shared.playing.store(false, Ordering::Relaxed);
        let started = callback
            .performance
            .start_listen_capture("listen", 0.10001, true, workspace.path())
            .unwrap();
        assert_eq!(started["target_frames"], 4801);
        assert!(callback
            .performance
            .start_listen_capture("listen", 0.1, false, workspace.path())
            .is_err());
        let result = listen_pump_until(&mut callback, started["job"].as_str().unwrap(), true);
        assert_eq!(result["state"], "done", "{result}");
        assert_eq!(result["collected_frames"], 4801);
        assert_eq!(result["result"]["analysis"]["frames"], 4801);
        assert_eq!(result["result"]["left_connected"], connected);
        assert_eq!(result["result"]["right_connected"], false);
        assert_eq!(
            result["result"]["analysis"]["channels"][0]["peakVolts"],
            if connected { 10.0 } else { 0.0 }
        );
        let file = result["result"]["wav_file"].as_str().unwrap();
        let bytes = std::fs::read(workspace.path().join(file)).unwrap();
        assert_eq!(bytes.len(), 58 + 4801 * 8);
        let sample = |offset| f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        assert_eq!(sample(58), if connected { 1.0 } else { 0.0 });
        assert_eq!(sample(62), sample(58));
        // Export mono duplication never changes the independent R output.
        assert_eq!(
            read_raw_pair(
                &mut callback.graphs.active.engine,
                ("listen", "left"),
                ("listen", "right")
            ),
            [if connected { 4.0 } else { 0.0 }, 0.0]
        );
        callback.performance.close();
    }
}

#[test]
fn listen_live_capture_survives_prepared_skipped_edits_rate_changes_and_crossfade() {
    let patch = listen_audio_patch(true);
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    callback.performance.sync_patch(&patch, 2);
    let skipped = sequence_graph(&callback, &patch, 2);
    callback.performance.sync_patch(&patch, 3);
    let prepared = sequence_graph(&callback, &patch, 3);
    assert!(updates.push(prepared).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 512], 1));
    while retired.pop().is_ok() {}
    let start = callback
        .performance
        .start_listen_capture("listen", 0.1, false, workspace.path())
        .unwrap();
    let job = start["job"].as_str().unwrap();
    listen_pump_until(&mut callback, job, false);
    callback.performance.sync_patch(&patch, 4);
    let mut rate_graph = prepare_graph(&patch, 96000, None, 4, true).unwrap();
    rate_graph.modules = callback
        .performance
        .plan(&patch, &rate_graph.engine, 4)
        .unwrap();
    rate_graph.revision = 4;
    // Keep the donor around long enough to observe its unarmed source lamp.
    callback.fade_length = 100_000;
    assert!(updates.push(rate_graph).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 512], 1));
    let active = callback
        .graphs
        .active
        .engine
        .resolve_module_lights("listen")
        .unwrap();
    let donor = callback.graphs.previous.as_ref().unwrap();
    let old = donor.engine.resolve_module_lights("listen").unwrap();
    assert!(
        callback
            .graphs
            .active
            .engine
            .listen_status(active)
            .unwrap()
            .capturing
    );
    assert!(!donor.engine.listen_status(old).unwrap().capturing);
    let result = listen_pump_until(&mut callback, job, true);
    assert_eq!(result["state"], "done", "{result}");
    assert_eq!(result["sample_rate"], 48000);
    assert_eq!(result["target_frames"], 4800);
    assert_eq!(result["collected_frames"], 4800);
    // An old prepared Listen graph cannot lower the callback high-water mark.
    callback.fade_length = 1;
    without_heap_activity(|| callback.render(&mut [0.0_f32; 512], 1));
    while retired.pop().is_ok() {}
    assert!(updates.push(skipped).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 512], 1));
    assert_eq!(callback.graphs.active.revision, 4);
    let next = callback
        .performance
        .start_listen_capture("listen", 0.1, false, workspace.path())
        .unwrap();
    assert_eq!(next["sample_rate"], 96000);
    assert_eq!(next["target_frames"], 9600);
    callback
        .performance
        .cancel_listen_capture(next["job"].as_str().unwrap())
        .unwrap();
    callback.performance.close();
}

#[test]
fn listen_retire_recreate_suspend_and_close_never_revive_capture() {
    for transition in ["retire", "suspend", "close"] {
        let patch = listen_audio_patch(true);
        let (mut callback, _, _retired) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        let start = callback
            .performance
            .start_listen_capture("listen", 1.0, false, workspace.path())
            .unwrap();
        let job = start["job"].as_str().unwrap();
        listen_pump_until(&mut callback, job, false);
        let before = callback.performance.snapshot()["modules"]["listen"]["instance"].clone();
        match transition {
            "retire" => {
                callback.performance.retire_modules(&["listen".into()]);
                callback.performance.sync_patch(&patch, 2);
                assert_ne!(
                    callback.performance.snapshot()["modules"]["listen"]["instance"],
                    before
                );
                // The old graph is still active, but its retired token is fenced.
            }
            "suspend" => callback
                .performance
                .request_document_execution(2, ExecutionDisposition::SuspendedUnresolved),
            _ => callback.performance.close(),
        }
        let result = listen_pump_until(&mut callback, job, true);
        assert_eq!(result["state"], "failed", "{transition}: {result}");
        assert!(result["collected_frames"].as_u64().unwrap() < 48000);
        assert!(callback
            .performance
            .start_listen_capture("listen", 0.1, false, workspace.path())
            .is_err());
        callback.performance.close();
    }
}

#[test]
fn listen_scope_and_capture_share_the_actual_callback_traversal() {
    let patch = listen_audio_patch(true);
    let (mut callback, _, _retired) = sequence_callback(&patch);
    let left = callback
        .graphs
        .active
        .engine
        .resolve_tap("listen", "left", 0)
        .unwrap();
    let right = callback
        .graphs
        .active
        .engine
        .resolve_tap("listen", "right", 0)
        .unwrap();
    callback.graphs.active.scope = ScopeSetup {
        generation: 1,
        enabled: true,
        taps: [Some(left), Some(right)],
    };
    let workspace = tempfile::tempdir().unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    let start = callback
        .performance
        .start_listen_capture("listen", 0.1, false, workspace.path())
        .unwrap();
    let result = listen_pump_until(&mut callback, start["job"].as_str().unwrap(), true);
    assert_eq!(result["state"], "done", "{result}");
    assert_eq!(
        result["result"]["analysis"]["channels"][0]["peakVolts"],
        10.0
    );
    let packets = callback.scope_packets.as_mut().unwrap();
    let packet = packets
        .pop()
        .expect("Scope must continue receiving actual taps");
    assert!(packet.x_active.iter().all(|value| *value));
    assert!(packet.x.contains(&4.0));
    assert!(packet.y.iter().all(|value| *value == 0.0));
    callback.performance.close();
}
