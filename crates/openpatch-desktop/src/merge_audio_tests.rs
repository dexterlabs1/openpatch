// Real callback and Session bridge; no hardware stream or GUI is opened.
fn merge_transport_patch() -> Patch {
    let mut patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
        "nodes":[
            {"id":"low","kind":"poly_constant","params":{"channels":2,"v1":3,"v2":-9}},
            {"id":"silent","kind":"poly_constant","params":{"channels":1,"v1":0}},
            {"id":"empty","kind":"poly_constant","params":{"channels":0}},
            {"id":"merge","kind":"poly_merge"}],
        "cables":[{"from":"low","to":"merge","input":"in1"},
            {"from":"silent","to":"merge","input":"in5"},
            {"from":"empty","to":"merge","input":"in16"}],
        // Merge's output is deliberately unpatched, including the device output.
        "output":"low"}))
    .unwrap();
    patch.assign_missing_cable_ids().unwrap();
    patch
}

fn merge_raw_lanes(engine: &mut Engine) -> [TapSample; 16] {
    let taps =
        std::array::from_fn(|channel| Some(engine.resolve_tap("merge", "out", channel).unwrap()));
    let mut samples = [[TapSample::default(); 16]; 1];
    without_heap_activity(|| {
        engine.process_audio_block_with_taps(&mut [[0.0; 2]; 1], &taps, &mut samples)
    });
    samples[0]
}

fn assert_merge_counts(lights: openpatch_core::ModuleLightState, output: u8, automatic: u8) {
    assert_eq!(lights.output_channels, Some(output));
    assert_eq!(lights.automatic_channels, Some(automatic));
    assert_eq!(lights.count, 0);
    assert!(!lights.manual);
    assert_eq!(
        lights.brightness,
        [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]
    );
    assert_eq!(lights.input_channels, None);
    assert_eq!(lights.waveform_position, None);
}

#[test]
fn merge_callback_publishes_real_sparse_zero_and_forced_counts_with_stale_graphs_unavailable() {
    let patch = merge_transport_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    assert!(callback.performance.module_lights("merge").is_none());
    assert!(callback
        .performance
        .trigger("merge", ModuleAction::BPress)
        .is_err());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    // Edges have one sample of delay, so the first actual input counts are zero.
    assert_merge_counts(callback.performance.module_lights("merge").unwrap(), 0, 0);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert_merge_counts(callback.performance.module_lights("merge").unwrap(), 5, 5);
    let raw = merge_raw_lanes(&mut callback.graphs.active.engine);
    for (channel, lane) in raw.iter().enumerate() {
        assert_eq!(lane.active, channel < 5);
        assert_eq!(lane.value, if channel == 0 { 3.0 } else { 0.0 });
    }
    let unchanged = session.snapshot();
    let inspected = session
        .execute("inspect_performance", serde_json::json!({}))
        .unwrap();
    let module = &inspected["performance"]["modules"]["merge"];
    assert_eq!(module["kind"], "poly_merge");
    assert_eq!(module["lights"]["output_channels"], 5);
    assert_eq!(module["lights"]["automatic_channels"], 5);
    assert_eq!(module["lights"]["count"], 0);
    assert!(module["state"].is_null());
    assert!(module["controls"].is_null());
    assert_eq!(session.snapshot(), unchanged);

    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"merge","parameter":"channels","value":2}),
        )
        .unwrap();
    let stale_pending = sequence_graph(&callback, session.patch(), session.revision());
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"merge","parameter":"channels","value":16}),
        )
        .unwrap();
    assert!(callback.performance.module_lights("merge").is_none());
    // A callback on the old graph and an adopted but superseded pending graph
    // both remain unavailable at the control revision.
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert!(callback.performance.module_lights("merge").is_none());
    assert!(updates.push(stale_pending).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    assert!(callback.performance.module_lights("merge").is_none());
    let unavailable = session
        .execute("inspect_performance", serde_json::json!({}))
        .unwrap();
    assert_eq!(
        unavailable["performance"]["modules"]["merge"]["available"],
        false
    );
    assert!(unavailable["performance"]["modules"]["merge"]["lights"].is_null());
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    // The retired two-channel graph also renders this crossfade callback.
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert_merge_counts(callback.performance.module_lights("merge").unwrap(), 16, 5);
    let published = callback.performance.snapshot();
    assert_eq!(
        published["modules"]["merge"]["revision"],
        session.revision()
    );
    let raw = merge_raw_lanes(&mut callback.graphs.active.engine);
    assert!(raw.iter().all(|lane| lane.active));
    assert_eq!(raw[0].value, 3.0);
    assert!(raw[1..].iter().all(|lane| lane.value == 0.0));
    without_heap_activity(|| callback.render(&mut [0.0_f32; 255], 1));
    while disposal.pop().is_ok() {}

    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"merge","parameter":"channels","value":-1}),
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
    assert_merge_counts(callback.performance.module_lights("merge").unwrap(), 0, 5);
    assert!(merge_raw_lanes(&mut callback.graphs.active.engine)
        .iter()
        .all(|lane| !lane.active && lane.value == 0.0));
    let forced_zero = session
        .execute("inspect_performance", serde_json::json!({}))
        .unwrap();
    assert_eq!(
        forced_zero["performance"]["modules"]["merge"]["available"],
        true
    );
    assert_eq!(
        forced_zero["performance"]["modules"]["merge"]["lights"]["output_channels"],
        0
    );
    assert_eq!(
        forced_zero["performance"]["modules"]["merge"]["lights"]["automatic_channels"],
        5
    );
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"merge","parameter":"channels","value":0}),
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
    assert_merge_counts(callback.performance.module_lights("merge").unwrap(), 5, 5);
}

