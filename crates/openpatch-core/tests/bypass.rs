use openpatch_core::*;
fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        bypassed: false,
        id: id.into(),
        kind: kind.into(),
        params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
    }
}
fn cable(from: &str, port: &str, to: &str, input: &str) -> Cable {
    Cable {
        input_order: None,
        output_order: None,
        color: None,
        id: None,
        from: from.into(),
        output: port.into(),
        to: to.into(),
        input: input.into(),
    }
}
fn patch(kind: &str, port: &str) -> Patch {
    Patch {
        nodes: vec![node("s", kind, &[])],
        output: Some("s".into()),
        output_port: port.into(),
        ..Patch::new_v2()
    }
}
fn one(e: &mut Engine) -> PortFrame {
    let mut frames = [PortFrame::default()];
    e.process_poly_block(&mut frames);
    frames[0]
}
fn on(e: &mut Engine) {
    e.handle_midi(MidiEvent::NoteOn {
        channel: 0,
        note: 60,
        velocity: 100,
    });
}
fn off(e: &mut Engine) {
    e.handle_midi(MidiEvent::NoteOff {
        channel: 0,
        note: 60,
        velocity: 0,
    });
}
#[test]
fn bypass_wire_default_is_compatible_strict_boolean_and_catalog_routes_are_explicit() {
    let old: Node = serde_json::from_value(serde_json::json!({"id":"s","kind":"gain"})).unwrap();
    assert!(!old.bypassed);
    assert!(serde_json::to_value(&old)
        .unwrap()
        .get("bypassed")
        .is_none());
    let mut enabled = old;
    enabled.bypassed = true;
    assert_eq!(serde_json::to_value(&enabled).unwrap()["bypassed"], true);
    for value in [
        serde_json::json!(1),
        serde_json::json!("true"),
        serde_json::Value::Null,
    ] {
        assert!(serde_json::from_value::<Node>(
            serde_json::json!({"id":"s","kind":"gain","bypassed":value})
        )
        .is_err());
    }
    let c = catalog();
    for m in c["modules"].as_array().unwrap() {
        assert_eq!(m["bypass"]["default"], false);
        assert!(m["bypass"]["routes"].is_array());
    }
    let m = c["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "mutes10")
        .unwrap();
    assert_eq!(m["bypass"]["routes"].as_array().unwrap().len(), 10);
}
#[test]
fn all_declared_routes_copy_exact_poly_frames_without_parameters_or_normaling() {
    for (kind, input, port) in [
        ("octave", "pitch", "out"),
        ("quantizer", "pitch", "out"),
        ("gain", "signal", "out"),
        ("lowpass", "signal", "out"),
        ("ladder_vcf", "signal", "out"),
        ("analog_vca", "signal", "out"),
        ("attenuverter", "signal", "out"),
        ("crossfade", "a", "out"),
    ] {
        let mut p = patch(kind, port);
        p.nodes[0].bypassed = true;
        assert_eq!(one(&mut p.compile().unwrap()), PortFrame::mono(0.0));
        p.nodes.push(node(
            "v",
            "poly_constant",
            &[("channels", 3.0), ("v1", 2.0), ("v2", -3.0), ("v3", 7.0)],
        ));
        p.cables.push(cable("v", "out", "s", input));
        let mut e = p.compile().unwrap();
        assert_eq!(one(&mut e), PortFrame::mono(0.0));
        let frame = one(&mut e);
        assert_eq!(frame.channels, 3);
        assert_eq!(&frame.values[..3], &[2.0, -3.0, 7.0]);
        assert_eq!(&frame.values[3..], &[0.0; 13]);
        p.nodes[1].params.insert("channels".into(), 0.0);
        let mut e = p.compile().unwrap();
        one(&mut e);
        assert_eq!(one(&mut e), PortFrame::mono(0.0));
    }
}
#[test]
fn mute_bypass_uses_individual_inputs_ignores_mute_and_does_not_normal_downward() {
    let mut p = patch("mutes10", "out1");
    p.nodes[0].bypassed = true;
    p.nodes[0].params.insert("mute1".into(), 1.0);
    p.nodes.push(node(
        "v",
        "poly_constant",
        &[("channels", 3.0), ("v1", 2.0), ("v2", 4.0), ("v3", 6.0)],
    ));
    p.cables.push(cable("v", "out", "s", "in1"));
    let mut e = p.compile().unwrap();
    one(&mut e);
    assert_eq!(&one(&mut e).values[..3], &[2.0, 4.0, 6.0]);
    p.output_port = "out2".into();
    let mut e = p.compile().unwrap();
    one(&mut e);
    assert_eq!(one(&mut e), PortFrame::mono(0.0));
}
#[test]
fn no_route_modules_publish_mono_zero_on_every_named_output() {
    for (kind, ports) in [
        ("attenuverter8", vec!["out1", "out8"]),
        ("cv_mix", vec!["out"]),
        ("mult", vec!["out1", "out8"]),
        ("multi_lfo", vec!["sine", "triangle", "saw", "square"]),
        ("seq3", vec!["cv1", "clock", "step1"]),
        ("switch_1to4", vec!["out1", "out4"]),
        ("poly_split", vec!["out1", "out16"]),
        ("audio_stereo", vec!["out"]),
    ] {
        for port in ports {
            let mut p = patch(kind, port);
            p.nodes[0].bypassed = true;
            let mut e = p.compile().unwrap();
            for _ in 0..130 {
                assert_eq!(one(&mut e), PortFrame::mono(0.0));
            }
        }
    }
}
#[test]
fn bypass_freezes_ordinary_dsp_and_resume_preserves_phase_in_both_versions() {
    for (version, kind, port) in [(1, "oscillator", "out"), (2, "multi_lfo", "sine")] {
        let mut p = patch(kind, port);
        p.version = version;
        let mut old = p.compile().unwrap();
        for _ in 0..37 {
            one(&mut old);
        }
        let mut expected = p.compile().unwrap();
        expected.adopt_state_from(&old);
        let mut next = [PortFrame::default(); 19];
        expected.process_poly_block(&mut next);
        p.nodes[0].bypassed = true;
        let mut muted = p.compile().unwrap();
        muted.adopt_state_from(&old);
        for _ in 0..100 {
            assert_eq!(one(&mut muted), PortFrame::mono(0.0));
        }
        p.nodes[0].bypassed = false;
        let mut resumed = p.compile().unwrap();
        resumed.adopt_state_from(&muted);
        let mut actual = [PortFrame::default(); 19];
        resumed.process_poly_block(&mut actual);
        assert_eq!(actual, next);
    }
}
#[test]
fn bypass_transition_preserves_incoming_frames_but_clears_outgoing_delay_to_mono_zero() {
    let mut p = patch("gain", "out");
    p.nodes[0].params.insert("gain".into(), 2.0);
    p.nodes.push(node("v", "constant", &[("value", 3.0)]));
    p.nodes.push(node("tail", "mult", &[]));
    p.cables = vec![
        cable("v", "out", "s", "signal"),
        cable("s", "out", "tail", "signal"),
    ];
    let mut old = p.compile().unwrap();
    one(&mut old);
    assert_eq!(one(&mut old), PortFrame::mono(6.0));
    p.nodes[0].bypassed = true;
    let mut active = p.compile().unwrap();
    active.adopt_state_from(&old);
    assert_eq!(one(&mut active), PortFrame::mono(3.0)); // Existing incoming frame remains.
    p.output = Some("tail".into());
    p.output_port = "out1".into();
    let mut tail = p.compile().unwrap();
    tail.adopt_state_from(&old);
    assert_eq!(one(&mut tail), PortFrame::mono(0.0));
    assert_eq!(one(&mut tail), PortFrame::mono(3.0));
    p.nodes[0].bypassed = false;
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&tail);
    assert_eq!(one(&mut resumed), PortFrame::mono(0.0));
    assert_eq!(one(&mut resumed), PortFrame::mono(6.0));
}
#[test]
fn bypassed_seq_rejects_actions_and_transitions_discard_pending_actions_but_keep_position() {
    let mut p = patch("seq3", "cv1");
    p.nodes[0].params.insert("tempo".into(), 15.0);
    let mut old = p.compile().unwrap();
    let h = old.resolve_module_event("s").unwrap();
    old.handle_module_event(h, ModuleAction::Clock);
    one(&mut old);
    old.handle_module_event(h, ModuleAction::Clock);
    p.nodes[0].bypassed = true;
    let mut muted = p.compile().unwrap();
    muted.adopt_state_from(&old);
    assert!(muted.resolve_module_event("s").is_err());
    let meter = muted.resolve_module_telemetry("s").unwrap();
    assert!(!muted.handle_module_event(meter, ModuleAction::Clock));
    for _ in 0..10 {
        one(&mut muted);
    }
    assert_eq!(muted.module_telemetry(meter).unwrap().step, 2);
    p.nodes[0].bypassed = false;
    let mut resumed = p.compile().unwrap();
    let h = resumed.resolve_module_event("s").unwrap();
    assert!(resumed.handle_module_event(h, ModuleAction::Clock));
    resumed.adopt_state_from(&muted);
    one(&mut resumed);
    assert_eq!(resumed.module_telemetry(h).unwrap().step, 2);
}
#[test]
fn midi_bypass_tracks_releases_sustain_and_ages_pulses_while_publishing_mono_zero() {
    let mut p = patch("midi_cv", "gate");
    p.nodes[0].params.insert("channels".into(), 1.0);
    let mut old = p.compile().unwrap();
    on(&mut old);
    assert_eq!(one(&mut old), PortFrame::mono(10.0));
    p.nodes[0].bypassed = true;
    let mut muted = p.compile().unwrap();
    muted.adopt_state_from(&old);
    muted.handle_midi(MidiEvent::ControlChange {
        channel: 0,
        controller: 64,
        value: 127,
    });
    off(&mut muted);
    assert_eq!(one(&mut muted), PortFrame::mono(0.0));
    muted.handle_midi(MidiEvent::ControlChange {
        channel: 0,
        controller: 64,
        value: 0,
    });
    for _ in 0..100 {
        one(&mut muted);
    }
    p.nodes[0].bypassed = false;
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&muted);
    assert_eq!(one(&mut resumed), PortFrame::mono(0.0));
    on(&mut resumed);
    assert_eq!(one(&mut resumed), PortFrame::mono(10.0));
    p.nodes[0].bypassed = true;
    p.output_port = "retrigger".into();
    let mut muted = p.compile().unwrap();
    muted.adopt_state_from(&resumed);
    for _ in 0..100 {
        one(&mut muted);
    }
    p.nodes[0].bypassed = false;
    let mut retrigger = p.compile().unwrap();
    retrigger.adopt_state_from(&muted);
    assert_eq!(one(&mut retrigger), PortFrame::mono(0.0));
    p.output_port = "gate".into();
    let mut held = p.compile().unwrap();
    held.adopt_state_from(&muted);
    assert_eq!(one(&mut held), PortFrame::mono(10.0));

    // Notes first pressed during bypass must also become audible on resume.
    p.nodes[0].bypassed = true;
    let mut initially_bypassed = p.compile().unwrap();
    on(&mut initially_bypassed);
    for _ in 0..100 {
        assert_eq!(one(&mut initially_bypassed), PortFrame::mono(0.0));
    }
    p.nodes[0].bypassed = false;
    let mut started_during_bypass = p.compile().unwrap();
    started_during_bypass.adopt_state_from(&initially_bypassed);
    assert_eq!(one(&mut started_during_bypass), PortFrame::mono(10.0));
}

