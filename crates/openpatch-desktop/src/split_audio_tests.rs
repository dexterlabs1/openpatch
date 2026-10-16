// The actual offline callback and Session bridge; no hardware stream or GUI.
fn split_transport_patch() -> Patch {
    let mut patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
        "nodes":[{"id":"source","kind":"poly_constant","params":{"channels":16}},
                 {"id":"split","kind":"poly_split"}],
        "cables":[{"from":"source","to":"split","input":"signal"}],
        "output":"source"})).unwrap();
    let source = patch.nodes.iter_mut().find(|node| node.id == "source").unwrap();
    for channel in 1..=16 {
        source.params.insert(format!("v{channel}"), 0.0);
    }
    assert!((1..=16).all(|channel| source.params[&format!("v{channel}")] == 0.0));
    patch.assign_missing_cable_ids().unwrap();
    patch
}
fn assert_split_count(lights: openpatch_core::ModuleLightState, channels: u8) {
    assert_eq!(lights.input_channels, Some(channels));
    assert_eq!(lights.count, 0);
    assert_eq!(lights.brightness, [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]);
    assert!(!lights.manual);
    assert_eq!(lights.output_channels, None);
    assert_eq!(lights.automatic_channels, None);
    assert_eq!(lights.waveform_position, None);
}

#[test]
fn split_callback_measures_silent_active_zero_and_stale_graph_counts_without_heap_work() {
    let patch = split_transport_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone())).unwrap();
    assert!(callback.performance.module_lights("split").is_none());
    assert!(callback.performance.trigger("split", ModuleAction::BPress).is_err());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert_split_count(callback.performance.module_lights("split").unwrap(), 0);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    // All sixteen source voltages are zero, but all sixteen lanes are active.
    assert_split_count(callback.performance.module_lights("split").unwrap(), 16);
    assert!(callback.performance.module_control_state("split").is_none());
    assert!(callback.performance.sequence_state("split").is_none());
    let unchanged = session.snapshot();
    let inspected = session.execute("inspect_performance", serde_json::json!({})).unwrap();
    let module = &inspected["performance"]["modules"]["split"];
    assert_eq!(module["kind"], "poly_split");
    assert_eq!(module["lights"]["input_channels"], 16);
    assert_eq!(module["lights"]["count"], 0);
    assert!(module["state"].is_null() && module["controls"].is_null());
    assert_eq!(session.snapshot(), unchanged);

    session.execute("set_parameter", serde_json::json!({"id":"source","parameter":"channels","value":0})).unwrap();
    let stale = sequence_graph(&callback, session.patch(), session.revision());
    session.execute("set_parameter", serde_json::json!({"id":"source","parameter":"channels","value":1})).unwrap();
    assert!(callback.performance.module_lights("split").is_none());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert!(callback.performance.module_lights("split").is_none());
    assert!(updates.push(stale).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    assert!(callback.performance.module_lights("split").is_none());
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    assert_split_count(callback.performance.module_lights("split").unwrap(), 1);
    assert_eq!(callback.performance.snapshot()["modules"]["split"]["revision"], session.revision());
    session.execute("set_parameter", serde_json::json!({"id":"source","parameter":"channels","value":0})).unwrap();
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    assert_split_count(callback.performance.module_lights("split").unwrap(), 0);
}

#[test]
fn split_callback_count_identity_survives_bypass_but_not_reused_document_or_backend() {
    let patch = split_transport_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch.clone(), Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone())).unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2], 1));
    let original = callback.performance.module_lights("split").unwrap();
    assert_split_count(original, 16);
    session.execute("set_bypass", serde_json::json!({"id":"split","bypassed":true})).unwrap();
    session.execute("set_parameter", serde_json::json!({"id":"source","parameter":"channels","value":0})).unwrap();
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    assert_eq!(callback.performance.module_lights("split").unwrap(), original);
    session.execute("set_bypass", serde_json::json!({"id":"split","bypassed":false})).unwrap();
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    assert_split_count(callback.performance.module_lights("split").unwrap(), 0);

    // Reusing the ID for a different meter must not expose Split's old packet.
    let replacement: Patch = serde_json::from_value(serde_json::json!({"version":2,
        "nodes":[{"id":"split","kind":"poly_sum"}],"output":"split"})).unwrap();
    session.replace_patch(replacement, Some(session.revision())).unwrap();
    assert!(callback.performance.module_lights("split").is_none());
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    assert_eq!(callback.performance.snapshot()["modules"]["split"]["kind"], "poly_sum");
    assert_eq!(callback.performance.module_lights("split").unwrap().count, 6);

    let stale = sequence_graph(&callback, session.patch(), session.revision());
    let mut reopened = patch.clone();
    reopened.nodes.iter_mut().find(|node| node.id == "split").unwrap().bypassed = true;
    session.replace_patch(reopened, Some(session.revision())).unwrap();
    assert!(callback.performance.module_lights("split").is_none());
    assert!(updates.push(stale).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    assert!(callback.performance.module_lights("split").is_none());
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert_split_count(callback.performance.module_lights("split").unwrap(), 0);
    let old_backend = callback.performance.clone();
    old_backend.close();
    assert!(old_backend.module_lights("split").is_none());
    let (mut replacement, _, _) = sequence_callback(&patch);
    assert!(replacement.performance.module_lights("split").is_none());
    without_heap_activity(|| replacement.render(&mut [0.0_f32; 2], 1));
    assert_split_count(replacement.performance.module_lights("split").unwrap(), 16);
    assert!(old_backend.module_lights("split").is_none());
}
