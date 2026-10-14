//! Public-API acceptance tests with analytical phase/waveform expectations.
use openpatch_core::{Cable, Engine, MidiEvent, Node, Patch, PortFrame};
use std::f64::consts::TAU;

fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        score: None,
        bypassed: false,
        id: id.into(),
        kind: kind.into(),
        params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
    }
}
fn source(id: &str, values: &[f32]) -> Node {
    let mut node = node(id, "poly_constant", &[("channels", values.len() as f32)]);
    for (index, value) in values.iter().enumerate() {
        node.params.insert(format!("v{}", index + 1), *value);
    }
    node
}
fn wire(from: &str, output: &str, to: &str, input: &str) -> Cable {
    Cable {
        input_order: None,
        output_order: None,
        color: None,
        id: None,
        from: from.into(),
        output: output.into(),
        to: to.into(),
        input: input.into(),
    }
}
fn lfo(rate: u32, params: &[(&str, f32)], output: &str) -> Patch {
    Patch {
        sample_rate: rate,
        nodes: vec![node("lfo", "multi_lfo", params)],
        output: Some("lfo".into()),
        output_port: output.into(),
        ..Patch::new_v2()
    }
}
fn render(patch: &Patch, count: usize) -> Vec<PortFrame> {
    let mut result = vec![PortFrame::default(); count];
    patch.compile().unwrap().process_poly_block(&mut result);
    result
}
fn close(actual: f32, expected: f64) {
    assert!(
        (f64::from(actual) - expected).abs() < 0.000_003,
        "{actual} != {expected}"
    );
}
fn wave(port: &str, phase: f64, width: f64) -> f64 {
    match port {
        "sine" => (TAU * phase).sin(),
        "triangle" => 1.0 - 4.0 * (phase - 0.5).abs(),
        "saw" => 2.0 * phase - 1.0,
        "square" => {
            if phase < width {
                1.0
            } else {
                -1.0
            }
        }
        _ => unreachable!(),
    }
}
fn note(channel: u8, on: bool) -> MidiEvent {
    if on {
        MidiEvent::NoteOn {
            channel,
            note: 60,
            velocity: 127,
        }
    } else {
        MidiEvent::NoteOff {
            channel,
            note: 60,
            velocity: 0,
        }
    }
}
fn controls(patch: &mut Patch) {
    patch.nodes.extend([
        node("clock", "midi_cv", &[("channel", 1.0), ("channels", 1.0)]),
        node("reset", "midi_cv", &[("channel", 2.0), ("channels", 1.0)]),
    ]);
    patch.cables.extend([
        wire("clock", "gate", "lfo", "clock"),
        wire("reset", "gate", "lfo", "reset"),
    ]);
}
fn driven(engine: &mut Engine, count: usize, events: &[(usize, MidiEvent)]) -> Vec<PortFrame> {
    let mut output = vec![PortFrame::default(); count];
    for (index, frame) in output.iter_mut().enumerate() {
        for (_, event) in events.iter().filter(|(at, _)| *at == index) {
            engine.handle_midi(*event);
        }
        engine.process_poly_block(std::slice::from_mut(frame));
    }
    output
}
fn sine_oracle(
    count: usize,
    rate: u32,
    initial_hz: f64,
    changes: &[(usize, f64)],
    resets: &[usize],
) -> Vec<f64> {
    let mut phase = 0.0;
    let mut frequency = initial_hz;
    (0..count)
        .map(|index| {
            if let Some((_, hz)) = changes.iter().find(|(at, _)| *at == index) {
                frequency = *hz;
            }
            if resets.contains(&index) {
                phase = 0.0;
            }
            let output = 5.0 * (TAU * phase).sin();
            phase = (phase + frequency / f64::from(rate)).fract();
            output
        })
        .collect()
}
fn assert_sine(actual: &[PortFrame], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (frame, expected) in actual.iter().zip(expected) {
        assert_eq!(frame.channels, 1);
        close(frame.values[0], *expected);
        assert!(frame.values[1..].iter().all(|v| *v == 0.0));
    }
}

