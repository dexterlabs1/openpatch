use openpatch_core::*;
use serde_json::json;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":48000,"nodes":[{"id":"vco","kind":"stock_vco"}],"output":"vco","output_port":"sine"})).unwrap()
}
fn step(engine: &mut Engine) -> PortFrame {
    let mut frame = [PortFrame::default()];
    engine.process_poly_block(&mut frame);
    frame[0]
}
fn lights(engine: &Engine) -> ModuleLightState {
    engine
        .module_lights(engine.resolve_module_lights("vco").unwrap())
        .unwrap()
}
#[test]
fn named_outputs_taps_consumers_and_five_actual_lamps_are_integrated() {
    let mut p = patch();
    p.output = None;
    p.output_port = "out".into();
    let mut engine = p.compile().unwrap();
    assert_eq!(lights(&engine).count, 5);
    assert_eq!(lights(&engine).brightness, [[0.0; 2]; 16]);
    assert_eq!(lights(&engine).clock_frequency, None);
    assert!(engine.resolve_tap("vco", "out", 0).is_err());
    assert!(engine.resolve_module_control("vco").is_err());
    let taps = std::array::from_fn::<_, 4, _>(|i| {
        Some(
            engine
                .resolve_tap("vco", ["sine", "triangle", "saw", "square"][i], 0)
                .unwrap(),
        )
    });
    let mut audio = [[0.0; 2]; 49];
    let mut samples = [[TapSample::default(); 4]; 49];
    engine.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    assert!(samples
        .iter()
        .flatten()
        .all(|s| s.active && s.value.is_finite()));
    assert_ne!(samples[1][1].value, samples[1][2].value);
    let observed = lights(&engine);
    assert!(observed.brightness[0][0] != 0.0 || observed.brightness[1][0] != 0.0);
    assert_eq!(observed.brightness[2][0], 0.0);
    assert!(observed.brightness.iter().all(|row| row[1] == 0.0));
}
#[test]
fn engine_initialize_preserves_signal_history_and_bypass_freezes_it() {
    let p = patch();
    let mut old = p.compile().unwrap();
    old.process_poly_block(&mut [PortFrame::default(); 53]);
    let mut baseline = p.compile().unwrap();
    baseline.adopt_state_from(&old);
    let mut initialized = p.compile().unwrap();
    initialized.adopt_state_from(&old);
    let handle = initialized.resolve_module_state("vco").unwrap();
    assert!(initialized.initialize_module_state(handle));
    for _ in 0..40 {
        assert_eq!(step(&mut initialized), step(&mut baseline));
    }
    assert_eq!(lights(&initialized), lights(&baseline));
    let mut bp = p.clone();
    bp.nodes[0].bypassed = true;
    let mut bypass = bp.compile().unwrap();
    bypass.adopt_state_from(&old);
    let before = lights(&bypass);
    for _ in 0..35 {
        assert_eq!(step(&mut bypass), PortFrame::mono(0.0));
    }
    assert_eq!(lights(&bypass), before);
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&bypass);
    let mut expected = p.compile().unwrap();
    expected.adopt_state_from(&old);
    assert_eq!(step(&mut resumed), step(&mut expected));
    let mut fresh = p.clone();
    fresh.sample_rate = 96000;
    let mut changed = fresh.compile().unwrap();
    changed.adopt_state_from(&old);
    assert_eq!(lights(&changed).brightness, [[0.0; 2]; 16]);
}
#[test]
fn receiving_controls_win_and_changed_sync_routing_keeps_native_detector_history() {
    let mut p = patch();
    p.nodes.push(
        serde_json::from_value(json!({"id":"sync","kind":"constant","params":{"value":-1}}))
            .unwrap(),
    );
    p.cables = serde_json::from_value(json!([{"from":"sync","to":"vco","input":"sync"}])).unwrap();
    let mut old = p.compile().unwrap();
    old.process_poly_block(&mut [PortFrame::default(); 20]);
    let mut next = p.clone();
    next.nodes[0].params.insert("frequency".into(), 12.0);
    let mut changed = next.compile().unwrap();
    changed.adopt_state_from(&old);
    let mut baseline = p.compile().unwrap();
    baseline.adopt_state_from(&old);
    assert_ne!(step(&mut changed), step(&mut baseline));
    // Dropping the sync cable must not trigger a control-thread detector reset.
    // No crossing arrives while disconnected, so it matches an unchanged held low source.
    let mut disconnected = p.clone();
    disconnected.cables.clear();
    let mut detached = disconnected.compile().unwrap();
    detached.adopt_state_from(&old);
    let mut held = p.compile().unwrap();
    held.adopt_state_from(&old);
    for _ in 0..20 {
        assert_eq!(step(&mut detached), step(&mut held));
    }
}
#[test]
fn control_schema_randomization_and_version_guard_preserve_old_kinds() {
    let catalog = catalog();
    let descriptor = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "stock_vco")
        .unwrap();
    assert_eq!(descriptor["params"]["frequency"]["default"], 0.0);
    assert_eq!(descriptor["params"]["frequency"]["min"], -76.0);
    assert_eq!(descriptor["params"]["frequency"]["max"], 76.0);
    let mut node=serde_json::from_value(json!({"id":"vco","kind":"stock_vco","params":{"fm_amount":0.75,"pwm_amount":-0.25,"linear_fm":1}})).unwrap();
    let actions = ModuleParameterActions::new(48000);
    actions.randomize(&mut node, 73).unwrap();
    assert_eq!(node.params["fm_amount"], 0.75);
    assert_eq!(node.params["pwm_amount"], -0.25);
    actions.initialize(&mut node).unwrap();
    assert_eq!(node.params["frequency"], 0.0);
    let mut p = patch();
    p.version = 1;
    assert!(p.compile().is_err());
    p.version = 2;
    for params in [
        json!({"soft_sync":0.5}),
        json!({"linear_fm":0.5}),
        json!({"frequency":77}),
        json!({"pwm_amount":2}),
    ] {
        p.nodes[0].params = serde_json::from_value(params).unwrap();
        assert!(p.compile().is_err());
    }
    let multi = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "multi_vco")
        .unwrap();
    assert_eq!(
        multi["params"]["frequency"]["default"],
        json!(261.62558_f32)
    );
}
#[test]
fn public_polyphonic_pitch_and_consumer_outputs_keep_distinct_channel_counts() {
    for width in 0..=16 {
        let mut p = patch();
        p.nodes.push(
            serde_json::from_value(
                json!({"id":"pitch","kind":"poly_constant","params":{"channels":width}}),
            )
            .unwrap(),
        );
        p.cables =
            serde_json::from_value(json!([{"from":"pitch","to":"vco","input":"pitch"}])).unwrap();
        let mut engine = p.compile().unwrap();
        let mut frame = PortFrame::default();
        for _ in 0..18 {
            frame = step(&mut engine);
        }
        assert_eq!(frame.channels, width.max(1) as u8);
        assert!(frame.values.iter().all(|v| v.is_finite()));
        assert_eq!(
            lights(&engine).brightness[2][0],
            if width > 1 { 1.0 } else { 0.0 }
        );
    }
}