#[test]
fn merge_callback_adoption_bypass_fresh_document_and_close_preserve_observation_identity() {
    let patch = merge_transport_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch.clone(), Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2], 1));
    let original = callback.performance.module_lights("merge").unwrap();
    assert_merge_counts(original, 5, 5);
    session
        .execute(
            "set_bypass",
            serde_json::json!({"id":"merge","bypassed":true}),
        )
        .unwrap();
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"silent","parameter":"channels","value":0}),
        )
        .unwrap();
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"merge","parameter":"channels","value":-1}),
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
    // Compatible adoption retains observations while the receiving override
    // waits for its first processed (unbypassed) frame.
    assert_eq!(
        callback.performance.module_lights("merge").unwrap(),
        original
    );
    assert_eq!(
        callback.performance.snapshot()["modules"]["merge"]["bypassed"],
        true
    );
    session
        .execute(
            "set_bypass",
            serde_json::json!({"id":"merge","bypassed":false}),
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
    assert_merge_counts(callback.performance.module_lights("merge").unwrap(), 0, 1);

    // An identical ID in a newly opened document is a new instance. A bypassed
    // fresh graph makes its initial zero counts observable without processing.
    let old_generation = sequence_graph(&callback, session.patch(), session.revision());
    let mut reopened = patch;
    reopened
        .nodes
        .iter_mut()
        .find(|node| node.id == "merge")
        .unwrap()
        .bypassed = true;
    session
        .replace_patch(reopened, Some(session.revision()))
        .unwrap();
    assert!(callback.performance.module_lights("merge").is_none());
    assert!(updates.push(old_generation).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    assert!(callback.performance.module_lights("merge").is_none());
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert_merge_counts(callback.performance.module_lights("merge").unwrap(), 0, 0);
    assert_eq!(
        callback.performance.snapshot()["modules"]["merge"]["revision"],
        session.revision()
    );
    callback.performance.close();
    assert!(callback.performance.module_lights("merge").is_none());
    let closed = session
        .execute("inspect_performance", serde_json::json!({}))
        .unwrap();
    assert_eq!(
        closed["performance"]["modules"]["merge"]["available"],
        false
    );
    assert!(closed["performance"]["modules"]["merge"]["lights"].is_null());
}
