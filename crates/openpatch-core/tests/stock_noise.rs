use openpatch_core::{
    catalog, Engine, HostPortLifecycle, InputLifecycleTransform, OutputLifecycleTransform, Patch,
    PortFrame, TapSample,
};
use serde_json::json;

const OUTPUTS: [&str; 7] = ["white", "pink", "red", "violet", "blue", "gray", "black"];
const NORMAL: [u32; 16] = [
    0x0000_0000,
    0x3f80_0000,
    0xbf80_0000,
    0x3f00_0000,
    0xbf00_0000,
    0x4000_0000,
    0xc000_0000,
    0x3e80_0000,
    0xbe80_0000,
    0x3fc0_0000,
    0xbfc0_0000,
    0x3e00_0000,
    0xbe00_0000,
    0x3f40_0000,
    0xbf40_0000,
    0x0000_0000,
];
const UNIFORM: [u32; 16] = [
    0x0000_0000,
    0x3e80_0000,
    0x3f00_0000,
    0x3f40_0000,
    0x3e00_0000,
    0x3f60_0000,
    0x3ec0_0000,
    0x3f20_0000,
    0x3d80_0000,
    0x3f70_0000,
    0x3e40_0000,
    0x3f50_0000,
    0x3e90_0000,
    0x3f10_0000,
    0x3ed0_0000,
    0x3f30_0000,
];

fn patch(port: &str) -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"noise","kind":"stock_noise"}],
        "output":"noise","output_port":port}))
    .unwrap()
}
fn controlled(port: &str) -> Engine {
    let mut engine = patch(port).compile().unwrap();
    let handle = engine.resolve_module_state("noise").unwrap();
    assert!(engine.set_stock_noise_controlled_draws(handle, &NORMAL, &UNIFORM));
    engine
}
fn step(engine: &mut Engine) -> PortFrame {
    let mut out = [PortFrame::default()];
    engine.process_poly_block(&mut out);
    out[0]
}

#[test]
fn catalog_has_seven_stock_ports_and_no_controls_without_changing_colored_noise() {
    let catalog = catalog();
    let descriptors = catalog["modules"].as_array().unwrap();
    let stock = descriptors
        .iter()
        .find(|m| m["kind"] == "stock_noise")
        .unwrap();
    let original = descriptors
        .iter()
        .find(|m| m["kind"] == "colored_noise")
        .unwrap();
    assert_eq!(stock["outputs"], json!(OUTPUTS));
    assert_eq!(stock["inputs"], json!([]));
    assert_eq!(stock["params"], json!({}));
    assert_eq!(stock["min_patch_version"], 2);
    assert_eq!(stock["light_names"], json!([]));
    assert_eq!(stock["bypass"]["routes"], json!([]));
    assert_eq!(original["params"]["seed"]["default"], 1.0);
    assert_eq!(original["params"]["level"]["default"], 1.0);
    for name in OUTPUTS {
        assert!(patch(name).compile().is_ok());
    }
    let mut invalid = patch("white");
    invalid.nodes[0].params.insert("seed".into(), 1.0);
    assert!(invalid.compile().is_err());
}

#[test]
fn actual_host_ports_and_controlled_draws_follow_family_consumers() {
    let mut engine = controlled("red");
    let handle = engine.resolve_module_state("noise").unwrap();
    assert_eq!(step(&mut engine), PortFrame::mono(0.0)); // first normal draw is +0
    let history = engine.stock_noise_snapshot(handle).unwrap();
    assert_eq!(history.normal_draws, 1);
    assert_eq!(history.uniform_draws, 0);
    assert_eq!(history.draw_events.len(), 1);
    assert_eq!(history.draw_events[0].kind, "normal");
    let raw = engine.host_module_snapshot(handle).unwrap();
    assert_eq!(raw.output_count, 7);
    assert_eq!(raw.output_consumers, 1 << 2);
    assert_eq!(raw.outputs[0].channels, 0);
    assert_eq!(raw.outputs[2].channels, 1);
    assert_eq!(raw.outputs[1].values[0].to_bits(), 0); // pink never ran
    let red = step(&mut engine);
    assert!(red.values[0] > 0.0);
    let raw = engine.host_module_snapshot(handle).unwrap();
    assert!(raw.outputs[0].values[0] > 3.0); // white setVoltage runs within red family
    assert_eq!(raw.outputs[0].channels, 0); // passive raw value is not a consumer
    assert_eq!(engine.stock_noise_snapshot(handle).unwrap().normal_draws, 2);
}