#[test]
fn legacy_bypass_routes_and_self_loop_transition_obey_the_same_boundary_policy() {
    let mut p = patch("gain", "out");
    p.version = 1;
    p.nodes[0].params.insert("gain".into(), 4.0);
    p.nodes[0].bypassed = true;
    p.nodes.push(node("v", "constant", &[("value", 3.0)]));
    p.cables.push(cable("v", "out", "s", "signal"));
    assert_eq!(one(&mut p.compile().unwrap()), PortFrame::mono(3.0)); // Legacy DAG still processes same-sample.
    p.cables.clear();
    assert_eq!(one(&mut p.compile().unwrap()), PortFrame::mono(0.0)); // Bypass does not normal input.
    let mut p = patch("seq3", "cv1");
    p.nodes[0].params.insert("row1_1".into(), 10.0);
    p.nodes[0].params.insert("row1_2".into(), 10.0);
    p.nodes[0].params.insert("tempo".into(), 15.0);
    p.cables.push(cable("s", "cv1", "s", "clock"));
    let mut old = p.compile().unwrap();
    one(&mut old);
    one(&mut old);
    p.nodes[0].bypassed = true;
    let mut muted = p.compile().unwrap();
    muted.adopt_state_from(&old);
    one(&mut muted);
    p.nodes[0].bypassed = false;
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&muted);
    let h = resumed.resolve_module_event("s").unwrap();
    one(&mut resumed);
    assert_eq!(resumed.module_telemetry(h).unwrap().step, 2); // Cleared self-loop low rearms.
    one(&mut resumed);
    assert_eq!(resumed.module_telemetry(h).unwrap().step, 3); // Fresh high arrives next sample.
}
