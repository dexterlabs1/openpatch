use openpatch_core::midi_cc::MidiCc;
use openpatch_core::*;
fn node(params: &[(&str, f32)]) -> Node {
    Node {
        id: "cc".into(),
        kind: "midi_cc".into(),
        params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
        content: None,
        score: None,
        bypassed: false,
    }
}
fn patch(params: &[(&str, f32)]) -> Patch {
    Patch {
        nodes: vec![node(params)],
        output: Some("cc".into()),
        output_port: "out1".into(),
        ..Patch::new_v2()
    }
}
fn cc(channel: u8, controller: u8, value: u8) -> MidiEvent {
    MidiEvent::ControlChange {
        channel,
        controller,
        value,
    }
}
fn frame(e: &mut Engine) -> PortFrame {
    let mut frames = [PortFrame::default()];
    e.process_poly_block(&mut frames);
    frames[0]
}
fn values(e: &Engine) -> ([i16; 16], [u8; 128]) {
    let SavedModuleState::MidiCc {
        assignments,
        values,
    } = e
        .module_saved_state(e.resolve_module_state("cc").unwrap())
        .unwrap()
    else {
        panic!()
    };
    (assignments, values)
}
#[test]
fn validates_and_normalizes_complete_persistent_state() {
    let mut n = node(&[("cc1", 127.0), ("cc2", 127.0), ("cc_value127", 99.0)]);
    let state = initial_midi_cc_state(&n).unwrap();
    let SavedModuleState::MidiCc {
        assignments,
        values,
    } = state
    else {
        panic!()
    };
    assert_eq!(&assignments[..2], &[-1, 127]);
    assert_eq!(values[127], 99);
    let mut params = Default::default();
    state.write_params(&mut params);
    n.params = params;
    assert_eq!(initial_midi_cc_state(&n).unwrap(), state);
    for (key, value) in [
        ("cc1", 128.0),
        ("cc1", 1.5),
        ("mpe", 0.5),
        ("channel", 17.0),
        ("cc_value0", 128.0),
        ("cc01", 1.0),
    ] {
        assert!(
            MidiCc::build(&node(&[(key, value)]), 48000).is_err(),
            "{key}"
        );
    }
}
#[test]
fn smoothing_jump_boundary_freeze_and_unmapped_history() {
    for rate in [44100, 48000, 96000] {
        let mut m = MidiCc::build(&node(&[]), rate).unwrap();
        let mut out = [PortFrame::default(); 16];
        m.handle_midi(cc(0, 1, 64));
        m.process(&mut out, 1);
        let first = out[0].values[0];
        assert!((first - 64.0 / 127.0 * 30.0 / rate as f32 * 10.0).abs() < 1e-8);
        m.process(&mut out, 0);
        assert_eq!(out[0].channels, 0);
        m.process(&mut out, 1);
        assert!(out[0].values[0] > first && out[0].values[0] < first * 2.0);
        let mut unmapped = [-1; 16];
        assert!(m.set_saved_state(SavedModuleState::MidiCcMappings(unmapped)));
        m.process(&mut out, 1);
        assert_eq!(out[0].values[0], 0.0);
        unmapped[0] = 1;
        m.set_saved_state(SavedModuleState::MidiCcMappings(unmapped));
        m.process(&mut out, 1);
        assert!(out[0].values[0] > first * 2.0);
        let mut fresh = MidiCc::build(&node(&[]), rate).unwrap();
        fresh.handle_midi(cc(0, 1, 127));
        fresh.process(&mut out, 1);
        assert_eq!(out[0].values[0], 10.0);
    }
}
#[test]
fn pairing_all_channels_filter_and_independent_lsb_outputs() {
    let mut m = MidiCc::build(
        &node(&[
            ("mpe", 1.0),
            ("smooth", 0.0),
            ("high_resolution", 1.0),
            ("cc1", 0.0),
            ("cc2", 32.0),
        ]),
        48000,
    )
    .unwrap();
    let mut out = [PortFrame::default(); 16];
    for lane in 0..16 {
        m.handle_midi(cc(lane, 0, lane + 1));
    }
    m.process(&mut out, 3);
    assert_eq!(out[0].channels, 16);
    assert_eq!(out[0].values, [0.0; 16]);
    for lane in 0..16 {
        m.handle_midi(cc(lane, 32, 127));
    }
    m.process(&mut out, 3);
    for lane in 0..16 {
        assert!(
            (out[0].values[lane] - (((lane + 1) * 128 + 127) as f32 / 16256.0 * 10.0)).abs() < 1e-6
        );
        assert_eq!(out[1].values[lane], 10.0);
    }
    m.handle_midi(cc(0, 0, 127));
    m.handle_midi(MidiEvent::Panic);
    m.handle_midi(cc(0, 32, 0));
    m.process(&mut out, 1);
    assert_eq!(out[0].values[0], 0.0);
    let mut filtered = patch(&[("channel", 2.0), ("smooth", 0.0)])
        .compile()
        .unwrap();
    filtered.handle_midi(cc(0, 1, 127));
    assert_eq!(frame(&mut filtered).values[0], 0.0);
    filtered.handle_midi(cc(1, 1, 127));
    assert_eq!(frame(&mut filtered).values[0], 10.0);
}
#[test]
fn learn_uses_precommit_value_collision_and_durable_slot() {
    let mut e = patch(&[("high_resolution", 1.0), ("channel", 2.0)])
        .compile()
        .unwrap();
    let h = e.resolve_module_state("cc").unwrap();
    assert!(!e.arm_midi_cc_learn(h, 0, 0));
    assert!(!e.arm_midi_cc_learn(h, 1, 16));
    assert!(e.arm_midi_cc_learn(h, 1, 0));
    e.handle_midi(cc(0, 2, 1));
    e.handle_midi(cc(1, 2, 0));
    assert_eq!(e.midi_cc_learning(h), Some((1, 0)));
    e.handle_midi(cc(1, 2, 1));
    assert_eq!(e.midi_cc_learning(h), None);
    assert!(!e.cancel_midi_cc_learn(h, 1));
    assert!(!e.arm_midi_cc_learn(h, 2, 3));
    e.handle_midi(MidiEvent::Panic);
    let done = e.take_midi_cc_learn_completion(h).unwrap();
    assert_eq!(done.serial, 1);
    assert_eq!(&done.after[..2], &[2, -1]);
    assert_eq!(values(&e).1[2], 0);
    assert!(e.arm_midi_cc_learn(h, 2, 3));
    e.handle_midi(cc(1, 2, 1));
    assert_eq!(e.take_midi_cc_learn_completion(h).unwrap().cell, 3);
}
#[test]
fn adoption_preserves_movement_filters_pairing_and_learn() {
    let p = patch(&[("high_resolution", 1.0)]);
    let mut old = p.compile().unwrap();
    let h = old.resolve_module_state("cc").unwrap();
    old.handle_midi(cc(0, 1, 64));
    old.handle_midi(cc(0, 33, 2));
    frame(&mut old);
    old.handle_midi(cc(0, 3, 70));
    assert!(old.arm_midi_cc_learn(h, 42, 0));
    let mut new = p.compile().unwrap();
    new.adopt_state_from(&old);
    let nh = new.resolve_module_state("cc").unwrap();
    assert_eq!(new.midi_cc_learning(nh), Some((42, 0)));
    assert_eq!(frame(&mut new), frame(&mut old));
    new.handle_midi(cc(0, 35, 3));
    assert_eq!(values(&new).1[3], 70);
    let mut edited = p.clone();
    edited.nodes[0].params.insert("cc1".into(), 9.0);
    let mut changed = edited.compile().unwrap();
    changed.adopt_state_from(&new);
    assert_eq!(
        changed.midi_cc_learning(changed.resolve_module_state("cc").unwrap()),
        None
    );
    assert_eq!(values(&changed).1, values(&new).1);
    let h = changed.resolve_module_state("cc").unwrap();
    let mut mappings = values(&changed).0;
    mappings[0] = 4;
    assert!(changed.set_module_saved_state(h, SavedModuleState::MidiCcMappings(mappings)));
    assert_eq!(values(&changed).1, values(&new).1);
    let mut explicit = edited.clone();
    explicit.nodes[0].params.insert("cc_value1".into(), 12.0);
    let mut replaced = explicit.compile().unwrap();
    replaced.adopt_state_from(&changed);
    assert_eq!(values(&replaced).1[1], 12);
    assert_eq!(values(&replaced).1[3], 70);
}
#[test]
fn temporary_probe_consumes_only_for_block_and_reconnect_resumes_filter() {
    let mut p = patch(&[]);
    p.output = None;
    p.output_port = "out".into();
    let mut e = p.compile().unwrap();
    e.handle_midi(cc(0, 1, 64));
    let tap = e.resolve_tap("cc", "out1", 0).unwrap();
    let taps = [Some(tap), None];
    let mut audio = [[0.0; 2]; 1];
    let mut samples = [[TapSample::default(); 2]; 1];
    for _ in 0..100 {
        frame(&mut e);
    }
    e.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    let first = samples[0][0].value;
    assert!(samples[0][0].active);
    assert!((first - 64.0 / 127.0 * 30.0 / 48000.0 * 10.0).abs() < 1e-8);
    for _ in 0..100 {
        frame(&mut e);
    }
    e.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    assert!(samples[0][0].value > first && samples[0][0].value < first * 2.0);
    let stale = taps;
    let mut other = p.compile().unwrap();
    other.adopt_state_from(&e);
    other.process_audio_block_with_taps(&mut audio, &stale, &mut samples);
    assert!(!samples[0][0].active);
}
#[test]
fn bypass_freezes_values_and_panic_pair_policy_but_initialize_retains_filter() {
    let p = patch(&[]);
    let mut e = p.compile().unwrap();
    e.handle_midi(cc(0, 1, 64));
    let first = frame(&mut e).values[0];
    let h = e.resolve_module_state("cc").unwrap();
    e.arm_midi_cc_learn(h, 1, 0);
    let mut bypass = p.clone();
    bypass.nodes[0].bypassed = true;
    let mut muted = bypass.compile().unwrap();
    muted.adopt_state_from(&e);
    let mh = muted.resolve_module_state("cc").unwrap();
    assert_eq!(muted.midi_cc_learning(mh), None);
    assert!(!muted.arm_midi_cc_learn(mh, 2, 0));
    muted.handle_midi(cc(0, 1, 127));
    assert_eq!(frame(&mut muted), PortFrame::mono(0.0));
    assert_eq!(values(&muted).1[1], 64);
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&muted);
    assert!(frame(&mut resumed).values[0] > first);
    let rh = resumed.resolve_module_state("cc").unwrap();
    resumed.initialize_module_state(rh);
    let tail = frame(&mut resumed).values[0];
    assert!(tail > 0.0);
    assert_eq!(values(&resumed).1, [0; 128]);
    resumed.reset_module_state(rh);
    assert_eq!(frame(&mut resumed).values[0], 0.0);
}

