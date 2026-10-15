use super::*;
fn synth(engine: usize, rate: u32) -> MacroVoice {
    let node:Node=serde_json::from_value(json!({"id":"macro","kind":"macro_voice","params":{"engine":engine,"frequency":110,"harmonics":0.38,"timbre":0.63,"morph":0.41}})).unwrap();
    MacroVoice::build(&node, rate).unwrap()
}
fn step(s: &mut MacroVoice, inputs: &[PortFrame; 7]) -> [PortFrame; 2] {
    let mut outputs = [PortFrame::default(); 2];
    s.process(inputs, &mut outputs);
    outputs
}

#[test]
fn engines_are_audible_distinct_and_bounded_at_supported_rates() {
    for rate in [8000, 44100, 48000, 96000, 192000] {
        let mut signals = Vec::new();
        for engine in 0..8 {
            let mut s = synth(engine, rate);
            let input = [PortFrame::default(); 7];
            let mut samples = Vec::new();
            for frame in 0..8192 {
                let output = step(&mut s, &input);
                assert!(output.iter().all(|p| p.channels == 1
                    && p.values.iter().all(|v| v.is_finite() && v.abs() <= 10.0)));
                if frame >= 4096 {
                    samples.push(output[0].values[0]);
                }
            }
            let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
            assert!(rms > 0.01, "engine {engine} at {rate} is silent: {rms}");
            signals.push(samples);
        }
        for a in 0..8 {
            for b in a + 1..8 {
                let difference = signals[a]
                    .iter()
                    .zip(&signals[b])
                    .map(|(x, y)| (x - y).abs())
                    .sum::<f32>()
                    / signals[a].len() as f32;
                assert!(
                    difference > 0.03,
                    "engines {a} and {b} must have distinct sounds, rate {rate}"
                );
            }
        }
    }
}

#[test]
fn sixteen_voices_retire_cleanly_and_control_cvs_do_not_create_voices() {
    let mut s = synth(2, 48000);
    let mut inputs = [PortFrame::default(); 7];
    inputs[0] = PortFrame {
        channels: 16,
        values: std::array::from_fn(|i| i as f32 / 12.0),
    };
    for _ in 0..1000 {
        let out = step(&mut s, &inputs);
        assert_eq!(out[0].channels, 16);
    }
    inputs[0] = PortFrame::mono(0.0);
    inputs[2] = PortFrame {
        channels: 16,
        values: [0.0; 16],
    };
    let output = step(&mut s, &inputs);
    assert_eq!(output[0].channels, 1);
    assert_eq!(output[0].values[1..], [0.0; 15]);
    let mut fresh = synth(2, 48000);
    for voice in 1..16 {
        assert_eq!(s.voices[voice].phase, fresh.voices[voice].phase);
    }
    inputs[0].channels = 16;
    let output = step(&mut s, &inputs);
    let baseline = step(&mut fresh, &inputs);
    assert_eq!(output[0].values[1..], baseline[0].values[1..]);
}

#[test]
fn unrelated_graph_adoption_preserves_audio_sample_for_sample() {
    for engine in 0..8 {
        let mut a = synth(engine, 48000);
        let mut b = synth(engine, 48000);
        let inputs = [PortFrame::default(); 7];
        for _ in 0..4000 {
            step(&mut a, &inputs);
        }
        b.adopt(&a, true);
        for _ in 0..1024 {
            assert_eq!(step(&mut a, &inputs), step(&mut b, &inputs));
        }
        b.reset_state();
        assert_eq!(b.active, 0);
    }
}

#[test]
fn trigger_is_a_real_decay_and_level_zero_silences_both_outputs() {
    let mut s = synth(2, 48000);
    let mut inputs = [PortFrame::default(); 7];
    inputs[1] = PortFrame::mono(0.0);
    for _ in 0..1000 {
        assert_eq!(step(&mut s, &inputs)[0].values[0], 0.0);
    }
    inputs[1].values[0] = 10.0;
    for _ in 0..1000 {
        step(&mut s, &inputs);
    }
    inputs[1].values[0] = 0.0;
    let mut early = 0.0;
    let mut late = 0.0;
    for frame in 0..144000 {
        let value = step(&mut s, &inputs)[0].values[0].abs();
        if frame < 1000 {
            early += value;
        }
        if frame >= 143000 {
            late += value;
        }
    }
    assert!(early > 1.0 && late < early * 0.2);
    inputs[6] = PortFrame::mono(0.0);
    for _ in 0..48000 {
        step(&mut s, &inputs);
    }
    assert!(step(&mut s, &inputs)
        .iter()
        .all(|f| f.values[0].abs() < 0.00001));
}

#[test]
fn invalid_controls_reject_and_nonfinite_cvs_cannot_poison_the_engine() {
    for params in [
        json!({"engine":1.5}),
        json!({"engine":8}),
        json!({"not_a_control":1}),
    ] {
        let node = serde_json::from_value(json!({"id":"bad","kind":"macro_voice","params":params}))
            .unwrap();
        assert!(MacroVoice::build(&node, 48000).is_err());
    }
    for engine in 0..8 {
        let mut s = synth(engine, 48000);
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30, -1e30] {
            let inputs = [PortFrame::mono(value); 7];
            for _ in 0..100 {
                assert!(step(&mut s, &inputs)
                    .iter()
                    .all(|p| p.values.iter().all(|s| s.is_finite())));
            }
        }
    }
}

#[test]
fn catalog_named_ports_compile_and_render_through_the_real_graph() {
    let patch:crate::Patch=serde_json::from_value(json!({"version":2,"nodes":[{"id":"voice","kind":"macro_voice"},{"id":"out","kind":"audio_stereo"}],"cables":[{"from":"voice","output":"out","to":"out","input":"left"},{"from":"voice","output":"aux","to":"out","input":"right"}],"output":"out"})).unwrap();
    let mut e = patch.compile().unwrap();
    let mut samples = [[0.0; 2]; 4096];
    e.process_audio_block(&mut samples);
    assert!(samples.iter().any(|s| s[0].abs() > 0.01));
    assert!(samples.iter().any(|s| s[0] != s[1]));
}

#[test]
fn sine_lookup_wraps_signed_phase_boundaries_without_panicking() {
    let table = &tables().sine;
    // The first values rounded to index TABLE_SIZE before hardening, causing
    // the interpolation's second read to leave the table on the audio thread.
    for phase in [
        -f64::from_bits(1),
        -f64::MIN_POSITIVE,
        -1e-20,
        -f64::EPSILON * 0.25,
        0.0,
        1.0 - f64::EPSILON,
        1.0,
        1.0 + f64::EPSILON,
        -1.0 - f64::EPSILON,
        -1.0,
        -1.0 + f64::EPSILON,
    ] {
        let actual = sin(phase, table);
        let expected = (TAU * phase).sin();
        assert!(actual.is_finite());
        assert!(
            (f64::from(actual) - expected).abs() < 1e-6,
            "phase {phase}: {actual} versus analytic sine {expected}"
        );
    }
    // Exercise signed fractional phases as supplied by FM and wavefolding,
    // preserving the existing interpolation accuracy across multiple periods.
    for step in -4096..=4096 {
        let phase = f64::from(step) / 997.0;
        assert!((f64::from(sin(phase, table)) - (TAU * phase).sin()).abs() < 1e-6);
    }
}
