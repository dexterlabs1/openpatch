use openpatch_core::{
    catalog, ModuleAction, ModuleParameterActions, Patch, PortFrame, StockProcessSnapshotBits,
};
use serde_json::json;

fn patch(rate: u32) -> Patch {
    serde_json::from_value(json!({
        "version": 2,
        "sample_rate": rate,
        "nodes": [
            {"id":"source","kind":"poly_constant","params":{"channels":1,"v1":2.0}},
            {"id":"process","kind":"stock_process","params":{"slew":-10.0}}
        ],
        "cables": [
            {"id":"signal","from":"source","to":"process","input":"signal"}
        ],
        "output":"process",
        "output_port":"sh1"
    }))
    .unwrap()
}

#[test]
fn stock_process_is_separate_from_existing_process_and_exposes_source_ports() {
    let modules = catalog()["modules"].as_array().unwrap().clone();
    assert_eq!(modules.len(), 80);
    let stock = modules
        .iter()
        .find(|module| module["kind"] == "stock_process")
        .unwrap();
    let legacy = modules
        .iter()
        .find(|module| module["kind"] == "process")
        .unwrap();
    assert_eq!(stock["inputs"], json!(["slew_cv", "signal", "gate"]));
    assert_eq!(
        stock["outputs"],
        json!(["sh1", "sh2", "track_hold", "hold_track", "slew", "glide"])
    );
    assert_eq!(stock["model"], "functional");
    assert_eq!(legacy["kind"], "process");
    assert_eq!(stock["light_names"], json!(["gate"]));
    assert_eq!(stock["params"]["slew"]["randomization"], "linear");
}

#[test]
fn public_actions_and_raw_gate_setter_change_the_actual_process_instance() {
    let mut engine = patch(48_000).compile().unwrap();
    let source = engine.resolve_module_state("source").unwrap();
    let process = engine.resolve_module_state("process").unwrap();
    let mut samples = [PortFrame::default(); 3];
    engine.process_poly_block(&mut samples);
    assert!(!engine.stock_process_snapshot(process).unwrap().engines[0].state);
    assert!(engine.handle_module_event(process, ModuleAction::GatePress));
    engine.process_poly_block(&mut samples[..1]);
    let high = engine.stock_process_snapshot(process).unwrap();
    assert!(high.engines[0].state);
    assert_eq!(high.gate_param, 1.0);
    assert_eq!(engine.module_lights(process).unwrap().brightness[0][0], 1.0);
    assert_eq!(
        engine.host_module_snapshot(process).unwrap().output_count,
        6
    );
    assert_eq!(engine.host_module_snapshot(process).unwrap().input_count, 3);

    assert!(engine.set_stock_process_gate_value(process, 0.25));
    assert_eq!(
        engine
            .stock_process_snapshot_bits(process)
            .unwrap()
            .parameter_bits[1],
        0.25_f32.to_bits()
    );
    assert!(engine.handle_module_event(process, ModuleAction::GateRelease));
    engine.process_poly_block(&mut samples[..1]);
    assert_eq!(
        engine.stock_process_snapshot(process).unwrap().gate_param,
        0.0
    );
    assert_eq!(engine.module_lights(process).unwrap().brightness[0][0], 0.0);
    assert_eq!(engine.stock_process_legacy_data(process), Some(json!(null)));
    assert!(engine.load_stock_process_legacy_data(process, &json!({"ignored":true})));
    assert!(engine.set_host_reference_source(source, PortFrame::mono(3.0)));
    engine.process_poly_block(&mut samples[..2]);
    assert_eq!(
        engine.host_module_snapshot(process).unwrap().outputs[0].values[0],
        2.0
    );
}