#[test]
fn mpe_mode_changes_keep_each_inactive_lane_and_all_sixteen_cells() {
    let mut p = patch(&[("mpe", 1.0)]);
    p.output_port = "out16".into();
    let mut old = p.compile().unwrap();
    for lane in 0..16 {
        for controller in 1..=16 {
            old.handle_midi(cc(lane, controller, 64));
        }
    }
    let before = frame(&mut old);
    assert_eq!(before.channels, 16);
    p.nodes[0].params.insert("mpe".into(), 0.0);
    let mut mono = p.compile().unwrap();
    mono.adopt_state_from(&old);
    assert_eq!(frame(&mut mono).channels, 1);
    for _ in 0..100 {
        frame(&mut mono);
    }
    p.nodes[0].params.insert("mpe".into(), 1.0);
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&mono);
    let after = frame(&mut resumed);
    assert_eq!(after.channels, 16);
    assert!(after.values[0] > before.values[0] * 2.0);
    for lane in 1..16 {
        assert!(
            after.values[lane] > before.values[lane]
                && after.values[lane] < before.values[lane] * 2.0
        );
    }
    let mut model = MidiCc::build(&node(&[("smooth", 0.0)]), 48000).unwrap();
    let mut outputs = [PortFrame::default(); 16];
    for controller in 1..=16 {
        model.handle_midi(cc(controller - 1, controller, controller * 7));
    }
    model.process(&mut outputs, u16::MAX);
    for (cell, out) in outputs.iter().enumerate() {
        assert_eq!(out.channels, 1);
        assert!((out.values[0] - (cell + 1) as f32 * 7.0 / 127.0 * 10.0).abs() < 1e-6);
    }
    model.process(&mut outputs, 0);
    assert!(outputs.iter().all(|out| *out == PortFrame::default()));
}

