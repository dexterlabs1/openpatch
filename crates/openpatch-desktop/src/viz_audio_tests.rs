// Exercise the real callback and MCP bridge without opening an audio device.
#[test]
fn viz_callback_publishes_all_signed_lanes_coherently_across_graph_changes() {
    let mut params = serde_json::json!({"channels":16});
    for i in 1..=16 { params[format!("v{i}")] = serde_json::json!(if i % 2 == 0 { -10 } else { 10 }); }
    let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
        "nodes":[{"id":"source","kind":"poly_constant","params":params},
                 {"id":"viz","kind":"poly_viz"}],
        "cables":[{"from":"source","to":"viz","input":"signal"}],
        "output":"source"})).unwrap();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch.clone(), Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone())).unwrap();
    assert!(callback.performance.module_lights("viz").is_none());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 15], 1));
    let before = callback.performance.module_lights("viz").unwrap();
    assert_eq!(before.input_channels, Some(0));
    assert_eq!(before.brightness, [[0.0;2];16]);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let first = callback.performance.module_lights("viz").unwrap();
    assert_eq!(first.input_channels, Some(16));
    assert_eq!(first.count, 16);
    for (i, pair) in first.brightness.iter().enumerate() {
        assert_eq!(*pair, if i % 2 == 0 { [1.0, -0.01] } else { [-0.01, 1.0] });
    }
    let observed = session.execute("inspect_performance", serde_json::json!({})).unwrap();
    let module = &observed["performance"]["modules"]["viz"];
    assert_eq!(module["lights"]["input_channels"], 16);
    assert_eq!(module["lights"]["brightness"].as_array().unwrap().len(), 16);
    assert!(module["state"].is_null());
    assert!(module["controls"].is_null());
    session.execute("set_parameter", serde_json::json!({"id":"source","parameter":"channels","value":1})).unwrap();
    assert!(callback.performance.module_lights("viz").is_none());
    without_heap_activity(|| callback.render(&mut [0.0_f32;16],1));
    assert!(callback.performance.module_lights("viz").is_none());
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32;16],1));
    while disposal.pop().is_ok() {}
    let retired = callback.performance.module_lights("viz").unwrap();
    assert_eq!(retired.input_channels, Some(1));
    assert_eq!(retired.brightness[15], [0.0,0.99]);
    assert_eq!(callback.performance.snapshot()["modules"]["viz"]["revision"], session.revision());
    // Complete the 240-frame crossfade before submitting another graph;
    // graph exchange intentionally defers adoption while a transition is live.
    without_heap_activity(|| callback.render(&mut [0.0_f32;224],1));
    while disposal.pop().is_ok() {}
    let settled = callback.performance.module_lights("viz").unwrap();
    session.execute("set_bypass", serde_json::json!({"id":"viz","bypassed":true})).unwrap();
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32;1024],1));
    while disposal.pop().is_ok() {}
    assert_eq!(callback.performance.module_lights("viz").unwrap(), settled);
    session.replace_patch(patch, Some(session.revision())).unwrap();
    assert!(callback.performance.module_lights("viz").is_none());
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32;1],1));
    let fresh = callback.performance.module_lights("viz").unwrap();
    assert_eq!(fresh.input_channels, Some(0));
    assert_eq!(fresh.brightness, [[0.0;2];16]);
    callback.performance.close();
    assert!(callback.performance.module_lights("viz").is_none());
}