#[test]
fn in_place_rate_event_keeps_process_and_raw_host_storage_then_initialize_keeps_engines() {
    let mut old = patch(48_000).compile().unwrap();
    let old_source = old.resolve_module_state("source").unwrap();
    let old_process = old.resolve_module_state("process").unwrap();
    assert!(old.set_host_reference_source(old_source, PortFrame::mono(3.0)));
    let mut samples = [PortFrame::default(); 4];
    old.process_poly_block(&mut samples);
    assert!(old.set_stock_process_gate_value(old_process, 0.25));
    old.process_poly_block(&mut samples[..1]);
    let before: StockProcessSnapshotBits = old.stock_process_snapshot_bits(old_process).unwrap();
    let raw_before = old.host_module_snapshot(old_process).unwrap();
    let processed = old.port_light_processed_frames();

    assert!(!old.stock_process_sample_rate_event(0));
    assert_eq!(old.host_reference_timing().unwrap().0, 48_000);
    assert!(old.stock_process_sample_rate_event(96_000));
    let rated = old.stock_process_snapshot_bits(old_process).unwrap();
    assert_eq!(rated.engines, before.engines);
    assert_eq!(rated.parameter_bits, before.parameter_bits);
    assert_eq!(rated.light_bits, before.light_bits);
    assert_eq!(rated.sample_rate, 96_000);
    assert_eq!(rated.sample_time_bits, (1.0_f32 / 96_000.0).to_bits());
    assert_eq!(old.host_module_snapshot(old_process).unwrap(), raw_before);
    assert_eq!(old.port_light_processed_frames(), processed);
    assert_eq!(
        old.get_host_reference_source(old_source).unwrap().values[0],
        3.0
    );

    let mut next_patch = patch(96_000);
    next_patch
        .nodes
        .iter_mut()
        .find(|node| node.id == "process")
        .unwrap()
        .params
        .insert("slew".into(), 0.0);
    let mut next = next_patch.compile().unwrap();
    next.adopt_state_from(&old);
    let process = next.resolve_module_state("process").unwrap();
    let source = next.resolve_module_state("source").unwrap();
    let after = next.stock_process_snapshot_bits(process).unwrap();
    assert_eq!(after.engines, before.engines);
    assert_eq!(after.parameter_bits[0], 0.0_f32.to_bits());
    assert_eq!(after.parameter_bits[1], 0.25_f32.to_bits());
    assert_eq!(after.sample_rate, 96_000);
    assert_eq!(next.host_module_snapshot(process).unwrap(), raw_before);
    assert_eq!(
        next.get_host_reference_source(source).unwrap().values[0],
        2.0
    );
    assert_eq!(next.port_light_processed_frames(), processed);

    let actions = ModuleParameterActions::new(96_000);
    let node = next_patch
        .nodes
        .iter_mut()
        .find(|node| node.id == "process")
        .unwrap();
    actions.initialize(node).unwrap();
    let mut initialized = next_patch.compile().unwrap();
    initialized.adopt_state_from(&next);
    let handle = initialized.resolve_module_state("process").unwrap();
    assert!(initialized.initialize_module_state(handle));
    let init = initialized.stock_process_snapshot_bits(handle).unwrap();
    assert_eq!(init.engines, after.engines);
    assert_eq!(init.parameter_bits[1], 0.0_f32.to_bits());
    assert_eq!(init.parameter_bits[0], 1e-3_f32.log2().to_bits());
}

