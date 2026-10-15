// Integrated callback/control tests: no hardware stream or GUI is opened.
#[test]
fn sum_meter_metadata_tracks_actual_graph_revision_and_instance_without_heap_work() {
    let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
        "nodes":[{"id":"source","kind":"poly_constant","params":{"channels":2,"v1":8,"v2":8}},
                 {"id":"sum","kind":"poly_sum","params":{"level":0.5}}],
        "cables":[{"from":"source","to":"sum","input":"signal"}],
        "output":"sum"})).unwrap();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch.clone(), Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone())).unwrap();
    assert!(callback.performance.module_lights("sum").is_none());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 511], 1));
    let initial = callback.performance.module_lights("sum").unwrap();
    assert_eq!(initial.input_channels, Some(0));
    assert_eq!(initial.brightness, [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let original = callback.performance.module_lights("sum").unwrap();
    assert_eq!(original.input_channels, Some(2));
    assert_eq!(original.count, 6);
    assert_eq!(original.brightness[0], [0.0; 2]);
    assert!((original.brightness[1][0] - (20.0_f32 * 0.8_f32.log10() + 3.0) / 3.0).abs() < 1e-5);
    let observed = session.execute("inspect_performance", serde_json::json!({})).unwrap();
    assert_eq!(observed["performance"]["modules"]["sum"]["lights"]["input_channels"], 2);
    assert!(observed["performance"]["modules"]["sum"]["state"].is_null());
    assert!(observed["performance"]["modules"]["sum"]["controls"].is_null());

    session.execute("set_parameter", serde_json::json!({"id":"source","parameter":"channels","value":16})).unwrap();
    assert!(callback.performance.module_lights("sum").is_none());
    // A completed callback on the superseded graph must remain unavailable.
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert!(callback.performance.module_lights("sum").is_none());
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1024], 1));
    while disposal.pop().is_ok() {}
    let expanded = callback.performance.module_lights("sum").unwrap();
    assert_eq!(expanded.input_channels, Some(16));
    assert_eq!(expanded.brightness[0][0], 1.0);
    assert_eq!(callback.performance.snapshot()["modules"]["sum"]["revision"], session.revision());

    session.execute("set_bypass", serde_json::json!({"id":"sum","bypassed":true})).unwrap();
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1024], 1));
    while disposal.pop().is_ok() {}
    assert_eq!(callback.performance.module_lights("sum").unwrap(), expanded);
    assert_eq!(callback.performance.snapshot()["modules"]["sum"]["bypassed"], true);

    // Reopening the document clears meter history, even with identical IDs.
    session.replace_patch(patch, Some(session.revision())).unwrap();
    assert!(callback.performance.module_lights("sum").is_none());
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let fresh = callback.performance.module_lights("sum").unwrap();
    assert_eq!(fresh.input_channels, Some(0));
    assert_eq!(fresh.brightness, [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]);
    callback.performance.close();
    assert!(callback.performance.module_lights("sum").is_none());
}
