use openpatch_core::{Cable, Node, Patch};
use std::f64::consts::TAU;

fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        params: params
            .iter()
            .map(|(key, value)| (key.to_string(), *value))
            .collect(),
    }
}

fn tone() -> Patch {
    Patch {
        nodes: vec![node(
            "osc",
            "oscillator",
            &[("frequency", 300.0), ("amplitude", 0.75)],
        )],
        output: Some("osc".into()),
        ..Patch::default()
    }
}

#[test]
fn frequency_and_wiring_edits_keep_phase_by_id_and_leave_old_branch_unchanged() {
    let mut patch = tone();
    let mut previous = patch.compile().unwrap();
    previous.process_block(&mut [0.0; 37]);
    patch.nodes[0].params.insert("frequency".into(), 900.0);
    patch.nodes[0].params.insert("amplitude".into(), 0.7);
    // A new source changes the oscillator's topological index as well as its FM.
    patch
        .nodes
        .insert(0, node("mod", "constant", &[("value", 16.0)]));
    patch.cables.push(Cable {
        from: "mod".into(),
        to: "osc".into(),
        input: "fm".into(),
    });
    let mut replacement = patch.compile().unwrap();
    replacement.adopt_state_from(&previous);
    let mut samples = [0.0; 64];
    replacement.process_block(&mut samples);
    let start_phase = 37.0 * 300.0 / patch.sample_rate as f64;
    for (index, sample) in samples.iter().enumerate() {
        let phase = start_phase + index as f64 * 916.0 / patch.sample_rate as f64;
        let expected = (phase * TAU).sin() as f32 * 0.7;
        assert!(
            (sample - expected).abs() < 0.000_002,
            "sample {index}: {sample} vs {expected}"
        );
    }
    let mut old_samples = [0.0; 64];
    previous.process_block(&mut old_samples);
    for (index, sample) in old_samples.iter().enumerate() {
        let phase = start_phase + index as f64 * 300.0 / patch.sample_rate as f64;
        assert!((sample - (phase * TAU).sin() as f32 * 0.75).abs() < 0.000_002);
    }
}

#[test]
fn cutoff_edit_preserves_filter_memory_and_uses_new_coefficient() {
    let mut patch = Patch {
        nodes: vec![
            node("dc", "constant", &[("value", 1.0)]),
            node("filter", "lowpass", &[("cutoff", 100.0)]),
        ],
        cables: vec![Cable {
            from: "dc".into(),
            to: "filter".into(),
            input: "signal".into(),
        }],
        output: Some("filter".into()),
        ..Patch::default()
    };
    let mut previous = patch.compile().unwrap();
    let mut warmup = [0.0; 200];
    previous.process_block(&mut warmup);
    let memory = *warmup.last().unwrap();
    assert!(memory > 0.9);
    patch.nodes[0].params.insert("value".into(), 0.0);
    patch.nodes[1].params.insert("cutoff".into(), 200.0);
    let mut replacement = patch.compile().unwrap();
    replacement.adopt_state_from(&previous);
    let mut output = [0.0; 1];
    replacement.process_block(&mut output);
    let coefficient = -(-TAU * 200.0 / patch.sample_rate as f64).exp_m1() as f32;
    assert!((output[0] - memory * (1.0 - coefficient)).abs() < 0.000_001);
    assert!(output[0] > 0.8, "filter memory was reset");
    previous.process_block(&mut output);
    assert!(
        (output[0]
            - (memory
                + (-(-TAU * 100.0 / patch.sample_rate as f64).exp_m1() as f32) * (1.0 - memory)))
            .abs()
            < 0.000_001
    );
}

#[test]
fn renamed_ids_and_replaced_models_start_with_fresh_state() {
    let patch = tone();
    let mut previous = patch.compile().unwrap();
    previous.process_block(&mut [0.0; 37]);
    let mut renamed = patch.clone();
    renamed.nodes[0].id = "new-osc".into();
    renamed.output = Some("new-osc".into());
    let mut replacement = renamed.compile().unwrap();
    replacement.adopt_state_from(&previous);
    let mut output = [1.0; 1];
    replacement.process_block(&mut output);
    assert_eq!(output[0], 0.0);

    // LFO and oscillator share the same DSP enum variant, but are different models.
    let mut lfo = patch.clone();
    lfo.nodes[0] = node(
        "osc",
        "lfo",
        &[("frequency", 1.0), ("amplitude", 1.0), ("offset", 0.0)],
    );
    let mut replacement = lfo.compile().unwrap();
    replacement.adopt_state_from(&previous);
    replacement.process_block(&mut output);
    assert_eq!(output[0], 0.0);
}