#[test]
fn tiny_finite_sync_fraction_uses_rust_safety_extension_without_source_table_access() {
    // Rust-only safety diagnostic, not an SDK equivalence assertion. These
    // ordinary finite voltages make source f32 interpolation reach oversample
    // index16; the SDK's sixteen-row table must never be indexed by this test.
    let node: Node = serde_json::from_value(json!({
        "id": "vco", "kind": "stock_vco", "params": {"soft_sync": 1}
    }))
    .unwrap();
    let mut probe = StockVcoProbe::new(&node, 48000).unwrap();
    let mut inputs = [PortFrame::default(); 4];
    let previous_voltage = -f32::from_bits((127 - 24) << 23); // -2^-24
    inputs[2] = PortFrame::mono(previous_voltage);
    probe.step(&inputs, 4); // Only saw consumes the nonzero-frequency oscillator.
    let before = probe.snapshot();
    assert_eq!(before.last_sync[0], previous_voltage);
    assert!(before.phases[0] > 0.0);
    assert_eq!(before.sync_directions[0], 1.0);

    let next_voltage = 2.0_f32;
    let fraction = -before.last_sync[0] / (next_voltage - before.last_sync[0]);
    let tiny_limit = f32::from_bits((127 - 25) << 23); // 2^-25
    assert!(fraction > 0.0 && fraction <= tiny_limit);
    assert_eq!(
        ((1.0_f32 - fraction) * 16.0) as usize,
        16,
        "source-derived index arithmetic exposes the unsafe row without accessing it"
    );

    inputs[2] = PortFrame::mono(next_voltage);
    let outputs = probe.step(&inputs, 4);
    let after = probe.snapshot();
    assert_eq!(outputs[2].channels, 1);
    assert_eq!(
        after.sync_directions[0], -1.0,
        "actual production soft sync reverses"
    );
    assert!(after
        .phases
        .iter()
        .all(|phase| phase.is_finite() && (0.0..=1.0).contains(phase)));
    assert!(after
        .sync_directions
        .iter()
        .all(|direction| *direction == 1.0 || *direction == -1.0));
    assert!(after
        .last_sync
        .iter()
        .chain(after.last_square.iter())
        .all(|value| value.is_finite()));
    assert!(outputs
        .iter()
        .all(|output| output.values.iter().all(|value| value.is_finite())));
    assert!(after
        .filter_state
        .iter()
        .flatten()
        .all(|value| value.is_finite()));
    assert!(after
        .correction_tails
        .iter()
        .flatten()
        .flatten()
        .all(|value| value.is_finite()));
    assert!(after.lights.iter().all(|value| value.is_finite()));
}

