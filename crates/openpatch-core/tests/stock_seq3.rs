use openpatch_core::*;
use serde_json::json;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":8000,"nodes":[{"id":"s","kind":"stock_seq3","params":{"row1_1":1,"row1_2":2}}],"output":"s","output_port":"cv1"})).unwrap()
}
fn step(engine: &mut Engine) -> PortFrame {
    let mut output = [PortFrame::default()];
    engine.process_poly_block(&mut output);
    output[0]
}
#[test]
fn physical_controls_telemetry_lights_and_all_sixteen_taps_share_actual_runtime() {
    let mut engine = patch().compile().unwrap();
    let handle = engine.resolve_module_telemetry("s").unwrap();
    assert_eq!(engine.module_control_state(handle).unwrap().pressed_mask, 0);
    assert_eq!(engine.module_lights(handle).unwrap().count, 19);
    assert!(engine.handle_module_event(handle, ModuleAction::ClockPress));
    step(&mut engine);
    assert_eq!(
        engine.module_telemetry(handle).unwrap().step,
        1,
        "fresh high arms without triggering"
    );
    assert!(engine.handle_module_event(handle, ModuleAction::ClockRelease));
    step(&mut engine);
    assert!(engine.handle_module_event(handle, ModuleAction::ClockPress));
    assert_eq!(step(&mut engine).values[0], 2.0);
    assert_eq!(engine.module_telemetry(handle).unwrap().step, 2);
    let ports = [
        "cv1", "cv2", "cv3", "trigger", "step1", "step2", "step3", "step4", "step5", "step6",
        "step7", "step8", "steps", "clock", "run", "reset",
    ];
    let taps =
        std::array::from_fn::<_, 16, _>(|i| Some(engine.resolve_tap("s", ports[i], 0).unwrap()));
    let mut audio = [[0.0; 2]; 3];
    let mut samples = [[TapSample::default(); 16]; 3];
    engine.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    assert!(samples
        .iter()
        .flatten()
        .all(|s| s.active && s.value.is_finite()));
    assert_eq!(samples[0][5].value, 10.0);
    assert_eq!(samples[0][12].value, 7.0);
    assert!(engine.resolve_tap("s", "out", 0).is_err());
}
#[test]
fn full_durable_state_adopts_then_explicit_load_wins_without_rewinding() {
    let p = patch();
    let mut old = p.compile().unwrap();
    let h = old.resolve_module_state("s").unwrap();
    old.handle_module_event(h, ModuleAction::Clock);
    step(&mut old);
    let flags = SavedModuleState::StockSeq3 {
        running: false,
        gates: 0x55,
        passthrough: true,
    };
    assert!(old.set_module_saved_state(h, flags));
    let mut newpatch = p.clone();
    newpatch.nodes[0].params.insert("row1_2".into(), 7.0);
    let mut receiving = newpatch.compile().unwrap();
    receiving.adopt_state_from(&old);
    let handle = receiving.resolve_module_state("s").unwrap();
    assert_eq!(receiving.module_saved_state(handle), Some(flags));
    assert_eq!(receiving.module_telemetry(handle).unwrap().step, 2);
    assert_eq!(step(&mut receiving).values[0], 7.0);
    let assignment = SavedModuleState::StockSeq3 {
        running: true,
        gates: 0xff,
        passthrough: false,
    };
    assert!(receiving.set_module_saved_state(handle, assignment));
    assert_eq!(receiving.module_saved_state(handle), Some(assignment));
    assert_eq!(receiving.module_telemetry(handle).unwrap().step, 2);
    assert!(!receiving.set_module_saved_state(handle, SavedModuleState::Seq3Run(false)));
    assert!(receiving.initialize_module_state(handle));
    assert_eq!(receiving.module_telemetry(handle).unwrap().step, 1);
    assert_eq!(receiving.module_saved_state(handle), Some(assignment));
    let mut changed = p.clone();
    changed.sample_rate = 16000;
    let mut rate = changed.compile().unwrap();
    rate.adopt_state_from(&old);
    let h = rate.resolve_module_state("s").unwrap();
    assert_eq!(rate.module_telemetry(h).unwrap().step, 1);
    assert_eq!(rate.module_saved_state(h), Some(assignment));
}
#[test]
fn bypass_freezes_signal_runtime_but_safety_release_clears_held_levels() {
    let p = patch();
    let mut old = p.compile().unwrap();
    let h = old.resolve_module_event("s").unwrap();
    step(&mut old);
    old.handle_module_event(h, ModuleAction::ClockPress);
    step(&mut old);
    let before = old.module_telemetry(h).unwrap();
    let flags = old.module_saved_state(h);
    let lamps = old.module_lights(h);
    let mut bypasspatch = p.clone();
    bypasspatch.nodes[0].bypassed = true;
    let mut bypass = bypasspatch.compile().unwrap();
    bypass.adopt_state_from(&old);
    let h = bypass
        .resolve_module_action("s", ModuleAction::ClockRelease)
        .unwrap();
    assert!(bypass
        .resolve_module_action("s", ModuleAction::ClockPress)
        .is_err());
    assert!(bypass.handle_module_event(h, ModuleAction::ClockRelease));
    assert_eq!(bypass.module_control_state(h).unwrap().pressed_mask, 0);
    for _ in 0..25 {
        assert_eq!(step(&mut bypass), PortFrame::mono(0.0));
    }
    assert_eq!(bypass.module_telemetry(h), Some(before));
    assert_eq!(bypass.module_saved_state(h), flags);
    assert_eq!(bypass.module_lights(h), lamps);
}
#[test]
fn chaining_reset_and_clock_uses_delayed_edges_and_reset_guard_in_receiver() {
    let mut p = patch();
    p.nodes
        .push(serde_json::from_value(json!({"id":"child","kind":"stock_seq3"})).unwrap());
    p.cables=serde_json::from_value(json!([{"from":"s","output":"clock","to":"child","input":"clock"},{"from":"s","output":"reset","to":"child","input":"reset"}])).unwrap();
    let mut engine = p.compile().unwrap();
    let master = engine.resolve_module_event("s").unwrap();
    let child = engine.resolve_module_telemetry("child").unwrap();
    step(&mut engine);
    engine.handle_module_event(master, ModuleAction::Clock);
    step(&mut engine);
    step(&mut engine);
    assert_eq!(engine.module_telemetry(child).unwrap().step, 2);
    for _ in 0..10 {
        step(&mut engine);
    }
    engine.handle_module_event(master, ModuleAction::Reset);
    step(&mut engine);
    step(&mut engine);
    assert_eq!(
        engine.module_telemetry(child).unwrap().step,
        1,
        "reset-induced master CLOCK must be guarded by simultaneous RESET in child"
    );
}
#[test]
fn catalog_actions_saved_projection_and_legacy_contract_are_separate() {
    let c = catalog();
    let m = c["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "stock_seq3")
        .unwrap();
    assert_eq!(m["min_patch_version"], 2);
    assert_eq!(m["params"]["tempo"]["default"], 1.0);
    assert_eq!(m["params"]["tempo"]["min"], -2.0);
    assert_eq!(m["params"]["running"]["initialize"], false);
    assert_eq!(m["params"]["tempo"]["randomization"], "disabled");
    let mut p = patch();
    p.nodes[0].params.insert("running".into(), 0.0);
    let actions = ModuleParameterActions::new(8000);
    actions.initialize(&mut p.nodes[0]).unwrap();
    assert_eq!(p.nodes[0].params["running"], 0.0);
    actions.randomize(&mut p.nodes[0], 42).unwrap();
    assert_eq!(p.nodes[0].params["tempo"], 1.0);
    let flags = SavedModuleState::StockSeq3 {
        running: false,
        gates: 0xa5,
        passthrough: true,
    };
    flags.write_params(&mut p.nodes[0].params);
    assert_eq!(SavedModuleState::from_node(&p.nodes[0]), Some(flags));
    assert_eq!(flags.kind(), "stock_seq3");
    assert_eq!(flags.boolean(), None);
    p.version = 1;
    assert!(p.compile().is_err());
    p.version = 2;
    for (name, value) in [
        ("running", 0.5),
        ("gate8", 0.5),
        ("passthrough", 0.5),
        ("steps", 3.5),
        ("tempo", 4.01),
    ] {
        let mut bad = p.clone();
        bad.nodes[0].params.insert(name.into(), value);
        assert!(bad.compile().is_err());
    }
    let mut legacy = patch();
    legacy.nodes[0].kind = "seq3".into();
    legacy.nodes[0].params.insert("steps".into(), 1.0);
    let mut old = legacy.compile().unwrap();
    let h = old.resolve_module_event("s").unwrap();
    old.handle_module_event(h, ModuleAction::Clock);
    let tap = old.resolve_tap("s", "clock", 0).unwrap();
    let mut audio = [[0.0; 2]; 1];
    let mut samples = [[TapSample::default(); 1]; 1];
    old.process_audio_block_with_taps(&mut audio, &[Some(tap)], &mut samples);
    assert_eq!(
        samples[0][0].value, 10.0,
        "legacy length-one clocks still pulse"
    );
}

#[test]
fn masked_state_load_after_adoption_preserves_newer_callback_run_and_gate_edges() {
    let p = patch();
    let mut live = p.compile().unwrap();
    let live_handle = live.resolve_module_state("s").unwrap();
    assert!(live.set_module_saved_state(
        live_handle,
        SavedModuleState::StockSeq3 {
            running: true,
            gates: 0xff,
            passthrough: true,
        }
    ));
    step(&mut live); // Arm the physical button detectors at LOW.
    let control_snapshot = live.module_saved_state(live_handle).unwrap();

    // These callback decisions happen after the control-thread snapshot.
    assert!(live.handle_module_event(live_handle, ModuleAction::RunPress));
    assert!(live.handle_module_event(live_handle, ModuleAction::SeqGatePress(2)));
    step(&mut live);
    let newer = SavedModuleState::StockSeq3 {
        running: false,
        gates: 0xfd,
        passthrough: true,
    };
    assert_eq!(live.module_saved_state(live_handle), Some(newer));

    let mut receiving_patch = p;
    // A graph may still contain the captured full snapshot. Adoption must take
    // the newer callback state; only the explicitly edited gate is assigned.
    control_snapshot.write_params(&mut receiving_patch.nodes[0].params);
    receiving_patch.nodes[0].params.insert("row1_1".into(), 5.0);
    let mut receiving = receiving_patch.compile().unwrap();
    receiving.adopt_state_from(&live);
    let handle = receiving.resolve_module_state("s").unwrap();
    assert_eq!(receiving.module_saved_state(handle), Some(newer));
    let partial = SavedModuleState::StockSeq3Fields {
        running: None,
        gates: 0,
        gate_mask: 1,
        passthrough: None,
    };
    assert!(receiving.set_module_saved_state(handle, partial));
    let expected = SavedModuleState::StockSeq3 {
        running: false,
        gates: 0xfc,
        passthrough: true,
    };
    assert_eq!(receiving.module_saved_state(handle), Some(expected));
    assert_eq!(step(&mut receiving).values[0], 5.0);
    assert_eq!(receiving.module_saved_state(handle), Some(expected));

    let mut projected = std::collections::BTreeMap::new();
    partial.write_params(&mut projected);
    assert_eq!(projected.len(), 1);
    assert_eq!(projected["gate1"], 0.0);
    assert_eq!(partial.kind(), "stock_seq3");
    assert_eq!(partial.boolean(), None);
    assert_eq!(partial.with_value(true), partial);
    // Captures always contain a complete state, never a pending assignment.
    assert!(matches!(
        SavedModuleState::from_node(&receiving_patch.nodes[0]),
        Some(SavedModuleState::StockSeq3 { .. })
    ));
}