#[test]
fn analytical_wave_landmarks_ranges_and_inversion_match_the_documented_origins() {
    for port in ["sine", "triangle", "saw", "square"] {
        let mut p = lfo(8192, &[("frequency", 32.0)], port);
        let bipolar = render(&p, 513);
        for (index, frame) in bipolar.iter().enumerate() {
            assert_eq!(frame.channels, 1);
            close(
                frame.values[0],
                5.0 * wave(port, (index % 256) as f64 / 256.0, 0.5),
            );
            assert!((-5.0..=5.0).contains(&frame.values[0]));
        }
        // Landmarks are exact cycle fractions, independent of implementation.
        for index in [0, 64, 128, 192, 256] {
            close(
                bipolar[index].values[0],
                5.0 * wave(port, (index % 256) as f64 / 256.0, 0.5),
            );
        }
        p.nodes[0].params.insert("invert".into(), 1.0);
        let inverted = render(&p, 513);
        p.nodes[0].params.insert("unipolar".into(), 1.0);
        let inverted_uni = render(&p, 513);
        p.nodes[0].params.insert("invert".into(), 0.0);
        let unipolar = render(&p, 513);
        for index in 0..513 {
            close(
                inverted[index].values[0],
                -f64::from(bipolar[index].values[0]),
            );
            close(
                unipolar[index].values[0],
                f64::from(bipolar[index].values[0]) + 5.0,
            );
            close(
                inverted_uni[index].values[0],
                10.0 - f64::from(unipolar[index].values[0]),
            );
            assert!((0.0..=10.0).contains(&unipolar[index].values[0]));
        }
    }
}

#[test]
fn old_lfo_keeps_its_legacy_phase_amplitude_offset_and_version_one_behavior() {
    let mut p = Patch {
        sample_rate: 8192,
        nodes: vec![node(
            "legacy",
            "lfo",
            &[("frequency", 32.0), ("amplitude", 0.25), ("offset", 0.5)],
        )],
        output: Some("legacy".into()),
        ..Patch::default()
    };
    let legacy = render(&p, 513);
    p.version = 2;
    let delayed = render(&p, 513);
    assert_eq!(legacy, delayed); // No cables: same legacy generator samples.
    for (index, frame) in legacy.iter().enumerate() {
        close(
            frame.values[0],
            0.5 + 0.25 * (TAU * (index % 256) as f64 / 256.0).sin(),
        );
    }
}

#[test]
fn exponential_fm_doubles_frequency_and_respects_floor_and_cap() {
    let mut p = lfo(8192, &[("frequency", 8.0), ("fm_amount", 1.0)], "sine");
    p.nodes.push(source("fm", &[1.0]));
    p.cables.push(wire("fm", "out", "lfo", "fm"));
    let actual = render(&p, 4097);
    assert_sine(
        &actual,
        &sine_oracle(actual.len(), 8192, 8.0, &[(1, 16.0)], &[]),
    );
    let crossings: Vec<_> = actual
        .windows(2)
        .enumerate()
        .filter(|(_, f)| f[0].values[0] <= 0.0 && f[1].values[0] > 0.0)
        .map(|(i, _)| i)
        .collect();
    assert!(crossings.len() >= 8);
    assert!(crossings
        .windows(2)
        .skip(1)
        .all(|pair| pair[1] - pair[0] == 512));
    p.sample_rate = 8000;
    p.nodes[0].params.insert("frequency".into(), 1024.0);
    p.nodes[1] = source("fm", &[16.0]);
    assert_sine(
        &render(&p, 1025),
        &sine_oracle(1025, 8000, 1024.0, &[], &[]),
    );
    p.nodes[0].params.insert("frequency".into(), 1.0 / 256.0);
    p.nodes[0].params.insert("fm_amount".into(), -1.0);
    p.nodes[1] = source("fm", &[16.0]);
    let actual = render(&p, 1025);
    assert_sine(
        &actual,
        &sine_oracle(1025, 8000, 1.0 / 256.0, &[(1, 1e-6)], &[]),
    );
    assert!(actual.last().unwrap().values[0] > actual[1].values[0]);
}

