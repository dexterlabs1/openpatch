fn stock_quantizer_transport_patch() -> Patch {
    serde_json::from_value(serde_json::json!({"version":2,"nodes":[{"id":"source","kind":"poly_constant","params":{"channels":4,"v1":0.0,"v2":0.25,"v3":0.5,"v4":0.75}},{"id":"q","kind":"stock_quantizer"}],"cables":[{"from":"source","to":"q","input":"pitch"}],"output":"q","output_port":"pitch"})).unwrap()
}
#[test]
fn stock_quantizer_callback_publishes_actual_bank_and_played_union_without_lamps_or_heap_activity()
{
    let patch = stock_quantizer_transport_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch.clone(), Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    assert!(callback.performance.module_lights("q").is_none());
    assert!(callback
        .performance
        .trigger("q", ModuleAction::Clock)
        .is_err());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 0], 1));
    let initial = callback.performance.module_lights("q").unwrap();
    assert_eq!(initial.playing_notes, None);
    assert_eq!(initial.count, 0);
    assert_eq!(
        initial.brightness,
        [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]
    );
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    // Cable frames publish only after every node finishes the sample. The first
    // real sample therefore quantizes the initially empty input to chromatic C;
    // the next sample observes all four exact source voltages.
    assert_eq!(
        callback
            .performance
            .module_lights("q")
            .unwrap()
            .playing_notes,
        Some(1)
    );
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let played = callback.performance.module_lights("q").unwrap();
    assert_eq!(played.playing_notes, Some(1 | 8 | 64 | 512));
    assert_eq!(played.count, 0);
    assert_eq!(
        played.brightness,
        [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]
    );
    assert!(callback.performance.module_control_state("q").is_none());
    assert!(callback.performance.sequence_state("q").is_none());
    let before = session.snapshot();
    let observed = session
        .execute("inspect_performance", serde_json::json!({}))
        .unwrap();
    let q = &observed["performance"]["modules"]["q"];
    assert_eq!(q["state"]["scale_mask"], 4095);
    assert_eq!(
        q["state"]["enabled_notes"],
        serde_json::json!(vec![true; 12])
    );
    assert_eq!(q["lights"]["playing_notes"], 585);
    assert_eq!(q["lights"]["count"], 0);
    assert_eq!(q["lights"]["brightness"], serde_json::json!([]));
    assert_eq!(session.snapshot(), before);
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"q","parameter":"scale_mask","value":1}),
        )
        .unwrap();
    assert!(callback.performance.module_lights("q").is_none());
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    let observed = callback.performance.snapshot();
    assert_eq!(observed["modules"]["q"]["state"]["scale_mask"], 1);
    assert_eq!(observed["modules"]["q"]["lights"]["playing_notes"], 1);
    assert_eq!(observed["modules"]["q"]["revision"], session.revision());
    session
        .execute("set_bypass", serde_json::json!({"id":"q","bypassed":true}))
        .unwrap();
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    let frozen = callback.performance.module_lights("q").unwrap();
    assert_eq!(frozen.playing_notes, Some(1));
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"q","parameter":"scale_mask","value":4095}),
        )
        .unwrap();
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    assert_eq!(
        callback.performance.module_lights("q").unwrap(),
        frozen,
        "Bypassed bank edits cannot fabricate a new output observation"
    );
    assert_eq!(
        callback.performance.snapshot()["modules"]["q"]["state"]["scale_mask"],
        4095
    );
    session
        .execute("set_bypass", serde_json::json!({"id":"q","bypassed":false}))
        .unwrap();
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    assert_eq!(
        callback
            .performance
            .module_lights("q")
            .unwrap()
            .playing_notes,
        Some(585)
    );
    let mut fresh = patch;
    fresh
        .nodes
        .iter_mut()
        .find(|node| node.id == "q")
        .unwrap()
        .bypassed = true;
    session
        .replace_patch(fresh, Some(session.revision()))
        .unwrap();
    assert!(callback.performance.module_lights("q").is_none());
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    assert_eq!(
        callback
            .performance
            .module_lights("q")
            .unwrap()
            .playing_notes,
        None,
        "Fresh bypassed identity has no measured playing notes"
    );
    callback.performance.close();
    assert!(callback.performance.module_lights("q").is_none());
    let (mut replacement, _, _) = sequence_callback(&stock_quantizer_transport_patch());
    assert!(replacement.performance.module_lights("q").is_none());
    without_heap_activity(|| replacement.render(&mut [0.0_f32; 1], 1));
    assert_eq!(
        replacement
            .performance
            .module_lights("q")
            .unwrap()
            .playing_notes,
        Some(1)
    );
    without_heap_activity(|| replacement.render(&mut [0.0_f32; 1], 1));
    assert_eq!(
        replacement
            .performance
            .module_lights("q")
            .unwrap()
            .playing_notes,
        Some(585)
    );
    assert!(callback.performance.module_lights("q").is_none());
}
#[test]
fn stock_quantizer_unpatched_and_empty_bank_are_actual_chromatic_c_not_unavailable_or_silent() {
    for mask in [0, 4095] {
        let patch:Patch=serde_json::from_value(serde_json::json!({"version":2,"nodes":[{"id":"q","kind":"stock_quantizer","params":{"scale_mask":mask}}],"output":"q","output_port":"pitch"})).unwrap();
        let (mut callback, _, _) = sequence_callback(&patch);
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let snapshot = callback.performance.snapshot();
        let q = &snapshot["modules"]["q"];
        assert_eq!(q["state"]["scale_mask"], mask);
        assert_eq!(q["lights"]["playing_notes"], 1);
        assert_eq!(q["lights"]["brightness"], serde_json::json!([]));
    }
}