#[test]
fn simultaneous_white_pink_black_consumers_use_one_ordered_uniform_cursor() {
    let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"noise","kind":"stock_noise"},{"id":"sink","kind":"poly_merge"}],
        "cables":[
            {"from":"noise","output":"pink","to":"sink","input":"in1"},
            {"from":"noise","output":"black","to":"sink","input":"in2"}
        ],"output":"noise","output_port":"white"}))
    .unwrap();
    let mut engine = patch.compile().unwrap();
    let handle = engine.resolve_module_state("noise").unwrap();
    assert!(engine.set_stock_noise_controlled_draws(handle, &NORMAL, &UNIFORM));
    step(&mut engine);
    let history = engine.stock_noise_snapshot(handle).unwrap();
    assert_eq!(history.normal_draws, 1);
    assert_eq!(history.uniform_draws, 9);
    assert_eq!(history.pink_frame, 0);
    assert_eq!(history.draw_events.len(), 10);
    assert_eq!(history.draw_events[0].kind, "normal");
    for event in &history.draw_events[1..9] {
        assert_eq!(event.kind, "pink");
    }
    assert_eq!(history.draw_events[9].kind, "black");
    assert_eq!(history.draw_events[9].bits, UNIFORM[8]);
    assert_eq!(history.last_uniform_bits, UNIFORM[8]);
    let ports = engine.host_module_snapshot(handle).unwrap();
    assert_eq!(ports.output_consumers, (1 << 0) | (1 << 1) | (1 << 6));
    assert_eq!(ports.outputs[6].values[0].to_bits(), (-4.375_f32).to_bits());
}

#[test]
fn gray_block_is_gated_by_its_own_consumer_and_initialize_preserves_history() {
    let mut engine = controlled("gray");
    let handle = engine.resolve_module_state("noise").unwrap();
    for _ in 0..1023 {
        step(&mut engine);
    }
    let before = engine.stock_noise_snapshot(handle).unwrap();
    assert_eq!(before.gray_frame, 1023);
    assert!(before.gray_output_bits.iter().all(|bit| *bit == 0));
    step(&mut engine);
    let after = engine.stock_noise_snapshot(handle).unwrap();
    assert_eq!(after.gray_frame, 0);
    assert!(after.gray_output_bits.iter().any(|bit| *bit != 0));
    assert!(engine.initialize_module_state(handle));
    assert_eq!(
        engine
            .stock_noise_snapshot(handle)
            .unwrap()
            .gray_output_bits,
        after.gray_output_bits
    );
    assert_eq!(
        engine.stock_noise_snapshot(handle).unwrap().normal_draws,
        1024
    );

    let mut disconnected = patch("black").compile().unwrap();
    let receiving = disconnected.resolve_module_state("noise").unwrap();
    assert!(disconnected.set_stock_noise_controlled_draws(receiving, &NORMAL, &UNIFORM));
    disconnected.adopt_state_from(&engine);
    for _ in 0..13 {
        step(&mut disconnected);
    }
    let frozen = disconnected.stock_noise_snapshot(receiving).unwrap();
    assert_eq!(frozen.gray_frame, 0);
    assert_eq!(frozen.gray_output_bits, after.gray_output_bits);
    assert_eq!(frozen.normal_draws, 1024);
    assert_eq!(frozen.uniform_draws, 13);
}

#[test]
fn recreated_noise_starts_fresh_histories_at_global_controlled_draw_position() {
    let mut old = controlled("black");
    let old_handle = old.resolve_module_state("noise").unwrap();
    for _ in 0..3 {
        step(&mut old);
    }
    let prior = old.stock_noise_snapshot(old_handle).unwrap();
    assert_eq!(prior.uniform_draws, 3);
    let mut fresh = patch("black").compile().unwrap();
    let handle = fresh.resolve_module_state("noise").unwrap();
    assert!(fresh.set_stock_noise_controlled_draws_at(
        handle,
        &NORMAL,
        &UNIFORM,
        prior.normal_draws,
        prior.uniform_draws
    ));
    let initial = fresh.stock_noise_snapshot(handle).unwrap();
    assert_eq!(initial.uniform_draws, 3);
    assert_eq!(initial.last_uniform_bits, UNIFORM[2]);
    assert_eq!(initial.pink_frame, -1);
    assert_eq!(initial.gray_frame, 0);
    assert_eq!(step(&mut fresh).values[0].to_bits(), 2.5_f32.to_bits());
    assert_eq!(fresh.stock_noise_snapshot(handle).unwrap().uniform_draws, 4);
}