#[test]
fn pwm_duty_is_additive_and_clamps_without_changing_other_waveforms() {
    let mut p = lfo(
        8192,
        &[
            ("frequency", 32.0),
            ("pulse_width", 0.25),
            ("pwm_amount", 1.0),
        ],
        "square",
    );
    p.nodes.push(source("pwm", &[0.0]));
    p.cables.push(wire("pwm", "out", "lfo", "pwm"));
    for (cv, width, high_samples) in [
        (0.0, 0.25, 64),
        (5.0, 0.75, 192),
        (-16.0, 0.01, 3),
        (16.0, 0.99, 254),
    ] {
        p.nodes[1] = source("pwm", &[cv]);
        let output = render(&p, 512);
        let cycle = &output[256..512]; // A settled, exact 256-sample cycle.
        assert_eq!(
            cycle.iter().filter(|f| f.values[0] > 0.0).count(),
            high_samples
        );
        for (index, frame) in cycle.iter().enumerate() {
            close(
                frame.values[0],
                5.0 * wave("square", index as f64 / 256.0, width),
            );
        }
        p.output_port = "sine".into();
        assert_sine(&render(&p, 513), &sine_oracle(513, 8192, 32.0, &[], &[]));
        p.output_port = "square".into();
    }
}

#[test]
fn fm_sets_voice_count_and_auxiliaries_broadcast_or_zero_fill_without_creating_voices() {
    let mut p = lfo(
        8192,
        &[("frequency", 8.0), ("fm_amount", 1.0), ("pwm_amount", 1.0)],
        "sine",
    );
    p.nodes
        .extend([source("fm", &[0.0, 1.0, -1.0]), source("pwm", &[5.0])]);
    p.cables.extend([
        wire("fm", "out", "lfo", "fm"),
        wire("pwm", "out", "lfo", "pwm"),
    ]);
    let frames = render(&p, 129);
    assert_eq!(frames[0].channels, 1);
    for (lane, hz) in [8.0, 16.0, 4.0].into_iter().enumerate() {
        let expected = sine_oracle(
            129,
            8192,
            if lane == 0 { 8.0 } else { 0.0 },
            &[(1, hz)],
            &[],
        );
        for index in 1..129 {
            assert_eq!(frames[index].channels, 3);
            close(frames[index].values[lane], expected[index]);
        }
    }
    p.output_port = "square".into();
    p.nodes[0].params.insert("fm_amount".into(), 0.0);
    p.nodes[0].params.insert("frequency".into(), 32.0);
    p.nodes[2] = source("pwm", &[5.0, -5.0]);
    let output = render(&p, 258);
    // Voice0 .99 width, voice1 .01 width, absent short-poly voice2 .5.
    assert_eq!(&output[100].values[..3], &[5.0, -5.0, 5.0]);
    assert_eq!(&output[200].values[..3], &[5.0, -5.0, -5.0]);
    p.nodes[1] = source("fm", &[]);
    p.nodes[2] = source("pwm", &[5.0; 16]);
    assert!(render(&p, 17).iter().all(|f| f.channels == 1));
}

#[test]
fn first_clock_only_arms_second_acquires_frequency_and_neither_resets_phase() {
    let mut p = lfo(8000, &[], "sine");
    controls(&mut p);
    let first_only = driven(
        &mut p.compile().unwrap(),
        801,
        &[(0, note(0, true)), (2, note(0, false))],
    );
    assert_sine(&first_only, &sine_oracle(801, 8000, 2.0, &[], &[]));
    let events = [
        (0, note(0, true)),
        (2, note(0, false)),
        (200, note(0, true)),
        (202, note(0, false)),
    ];
    let output = driven(&mut p.compile().unwrap(), 1001, &events);
    assert_sine(&output, &sine_oracle(1001, 8000, 2.0, &[(201, 40.0)], &[]));
    assert!(output[201].values[0] > 1.0); // Clock acquisition did not reset phase to zero.
}