#[test]
fn saved_load_restores_only_channel_zero_and_preserves_filter_history() {
    let mut e = patch(&[("mpe", 1.0)]).compile().unwrap();
    let h = e.resolve_module_state("cc").unwrap();
    e.handle_midi(cc(0, 1, 64));
    e.handle_midi(cc(15, 1, 64));
    let before = frame(&mut e);
    let mut saved_values = [0; 128];
    saved_values[1] = 32;
    let mut maps = values(&e).0;
    maps[1] = -1;
    assert!(e.set_module_saved_state(
        h,
        SavedModuleState::MidiCc {
            assignments: maps,
            values: saved_values
        }
    ));
    let after = frame(&mut e);
    assert!(after.values[0] > before.values[0]);
    assert!(after.values[15] > before.values[15]);
    assert_eq!(values(&e).1, saved_values);
    e.handle_midi(cc(0, 1, 100));
    assert!(e.set_module_saved_state(h, SavedModuleState::MidiCcMappings(maps)));
    assert_eq!(values(&e).1[1], 100);
}

#[test]
fn smoothing_settles_when_an_euler_step_rounds_to_the_previous_value() {
    let mut model = MidiCc::build(&node(&[("mpe", 1.0), ("cc15", 8.0)]), 96000).unwrap();
    let mut outputs = [PortFrame::default(); 16];
    for sample in 0..=2212 {
        if sample % 79 == 0 {
            model.handle_midi(cc(3, 8, ((8 * 13 + 3 * 5 + sample / 79) % 128) as u8));
        }
        model.process(&mut outputs, 1 << 14);
    }
    assert_eq!(outputs[14].values[3], 19.0_f32 / 127.0 * 10.0);
}