#[test]
fn soft_sync_preserves_the_source_float32_upper_phase_endpoint() {
    // IEEE binary32 wrapping can reach the upper endpoint even though its
    // real-number expression is half-open. No phase clamp belongs here.
    let negative = -2.0_f32.powi(-26);
    assert_eq!((negative - negative.floor()).to_bits(), 1.0_f32.to_bits());
    let mut node: Node = serde_json::from_value(json!({"id":"vco","kind":"stock_vco",
        "params":{"linear_fm":1,"soft_sync":1,"fm_amount":1}}))
    .unwrap();
    let mut stopped = StockVcoProbe::new(&node, 48_000).unwrap();
    let mut inputs = [PortFrame::default(); 4];
    inputs[1] = PortFrame::mono(-1.0);
    inputs[2] = PortFrame::mono(-f32::from_bits(1.0_f32.to_bits() - 1));
    stopped.step(&inputs, 15);
    assert_eq!(stopped.snapshot().phases[0], 0.0);
    node.params.insert("linear_fm".into(), 0.0);
    node.params.insert("fm_amount".into(), 0.0);
    let mut moving = StockVcoProbe::new(&node, 48_000).unwrap();
    moving.adopt(&stopped);
    inputs[1] = PortFrame::default();
    inputs[2] = PortFrame::mono(1.0);
    let output = moving.step(&inputs, 15);
    assert_eq!(moving.snapshot().sync_directions[0], -1.0);
    assert_eq!(moving.snapshot().phases[0].to_bits(), 1.0_f32.to_bits());
    assert!(output
        .iter()
        .flat_map(|port| port.values)
        .all(|value| value.is_finite()));
    assert!(moving
        .snapshot()
        .phases
        .iter()
        .all(|phase| (0.0..=1.0).contains(phase)));
}
