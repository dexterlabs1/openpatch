use openpatch_core::*;
use serde_json::json;
fn patch(width: u8, output: Option<&str>) -> Patch {
    let mut p: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"source","kind":"poly_constant","params":{"channels":width,"v1":2.0,"v2":-1.0,"v5":3.0,"v16":-2.0}},
                 {"id":"filter","kind":"stock_vcf","params":{"frequency":0.7,"resonance":0.35}}],
        "cables":[{"from":"source","to":"filter","input":"signal"}]})).unwrap();
    if let Some(port) = output {
        p.output = Some("filter".into());
        p.output_port = port.into();
    }
    p.assign_missing_cable_ids().unwrap();
    p
}
fn trace(engine: &mut Engine, n: usize, ports: [Option<&str>; 2]) -> Vec<[TapSample; 2]> {
    let taps = ports.map(|p| p.map(|p| engine.resolve_tap("filter", p, 0).unwrap()));
    let mut samples = vec![[TapSample::default(); 2]; n];
    engine.process_audio_block_with_taps(&mut vec![[0.0; 2]; n], &taps, &mut samples);
    samples
}
#[test]
fn catalog_exposes_six_controls_four_inputs_two_outputs_and_audio_polyphony() {
    let c = catalog();
    let m = c["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "stock_vcf")
        .unwrap();
    assert_eq!(
        m["inputs"],
        json!(["cutoff_cv", "resonance_cv", "drive_cv", "signal"])
    );
    assert_eq!(m["outputs"], json!(["lowpass", "highpass"]));
    assert_eq!(m["params"].as_object().unwrap().len(), 6);
    assert_eq!(m["polyphony"]["primary_input"], "signal");
    assert_eq!(m["min_patch_version"], 2);
    assert_eq!(m["params"]["frequency"]["default"], 0.5);
    assert_eq!(m["params"]["resonance"]["default"], 0.0);
    assert_eq!(m["params"]["drive"]["default"], 0.0);
    for name in ["cutoff_cv_amount", "resonance_cv_amount", "drive_cv_amount"] {
        assert_eq!(m["params"][name]["min"], -1.0);
        assert_eq!(m["params"][name]["max"], 1.0);
        assert_eq!(m["params"][name]["default"], 0.0);
    }
    let mut bad = patch(1, Some("lowpass"));
    bad.version = 1;
    assert!(bad.compile().is_err());
    for (name, value) in [
        ("resonance", 1.01),
        ("drive", -1.01),
        ("cutoff_cv_amount", 1.01),
        ("frequency", f32::NAN),
        ("unknown", 0.0),
    ] {
        let mut bad = patch(1, Some("lowpass"));
        bad.nodes[1].params.insert(name.into(), value);
        assert!(bad.compile().is_err(), "{name}");
    }
}
#[test]
fn selected_output_and_temporary_taps_are_real_consumers() {
    for port in ["lowpass", "highpass"] {
        let mut selected = patch(1, Some(port)).compile().unwrap();
        let mut captured = patch(1, None).compile().unwrap();
        let mut output = [PortFrame::default(); 257];
        selected.process_poly_block(&mut output);
        let samples = trace(&mut captured, 257, [Some(port), None]);
        for (a, b) in output.iter().zip(samples) {
            assert!(b[0].active);
            assert_eq!(a.values[0], b[0].value);
        }
        assert!(output.iter().any(|s| s.values[0].abs() > 0.01));
    }
}
#[test]
fn cable_consumers_work_without_selected_filter_or_capture() {
    let mut p = patch(1, None);
    p.nodes
        .push(serde_json::from_value(json!({"id":"sink","kind":"gain"})).unwrap());
    p.cables.push(
        serde_json::from_value(
            json!({"from":"filter","output":"lowpass","to":"sink","input":"signal"}),
        )
        .unwrap(),
    );
    p.output = Some("sink".into());
    p.output_port = "out".into();
    p.assign_missing_cable_ids().unwrap();
    let mut engine = p.compile().unwrap();
    let mut samples = [0.0; 257];
    engine.process_block(&mut samples);
    assert!(samples.iter().any(|v| v.abs() > 0.01));
}
#[test]
fn unconsumed_frames_freeze_dsp_but_input_cables_continue_delivering() {
    let p = patch(1, None);
    let mut idle = p.compile().unwrap();
    let mut ready = p.compile().unwrap();
    idle.process_block(&mut [0.0; 4097]);
    ready.process_block(&mut [0.0; 1]);
    // Both now have the same delayed constant input, and neither filter ran.
    assert_eq!(
        trace(&mut idle, 513, [Some("lowpass"), Some("highpass")]),
        trace(&mut ready, 513, [Some("lowpass"), Some("highpass")])
    );
    idle.process_block(&mut [0.0; 1025]);
    assert_eq!(
        trace(&mut idle, 129, [Some("lowpass"), Some("highpass")]),
        trace(&mut ready, 129, [Some("lowpass"), Some("highpass")])
    );
}
#[test]
fn disconnected_output_preserves_its_decimator_and_receiving_mask_survives_adoption() {
    let mut lp = patch(1, Some("lowpass")).compile().unwrap();
    let mut both = patch(1, Some("lowpass")).compile().unwrap();
    trace(&mut lp, 193, [Some("lowpass"), None]);
    trace(&mut both, 193, [Some("lowpass"), Some("highpass")]);
    let a = trace(&mut lp, 32, [Some("lowpass"), Some("highpass")]);
    let b = trace(&mut both, 32, [Some("lowpass"), Some("highpass")]);
    for (a, b) in a.iter().zip(&b) {
        assert_eq!(a[0], b[0]);
    }
    assert!(a.iter().zip(&b).any(|(a, b)| a[1].value != b[1].value));
    let mut receiving = patch(1, Some("highpass")).compile().unwrap();
    receiving.adopt_state_from(&lp);
    let mut output = [PortFrame::default(); 129];
    receiving.process_poly_block(&mut output);
    let expected = trace(&mut lp, 129, [None, Some("highpass")]);
    for (a, b) in output.iter().zip(expected) {
        assert_eq!(a.values[0], b[1].value);
    }
}
#[test]
fn partitioning_adoption_initialization_and_polyphonic_bypass_are_explicit() {
    let p = patch(16, Some("lowpass"));
    let mut a = p.compile().unwrap();
    let mut b = p.compile().unwrap();
    let expected = trace(&mut a, 2049, [Some("lowpass"), Some("highpass")]);
    let mut actual = Vec::new();
    for n in [0, 1, 3, 127, 128, 129, 513, 1025, 123] {
        actual.extend(trace(&mut b, n, [Some("lowpass"), Some("highpass")]));
    }
    assert_eq!(expected, actual);
    let mut adopted = p.compile().unwrap();
    adopted.adopt_state_from(&a);
    assert_eq!(
        trace(&mut adopted, 257, [Some("lowpass"), Some("highpass")]),
        trace(&mut a, 257, [Some("lowpass"), Some("highpass")])
    );
    let h = adopted.resolve_module_state("filter").unwrap();
    assert!(adopted.reset_module_state(h));
    let mut fresh = p.compile().unwrap();
    assert_eq!(
        trace(&mut adopted, 257, [Some("lowpass"), Some("highpass")]),
        trace(&mut fresh, 257, [Some("lowpass"), Some("highpass")])
    );
    let mut bypass = p.clone();
    bypass.nodes[1].bypassed = true;
    let mut bypass = bypass.compile().unwrap();
    let samples = trace(&mut bypass, 3, [Some("lowpass"), Some("highpass")]);
    for frame in &samples[1..] {
        assert_eq!(frame[0].value, 2.0);
        assert_eq!(frame[1].value, 2.0);
    }
    let mut frames = [PortFrame::default(); 1];
    bypass.process_poly_block(&mut frames);
    assert_eq!(frames[0].channels, 16);
    assert_eq!(frames[0].values[15], -2.0);
}
