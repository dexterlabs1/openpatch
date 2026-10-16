use openpatch_core::{catalog, ModuleParameterActions, Node, PortFrame, RandomProbe};
use serde_json::json;
fn node(params: serde_json::Value) -> Node {
    serde_json::from_value(json!({"id":"r","kind":"random","params":params})).unwrap()
}
fn inputs(value: f32) -> [PortFrame; 6] {
    let mut inputs = [PortFrame::default(); 6];
    inputs[2] = PortFrame::mono(0.0);
    inputs[3] = PortFrame::mono(value);
    inputs
}
fn edge(probe: &mut RandomProbe, inputs: &mut [PortFrame; 6]) -> [PortFrame; 5] {
    inputs[2] = PortFrame::mono(0.0);
    probe.step(inputs);
    inputs[2] = PortFrame::mono(10.0);
    probe.step(inputs)
}
#[test]
fn startup_high_is_not_a_trigger_and_threshold_is_two_volts() {
    let mut p = RandomProbe::new(&node(json!({"shape":0})), 48000).unwrap();
    let mut input = inputs(7.0);
    input[2] = PortFrame::mono(10.0);
    assert_eq!(p.step(&input)[4].values[0], 0.0);
    assert_eq!(p.snapshot().next_voltage[0], 0.0);
    input[2] = PortFrame::mono(0.0);
    p.step(&input);
    input[2] = PortFrame::mono(1.999);
    assert_eq!(p.step(&input)[4].values[0], 0.0);
    input[2] = PortFrame::mono(2.0);
    assert_eq!(p.step(&input)[4].values[0], 10.0);
    assert_eq!(p.snapshot().next_voltage[0], 7.0);
}
#[test]
fn probability_zero_accepts_zero_draw_and_rejection_does_not_reset_phase() {
    let mut p = RandomProbe::new(&node(json!({"probability":0,"shape":0})), 48000).unwrap();
    let mut input = inputs(4.0);
    // Establish a nonzero interpolation history before rejecting a later edge.
    for _ in 0..128 {
        p.step_with_uniform(&input, &mut || panic!("unexpected draw"));
    }
    input[2] = PortFrame::mono(10.0);
    p.step_with_uniform(&input, &mut || 0.0);
    assert_eq!(p.snapshot().next_voltage[0], 4.0);
    input[2] = PortFrame::mono(0.0);
    for _ in 0..3 {
        p.step(&input);
    }
    let before = p.snapshot();
    assert!(before.phase > 0.0 && before.phase < 0.5);
    input[2] = PortFrame::mono(10.0);
    input[3] = PortFrame::mono(9.0);
    p.step_with_uniform(&input, &mut || 0.5);
    let rejected = p.snapshot();
    let dt = 1.0 / 48000.0_f32;
    let expected_frequency = 1.0 / (before.timer + dt);
    let delta = (expected_frequency * dt).min(0.5);
    assert_eq!(rejected.clock_frequency, expected_frequency);
    assert_eq!(rejected.timer, 0.0);
    assert_eq!(rejected.phase, (before.phase + delta).min(1.0));
    assert_ne!(rejected.phase, delta, "rejected edge must not reset phase");
    assert_eq!(rejected.last_voltage, before.last_voltage);
    assert_eq!(rejected.next_voltage, before.next_voltage);
    input[2] = PortFrame::mono(0.0);
    p.step(&input);
    input[2] = PortFrame::mono(10.0);
    p.step_with_uniform(&input, &mut || 0.0);
    assert_eq!(p.snapshot().next_voltage[0], 9.0);
    assert!(p.snapshot().lights[0] > 1.0);
    assert!(p.snapshot().lights[2] > 1.0);
}
#[test]
fn external_polyphony_preserves_hidden_holds_and_controls_stay_mono() {
    let mut p = RandomProbe::new(&node(json!({"shape":0})), 48000).unwrap();
    let mut input = inputs(4.0);
    input[3].channels = 16;
    input[3].values = std::array::from_fn(|i| i as f32);
    let out = edge(&mut p, &mut input);
    assert_eq!(out[0].channels, 16);
    assert_eq!(out[4].channels, 1);
    assert_eq!(out[0].values, input[3].values);
    input[3] = PortFrame::mono(-2.0);
    edge(&mut p, &mut input);
    assert_eq!(p.snapshot().next_voltage[15], 15.0);
    input[3].channels = 16;
    // Merely increasing width exposes the preserved lane until the next capture.
    input[2] = PortFrame::mono(10.0);
    assert_eq!(p.step(&input)[0].values[15], 15.0);

    // Higher lanes of every auxiliary input must be ignored, even when their
    // values would radically alter timing, probability, shape or spread.
    let controls = node(json!({"rate":10,"probability":0.5,"spread":0.5,"shape":0.7,
        "rate_cv":1,"probability_cv":1,"spread_cv":1,"shape_cv":1}));
    let mut mono = RandomProbe::new(&controls, 48000).unwrap();
    let mut poly = RandomProbe::new(&controls, 48000).unwrap();
    for frame in 0..512 {
        let mut ordinary = [PortFrame::default(); 6];
        for index in [0, 1, 4, 5] {
            ordinary[index] = PortFrame::mono((frame % 11) as f32 / 10.0);
        }
        if frame < 256 {
            ordinary[2] = PortFrame::mono(if frame % 17 == 0 { 10.0 } else { 0.0 });
            ordinary[3] = PortFrame {
                channels: 4,
                values: [2.0; 16],
            };
        }
        let mut poisoned = ordinary;
        for index in [0, 1, 2, 4, 5] {
            if ordinary[index].channels == 0 {
                continue;
            }
            poisoned[index].channels = 16;
            for lane in 1..16 {
                poisoned[index].values[lane] = match lane % 3 {
                    0 => f32::NAN,
                    1 => 1_000_000.0,
                    _ => -1_000_000.0,
                };
            }
        }
        assert_eq!(mono.step(&ordinary), poly.step(&poisoned));
        assert_eq!(mono.snapshot().lights, poly.snapshot().lights);
        assert_eq!(mono.snapshot().phase, poly.snapshot().phase);
        assert_eq!(
            mono.snapshot().clock_frequency,
            poly.snapshot().clock_frequency
        );
    }
}
#[test]
fn seed_edit_restarts_only_generator_and_rate_change_starts_fresh() {
    let mut p = RandomProbe::new(&node(json!({})), 48000).unwrap();
    let mut input = inputs(3.0);
    edge(&mut p, &mut input);
    let before = p.snapshot();
    let mut next = RandomProbe::new(&node(json!({"seed":2,"shape":0})), 48000).unwrap();
    next.adopt(&p);
    assert_eq!(next.snapshot().next_voltage, before.next_voltage);
    assert_eq!(next.snapshot().phase, before.phase);
    assert_ne!(next.snapshot().rng, before.rng);
    let mut rate = RandomProbe::new(&node(json!({})), 96000).unwrap();
    rate.adopt(&p);
    assert_eq!(rate.snapshot().next_voltage, [0.0; 16]);
    assert_eq!(rate.snapshot().schmitt, 2);
}
#[test]
fn native_generator_is_repeatable_and_nonfinite_input_is_safe() {
    let n = node(json!({"rate":10,"rate_cv":1,"shape_cv":1}));
    let mut a = RandomProbe::new(&n, 48000).unwrap();
    let mut b = RandomProbe::new(&n, 48000).unwrap();
    for _ in 0..4096 {
        assert_eq!(
            a.step(&[PortFrame::default(); 6]),
            b.step(&[PortFrame::default(); 6])
        );
    }
    let mut input = inputs(f32::NAN);
    input[0] = PortFrame::mono(f32::INFINITY);
    input[1] = PortFrame::mono(f32::NEG_INFINITY);
    let output = edge(&mut a, &mut input);
    assert!(output.iter().flat_map(|x| x.values).all(f32::is_finite));
}
#[test]
fn catalog_validation_and_randomization_contracts() {
    let catalog = catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "random")
        .unwrap();
    assert_eq!(module["inputs"].as_array().unwrap().len(), 6);
    assert_eq!(module["outputs"].as_array().unwrap().len(), 5);
    assert_eq!(module["light_names"].as_array().unwrap().len(), 5);
    assert_eq!(module["params"]["seed"]["randomization"], "disabled");
    assert_eq!(module["params"]["rate_cv"]["randomization"], "disabled");
    for params in [
        json!({"seed":0}),
        json!({"seed":1.5}),
        json!({"unipolar":0.5}),
        json!({"unknown":1}),
    ] {
        assert!(RandomProbe::new(&node(params), 48000).is_err());
    }
    for rate in [0, 1, 7999, 192001, u32::MAX] {
        let error = RandomProbe::new(&node(json!({})), rate).unwrap_err();
        assert!(error.contains("8000..=192000"));
    }
    for rate in [8000, 192000] {
        assert!(RandomProbe::new(&node(json!({})), rate).is_ok());
    }
    let mut n = node(json!({"seed":7,"rate_cv":0.75}));
    let actions = ModuleParameterActions::new(48000);
    actions.randomize(&mut n, 1234).unwrap();
    assert_eq!(n.params["seed"], 7.0);
    assert_eq!(n.params["rate_cv"], 0.75);
}
#[test]
fn engine_named_outputs_bypass_and_initialize_preserve_actual_lights() {
    use openpatch_core::{Engine, Patch};
    fn lights(engine: &Engine) -> openpatch_core::ModuleLightState {
        engine
            .module_lights(engine.resolve_module_lights("r").unwrap())
            .unwrap()
    }
    let mut patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"r","kind":"random","params":{"rate":10,"shape":0}}],
        "cables":[],"output":"r","output_port":"trigger"}))
    .unwrap();
    let mut engine = patch.compile().unwrap();
    let mut rendered = [PortFrame::default(); 256];
    engine.process_poly_block(&mut rendered);
    assert!(rendered.iter().any(|out| out.values[0] == 10.0));
    assert!(rendered.iter().all(|out| out.channels == 1));
    let before = lights(&engine);
    assert_eq!(before.count, 5);
    let handle = engine.resolve_module_state("r").unwrap();
    assert!(engine.initialize_module_state(handle));
    assert_eq!(lights(&engine), before);
    patch.nodes[0].bypassed = true;
    let mut bypass = patch.compile().unwrap();
    bypass.adopt_state_from(&engine);
    bypass.process_poly_block(&mut rendered);
    assert!(rendered.iter().all(|out| *out == PortFrame::mono(0.0)));
    assert_eq!(lights(&bypass), before);
    patch.nodes[0].bypassed = false;
    for port in ["stepped", "linear", "smooth", "exponential", "trigger"] {
        patch.output_port = port.into();
        assert!(patch.compile().is_ok());
    }
    let fresh_handle = bypass.resolve_module_state("r").unwrap();
    assert!(bypass.reset_module_state(fresh_handle));
    assert_eq!(
        lights(&bypass).brightness,
        [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]
    );
    patch.version = 1;
    assert!(patch.compile().is_err());
}