#[test]
fn stock_quantizer_acknowledged_paint_commits_reach_callback_as_one_undoable_bank() {
    let patch = stock_quantizer_transport_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let undo_before = session.snapshot()["history"]["undo_steps"]
        .as_u64()
        .unwrap();
    let mut accepted_revision = session.revision();
    let mut intermediate = None;
    // Only the first commit can capture the published original bank. Subsequent
    // commits must use this gesture's acknowledged full bank, without a callback
    // or a prepared-graph publication making their new revisions available.
    for mask in [2, 8, 16] {
        let receipt = session
            .execute(
                "set_parameter",
                serde_json::json!({"id":"q","parameter":"scale_mask","value":mask,
                    "history_group":"actual-callback-paint","expected_revision":accepted_revision}),
            )
            .unwrap();
        let revision = receipt["revision"].as_u64().unwrap();
        assert_ne!(revision, accepted_revision);
        assert_eq!(revision, session.revision());
        accepted_revision = revision;
        assert!(callback.performance.module_lights("q").is_none());
        if intermediate.is_none() {
            intermediate = Some(sequence_graph(&callback, session.patch(), revision));
        }
    }
    assert_eq!(session.snapshot()["history"]["undo_steps"], undo_before + 1);
    let latest = sequence_graph(&callback, session.patch(), accepted_revision);

    // A graph prepared for the first bank may arrive before the latest graph.
    // It must not consume the newer saved generation or expose stale telemetry.
    assert!(updates.push(intermediate.unwrap()).is_ok());
    let mut output = [0.0_f32; 256];
    without_heap_activity(|| callback.render(&mut output, 1));
    while disposal.pop().is_ok() {}
    let handle = callback
        .graphs
        .active
        .engine
        .resolve_module_state("q")
        .unwrap();
    assert_eq!(
        callback.graphs.active.engine.module_saved_state(handle),
        Some(openpatch_core::SavedModuleState::StockQuantizerNotes(2))
    );
    assert!(output[240..].iter().all(|sample| *sample == 1.0_f32 / 12.0));
    assert!(callback.performance.module_lights("q").is_none());

    assert!(updates.push(latest).is_ok());
    without_heap_activity(|| callback.render(&mut output, 1));
    while disposal.pop().is_ok() {}
    let assert_bank = |callback: &Callback, revision, mask: u16, playing| {
        let snapshot = callback.performance.snapshot();
        let q = &snapshot["modules"]["q"];
        assert_eq!(q["revision"], revision);
        assert_eq!(q["frame"], snapshot["frame"]);
        assert_eq!(q["state"]["scale_mask"], mask);
        assert_eq!(
            q["state"]["enabled_notes"],
            serde_json::json!(std::array::from_fn::<_, 12, _>(
                |note| mask & (1 << note) != 0
            ))
        );
        assert_eq!(q["lights"]["playing_notes"], playing);
        let handle = callback
            .graphs
            .active
            .engine
            .resolve_module_state("q")
            .unwrap();
        assert_eq!(
            callback.graphs.active.engine.module_saved_state(handle),
            Some(openpatch_core::SavedModuleState::StockQuantizerNotes(mask))
        );
    };
    assert_bank(&callback, accepted_revision, 16, 16);
    assert!(output[240..].iter().all(|sample| *sample == 4.0_f32 / 12.0));

    // One undo restores the original bank, not either intermediate bank; one
    // redo restores the final bank through the same real graph-adoption path.
    for (tool, mask, playing, pitch) in [
        ("undo", 4095, 585, 0.0_f32),
        ("redo", 16, 16, 4.0_f32 / 12.0),
    ] {
        let receipt = session
            .execute(
                tool,
                serde_json::json!({"expected_revision":accepted_revision}),
            )
            .unwrap();
        accepted_revision = receipt["revision"].as_u64().unwrap();
        assert_eq!(accepted_revision, session.revision());
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                accepted_revision
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut output, 1));
        while disposal.pop().is_ok() {}
        assert_bank(&callback, accepted_revision, mask, playing);
        assert!(output[240..].iter().all(|sample| *sample == pitch));
        assert_eq!(
            receipt["history"]["undo_steps"],
            undo_before + u64::from(tool == "redo")
        );
        assert_eq!(receipt["history"]["redo_steps"], u64::from(tool == "undo"));
    }
}