#[test]
fn temporary_tap_is_a_real_black_consumer_and_disconnection_keeps_raw_value() {
    let mut patch = patch("black");
    patch.output = None;
    patch.output_port = "out".into();
    let mut engine = patch.compile().unwrap();
    let handle = engine.resolve_module_state("noise").unwrap();
    assert!(engine.set_stock_noise_controlled_draws(handle, &NORMAL, &UNIFORM));
    let tap = engine.resolve_tap("noise", "black", 0).unwrap();
    let mut audio = [[0.0; 2]; 1];
    let mut samples = [[TapSample::default(); 1]; 1];
    engine.process_audio_block_with_taps(&mut audio, &[Some(tap)], &mut samples);
    assert!(samples[0][0].active);
    assert_eq!(samples[0][0].value.to_bits(), (-5.0_f32).to_bits());
    assert_eq!(
        engine.stock_noise_snapshot(handle).unwrap().uniform_draws,
        1
    );
    let raw = engine.host_module_snapshot(handle).unwrap();
    assert_eq!(raw.output_consumers, 0);
    assert_eq!(raw.outputs[6].channels, 0);
    assert_eq!(raw.outputs[6].values[0].to_bits(), (-5.0_f32).to_bits());
}

#[test]
fn gray_fft_state_is_independent_of_callback_partition() {
    let mut whole = controlled("gray");
    let mut split = controlled("gray");
    let mut block = [PortFrame::default(); 1025];
    whole.process_poly_block(&mut block);
    let mut last = PortFrame::default();
    for _ in 0..1025 {
        last = step(&mut split);
    }
    assert_eq!(block[1024], last);
    let whole = whole
        .stock_noise_snapshot(whole.resolve_module_state("noise").unwrap())
        .unwrap();
    let split = split
        .stock_noise_snapshot(split.resolve_module_state("noise").unwrap())
        .unwrap();
    assert_eq!(whole.normal_draws, split.normal_draws);
    assert_eq!(whole.gray_frame, split.gray_frame);
    assert_eq!(whole.gray_input_bits, split.gray_input_bits);
    assert_eq!(whole.gray_output_bits, split.gray_output_bits);
}

#[test]
fn separate_stock_noise_instances_keep_independent_controlled_cursors() {
    let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"white","kind":"stock_noise"},
                 {"id":"black","kind":"stock_noise"},
                 {"id":"sink","kind":"poly_viz"}],
        "cables":[{"from":"black","output":"black","to":"sink","input":"signal"}],
        "output":"white","output_port":"white"}))
    .unwrap();
    let mut engine = patch.compile().unwrap();
    for id in ["white", "black"] {
        let handle = engine.resolve_module_state(id).unwrap();
        assert!(engine.set_stock_noise_controlled_draws(handle, &NORMAL, &UNIFORM));
    }
    step(&mut engine);
    let white = engine
        .stock_noise_snapshot(engine.resolve_module_state("white").unwrap())
        .unwrap();
    let black = engine
        .stock_noise_snapshot(engine.resolve_module_state("black").unwrap())
        .unwrap();
    assert_eq!((white.normal_draws, white.uniform_draws), (1, 0));
    assert_eq!((black.normal_draws, black.uniform_draws), (0, 1));
    assert_eq!(white.draw_events[0].kind, "normal");
    assert_eq!(black.draw_events[0].kind, "black");
}

