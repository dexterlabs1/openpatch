use openpatch_core::midi_gate::{MidiGate, DEFAULT_ASSIGNMENTS};
use openpatch_core::*;
fn node(params: &[(&str, f32)]) -> Node {
    Node {
        id: "gate".into(),
        kind: "midi_gate".into(),
        params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
        score: None,
        bypassed: false,
    }
}
fn patch(params: &[(&str, f32)]) -> Patch {
    Patch {
        nodes: vec![node(params)],
        output: Some("gate".into()),
        output_port: "gate1".into(),
        ..Patch::new_v2()
    }
}
fn on(channel: u8, note: u8, velocity: u8) -> MidiEvent {
    MidiEvent::NoteOn {
        channel,
        note,
        velocity,
    }
}
fn off(channel: u8, note: u8) -> MidiEvent {
    MidiEvent::NoteOff {
        channel,
        note,
        velocity: 0,
    }
}
fn frame(engine: &mut Engine) -> PortFrame {
    let mut frames = [PortFrame::default()];
    engine.process_poly_block(&mut frames);
    frames[0]
}
fn process(module: &mut MidiGate) -> [PortFrame; 16] {
    let mut frames = [PortFrame::default(); 16];
    module.process(&mut frames);
    frames
}
fn maps(module: &MidiGate) -> [i16; 16] {
    let SavedModuleState::MidiGateMappings(assignments) = module.saved_state() else {
        panic!()
    };
    assignments
}
#[test]
fn catalog_defaults_saved_state_and_strict_parameters() {
    let n = node(&[]);
    assert_eq!(
        initial_midi_gate_state(&n).unwrap(),
        SavedModuleState::MidiGateMappings(DEFAULT_ASSIGNMENTS)
    );
    let mut params = Default::default();
    SavedModuleState::MidiGateMappings(DEFAULT_ASSIGNMENTS).write_params(&mut params);
    let mut n = n;
    n.params = params;
    assert_eq!(
        initial_midi_gate_state(&n).unwrap(),
        SavedModuleState::from_node(&n).unwrap()
    );
    let c = catalog();
    let c = c["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "midi_gate")
        .unwrap();
    assert_eq!(c["outputs"].as_array().unwrap().len(), 16);
    assert_eq!(c["inputs"], serde_json::json!([]));
    assert_eq!(c["min_patch_version"], 2);
    assert_eq!(c["runtime_actions"], serde_json::json!(["midi_panic"]));
    for (key, value) in [
        ("note1", 128.0),
        ("note1", -2.0),
        ("note1", 60.5),
        ("note01", 60.0),
        ("note17", 60.0),
        ("velocity", 0.5),
        ("mpe", 2.0),
        ("channel", 17.0),
        ("channel", f32::NAN),
    ] {
        assert!(
            MidiGate::build(&node(&[(key, value)]), 48000).is_err(),
            "{key}={value}"
        );
    }
    let mut invalid = patch(&[]);
    invalid.version = 1;
    assert!(invalid.compile().is_err());
    invalid = patch(&[]);
    invalid.output_port = "gate0".into();
    assert!(invalid.compile().is_err());
    let mut m = MidiGate::build(&node(&[("note1", 60.0), ("note2", 60.0)]), 48000).unwrap();
    assert_eq!(&maps(&m)[..2], &[-1, 60]);
    assert!(!m.set_saved_state(SavedModuleState::MidiGateMappings([128; 16])));
    assert_eq!(&maps(&m)[..2], &[-1, 60]);
}
#[test]
fn f32_minimum_pulse_edges_are_exact_at_each_rate_and_velocity_endpoint() {
    for (rate, samples) in [(44100, 45), (48000, 48), (96000, 97)] {
        for velocity in [1, 127] {
            let mut m = MidiGate::build(&node(&[("velocity", 1.0)]), rate).unwrap();
            assert_eq!(process(&mut m)[0], PortFrame::mono(0.0));
            m.handle_midi(on(0, 48, velocity));
            m.handle_midi(off(0, 48));
            for sample in 0..samples {
                let out = process(&mut m);
                assert_eq!(
                    out[0].values[0],
                    velocity as f32 / 127.0 * 10.0,
                    "rate {rate} sample {sample}"
                );
                assert_eq!(out[0].channels, 1);
            }
            assert_eq!(process(&mut m)[0].values[0], 0.0, "rate {rate}");
        }
    }
}
#[test]
fn repeat_restarts_minimum_pulse_and_replaces_velocity_without_note_counting() {
    let mut m = MidiGate::build(&node(&[("velocity", 1.0)]), 48000).unwrap();
    m.handle_midi(on(0, 48, 1));
    for _ in 0..40 {
        process(&mut m);
    }
    m.handle_midi(on(1, 48, 127));
    m.handle_midi(on(2, 48, 0));
    for _ in 0..48 {
        assert_eq!(process(&mut m)[0].values[0], 10.0);
    }
    assert_eq!(process(&mut m)[0].values[0], 0.0);
    m.handle_midi(on(0, 48, 64));
    m.handle_midi(on(1, 48, 64));
    for _ in 0..100 {
        process(&mut m);
    }
    m.handle_midi(off(15, 48));
    assert_eq!(process(&mut m)[0].values[0], 0.0);
    m.handle_midi(off(0, 48));
    m.handle_midi(on(0, 48, 64));
    for _ in 0..100 {
        assert!(process(&mut m)[0].values[0] > 0.0);
    }
}
#[test]
fn held_remapping_unmapping_and_mapping_load_preserve_cell_signals() {
    let mut m = MidiGate::build(&node(&[]), 48000).unwrap();
    m.handle_midi(on(0, 48, 73));
    for _ in 0..100 {
        process(&mut m);
    }
    let mut assignments = maps(&m);
    assignments[0] = 80;
    m.set_saved_state(SavedModuleState::MidiGateMappings(assignments));
    m.handle_midi(off(0, 48));
    assert_eq!(process(&mut m)[0].values[0], 10.0);
    assignments[0] = -1;
    m.set_saved_state(SavedModuleState::MidiGateMappings(assignments));
    assert_eq!(process(&mut m)[0].values[0], 10.0);
    assignments[0] = 80;
    m.set_saved_state(SavedModuleState::MidiGateMappings(assignments));
    m.handle_midi(off(0, 80));
    assert_eq!(process(&mut m)[0].values[0], 0.0);
}
#[test]
fn mpe_all_cells_all_lanes_and_actual_lights_follow_published_amplitudes() {
    let mut m = MidiGate::build(&node(&[("mpe", 1.0), ("velocity", 1.0)]), 48000).unwrap();
    for lane in 0..16 {
        for note in DEFAULT_ASSIGNMENTS {
            m.handle_midi(on(lane, note as u8, 1 + lane * 8));
        }
    }
    let out = process(&mut m);
    for output in out {
        assert_eq!(output.channels, 16);
        for lane in 0..16 {
            assert_eq!(output.values[lane], (1 + lane * 8) as f32 / 127.0 * 10.0);
        }
    }
    let lights = m.lights();
    assert_eq!(lights.count, 16);
    for light in lights.brightness {
        assert_eq!(light, [121.0 / 127.0 * 10.0 / 10.0, 0.0]);
    }
    for note in DEFAULT_ASSIGNMENTS {
        m.handle_midi(off(15, note as u8));
    }
    for _ in 0..100 {
        process(&mut m);
    }
    assert_eq!(m.lights().brightness[0][0], 113.0 / 127.0 * 10.0 / 10.0);
}
#[test]
fn mode_edits_freeze_hidden_timers_and_keep_gate_velocity() {
    let p = patch(&[("mpe", 1.0), ("velocity", 1.0)]);
    let mut e = p.compile().unwrap();
    e.handle_midi(on(15, 48, 1));
    e.handle_midi(off(15, 48));
    assert_eq!(frame(&mut e).channel(15), 1.0 / 127.0 * 10.0);
    let mut mono_p = p.clone();
    mono_p.nodes[0].params.insert("mpe".into(), 0.0);
    let mut mono = mono_p.compile().unwrap();
    mono.adopt_state_from(&e);
    for _ in 0..1000 {
        assert_eq!(frame(&mut mono), PortFrame::mono(0.0));
    }
    let mut restored = p.compile().unwrap();
    restored.adopt_state_from(&mono);
    for _ in 0..47 {
        assert!(frame(&mut restored).channel(15) > 0.0);
    }
    assert_eq!(frame(&mut restored).channel(15), 0.0);
    e.handle_midi(on(0, 48, 64));
    let mut fixed = mono_p.clone();
    fixed.nodes[0].params.insert("velocity".into(), 0.0);
    let mut fixed = fixed.compile().unwrap();
    fixed.adopt_state_from(&e);
    assert_eq!(frame(&mut fixed).values[0], 10.0);
}
#[test]
fn learn_precedes_note_match_and_accepts_repeated_positive_note_only() {
    let mut m = MidiGate::build(&node(&[("channel", 2.0)]), 48000).unwrap();
    m.handle_midi(on(1, 49, 64));
    for _ in 0..100 {
        process(&mut m);
    }
    assert!(m.arm_learn(1, 0));
    m.handle_midi(on(0, 60, 127));
    m.handle_midi(off(1, 60));
    m.handle_midi(on(1, 60, 0));
    m.handle_midi(MidiEvent::ControlChange {
        channel: 1,
        controller: 60,
        value: 127,
    });
    assert_eq!(m.learning(), Some((1, 0)));
    m.handle_midi(on(1, 49, 127));
    let done = m.take_learn_completion().unwrap();
    assert_eq!(done.before, DEFAULT_ASSIGNMENTS);
    assert_eq!(&done.after[..2], &[49, -1]);
    let out = process(&mut m);
    assert_eq!(out[0].values[0], 10.0);
    assert_eq!(out[1].values[0], 10.0); // displaced held cell stays active
    assert!(m.arm_learn(2, 15));
    m.handle_midi(on(1, 49, 127));
    assert!(!m.arm_learn(3, 14));
    assert!(!m.cancel_learn(2));
    m.host_panic();
    assert_eq!(m.take_learn_completion().unwrap().cell, 15);
}
#[test]
fn source_panic_keeps_learn_but_host_panic_cancels_and_completion_is_durable() {
    let mut m = MidiGate::build(&node(&[]), 48000).unwrap();
    assert!(!m.arm_learn(0, 0));
    assert!(!m.arm_learn(1, 16));
    assert!(m.arm_learn(1, 0));
    m.panic();
    assert_eq!(m.learning(), Some((1, 0)));
    m.handle_midi(on(0, 48, 127));
    assert_eq!(process(&mut m)[0].values[0], 10.0);
    m.host_panic();
    assert_eq!(process(&mut m)[0].values[0], 0.0);
    assert_eq!(m.take_learn_completion().unwrap().serial, 1);
    assert!(m.arm_learn(2, 1));
    m.host_panic();
    assert_eq!(m.learning(), None);
}
#[test]
fn generic_learn_is_kind_and_engine_bound_and_simultaneous_with_cc() {
    let mut p = patch(&[]);
    let mut cc = node(&[]);
    cc.id = "cc".into();
    cc.kind = "midi_cc".into();
    p.nodes.push(cc);
    let mut e = p.compile().unwrap();
    let gh = e.resolve_module_state("gate").unwrap();
    let ch = e.resolve_module_state("cc").unwrap();
    assert_eq!(MidiLearnKind::Gate.module_kind(), "midi_gate");
    assert_eq!(
        MidiLearnKind::from_module_kind("midi_cc"),
        Some(MidiLearnKind::Cc)
    );
    assert_eq!(MidiLearnKind::from_module_kind("midi_cv"), None);
    assert!(!e.arm_midi_learn(gh, MidiLearnKind::Cc, 1, 0));
    assert!(!e.arm_midi_learn(ch, MidiLearnKind::Gate, 1, 0));
    assert!(e.arm_midi_learn(gh, MidiLearnKind::Gate, 1, 0));
    assert!(e.arm_midi_learn(ch, MidiLearnKind::Cc, 2, 0));
    e.handle_midi(on(0, 60, 127));
    e.handle_midi(MidiEvent::ControlChange {
        channel: 0,
        controller: 16,
        value: 1,
    });
    assert!(e
        .take_midi_learn_completion(gh, MidiLearnKind::Cc)
        .is_none());
    assert!(e
        .take_midi_learn_completion(ch, MidiLearnKind::Gate)
        .is_none());
    assert_eq!(
        e.take_midi_learn_completion(gh, MidiLearnKind::Gate)
            .unwrap()
            .after[0],
        60
    );
    assert_eq!(e.take_midi_cc_learn_completion(ch).unwrap().after[0], 16);
    let mut replacement = p.compile().unwrap();
    assert!(!replacement.arm_midi_gate_learn(gh, 3, 0));
    assert!(replacement.take_midi_gate_learn_completion(gh).is_none());
}
#[test]
fn panic_action_is_typed_immediate_bypass_safe_and_never_queue_limited() {
    let mut e = patch(&[]).compile().unwrap();
    let h = e
        .resolve_module_action("gate", ModuleAction::MidiPanic)
        .unwrap();
    assert!(e
        .resolve_module_action("gate", ModuleAction::Clock)
        .is_err());
    assert_eq!(
        serde_json::to_value(ModuleAction::MidiPanic).unwrap(),
        "midi_panic"
    );
    assert_eq!(
        serde_json::from_str::<ModuleAction>("\"midi_panic\"").unwrap(),
        ModuleAction::MidiPanic
    );
    e.handle_midi(on(0, 48, 127));
    frame(&mut e);
    for _ in 0..1000 {
        assert!(e.handle_module_event(h, ModuleAction::MidiPanic));
    }
    assert_eq!(frame(&mut e).values[0], 0.0);
    e.arm_midi_gate_learn(h, 1, 0);
    assert!(e.release_module_controls(h));
    assert_eq!(e.midi_gate_learning(h), None);
    let mut p = patch(&[]);
    p.nodes[0].bypassed = true;
    let mut bypass = p.compile().unwrap();
    let h = bypass
        .resolve_module_action("gate", ModuleAction::MidiPanic)
        .unwrap();
    assert!(bypass.resolve_module_event("gate").is_ok());
    assert!(bypass.handle_module_event(h, ModuleAction::MidiPanic));
    assert!(!bypass.arm_midi_gate_learn(h, 2, 0));
    bypass.handle_midi(on(0, 48, 127));
    assert_eq!(frame(&mut bypass), PortFrame::mono(0.0));
    assert_eq!(bypass.module_lights(h).unwrap().brightness, [[0.0; 2]; 16]);
    let mut cc = patch(&[]);
    cc.nodes[0].kind = "midi_cc".into();
    cc.output_port = "out1".into();
    let mut cc = cc.compile().unwrap();
    assert!(cc
        .resolve_module_action("gate", ModuleAction::MidiPanic)
        .is_err());
    let h = cc.resolve_module_state("gate").unwrap();
    assert!(!cc.handle_module_event(h, ModuleAction::MidiPanic));
}
#[test]
fn disconnected_outputs_keep_advancing_and_consistent_lights() {
    let mut p = patch(&[("velocity", 1.0)]);
    p.output = None;
    p.output_port = "out".into();
    let mut e = p.compile().unwrap();
    let h = e.resolve_module_lights("gate").unwrap();
    e.handle_midi(on(0, 48, 64));
    e.handle_midi(off(0, 48));
    frame(&mut e);
    assert_eq!(
        e.module_lights(h).unwrap().brightness[0][0],
        64.0 / 127.0 * 10.0 / 10.0
    );
    for _ in 0..100 {
        frame(&mut e);
    }
    assert_eq!(e.module_lights(h).unwrap().brightness[0][0], 0.0);
    let taps = [
        Some(e.resolve_tap("gate", "gate1", 0).unwrap()),
        Some(e.resolve_tap("gate", "gate16", 0).unwrap()),
    ];
    let mut audio = [[0.0; 2]; 1];
    let mut samples = [[TapSample::default(); 2]; 1];
    e.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    assert!(samples[0].iter().all(|s| s.active && s.value == 0.0));
}
#[test]
fn adoption_mapping_history_initialize_reset_and_bypass_keep_explicit_policies() {
    let p = patch(&[("velocity", 1.0), ("mpe", 1.0)]);
    let mut e = p.compile().unwrap();
    let h = e.resolve_module_state("gate").unwrap();
    e.handle_midi(on(15, 48, 73));
    e.arm_midi_gate_learn(h, 1, 0);
    let mut same = p.compile().unwrap();
    same.adopt_state_from(&e);
    let sh = same.resolve_module_state("gate").unwrap();
    assert_eq!(same.midi_gate_learning(sh), Some((1, 0)));
    assert_eq!(frame(&mut same), frame(&mut e));
    let mut changed = p.clone();
    changed.nodes[0].params.insert("note1".into(), 80.0);
    let mut changed = changed.compile().unwrap();
    changed.adopt_state_from(&same);
    let h = changed.resolve_module_state("gate").unwrap();
    assert_eq!(changed.midi_gate_learning(h), None);
    assert!(frame(&mut changed).channel(15) > 0.0);
    changed.handle_midi(off(15, 48));
    assert!(frame(&mut changed).channel(15) > 0.0);
    assert!(
        changed.set_module_saved_state(h, SavedModuleState::MidiGateMappings(DEFAULT_ASSIGNMENTS))
    );
    assert!(frame(&mut changed).channel(15) > 0.0);
    changed.handle_midi(off(15, 48));
    for _ in 0..100 {
        frame(&mut changed);
    }
    assert_eq!(frame(&mut changed).channel(15), 0.0);
    changed.handle_midi(on(0, 48, 127));
    changed.arm_midi_gate_learn(h, 3, 1);
    changed.initialize_module_state(h);
    assert_eq!(frame(&mut changed), PortFrame::mono(0.0));
    assert_eq!(changed.midi_gate_learning(h), None);
    assert_eq!(
        changed.module_saved_state(h),
        Some(SavedModuleState::MidiGateMappings(DEFAULT_ASSIGNMENTS))
    );
    changed.handle_midi(on(0, 48, 127));
    changed.reset_module_state(h);
    assert_eq!(frame(&mut changed).values[0], 0.0);
    assert_eq!(
        changed.module_saved_state(h),
        Some(SavedModuleState::MidiGateMappings(DEFAULT_ASSIGNMENTS))
    );
    let mut muted_p = p.clone();
    muted_p.nodes[0].bypassed = true;
    let mut muted = muted_p.compile().unwrap();
    muted.adopt_state_from(&e);
    assert_eq!(frame(&mut muted), PortFrame::mono(0.0));
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&muted);
    assert_eq!(frame(&mut resumed).values, [0.0; 16]);
}
