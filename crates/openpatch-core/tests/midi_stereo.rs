use openpatch_core::*;
use std::collections::BTreeMap;
fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        content: None,
        score: None,
        bypassed: false,
        id: id.into(),
        kind: kind.into(),
        params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
    }
}
fn midi(output: &str, params: &[(&str, f32)]) -> Patch {
    Patch {
        nodes: vec![node("m", "midi_cv", params)],
        output: Some("m".into()),
        output_port: output.into(),
        ..Patch::new_v2()
    }
}
fn on(channel: u8, note: u8) -> MidiEvent {
    MidiEvent::NoteOn {
        channel,
        note,
        velocity: 127,
    }
}
fn off(channel: u8, note: u8) -> MidiEvent {
    MidiEvent::NoteOff {
        channel,
        note,
        velocity: 0,
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
    let mut f = [PortFrame::default()];
    e.process_poly_block(&mut f);
    f[0]
}
#[test]
fn parser_rejects_partial_trailing_running_and_invalid_messages() {
    assert_eq!(parse_midi(&[0x90, 60, 0]), Some(off(0, 60)));
    assert_eq!(
        parse_midi(&[0xef, 0, 64]),
        Some(MidiEvent::PitchBend {
            channel: 15,
            value: 8192
        })
    );
    for bytes in [
        &[][..],
        &[0x90, 60],
        &[0x90, 60, 128],
        &[60, 1, 2],
        &[0x90, 60, 1, 2],
        &[0xc0, 5],
        &[0xf0, 0xf7],
        &[0xf8, 0],
    ] {
        assert_eq!(parse_midi(bytes), None, "{bytes:?}");
    }
    assert_eq!(
        parse_midi(&[0xd3, 127]),
        Some(MidiEvent::ChannelPressure {
            channel: 3,
            pressure: 127
        })
    );
    assert!(on(16, 60).validate().is_err());
    assert!(MidiEvent::PitchBend {
        channel: 0,
        value: 16384
    }
    .validate()
    .is_err());
    assert!(serde_json::from_str::<MidiEvent>(r#"{"type":"clock","unexpected":1}"#).is_err());
    assert!(serde_json::from_str::<TimedMidiEvent>(
        r#"{"frame":0,"event":{"type":"clock"},"extra":1}"#
    )
    .is_err());
}
#[test]
fn stable_voice_frames_rotate_steal_release_and_panic() {
    let mut e = midi("pitch", &[("channels", 2.0)]).compile().unwrap();
    assert_eq!(
        frame(&mut e),
        PortFrame {
            channels: 2,
            ..PortFrame::default()
        }
    );
    e.handle_midi(on(0, 60));
    e.handle_midi(on(0, 72));
    assert_eq!(&frame(&mut e).values[..2], &[0.0, 1.0]);
    e.handle_midi(on(0, 84));
    assert_eq!(&frame(&mut e).values[..2], &[2.0, 1.0]);
    e.handle_midi(off(0, 60)); // stolen note cannot release note 84
    let mut gates = midi("gate", &[("channels", 2.0)]).compile().unwrap();
    gates.adopt_state_from(&e);
    assert_eq!(&frame(&mut gates).values[..2], &[10.0, 10.0]);
    e.handle_midi(off(0, 84));
    assert_eq!(&frame(&mut e).values[..2], &[2.0, 1.0]); // tail pitch latches
    gates.adopt_state_from(&e);
    assert_eq!(&frame(&mut gates).values[..2], &[0.0, 10.0]);
    gates.handle_midi(MidiEvent::Panic);
    assert_eq!(&frame(&mut gates).values[..2], &[0.0, 0.0]);
    gates.handle_midi(on(0, 60));
    assert_eq!(&frame(&mut gates).values[..2], &[10.0, 0.0]);
}
#[test]
fn reuse_duplicate_notes_balance_and_sustain_is_channel_local() {
    let mut e = midi("gate", &[("channels", 2.0), ("allocation", 1.0)])
        .compile()
        .unwrap();
    e.handle_midi(on(0, 60));
    e.handle_midi(on(0, 60));
    e.handle_midi(off(0, 60));
    assert_eq!(&frame(&mut e).values[..2], &[10.0, 0.0]);
    e.handle_midi(cc(0, 64, 127));
    e.handle_midi(off(0, 60));
    e.handle_midi(on(1, 67));
    e.handle_midi(off(1, 67));
    assert_eq!(&frame(&mut e).values[..2], &[10.0, 0.0]);
    e.handle_midi(cc(0, 64, 0));
    assert_eq!(&frame(&mut e).values[..2], &[0.0, 0.0]);
    e.handle_midi(on(0, 60));
    e.handle_midi(cc(0, 64, 127));
    e.handle_midi(cc(0, 123, 0));
    assert_eq!(frame(&mut e).values[0], 10.0);
    e.handle_midi(cc(0, 121, 0));
    assert_eq!(frame(&mut e).values[0], 0.0);
    e.handle_midi(on(0, 60));
    e.handle_midi(cc(0, 64, 127));
    e.handle_midi(cc(0, 120, 0));
    assert_eq!(frame(&mut e).values[0], 0.0);
}
#[test]
fn reset_compacts_and_mono_falls_back_to_last_held_note() {
    let mut e = midi("pitch", &[("channels", 3.0), ("allocation", 2.0)])
        .compile()
        .unwrap();
    for n in [60, 72, 84] {
        e.handle_midi(on(0, n));
    }
    e.handle_midi(off(0, 72));
    assert_eq!(&frame(&mut e).values[..2], &[0.0, 2.0]);
    let mut g = midi("gate", &[("channels", 3.0), ("allocation", 2.0)])
        .compile()
        .unwrap();
    g.adopt_state_from(&e);
    assert_eq!(&frame(&mut g).values[..3], &[10.0, 10.0, 0.0]);
    let mut m = midi("pitch", &[("channels", 1.0)]).compile().unwrap();
    m.handle_midi(on(0, 60));
    m.handle_midi(on(0, 72));
    m.handle_midi(on(0, 67));
    assert_eq!(frame(&mut m).values[0], 7.0 / 12.0);
    m.handle_midi(off(0, 67));
    assert_eq!(frame(&mut m).values[0], 1.0);
    m.handle_midi(off(0, 72));
    assert_eq!(frame(&mut m).values[0], 0.0);
}
#[test]
fn controllers_channel_filter_and_trigger_duration() {
    let mut e = midi("pitchwheel", &[("channel", 2.0)]).compile().unwrap();
    e.handle_midi(MidiEvent::PitchBend {
        channel: 0,
        value: 16383,
    });
    assert_eq!(frame(&mut e).values[0], 0.0);
    e.handle_midi(MidiEvent::PitchBend {
        channel: 1,
        value: 16383,
    });
    assert_eq!(frame(&mut e).values[0], 5.0);
    e.handle_midi(MidiEvent::PitchBend {
        channel: 1,
        value: 0,
    });
    assert_eq!(frame(&mut e).values[0], -5.0);
    let mut p = midi("pitch", &[("channel", 2.0)]).compile().unwrap();
    p.adopt_state_from(&e);
    p.handle_midi(on(1, 72));
    assert_eq!(frame(&mut p).values[0], 1.0);
    let mut t = midi("retrigger", &[]).compile().unwrap();
    t.handle_midi(on(0, 60));
    let mut frames = [PortFrame::default(); 49];
    t.process_poly_block(&mut frames);
    assert!(frames[..48].iter().all(|f| f.values[0] == 10.0));
    assert_eq!(frames[48].values[0], 0.0);
    for (port, event) in [
        ("clock", MidiEvent::Clock),
        ("start", MidiEvent::Start),
        ("stop", MidiEvent::Stop),
        ("continue", MidiEvent::Continue),
    ] {
        let mut e = midi(port, &[]).compile().unwrap();
        e.handle_midi(event);
        assert_eq!(frame(&mut e).values[0], 10.0);
        e.handle_midi(MidiEvent::Panic);
        assert_eq!(frame(&mut e).values[0], 0.0);
    }
}
#[test]
fn adoption_preserves_held_sustain_and_resets_changed_config() {
    let mut old = midi("gate", &[]).compile().unwrap();
    old.handle_midi(on(0, 60));
    old.handle_midi(cc(0, 64, 127));
    old.handle_midi(off(0, 60));
    let mut new = midi("gate", &[]).compile().unwrap();
    new.adopt_state_from(&old);
    assert_eq!(frame(&mut new).values[0], 10.0);
    new.handle_midi(cc(0, 64, 0));
    assert_eq!(frame(&mut new).values[0], 0.0);
    for params in [
        [("channels", 2.0)],
        [("channel", 1.0)],
        [("allocation", 1.0)],
    ] {
        let mut reset = midi("gate", &params).compile().unwrap();
        reset.adopt_state_from(&old);
        assert_eq!(frame(&mut reset).values[0], 0.0);
    }
}
#[test]
fn event_trace_sample_timing_same_frame_order_and_partition_invariance() {
    let p = midi("gate", &[("channels", 1.0)]);
    let events = [
        TimedMidiEvent {
            frame: 2,
            event: on(0, 60),
        },
        TimedMidiEvent {
            frame: 5,
            event: off(0, 60),
        },
        TimedMidiEvent {
            frame: 5,
            event: on(0, 72),
        },
        TimedMidiEvent {
            frame: 9,
            event: MidiEvent::Panic,
        },
    ];
    let audio = render_audio_with_events(&p, 12.0 / 48000.0, &events).unwrap();
    assert_eq!(
        audio.samples,
        vec![0.0, 0.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 0.0, 0.0, 0.0]
    );
    let mut e = p.compile().unwrap();
    let mut manual = Vec::new();
    let mut next = 0;
    for index in 0..12 {
        while next < events.len() && events[next].frame == index {
            e.handle_midi(events[next].event);
            next += 1;
        }
        let mut sample = [0.0];
        e.process_block(&mut sample);
        manual.extend(sample);
    }
    assert_eq!(manual, audio.samples);
    assert!(render_audio_with_events(&p, 1.0, &[events[1], events[0]]).is_err());
    assert!(validate_midi_events(
        &[TimedMidiEvent {
            frame: 12,
            event: MidiEvent::Clock
        }],
        12
    )
    .is_err());
    assert!(validate_midi_events(&vec![events[0]; MAX_MIDI_EVENTS + 1], 12).is_err());
    assert!(render_audio_with_events(&p, 0.0, &events[..1]).is_err());
}
fn stereo(right: bool, channels: f32) -> Patch {
    let mut p = Patch {
        nodes: vec![
            node(
                "l",
                "poly_constant",
                &[("channels", channels), ("v1", 2.0), ("v2", 3.0)],
            ),
            node("r", "constant", &[("value", -4.0)]),
            node("a", "audio_stereo", &[]),
        ],
        output: Some("a".into()),
        ..Patch::new_v2()
    };
    p.cables.push(Cable {
        input_order: None,
        output_order: None,
        color: None,
        id: None,
        from: "l".into(),
        to: "a".into(),
        input: "left".into(),
        output: "out".into(),
    });
    if right {
        p.cables.push(Cable {
            input_order: None,
            output_order: None,
            color: None,
            id: None,
            from: "r".into(),
            to: "a".into(),
            input: "right".into(),
            output: "out".into(),
        });
    }
    p
}
#[test]
fn stereo_is_explicit_isolated_summed_and_right_normalled() {
    let p = stereo(true, 2.0);
    let mut e = p.compile().unwrap();
    assert_eq!(e.output_channels(), 2);
    let mut block = [[0.0; 2]; 3];
    e.process_audio_block(&mut block);
    assert_eq!(block, [[0.0; 2], [0.5, -0.4], [0.5, -0.4]]);
    let audio = render_audio(&p, 3.0 / 48000.0).unwrap();
    assert_eq!(audio.channels, 2);
    assert_eq!(audio.samples, vec![0.0, 0.0, 0.5, -0.4, 0.5, -0.4]);
    let mut e = stereo(false, 2.0).compile().unwrap();
    e.process_audio_block(&mut block);
    assert_eq!(block[2], [0.5, 0.5]);
    let mut p = stereo(true, 2.0);
    p.nodes[1] = node("r", "poly_constant", &[("channels", 0.0)]);
    let mut e = p.compile().unwrap();
    e.process_audio_block(&mut block);
    assert_eq!(block[2], [0.5, 0.5]);
    p.output = Some("l".into());
    let mut e = p.compile().unwrap();
    assert_eq!(e.output_channels(), 1);
    e.process_audio_block(&mut block);
    assert_eq!(block[2], [2.0, 2.0]);
    assert_eq!(
        render(&stereo(true, 2.0), 3.0 / 48000.0).unwrap(),
        vec![0.0, 0.5, 0.5]
    );
}
#[test]
fn stereo_wav_header_alignment_clipping_and_no_overwrite() {
    let path = std::env::temp_dir().join(format!("openpatch-stereo-{}.wav", std::process::id()));
    let _ = std::fs::remove_file(&path);
    assert!(write_wav_interleaved(&path, &[1.0], 48000, 2).is_err());
    assert!(!path.exists());
    write_wav_interleaved(&path, &[-2.0, 2.0, f32::NAN, 0.0], 48000, 2).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[22..24], &2u16.to_le_bytes());
    assert_eq!(&bytes[28..32], &192000u32.to_le_bytes());
    assert_eq!(&bytes[32..34], &4u16.to_le_bytes());
    assert_eq!(&bytes[44..48], &[0, 128, 255, 127]);
    assert!(write_wav_interleaved(&path, &[], 48000, 2).is_err());
    std::fs::remove_file(&path).unwrap();
}
#[test]
fn invalid_module_parameters_and_v1_modules_rejected() {
    for (name, value) in [
        ("channels", 0.0),
        ("channels", 1.5),
        ("channel", 17.0),
        ("allocation", 3.0),
        ("clock_div", 0.0),
        ("clock_div", 97.0),
        ("unknown", 1.0),
    ] {
        assert!(midi("gate", &[(name, value)]).validate().is_err());
    }
    let mut p = midi("gate", &[]);
    p.version = 1;
    assert!(p.validate().is_err());
    let mut p = stereo(true, 2.0);
    p.nodes[2].params = BTreeMap::from([("gain".into(), 3.0)]);
    assert!(p.validate().is_err());
}

#[test]
fn controllers_have_exact_ranges_and_per_node_state() {
    let mut p = midi("aftertouch", &[("channels", 2.0)]);
    let mut pressure = p.compile().unwrap();
    pressure.handle_midi(on(0, 60));
    pressure.handle_midi(on(1, 72));
    pressure.handle_midi(MidiEvent::ChannelPressure {
        channel: 0,
        pressure: 127,
    });
    pressure.handle_midi(MidiEvent::PolyPressure {
        channel: 1,
        note: 72,
        pressure: 127,
    });
    assert_eq!(&frame(&mut pressure).values[..2], &[10.0, 0.0]);
    p.output_port = "modwheel".into();
    let mut modulation = p.compile().unwrap();
    modulation.adopt_state_from(&pressure);
    modulation.handle_midi(cc(1, 1, 127));
    assert_eq!(&frame(&mut modulation).values[..2], &[0.0, 10.0]);
    modulation.handle_midi(cc(1, 121, 0));
    assert_eq!(&frame(&mut modulation).values[..2], &[0.0, 0.0]);
    p.output_port = "velocity".into();
    let mut velocity = p.compile().unwrap();
    velocity.adopt_state_from(&pressure);
    velocity.handle_midi(off(0, 60));
    assert_eq!(&frame(&mut velocity).values[..2], &[10.0, 10.0]);
    // Independent filters and allocators on two nodes receiving one event bus.
    let mut p = Patch {
        nodes: vec![
            node("a", "midi_cv", &[("channel", 1.0)]),
            node("b", "midi_cv", &[("channel", 2.0)]),
        ],
        output: Some("a".into()),
        output_port: "gate".into(),
        ..Patch::new_v2()
    };
    let mut a = p.compile().unwrap();
    a.handle_midi(on(1, 60));
    assert_eq!(frame(&mut a).values[0], 0.0);
    p.output = Some("b".into());
    let mut b = p.compile().unwrap();
    b.adopt_state_from(&a);
    assert_eq!(frame(&mut b).values[0], 10.0);
}
#[test]
fn divided_clock_adopts_phase_and_saturating_duplicate_count_never_wraps() {
    let p = midi("clock_div", &[("clock_div", 3.0)]);
    let mut e = p.compile().unwrap();
    e.handle_midi(MidiEvent::Start);
    for (index, expected) in [10.0, 0.0, 0.0, 10.0, 0.0, 0.0].into_iter().enumerate() {
        e.handle_midi(MidiEvent::Clock);
        assert_eq!(frame(&mut e).values[0], expected, "clock {index}");
        let mut drain = [PortFrame::default(); 48];
        e.process_poly_block(&mut drain);
        let mut next = p.compile().unwrap();
        next.adopt_state_from(&e);
        e = next;
    }
    let mut e = midi("gate", &[("allocation", 1.0)]).compile().unwrap();
    for _ in 0..65_537 {
        e.handle_midi(on(0, 60));
    }
    e.handle_midi(off(0, 60));
    assert_eq!(frame(&mut e).values[0], 10.0);
    e.handle_midi(MidiEvent::Panic);
    e.handle_midi(on(0, 72));
    assert_eq!(frame(&mut e).values[0], 10.0);
}
#[test]
fn released_midi_voices_keep_downstream_adsr_tail_and_configuration_reset_clears_edge() {
    let mut p = Patch {
        nodes: vec![
            node("m", "midi_cv", &[("channels", 2.0)]),
            node("e", "adsr", &[("attack", 0.001), ("release", 0.1)]),
        ],
        cables: vec![Cable {
            input_order: None,
            output_order: None,
            color: None,
            id: None,
            from: "m".into(),
            to: "e".into(),
            output: "gate".into(),
            input: "gate".into(),
        }],
        output: Some("e".into()),
        ..Patch::new_v2()
    };
    let mut e = p.compile().unwrap();
    e.handle_midi(on(0, 60));
    e.handle_midi(on(0, 72));
    let mut held = [PortFrame::default(); 1000];
    e.process_poly_block(&mut held);
    e.handle_midi(off(0, 72));
    let mut release = [PortFrame::default(); 100];
    e.process_poly_block(&mut release);
    assert!(release.iter().all(|f| f.channels == 2 && f.values[1] > 0.0));
    assert!(release[99].values[1] < release[0].values[1]);
    // Changing MIDI config must not publish an old held gate for one extra sample.
    p.nodes[0].params.insert("channels".into(), 1.0);
    p.nodes[1] = node("e", "output", &[]);
    p.cables[0].input = "signal".into();
    let mut old = p.clone();
    old.nodes[0].params.insert("channels".into(), 2.0);
    let mut old = old.compile().unwrap();
    old.handle_midi(on(0, 60));
    frame(&mut old);
    let mut reset = p.compile().unwrap();
    reset.adopt_state_from(&old);
    assert_eq!(frame(&mut reset).values[0], 0.0);
}