#[test]
fn tempo_changes_measure_full_interval_and_missing_clocks_hold_last_frequency() {
    let mut p = lfo(8000, &[], "sine");
    controls(&mut p);
    let events = [
        (0, note(0, true)),
        (2, note(0, false)),
        (200, note(0, true)),
        (202, note(0, false)),
        (500, note(0, true)),
        (502, note(0, false)),
        (1400, note(0, true)),
        (1402, note(0, false)),
    ];
    let output = driven(&mut p.compile().unwrap(), 2001, &events);
    assert_sine(
        &output,
        &sine_oracle(
            2001,
            8000,
            2.0,
            &[(201, 40.0), (501, 8000.0 / 300.0), (1401, 8000.0 / 900.0)],
            &[],
        ),
    );
}

#[test]
fn held_reset_triggers_once_and_simultaneous_clock_retains_the_acquired_period() {
    let mut p = lfo(8000, &[], "sine");
    controls(&mut p);
    let held = driven(&mut p.compile().unwrap(), 257, &[(10, note(1, true))]);
    assert_sine(&held, &sine_oracle(257, 8000, 2.0, &[], &[11]));
    assert!(held[12].values[0] > 0.0); // High reset is not a continuous freeze.
    let events = [
        (0, note(0, true)),
        (2, note(0, false)),
        (200, note(0, true)),
        (200, note(1, true)),
        (202, note(0, false)),
    ];
    let output = driven(&mut p.compile().unwrap(), 801, &events);
    assert_sine(
        &output,
        &sine_oracle(801, 8000, 2.0, &[(201, 40.0)], &[201]),
    );
    assert_eq!(output[201].values[0], 0.0);
    assert!(output[202].values[0] > 0.1); // New clock frequency advances after reset.
}

#[test]
fn parameter_adoption_preserves_phase_and_held_reset_detector_but_rewiring_rearms_it() {
    let mut p = lfo(8192, &[("frequency", 32.0)], "sine");
    controls(&mut p);
    let mut old = p.compile().unwrap();
    driven(&mut old, 17, &[(0, note(1, true))]);
    p.nodes[0].params.insert("frequency".into(), 16.0);
    let mut current = p.compile().unwrap();
    current.adopt_state_from(&old);
    let mut output = [PortFrame::default(); 3];
    current.process_poly_block(&mut output);
    for (index, frame) in output.iter().enumerate() {
        close(
            frame.values[0],
            5.0 * (TAU * (16.0 / 256.0 + index as f64 * 16.0 / 8192.0)).sin(),
        );
    }
    p.nodes.push(source("high", &[10.0]));
    p.cables
        .iter_mut()
        .find(|c| c.input == "reset")
        .unwrap()
        .from = "high".into();
    p.cables
        .iter_mut()
        .find(|c| c.input == "reset")
        .unwrap()
        .output = "out".into();
    let mut rewired = p.compile().unwrap();
    rewired.adopt_state_from(&current);
    rewired.process_poly_block(&mut output);
    close(
        output[0].values[0],
        5.0 * (TAU * (16.0 / 256.0 + 3.0 * 16.0 / 8192.0)).sin(),
    );
    assert_eq!(output[1].values[0], 0.0); // New high endpoint is a fresh rising edge.
    close(output[2].values[0], 5.0 * (TAU * 16.0 / 8192.0).sin());
}

#[test]
fn clock_disconnection_and_rewiring_clear_tempo_without_resetting_phase() {
    let mut p = lfo(8000, &[], "sine");
    controls(&mut p);
    let mut old = p.compile().unwrap();
    driven(
        &mut old,
        401,
        &[
            (0, note(0, true)),
            (2, note(0, false)),
            (200, note(0, true)),
            (202, note(0, false)),
        ],
    );
    let phase = (201.0_f64 * 2.0 / 8000.0 + 200.0 * 40.0 / 8000.0).fract();
    let mut parameter_edit = p.clone();
    parameter_edit.nodes[0]
        .params
        .insert("frequency".into(), 4.0);
    let mut edited = parameter_edit.compile().unwrap();
    edited.adopt_state_from(&old);
    let mut output = [PortFrame::default(); 17];
    edited.process_poly_block(&mut output);
    for (index, frame) in output.iter().enumerate() {
        close(
            frame.values[0],
            5.0 * (TAU * (phase + index as f64 * 80.0 / 8000.0)).sin(),
        );
    }

    p.cables.retain(|c| c.input != "clock");
    let mut disconnected = p.compile().unwrap();
    disconnected.adopt_state_from(&old);
    disconnected.process_poly_block(&mut output);
    for (index, frame) in output.iter().enumerate() {
        close(
            frame.values[0],
            5.0 * (TAU * (phase + index as f64 * 2.0 / 8000.0)).sin(),
        );
    }
    p.nodes.push(source("high", &[10.0]));
    p.cables.push(wire("high", "out", "lfo", "clock"));
    let mut rewired = p.compile().unwrap();
    rewired.adopt_state_from(&old);
    rewired.process_poly_block(&mut output);
    for (index, frame) in output.iter().enumerate() {
        close(
            frame.values[0],
            5.0 * (TAU * (phase + index as f64 * 2.0 / 8000.0)).sin(),
        );
    }
    // Constant-high new source arms one edge, never inherits the old period.
}

