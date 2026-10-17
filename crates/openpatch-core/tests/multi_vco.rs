//! Public contracts for the new version-2 functional four-output VCO.
use openpatch_core::{Cable, Node, Patch, PortFrame, TapSample};
use std::collections::BTreeMap;

fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        content: None,
        rackmcp_sequence: None,
        score: None,
        id: id.into(),
        kind: kind.into(),
        bypassed: false,
        params: params
            .iter()
            .map(|(key, value)| (key.to_string(), *value))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn cable(from: &str, to: &str, input: &str) -> Cable {
    Cable {
        input_order: None,
        output_order: None,
        color: None,
        id: None,
        from: from.into(),
        output: "out".into(),
        to: to.into(),
        input: input.into(),
    }
}

fn patch(params: &[(&str, f32)], output: &str) -> Patch {
    Patch {
        nodes: vec![node("vco", "multi_vco", params)],
        output: Some("vco".into()),
        output_port: output.into(),
        ..Patch::new_v2()
    }
}

#[test]
fn four_named_outputs_share_a_running_voice_and_have_no_out_alias() {
    let p = patch(&[], "sine");
    let mut engine = p.compile().unwrap();
    let taps = [
        Some(engine.resolve_tap("vco", "triangle", 0).unwrap()),
        Some(engine.resolve_tap("vco", "saw", 0).unwrap()),
    ];
    assert!(engine.resolve_tap("vco", "out", 0).is_err());
    let mut audio = [[0.0; 2]; 128];
    let mut samples = [[TapSample::default(); 2]; 128];
    engine.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    let phase = 261.625_58_f64 / 48_000.0;
    let first_sine = 5.0 * (1.0 - 16.0 * (phase - 0.25).powi(2));
    assert!((f64::from(audio[0][0]) - first_sine).abs() < 0.000_01);
    assert!(audio.iter().all(|frame| frame[0].is_finite()));
    assert!(samples
        .iter()
        .all(|frame| frame.iter().all(|tap| tap.active && tap.value.is_finite())));
    assert!(audio.windows(2).any(|pair| pair[0] != pair[1]));
    assert_ne!(samples[10][0].value, samples[10][1].value);
    let mut square = patch(&[], "square").compile().unwrap();
    let mut frames = [PortFrame::default(); 128];
    square.process_poly_block(&mut frames);
    assert!(frames
        .iter()
        .all(|frame| frame.channels == 1 && frame.values[0].is_finite()));
}

#[test]
fn v2_only_ports_and_flags_validate_without_changing_legacy_vco() {
    let mut p = patch(&[], "sine");
    p.version = 1;
    assert!(p.compile().is_err());
    let mut p = patch(&[], "out");
    assert!(p.compile().is_err());
    p.output_port = "sine".into();
    p.nodes[0].params.insert("soft_sync".into(), 0.5);
    assert!(p.compile().is_err());
    p.nodes[0].params.insert("soft_sync".into(), 1.0);
    p.nodes[0].params.insert("linear_fm".into(), 1.0);
    assert!(p.compile().is_ok());
    p.nodes[0].params.insert("pulse_width".into(), 0.0);
    assert!(p.compile().is_err());
    assert!(openpatch_core::analog_demo_patch().compile().is_ok());
}

#[test]
fn pitch_sets_voices_and_auxiliary_mono_is_broadcast() {
    let mut p = patch(&[("fm_amount", 0.5)], "sine");
    p.nodes.extend([
        node(
            "pitch",
            "poly_constant",
            &[("channels", 2.0), ("v1", 0.0), ("v2", 1.0)],
        ),
        node("fm", "constant", &[("value", 1.0)]),
    ]);
    p.cables
        .extend([cable("pitch", "vco", "pitch"), cable("fm", "vco", "fm")]);
    let mut engine = p.compile().unwrap();
    let mut frames = [PortFrame::default(); 64];
    engine.process_poly_block(&mut frames);
    assert_eq!(frames[0].channels, 1);
    assert!(frames[1..].iter().all(|frame| frame.channels == 2));
    assert!(frames
        .iter()
        .all(|frame| frame.values.iter().all(|value| value.is_finite())));
    assert!(frames[16].values[0] != frames[16].values[1]);
}

#[test]
fn matching_recompile_preserves_phase_and_correction_history_exactly() {
    let p = patch(&[("frequency", 1000.0), ("pulse_width", 0.3)], "saw");
    let mut previous = p.compile().unwrap();
    previous.process_poly_block(&mut [PortFrame::default(); 137]);
    let mut adopted = p.compile().unwrap();
    adopted.adopt_state_from(&previous);
    let mut expected = [PortFrame::default(); 128];
    let mut actual = [PortFrame::default(); 128];
    previous.process_poly_block(&mut expected);
    adopted.process_poly_block(&mut actual);
    assert_eq!(actual, expected);
}

#[test]
fn bypass_clears_all_outputs_and_freezes_vco_history() {
    let p = patch(&[("frequency", 1000.0)], "saw");
    let mut running = p.compile().unwrap();
    running.process_poly_block(&mut [PortFrame::default(); 137]);
    let mut muted_patch = p.clone();
    muted_patch.nodes[0].bypassed = true;
    let mut muted = muted_patch.compile().unwrap();
    muted.adopt_state_from(&running);
    let mut silent = [PortFrame::default(); 31];
    muted.process_poly_block(&mut silent);
    assert!(silent.iter().all(|frame| frame.values[0] == 0.0));
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&muted);
    let mut expected = [PortFrame::default(); 32];
    let mut actual = [PortFrame::default(); 32];
    running.process_poly_block(&mut expected);
    resumed.process_poly_block(&mut actual);
    assert_eq!(actual, expected);
}
