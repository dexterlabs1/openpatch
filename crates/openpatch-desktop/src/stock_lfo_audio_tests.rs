fn stock_lfo_transport_patch() -> Patch {
    serde_json::from_value(serde_json::json!({"version":2,"nodes":[{"id":"lfo","kind":"stock_lfo"}],"output":"lfo","output_port":"sine"})).unwrap()
}
#[test]
fn stock_lfo_callback_publishes_actual_clock_and_divided_lamps_without_heap_activity() {
    let patch = stock_lfo_transport_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch.clone(), Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone())).unwrap();
    assert!(callback.performance.module_lights("lfo").is_none());
    assert!(callback.performance.trigger("lfo", ModuleAction::BPress).is_err());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 15], 1));
    let pending = callback.performance.module_lights("lfo").unwrap();
    assert_eq!(pending.clock_frequency, Some(2.0));
    assert_eq!(pending.count, 5);
    assert_eq!(pending.brightness, [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let first = callback.performance.module_lights("lfo").unwrap();
    assert_eq!(first.clock_frequency, Some(2.0));
    assert_eq!(first.brightness[3][0], 0.0);
    assert_eq!(first.brightness[4][0], 1.0);
    assert!(!first.manual);
    assert!(callback.performance.module_control_state("lfo").is_none());
    assert!(callback.performance.sequence_state("lfo").is_none());
    let before = session.snapshot();
    let observed = session.execute("inspect_performance", serde_json::json!({})).unwrap();
    assert_eq!(observed["performance"]["modules"]["lfo"]["lights"]["clock_frequency"], 2.0);
    assert_eq!(session.snapshot(), before);
    session.execute("set_parameter", serde_json::json!({"id":"lfo","parameter":"invert","value":1})).unwrap();
    assert!(callback.performance.module_lights("lfo").is_none());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert!(callback.performance.module_lights("lfo").is_none());
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    let inverted = callback.performance.module_lights("lfo").unwrap();
    assert_eq!(inverted.clock_frequency, Some(2.0));
    assert_eq!(inverted.brightness[3][0], 1.0);
    session.execute("set_bypass", serde_json::json!({"id":"lfo","bypassed":true})).unwrap();
    session.execute("set_parameter", serde_json::json!({"id":"lfo","parameter":"invert","value":0})).unwrap();
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    assert_eq!(callback.performance.module_lights("lfo").unwrap(), inverted);
    let mut fresh = patch;
    fresh.nodes[0].bypassed = true;
    session.replace_patch(fresh, Some(session.revision())).unwrap();
    assert!(callback.performance.module_lights("lfo").is_none());
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let fresh = callback.performance.module_lights("lfo").unwrap();
    assert_eq!(fresh.clock_frequency, Some(1.0));
    assert_eq!(fresh.brightness, [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]);
    callback.performance.close();
    assert!(callback.performance.module_lights("lfo").is_none());
    let (mut replacement, _, _) = sequence_callback(&stock_lfo_transport_patch());
    assert!(replacement.performance.module_lights("lfo").is_none());
    without_heap_activity(|| replacement.render(&mut [0.0_f32; 16], 1));
    assert_eq!(replacement.performance.module_lights("lfo").unwrap().clock_frequency, Some(2.0));
    assert!(callback.performance.module_lights("lfo").is_none());
}
#[test]
fn stock_lfo_callback_clock_metadata_follows_actual_edges_instead_of_cable_presence() {
    let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,"nodes":[
        {"id":"clock","kind":"multi_lfo","params":{"frequency":8,"unipolar":1}},
        {"id":"lfo","kind":"stock_lfo"}],
        "cables":[{"from":"clock","output":"square","to":"lfo","input":"clock"}],
        "output":"lfo","output_port":"sine"})).unwrap();
    let (mut callback, _, _) = sequence_callback(&patch);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 16000], 1));
    let actual = callback.performance.module_lights("lfo").unwrap().clock_frequency.unwrap();
    assert!((actual - 8.0).abs() < 0.1, "Expected actual 8 Hz clock, got {actual}");
}