#[test]
fn retired_then_reactivated_voices_restart_while_retained_voice_keeps_phase() {
    let mut p = lfo(8192, &[("frequency", 32.0)], "sine");
    p.nodes.push(source("fm", &[0.0; 3]));
    p.cables.push(wire("fm", "out", "lfo", "fm"));
    let mut old = p.compile().unwrap();
    old.process_poly_block(&mut [PortFrame::default(); 37]);
    p.nodes[1] = source("fm", &[0.0]);
    let mut retired = p.compile().unwrap();
    retired.adopt_state_from(&old);
    let mut scratch = [PortFrame::default(); 3];
    retired.process_poly_block(&mut scratch);
    assert_eq!(scratch[0].channels, 3); // Existing cable frame carries count for one sample.
    assert_eq!(scratch[1].channels, 1);
    p.nodes[1] = source("fm", &[0.0; 3]);
    let mut regrown = p.compile().unwrap();
    regrown.adopt_state_from(&retired);
    regrown.process_poly_block(&mut scratch);
    assert_eq!(scratch[0].channels, 1);
    assert_eq!(scratch[1].channels, 3);
    close(scratch[1].values[0], 5.0 * (TAU * 41.0 / 256.0).sin());
    assert_eq!(&scratch[1].values[1..3], &[0.0, 0.0]);
    close(scratch[2].values[1], 5.0 * (TAU / 256.0).sin());
}

#[test]
fn uneven_callback_partitions_and_node_order_preserve_samples_at_rate_extremes() {
    for rate in [8000, 192000] {
        let mut p = lfo(
            rate,
            &[
                ("frequency", 1024.0),
                ("fm_amount", 1.0),
                ("pwm_amount", -0.5),
            ],
            "saw",
        );
        p.nodes
            .extend([source("fm", &[0.0, 1.0, -1.0]), source("pwm", &[5.0])]);
        p.cables.extend([
            wire("fm", "out", "lfo", "fm"),
            wire("pwm", "out", "lfo", "pwm"),
        ]);
        let expected = render(&p, 1031);
        p.nodes.reverse();
        p.cables.reverse();
        for partition in [1, 3, 127, 128, 129, 517] {
            let mut engine = p.compile().unwrap();
            let mut actual = vec![PortFrame::default(); expected.len()];
            for chunk in actual.chunks_mut(partition) {
                engine.process_poly_block(chunk);
            }
            assert_eq!(actual, expected, "rate={rate}, partition={partition}");
            assert!(actual
                .iter()
                .all(|f| f.values.iter().all(|v| v.is_finite() && v.abs() <= 5.0)));
        }
    }
}