#[test]
fn passive_sink_observation_sees_process_time_input_without_changing_consumers() {
    let patch: Patch = serde_json::from_value(json!({
        "version":2,"sample_rate":48000,
        "nodes":[{"id":"noise","kind":"stock_noise"},{"id":"sink","kind":"poly_viz"}],
        "cables":[{"id":"out6","from":"noise","output":"black","to":"sink","input":"signal"}],
        "output":"noise","output_port":"black"
    }))
    .unwrap();
    let mut observed = patch.compile().unwrap();
    let mut plain = patch.compile().unwrap();
    for engine in [&mut observed, &mut plain] {
        let noise = engine.resolve_module_state("noise").unwrap();
        assert!(engine.set_stock_noise_controlled_draws(noise, &NORMAL, &UNIFORM));
    }
    let sink = observed.resolve_module_state("sink").unwrap();
    let noise = observed.resolve_module_state("noise").unwrap();
    assert!(observed.enable_host_input_observer(sink, 0));
    assert!(observed.enable_host_input_observer(sink, 0)); // idempotent
    assert!(!observed.enable_host_input_observer(sink, 1));
    assert!(plain
        .observed_host_input(plain.resolve_module_state("sink").unwrap(), 0)
        .is_none());
    let initial_input = observed.host_module_snapshot(sink).unwrap().inputs[0];
    assert_eq!(
        observed
            .host_module_snapshot(noise)
            .unwrap()
            .output_consumers,
        1 << 6
    );

    let first_frame = step(&mut observed);
    assert_eq!(first_frame, step(&mut plain));
    assert_eq!(
        observed.observed_host_input(sink, 0).unwrap(),
        initial_input
    );
    let first_input = observed.host_module_snapshot(sink).unwrap().inputs[0];
    assert_eq!(
        first_input,
        plain
            .host_module_snapshot(plain.resolve_module_state("sink").unwrap())
            .unwrap()
            .inputs[0]
    );
    assert_eq!(
        observed.host_module_snapshot(noise).unwrap().outputs,
        plain
            .host_module_snapshot(plain.resolve_module_state("noise").unwrap())
            .unwrap()
            .outputs
    );

    let second_frame = step(&mut observed);
    assert_eq!(second_frame, step(&mut plain));
    assert_ne!(
        first_frame.values[0].to_bits(),
        second_frame.values[0].to_bits()
    );
    assert_eq!(observed.observed_host_input(sink, 0).unwrap(), first_input);
    assert_eq!(
        observed
            .host_module_snapshot(noise)
            .unwrap()
            .output_consumers,
        1 << 6
    );
    assert_eq!(
        observed.stock_noise_snapshot(noise).unwrap().uniform_draws,
        plain
            .stock_noise_snapshot(plain.resolve_module_state("noise").unwrap())
            .unwrap()
            .uniform_draws
    );

    let last_seen = observed.observed_host_input(sink, 0).unwrap();
    let mut recreated = patch.compile().unwrap();
    let recreated_sink = recreated.resolve_module_state("sink").unwrap();
    assert!(recreated.enable_host_input_observer(recreated_sink, 0));
    let recreated_noise = recreated.resolve_module_state("noise").unwrap();
    recreated.adopt_state_from_excluding(&observed, &[recreated_noise]);
    assert_eq!(
        recreated.observed_host_input(recreated_sink, 0).unwrap(),
        last_seen
    );

    let mut disconnected_patch = patch;
    disconnected_patch.cables.clear();
    disconnected_patch.output = None;
    disconnected_patch.output_port = "out".into();
    let mut disconnected = disconnected_patch.compile().unwrap();
    let new_sink = disconnected.resolve_module_state("sink").unwrap();
    assert!(disconnected.enable_host_input_observer(new_sink, 0));
    disconnected.adopt_state_from(&observed);
    assert_eq!(
        disconnected.observed_host_input(new_sink, 0).unwrap(),
        last_seen
    );
    let mut delta = HostPortLifecycle::default();
    delta.inputs[0] = InputLifecycleTransform::disconnect();
    assert!(disconnected.apply_host_port_lifecycle(new_sink, &delta));
    assert_eq!(
        disconnected.observed_host_input(new_sink, 0).unwrap(),
        last_seen
    );
    let new_noise = disconnected.resolve_module_state("noise").unwrap();
    let mut output_delta = HostPortLifecycle::default();
    output_delta.outputs[6] = OutputLifecycleTransform::disconnect();
    assert!(disconnected.apply_host_port_lifecycle(new_noise, &output_delta));
    assert_eq!(
        disconnected
            .host_module_snapshot(new_noise)
            .unwrap()
            .output_consumers,
        0
    );
    let pre_step_input = disconnected.host_module_snapshot(new_sink).unwrap().inputs[0];
    step(&mut disconnected);
    assert_eq!(
        disconnected.observed_host_input(new_sink, 0).unwrap(),
        pre_step_input
    );
}

#[test]
fn noop_noise_reset_and_initialize_preserve_observed_signal_lights() {
    use openpatch_core::PortDirection;
    let p: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"noise","kind":"stock_noise"},{"id":"sink","kind":"poly_viz"}],
        "cables":[{"from":"noise","output":"black","to":"sink","input":"signal"}]}))
    .unwrap();
    let mut engine = p.compile().unwrap();
    engine.set_port_lights_enabled(true);
    let handle = engine.resolve_module_state("noise").unwrap();
    assert!(engine.set_stock_noise_controlled_draws(handle, &NORMAL, &UNIFORM));
    let light = engine
        .resolve_port_light("noise", PortDirection::Output, "black")
        .unwrap();
    for _ in 0..32 {
        step(&mut engine);
    }
    let before = engine
        .port_light(light)
        .expect("Actual acquired signal observation");
    assert!(before.connected && before.channels == 1);
    assert!(before.raw_rgb.iter().any(|value| *value != 0.0));
    let frame = engine.port_light_observation_frame();
    let ports = engine.host_module_snapshot(handle).unwrap();
    let draws = engine.stock_noise_snapshot(handle).unwrap().uniform_draws;
    assert!(engine.initialize_module_state(handle));
    assert_eq!(engine.port_light(light), Some(before));
    assert_eq!(engine.port_light_observation_frame(), frame);
    assert!(engine.reset_module_state(handle));
    assert_eq!(engine.port_light(light), Some(before));
    assert_eq!(engine.host_module_snapshot(handle).unwrap(), ports);
    assert_eq!(
        engine.stock_noise_snapshot(handle).unwrap().uniform_draws,
        draws
    );
}
