fn live_parameter_audio_patch() -> Patch {
    serde_json::from_value(serde_json::json!({
        "version":2,"sample_rate":48000,"nodes":[
            {"id":"signal","kind":"poly_constant","params":{"v1":5}},
            {"id":"vca","kind":"stock_vca","params":{"level":1}},
            {"id":"filter","kind":"rack264_vcf","params":{"frequency":0.5}}
        ],"cables":[{"from":"signal","to":"vca","input":"signal"}],
        "output":"vca","output_port":"out"
    }))
    .unwrap()
}

#[test]
fn live_parameter_callback_publication_saves_actual_values_and_reopens_without_history_mutation() {
    let patch = live_parameter_audio_patch();
    let (mut callback, _, _retired) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    let gain = callback
        .graphs
        .active
        .engine
        .resolve_parameter("vca", "level")
        .unwrap();
    let cutoff = callback
        .graphs
        .active
        .engine
        .resolve_parameter("filter", "frequency")
        .unwrap();
    without_heap_activity(|| {
        callback
            .graphs
            .active
            .engine
            .set_parameter_value(gain, 0.25);
        callback
            .graphs
            .active
            .engine
            .set_parameter_value(cutoff, 0.3);
        callback.render(&mut [0.0_f32; 256], 1);
    });
    let before = session.snapshot();
    let published = callback.performance.snapshot();
    assert_eq!(published["modules"]["vca"]["parameters"]["level"], 0.25);
    assert_eq!(
        published["modules"]["filter"]["parameters"]["frequency"]
            .as_f64()
            .unwrap() as f32,
        0.3
    );
    session
        .execute(
            "save_document",
            serde_json::json!({"filename":"effective.json"}),
        )
        .unwrap();
    assert_eq!(session.snapshot(), before);
    let mut reopened = openpatch::control::Session::new(workspace.path()).unwrap();
    reopened
        .execute(
            "load_document",
            serde_json::json!({"filename":"effective.json"}),
        )
        .unwrap();
    let mut engine = reopened.patch().compile().unwrap();
    let gain = engine.resolve_parameter("vca", "level").unwrap();
    let cutoff = engine.resolve_parameter("filter", "frequency").unwrap();
    assert_eq!(engine.parameter_value(gain), Some(0.25));
    assert_eq!(engine.parameter_value(cutoff), Some(0.3));
    let mut frames = [openpatch_core::PortFrame::default(); 2];
    engine.process_poly_block(&mut frames);
    assert_eq!(frames[1].values[0], 1.25);
}

#[test]
fn live_parameter_publication_and_graph_retirement_pressure_use_no_callback_heap() {
    let mut patch = live_parameter_audio_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    for revision in 2..=5 {
        patch.nodes[1]
            .params
            .insert("level".into(), revision as f32 / 10.0);
        callback.performance.sync_patch(&patch, revision);
        let graph = sequence_graph(&callback, &patch, revision);
        assert!(updates.push(graph).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    }
    // Control-side draining releases pressure. All callback-owned boxes survive
    // until retirement succeeds; none may be freed on the rendering thread.
    while retired.pop().is_ok() {}
    for _ in 0..3 {
        without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
        while retired.pop().is_ok() {}
    }
    let snapshot = callback.performance.snapshot();
    assert_eq!(snapshot["modules"]["vca"]["available"], true);
    assert_eq!(snapshot["modules"]["vca"]["revision"], 5);
    assert_eq!(snapshot["modules"]["vca"]["parameters"]["level"], 0.5);
}