#[test]
fn catalog_persistence_and_invalid_parameter_ports_are_explicitly_v2() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "multi_lfo")
        .unwrap();
    assert_eq!(module["min_patch_version"], 2);
    assert_eq!(module["model"], "functional");
    assert_eq!(
        module["inputs"],
        serde_json::json!(["fm", "pwm", "reset", "clock"])
    );
    assert_eq!(
        module["outputs"],
        serde_json::json!(["sine", "triangle", "saw", "square"])
    );
    let p = lfo(48000, &[], "sine");
    let json = serde_json::to_value(&p).unwrap();
    let restored: Patch = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(serde_json::to_value(&restored).unwrap(), json);
    assert_eq!(render(&p, 37), render(&restored, 37));
    let mut legacy = p.clone();
    legacy.version = 1;
    assert!(legacy.validate().is_err());
    for (name, value) in [
        ("frequency", 0.0),
        ("frequency", 1025.0),
        ("frequency", f32::NAN),
        ("fm_amount", 1.01),
        ("pwm_amount", -1.01),
        ("pulse_width", 0.0),
        ("pulse_width", 1.0),
        ("unipolar", 0.5),
        ("invert", 2.0),
        ("unknown", 1.0),
    ] {
        let p = lfo(48000, &[(name, value)], "sine");
        assert!(p.validate().is_err(), "{name}={value}");
    }
    for port in ["out", "sin", "pulse", "sine2"] {
        assert!(lfo(48000, &[], port).validate().is_err());
    }
}

fn replace_source(patch: &mut Patch, old: &Engine, id: &str, values: &[f32]) -> Engine {
    *patch.nodes.iter_mut().find(|n| n.id == id).unwrap() = source(id, values);
    let mut next = patch.compile().unwrap();
    next.adopt_state_from(old);
    next
}
fn append(engine: &mut Engine, output: &mut Vec<PortFrame>, count: usize) {
    let offset = output.len();
    output.resize(offset + count, PortFrame::default());
    engine.process_poly_block(&mut output[offset..]);
}

#[test]
fn zero_channel_clock_clears_acquisition_then_requires_two_new_edges() {
    let mut p = lfo(8000, &[], "sine");
    p.nodes.push(source("clock", &[10.0]));
    p.cables.push(wire("clock", "out", "lfo", "clock"));
    let mut engine = p.compile().unwrap();
    let mut output = Vec::new();
    append(&mut engine, &mut output, 101); // First edge frame1 only arms.
    engine = replace_source(&mut p, &engine, "clock", &[0.0]);
    append(&mut engine, &mut output, 99);
    engine = replace_source(&mut p, &engine, "clock", &[10.0]);
    append(&mut engine, &mut output, 101); // Edge201 locks 200-frame period.
    engine = replace_source(&mut p, &engine, "clock", &[]);
    append(&mut engine, &mut output, 99); // Channel count reaches LFO frame302.
    engine = replace_source(&mut p, &engine, "clock", &[10.0]);
    append(&mut engine, &mut output, 50); // Edge401 must only arm, not lock old interval.
    engine = replace_source(&mut p, &engine, "clock", &[0.0]);
    append(&mut engine, &mut output, 50);
    engine = replace_source(&mut p, &engine, "clock", &[10.0]);
    append(&mut engine, &mut output, 301); // Edge501 locks fresh 100-frame interval.
    assert_sine(
        &output,
        &sine_oracle(801, 8000, 2.0, &[(201, 40.0), (302, 2.0), (501, 80.0)], &[]),
    );
}

#[test]
fn short_poly_clock_lanes_keep_tempo_while_the_port_remains_connected() {
    let mut p = lfo(8000, &[], "sine");
    p.nodes
        .extend([source("fm", &[0.0; 3]), source("clock", &[10.0; 3])]);
    p.cables.extend([
        wire("fm", "out", "lfo", "fm"),
        wire("clock", "out", "lfo", "clock"),
    ]);
    let mut engine = p.compile().unwrap();
    engine.process_poly_block(&mut [PortFrame::default(); 101]);
    engine = replace_source(&mut p, &engine, "clock", &[0.0; 3]);
    engine.process_poly_block(&mut [PortFrame::default(); 99]);
    engine = replace_source(&mut p, &engine, "clock", &[10.0; 3]);
    engine.process_poly_block(&mut [PortFrame::default(); 101]);
    let mut reference = p.compile().unwrap();
    reference.adopt_state_from(&engine);
    let mut shortened = replace_source(&mut p, &engine, "clock", &[10.0]);
    let mut expected = [PortFrame::default(); 513];
    let mut actual = [PortFrame::default(); 513];
    reference.process_poly_block(&mut expected);
    shortened.process_poly_block(&mut actual);
    assert_eq!(actual, expected);
    assert!(actual.iter().all(|f| f.channels == 3));
}
