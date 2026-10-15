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

// These fixtures exercise the public graph/callback boundary. They deliberately
// do not copy oscillator formulas or inspect private voice histories.
fn contract_patch(engine: usize, rate: u32, inputs: &[(&str, Vec<f32>)]) -> crate::Patch {
    let mut nodes = vec![json!({"id":"voice","kind":"macro_voice","params":{
        "engine":engine,"frequency":110,"harmonics":0.38,"timbre":0.63,"morph":0.41}})];
    let mut cables = Vec::new();
    for (port, values) in inputs {
        let mut params = json!({"channels":values.len()});
        for (i, value) in values.iter().enumerate() {
            params[format!("v{}", i + 1)] = json!(value);
        }
        let id = format!("source_{port}");
        nodes.push(json!({"id":id,"kind":"poly_constant","params":params}));
        cables.push(
            json!({"id":format!("wire_{port}"),"from":id,"output":"out","to":"voice","input":port}),
        );
    }
    serde_json::from_value(json!({"version":2,"sample_rate":rate,
        "nodes":nodes,"cables":cables,"output":"voice"}))
    .unwrap()
}

fn modulated_contract_patch(engine: usize) -> crate::Patch {
    let mut patch = contract_patch(
        engine,
        48000,
        &[("pitch", vec![-0.5, 0.0, 0.25, 0.75]), ("level", vec![8.0])],
    );
    for (id, kind, params, port) in [
        (
            "slow",
            "lfo",
            json!({"frequency":7,"amplitude":0.9,"offset":0.1}),
            "timbre",
        ),
        (
            "fast",
            "lfo",
            json!({"frequency":31,"amplitude":0.3,"offset":0}),
            "fm",
        ),
        (
            "clock",
            "gate",
            json!({"frequency":67,"duty":0.25}),
            "trigger",
        ),
    ] {
        patch
            .nodes
            .push(serde_json::from_value(json!({"id":id,"kind":kind,"params":params})).unwrap());
        patch.cables.push(
            serde_json::from_value(json!({"id":format!("wire_{id}"),
            "from":id,"to":"voice","input":port}))
            .unwrap(),
        );
    }
    patch
}

fn contract_frames(
    engine: &mut crate::Engine,
    count: usize,
    partitions: &[usize],
) -> Vec<PortFrame> {
    let mut frames = vec![PortFrame::default(); count];
    let mut start = 0;
    for length in partitions.iter().copied().cycle() {
        let end = (start + length).min(count);
        engine.process_poly_block(&mut frames[start..end]);
        start = end;
        if start == count {
            break;
        }
    }
    frames
}

fn assert_frame_bits(actual: &[PortFrame], expected: &[PortFrame]) {
    assert_eq!(actual.len(), expected.len());
    for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(a.channels, b.channels, "width at frame {i}");
        assert_eq!(
            a.values.map(f32::to_bits),
            b.values.map(f32::to_bits),
            "samples at frame {i}"
        );
    }
}

fn primed_contract_engine(patch: &crate::Patch) -> crate::Engine {
    // Acquire constant source cable frames without advancing the voice. All
    // lanes then begin together, rather than mono references starting one
    // sample before poly lanes activated by the first delayed pitch frame.
    let mut bypassed = patch.clone();
    bypassed
        .nodes
        .iter_mut()
        .find(|n| n.kind == "macro_voice")
        .unwrap()
        .bypassed = true;
    let mut sources = bypassed.compile().unwrap();
    sources.process_poly_block(&mut [PortFrame::default()]);
    let mut engine = patch.compile().unwrap();
    engine.adopt_state_from(&sources);
    engine
}

#[test]
fn public_callbacks_are_partition_bit_identical_with_modulated_polyphonic_inputs() {
    for engine in 0..8 {
        for port in ["out", "aux"] {
            let mut patch = modulated_contract_patch(engine);
            patch.output_port = port.into();
            let expected = contract_frames(&mut patch.compile().unwrap(), 1537, &[1537]);
            let actual = contract_frames(
                &mut patch.compile().unwrap(),
                1537,
                &[0, 1, 3, 23, 24, 63, 64, 65, 127, 128, 129],
            );
            assert_frame_bits(&actual, &expected);
            assert!(actual[1..].iter().all(|frame| frame.channels == 4));
            assert!(actual.iter().any(|frame| frame.values[0].abs() > 0.01));
            // The public stereo adapter must see the same stream under its
            // own callback partitioning, including zero-sized callbacks.
            let mut audio_engine = patch.compile().unwrap();
            let mut audio = vec![[0.0; 2]; actual.len()];
            audio_engine.process_audio_block(&mut []);
            for chunk in audio.chunks_mut(97) {
                audio_engine.process_audio_block(chunk);
            }
            for (sample, frame) in audio.iter().zip(&expected) {
                assert_eq!(sample.map(f32::to_bits), [frame.values[0].to_bits(); 2]);
            }
        }
    }
}