#[test]
fn cross_rate_rebuild_stays_cold_for_every_kind_and_receiving_constant_wins() {
    let mut old_patch = patch(48_000);
    old_patch.nodes.push(
        serde_json::from_value(json!({
            "id":"noise","kind":"stock_noise"
        }))
        .unwrap(),
    );
    old_patch.nodes.push(
        serde_json::from_value(json!({
            "id":"noise_sink","kind":"mult"
        }))
        .unwrap(),
    );
    old_patch.cables.push(
        serde_json::from_value(json!({
            "id":"noise_sink_cable","from":"noise","output":"white",
            "to":"noise_sink","input":"signal"
        }))
        .unwrap(),
    );
    let mut old = old_patch.compile().unwrap();
    let source = old.resolve_module_state("source").unwrap();
    let process = old.resolve_module_state("process").unwrap();
    let noise = old.resolve_module_state("noise").unwrap();
    assert!(old.set_host_reference_source(source, PortFrame::mono(99.0)));
    assert!(old.set_stock_process_gate_value(process, 1.0));
    let mut samples = [PortFrame::default(); 5];
    old.process_poly_block(&mut samples);
    assert!(old.stock_process_snapshot_bits(process).unwrap().engines[0].state);
    assert!(old.stock_noise_snapshot(noise).unwrap().normal_draws > 0);

    // The actual in-place rate event rejects an unrelated DSP model without
    // mutating Process, source controls or the global sample rate.
    let before = old.stock_process_snapshot_bits(process).unwrap();
    assert!(!old.stock_process_sample_rate_event(96_000));
    assert_eq!(old.stock_process_snapshot_bits(process).unwrap(), before);
    assert_eq!(old.host_reference_timing().unwrap().0, 48_000);
    assert_eq!(
        old.get_host_reference_source(source).unwrap().values[0],
        99.0
    );

    let mut receiving = old_patch;
    receiving.sample_rate = 96_000;
    receiving
        .nodes
        .iter_mut()
        .find(|node| node.id == "source")
        .unwrap()
        .params
        .insert("v1".into(), 4.0);
    let mut next = receiving.compile().unwrap();
    next.adopt_state_from(&old);
    let next_source = next.resolve_module_state("source").unwrap();
    let next_process = next.resolve_module_state("process").unwrap();
    let next_noise = next.resolve_module_state("noise").unwrap();
    assert_eq!(
        next.get_host_reference_source(next_source).unwrap().values[0],
        4.0
    );
    let fresh = next.stock_process_snapshot_bits(next_process).unwrap();
    assert!(!fresh.engines[0].state);
    assert_eq!(fresh.engines[0].sample1_bits, 0.0_f32.to_bits());
    assert_eq!(fresh.parameter_bits[1], 0.0_f32.to_bits());
    assert_eq!(
        next.stock_noise_snapshot(next_noise).unwrap().normal_draws,
        0
    );
    assert_eq!(next.port_light_processed_frames(), 0);
}

#[test]
fn fresh_reset_restores_all_six_connected_output_ports_to_live_channels() {
    let mut graph = patch(48_000);
    for (index, output) in ["sh1", "sh2", "track_hold", "hold_track", "slew", "glide"]
        .iter()
        .enumerate()
    {
        let sink = format!("sink{index}");
        graph.nodes.push(
            serde_json::from_value(json!({
                "id":sink,"kind":"mult"
            }))
            .unwrap(),
        );
        graph.cables.push(
            serde_json::from_value(json!({
                "id":format!("process_sink{index}"),
                "from":"process","output":output,"to":sink,"input":"signal"
            }))
            .unwrap(),
        );
    }
    let mut engine = graph.compile().unwrap();
    let process = engine.resolve_module_state("process").unwrap();
    let mut samples = [PortFrame::default(); 2];
    engine.process_poly_block(&mut samples);
    assert!(engine.reset_module_state(process));
    let reset = engine.host_module_snapshot(process).unwrap();
    assert_eq!(reset.inputs[1].channels, 1);
    for output in &reset.outputs[..6] {
        assert_eq!(output.channels, 1);
        assert_eq!(output.values, [0.0; 16]);
    }
    // Refill the synchronous source-to-Process cable before the rising edge.
    engine.process_poly_block(&mut samples);
    assert!(engine.set_stock_process_gate_value(process, 1.0));
    engine.process_poly_block(&mut samples[..1]);
    let resumed = engine.host_module_snapshot(process).unwrap();
    for output in &resumed.outputs[..6] {
        assert_eq!(output.channels, 1);
    }
    assert_eq!(resumed.outputs[0].values[0], 2.0);
}
