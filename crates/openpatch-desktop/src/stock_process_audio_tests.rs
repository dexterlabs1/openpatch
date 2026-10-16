// These checks cross Session, the bounded performance queue, graph adoption,
// and the production audio callback without opening an audio device.
fn stock_process_host_patch() -> Patch {
    let mut patch: Patch = serde_json::from_value(serde_json::json!({
        "version":2,"sample_rate":48000,
        "nodes":[
            {"id":"source","kind":"poly_constant","params":{"channels":1,"v1":4}},
            {"id":"process","kind":"stock_process","params":{"slew":0}}
        ],
        "cables":[{"from":"source","output":"out","to":"process","input":"signal"}],
        "output":"process","output_port":"sh1"
    })).unwrap();
    patch.assign_missing_cable_ids().unwrap();
    patch
}

fn stock_process_callback_state(callback: &Callback) -> openpatch_core::StockProcessSnapshot {
    let engine = &callback.graphs.active.engine;
    engine.stock_process_snapshot(engine.resolve_module_state("process").unwrap()).unwrap()
}

#[test]
fn stock_process_session_gate_actions_reach_actual_callback_and_initialize_keeps_histories() {
    let patch = stock_process_host_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let directory = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(directory.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone())).unwrap();
    assert_eq!(session.revision(), 1);
    assert!(callback.performance.module_lights("process").is_none());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
    let initial = stock_process_callback_state(&callback);
    assert_eq!(initial.gate_param, 0.0);
    assert_eq!(initial.gate_light_brightness, 0.0);
    let lamp = callback.performance.module_lights("process").unwrap();
    assert_eq!(lamp.count, 1);
    assert_eq!(lamp.brightness[0][0], 0.0);
    let before_action = session.snapshot();

    let press = session.execute("trigger_module", serde_json::json!({
        "id":"process","action":"gate_press","expected_revision":session.revision()
    })).unwrap();
    assert_eq!(press["receipt"]["queued"], true);
    assert_eq!(press["receipt"]["applied"], false);
    assert_eq!(session.snapshot(), before_action, "momentary gate is runtime only");
    without_heap_activity(|| callback.render(&mut [0.0_f32; 4], 1));
    let pressed = stock_process_callback_state(&callback);
    assert_eq!(pressed.gate_param, 1.0);
    assert_eq!(pressed.gate_light_brightness, 1.0);
    assert!(pressed.engines[0].state);
    assert_eq!(pressed.engines[0].sample1, 4.0);
    assert!(callback.performance.module_control_state("process").unwrap().pressed);
    assert_eq!(callback.performance.module_lights("process").unwrap().brightness[0][0], 1.0);
    let observed = session.execute("inspect_performance", serde_json::json!({})).unwrap();
    assert_eq!(observed["performance"]["modules"]["process"]["lights"]["brightness"][0][0], 1.0);

    session.execute("trigger_module", serde_json::json!({
        "id":"process","action":"gate_release","expected_revision":session.revision()
    })).unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 4], 1));
    let released = stock_process_callback_state(&callback);
    assert_eq!(released.gate_param, 0.0);
    assert_eq!(released.gate_light_brightness, 0.0);
    assert!(!released.engines[0].state);
    assert!(!callback.performance.module_control_state("process").unwrap().pressed);
    assert_eq!(callback.performance.module_lights("process").unwrap().brightness[0][0], 0.0);
    let project = session.capture_project_patch(Some(session.revision())).unwrap();
    assert!(!project.patch.nodes[1].params.contains_key("gate"));
    let preset = session.execute("capture_module_preset", serde_json::json!({"id":"process"})).unwrap();
    assert_eq!(preset["preset"]["params"], serde_json::json!({"slew":0.0}));

    let old_revision = session.revision();
    session.execute("initialize_modules", serde_json::json!({"ids":["process"]})).unwrap();
    assert!(session.revision() > old_revision);
    assert!(updates.push(sequence_graph(&callback, session.patch(), session.revision())).is_ok());
    // Adopt without advancing a sample so frozen histories are compared before
    // another callback can update them under the reset slew setting.
    without_heap_activity(|| callback.render(&mut [] as &mut [f32], 1));
    while disposal.pop().is_ok() {}
    let initialized = stock_process_callback_state(&callback);
    assert_eq!(initialized.engines, released.engines);
    assert_eq!(initialized.gate_param, 0.0);
    assert_eq!(initialized.gate_light_brightness, released.gate_light_brightness);
    assert_eq!(initialized.slew_param, 1e-3_f32.log2());
    assert!(session.execute("trigger_module", serde_json::json!({
        "id":"process","action":"gate_press","expected_revision":old_revision
    })).is_err(), "an old UI revision cannot queue a new manual press");
    session.execute("trigger_module", serde_json::json!({
        "id":"process","action":"gate_press","expected_revision":session.revision()
    })).unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert_eq!(stock_process_callback_state(&callback).gate_light_brightness, 1.0);
}