#[test]
fn public_graph_adoption_preserves_same_identity_engine_rate_and_modulation_histories() {
    for engine in 0..8 {
        for port in ["out", "aux"] {
            let mut patch = modulated_contract_patch(engine);
            patch.output_port = port.into();
            let mut old = patch.compile().unwrap();
            contract_frames(&mut old, 1013, &[61, 128]);
            // Force a real changed graph without changing any voice input.
            patch.nodes.insert(
                0,
                serde_json::from_value(
                    json!({"id":"unrelated","kind":"constant","params":{"value":3}}),
                )
                .unwrap(),
            );
            let mut adopted = patch.compile().unwrap();
            adopted.adopt_state_from(&old);
            let expected = contract_frames(&mut old, 521, &[521]);
            let actual = contract_frames(&mut adopted, 521, &[1, 63, 64, 129]);
            assert_frame_bits(&actual, &expected);
        }
    }
}

#[test]
fn public_engine_rate_identity_and_incarnation_changes_restart_against_fresh_baselines() {
    // No incoming edges: selector changes may preserve compatible cable
    // frames, which are a different lifetime from the synthesis histories.
    for port in ["out", "aux"] {
        let mut patch = contract_patch(2, 48000, &[]);
        patch.output_port = port.into();
        let mut old = patch.compile().unwrap();
        contract_frames(&mut old, 1237, &[1237]);
        for change in ["engine", "rate", "identity", "incarnation"] {
            let mut replacement = patch.clone();
            match change {
                "engine" => {
                    replacement.nodes[0].params.insert("engine".into(), 3.0);
                }
                "rate" => replacement.sample_rate = 96000,
                "identity" => {
                    replacement.nodes[0].id = "replacement".into();
                    replacement.output = Some("replacement".into());
                }
                _ => {}
            }
            let mut actual = replacement.compile().unwrap();
            if change == "incarnation" {
                let handle = actual.resolve_module_state("voice").unwrap();
                actual.adopt_state_from_excluding(&old, &[handle]);
            } else {
                actual.adopt_state_from(&old);
            }
            let expected = contract_frames(&mut replacement.compile().unwrap(), 257, &[257]);
            assert_frame_bits(&contract_frames(&mut actual, 257, &[1, 64, 63]), &expected);
            assert!(expected.iter().any(|frame| frame.values[0].abs() > 0.01));
        }
    }
}

#[test]
fn changed_trigger_route_rearms_only_the_edge_while_preserving_synthesis_history() {
    for engine in [1, 2, 3, 4] {
        for port in ["out", "aux"] {
            let mut patch = contract_patch(engine, 48000, &[("trigger", vec![10.0])]);
            patch.output_port = port.into();
            patch.nodes.push(
                serde_json::from_value(
                    json!({"id":"other_trigger","kind":"constant","params":{"value":10}}),
                )
                .unwrap(),
            );
            let mut held = patch.compile().unwrap();
            contract_frames(&mut held, 4093, &[4093]);
            // Release the reference detector without changing the envelope:
            // gate level does not sustain this instrument's decaying LPG.
            let mut low_patch = patch.clone();
            low_patch
                .nodes
                .iter_mut()
                .find(|n| n.id == "source_trigger")
                .unwrap()
                .params
                .insert("v1".into(), 0.0);
            let mut released = low_patch.compile().unwrap();
            released.adopt_state_from(&held);
            let held_release_frames = contract_frames(&mut held, 2, &[2]);
            assert_frame_bits(
                &contract_frames(&mut released, 2, &[1]),
                &held_release_frames,
            );
            // Explicitly retire only the reference cable history. Like the
            // moved route it initially has zero channels (unpatched normal),
            // then acquires high; its already released detector is independent
            // of the route-change detector reset under test.
            let mut reference = patch.compile().unwrap();
            let input = reference.resolve_cable_state("wire_trigger").unwrap();
            reference.adopt_state_from_excluding_routes(&released, &[], &[input]);
            let mut moved_patch = patch.clone();
            moved_patch.cables[0].from = "other_trigger".into();
            let mut moved = moved_patch.compile().unwrap();
            moved.adopt_state_from(&held);
            let expected = contract_frames(&mut reference, 521, &[521]);
            let actual = contract_frames(&mut moved, 521, &[1, 64, 63]);
            assert_frame_bits(&actual, &expected);
            let unchanged = contract_frames(&mut held, 521, &[521]);
            assert!(
                actual
                    .iter()
                    .zip(&unchanged)
                    .any(|(a, b)| (a.values[0] - b.values[0]).abs() > 0.01),
                "engine {engine} {port}: changed high route must retrigger, held high must not"
            );
        }
    }
}

