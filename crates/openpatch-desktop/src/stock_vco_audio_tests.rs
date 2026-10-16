fn stock_vco_transport_patch() -> Patch {
    serde_json::from_value(serde_json::json!({"version":2,"nodes":[{"id":"vco","kind":"stock_vco"}],"output":"vco","output_port":"sine"})).unwrap()
}
#[test]
fn stock_vco_callback_publishes_actual_lamps_and_guards_revision_bypass_and_backend_without_heap_work() {
    let patch = stock_vco_transport_patch(); let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap(); let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch.clone(), Some(0)).unwrap(); session.set_live_module_events(Some(callback.performance.clone())).unwrap();
    assert!(callback.performance.module_lights("vco").is_none()); assert!(callback.performance.trigger("vco", ModuleAction::BPress).is_err());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 15], 1));
    let initial = callback.performance.module_lights("vco").unwrap(); assert_eq!(initial.count, 5); assert_eq!(initial.brightness, [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]);
    assert_eq!(initial.clock_frequency, None);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let actual = callback.performance.module_lights("vco").unwrap();
    assert!(actual.brightness[..3].iter().any(|pair| pair[0] > 0.0)); assert_eq!([actual.brightness[3][0], actual.brightness[4][0]], [0.0, 0.0]);
    assert!(callback.performance.module_control_state("vco").is_none()); assert!(callback.performance.sequence_state("vco").is_none());
    let before = session.snapshot(); let inspected = session.execute("inspect_performance", serde_json::json!({})).unwrap();
    let published = &inspected["performance"]["modules"]["vco"]; assert_eq!(published["kind"], "stock_vco"); assert_eq!(published["lights"]["count"], 5);
    assert!(published["lights"]["clock_frequency"].is_null()); assert_eq!(session.snapshot(), before);
    for name in ["linear_fm", "soft_sync"] { session.execute("set_parameter", serde_json::json!({"id":"vco","parameter":name,"value":1})).unwrap(); }
    assert!(callback.performance.module_lights("vco").is_none()); without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1)); assert!(callback.performance.module_lights("vco").is_none());
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1)); while disposal.pop().is_ok() {}
    let modes = callback.performance.module_lights("vco").unwrap(); assert_eq!([modes.brightness[3][0], modes.brightness[4][0]], [1.0, 1.0]);
    assert_eq!(callback.performance.snapshot()["modules"]["vco"]["revision"], session.revision());
    session.execute("set_bypass", serde_json::json!({"id":"vco","bypassed":true})).unwrap();
    for name in ["linear_fm", "soft_sync"] { session.execute("set_parameter", serde_json::json!({"id":"vco","parameter":name,"value":0})).unwrap(); }
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok()); without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1)); while disposal.pop().is_ok() {}
    assert_eq!(callback.performance.module_lights("vco").unwrap(), modes);
    let mut fresh = patch; fresh.nodes[0].bypassed = true; session.replace_patch(fresh, Some(session.revision())).unwrap();
    assert!(callback.performance.module_lights("vco").is_none()); assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1)); let fresh = callback.performance.module_lights("vco").unwrap();
    assert_eq!(fresh.brightness, [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]); assert_eq!(fresh.clock_frequency, None);
    callback.performance.close(); assert!(callback.performance.module_lights("vco").is_none());
    let (mut replacement, _, _) = sequence_callback(&stock_vco_transport_patch()); assert!(replacement.performance.module_lights("vco").is_none());
    without_heap_activity(|| replacement.render(&mut [0.0_f32; 16], 1)); assert_eq!(replacement.performance.module_lights("vco").unwrap().count, 5);
    assert!(callback.performance.module_lights("vco").is_none());
}
#[test]
fn stock_vco_callback_polyphony_sets_actual_blue_phase_lamp_without_invented_clock_state() {
    let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,"nodes":[{"id":"source","kind":"poly_constant","params":{"channels":4}},
        {"id":"vco","kind":"stock_vco"}],"cables":[{"from":"source","to":"vco","input":"pitch"}],"output":"vco","output_port":"sine"})).unwrap();
    let (mut callback, _, _) = sequence_callback(&patch);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 32], 1));
    let actual = callback.performance.module_lights("vco").unwrap(); assert_eq!(actual.count, 5); assert_eq!(actual.brightness[2][0], 1.0); assert_eq!(actual.clock_frequency, None);
}