#[test]
fn zero_width_trigger_and_level_normalize_but_connected_zero_volts_do_not() {
    for port in ["out", "aux"] {
        let mut base = contract_patch(2, 48000, &[]);
        base.output_port = port.into();
        let expected = contract_frames(&mut primed_contract_engine(&base), 2049, &[2049]);
        assert!(expected.iter().any(|f| f.values[0].abs() > 0.01));
        for input in ["trigger", "level"] {
            let mut empty = contract_patch(2, 48000, &[(input, vec![])]);
            empty.output_port = port.into();
            assert_frame_bits(
                &contract_frames(&mut primed_contract_engine(&empty), 2049, &[31, 64]),
                &expected,
            );
            let mut zero = contract_patch(2, 48000, &[(input, vec![0.0])]);
            zero.output_port = port.into();
            let silence = contract_frames(&mut primed_contract_engine(&zero), 2049, &[31, 64]);
            assert!(
                silence
                    .iter()
                    .all(|f| f.channels == 1 && f.values == [0.0; 16]),
                "connected zero {input} {port} must be silent from a fresh primed voice"
            );
        }
        let mut ten = contract_patch(2, 48000, &[("level", vec![10.0])]);
        ten.output_port = port.into();
        assert_frame_bits(
            &contract_frames(&mut primed_contract_engine(&ten), 2049, &[127]),
            &expected,
        );
    }
}

#[test]
fn polyphonic_mono_and_short_cv_inputs_match_independent_lanes_without_cross_voice_state() {
    for engine in [0, 2, 3, 5] {
        for port in ["out", "aux"] {
            for variant in 0..3 {
                let inputs = vec![
                    (
                        "pitch",
                        if variant == 1 {
                            vec![-0.5]
                        } else {
                            vec![-0.5, 0.0, 0.25, 0.75]
                        },
                    ),
                    (
                        "trigger",
                        match variant {
                            1 => vec![10.0; 4],
                            2 => vec![10.0, 0.0],
                            _ => vec![10.0],
                        },
                    ),
                    ("harmonics", vec![1.25]),
                    ("timbre", vec![-0.75, 2.25]),
                    ("morph", vec![1.0, -2.0, 0.5]),
                    ("fm", vec![-0.2]),
                    (
                        "level",
                        if variant == 1 {
                            vec![10.0, 5.0]
                        } else {
                            vec![7.0]
                        },
                    ),
                ];
                let mut patch = contract_patch(engine, 48000, &inputs);
                patch.output_port = port.into();
                let actual = contract_frames(&mut primed_contract_engine(&patch), 521, &[521]);
                assert!(actual
                    .iter()
                    .all(|frame| frame.channels == 4 && frame.values[4..] == [0.0; 12]));
                for lane in 0..4 {
                    let mono: Vec<_> = inputs
                        .iter()
                        .map(|(name, values)| {
                            let value = if values.len() == 1 {
                                values[0]
                            } else {
                                values.get(lane).copied().unwrap_or(0.0)
                            };
                            (*name, vec![value])
                        })
                        .collect();
                    let mut independent = contract_patch(engine, 48000, &mono);
                    independent.output_port = port.into();
                    let expected =
                        contract_frames(&mut primed_contract_engine(&independent), 521, &[1, 64]);
                    for (a, b) in actual.iter().zip(expected) {
                        assert_eq!(
                            a.values[lane].to_bits(),
                            b.values[0].to_bits(),
                            "engine {engine} {port} lane {lane}"
                        );
                    }
                }
                // A single lane's CV edit must not disturb another lane's
                // oscillator, FIR, spectral, LPG or DC history.
                let mut changed = inputs.clone();
                if changed[0].1.len() == 1 {
                    changed[0].1 = vec![changed[0].1[0]; 4];
                }
                changed[0].1[0] = 0.8;
                let mut changed_patch = contract_patch(engine, 48000, &changed);
                changed_patch.output_port = port.into();
                let changed =
                    contract_frames(&mut primed_contract_engine(&changed_patch), 521, &[63, 64]);
                assert!(actual
                    .iter()
                    .zip(&changed)
                    .any(|(a, b)| a.values[0].to_bits() != b.values[0].to_bits()));
                for (a, b) in actual.iter().zip(changed) {
                    for lane in 1..MAX_CHANNELS {
                        assert_eq!(a.values[lane].to_bits(), b.values[lane].to_bits());
                    }
                }
            }
        }
    }
}
