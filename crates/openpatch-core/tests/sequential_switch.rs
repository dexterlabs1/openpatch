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
        from: from.into(),
        output: port.into(),
        to: to.into(),
        input: input.into(),
    }
}
fn patch(kind: &str, port: &str, declick: f32) -> Patch {
    let mut p = Patch {
        sample_rate: 8000,
        nodes: vec![
            node("s", kind, &[("declick", declick)]),
            node("c", "midi_cv", &[("channels", 1.0), ("channel", 1.0)]),
            node("r", "midi_cv", &[("channels", 1.0), ("channel", 2.0)]),
            node(
                "v",
                "poly_constant",
                &[("channels", 3.0), ("v1", 2.0), ("v2", -3.0), ("v3", 7.0)],
            ),
        ],
        cables: vec![
            cable("c", "gate", "s", "clock"),
            cable("r", "gate", "s", "reset"),
        ],
        output: Some("s".into()),
        output_port: port.into(),
        ..Patch::new_v2()
    };
    p.cables.push(cable(
        "v",
        "out",
        "s",
        if kind == "switch_1to4" {
            "signal"
        } else {
            "in1"
        },
    ));
    p
}
fn one(e: &mut Engine) -> PortFrame {
    let mut out = [PortFrame::default()];
    e.process_poly_block(&mut out);
    out[0]
}
fn note(e: &mut Engine, channel: u8, down: bool) {
    e.handle_midi(if down {
        MidiEvent::NoteOn {
            channel,
            note: 60,
            velocity: 100,
        }
    } else {
        MidiEvent::NoteOff {
            channel,
            note: 60,
            velocity: 0,
        }
    });
}
fn pulse(e: &mut Engine, channel: u8) {
    note(e, channel, false);
    one(e);
    one(e);
    note(e, channel, true);
    one(e);
    one(e);
}
#[test]
fn exact_catalog_ports_defaults_and_strict_validation_preserve_legacy() {
    let catalog = catalog();
    for (kind, inputs, outputs) in [
        (
            "switch_1to4",
            vec!["clock", "reset", "signal"],
            vec!["out1", "out2", "out3", "out4"],
        ),
        (
            "switch_4to1",
            vec!["clock", "reset", "in1", "in2", "in3", "in4"],
            vec!["out"],
        ),
    ] {
        let entry = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        assert_eq!(entry["inputs"], serde_json::json!(inputs));
        assert_eq!(entry["outputs"], serde_json::json!(outputs));
        assert_eq!(entry["params"]["steps"]["default"], 4.0);
        assert_eq!(entry["params"]["declick"]["default"], 0.0);
        assert_eq!(entry["min_patch_version"], 2);
        for metadata in entry["output_metadata"].as_array().unwrap() {
            assert_eq!(metadata["maximum_channels"], 16);
            assert_eq!(metadata["units"], "V");
        }
        let mut p = patch(kind, outputs[0], 0.0);
        assert!(p.compile().is_ok());
        p.version = 1;
        assert!(p.compile().is_err());
        p.version = 2;
        for (name, value) in [
            ("steps", 1.0),
            ("steps", 5.0),
            ("steps", 2.5),
            ("declick", 0.5),
            ("unknown", 1.0),
        ] {
            let mut bad = p.clone();
            bad.nodes[0].params.insert(name.into(), value);
            assert!(bad.compile().is_err());
        }
        p.output_port = "missing".into();
        assert!(p.compile().is_err());
    }
}
#[test]
fn telemetry_handles_are_engine_bound_and_do_not_grant_switch_actions() {
    for (kind, port) in [("switch_1to4", "out1"), ("switch_4to1", "out")] {
        let p = patch(kind, port, 0.0);
        let mut a = p.compile().unwrap();
        let mut b = p.compile().unwrap();
        let h = a.resolve_module_telemetry("s").unwrap();
        assert_eq!(
            a.module_telemetry(h),
            Some(ModuleTelemetry {
                step: 1,
                length: 4,
                running: true
            })
        );
        assert!(a.resolve_module_event("s").is_err());
        assert!(a.resolve_module_telemetry("v").is_err());
        assert!(a.resolve_module_telemetry("missing").is_err());
        for action in [ModuleAction::Clock, ModuleAction::Run, ModuleAction::Reset] {
            assert!(!a.handle_module_event(h, action));
        }
        assert_eq!(b.module_telemetry(h), None);
        assert!(!b.reset_module_state(h));
        pulse(&mut a, 0);
        assert_eq!(a.module_telemetry(h).unwrap().step, 2);
        assert!(a.reset_module_state(h));
        assert_eq!(a.module_telemetry(h).unwrap().step, 1);
    }
    let seq = Patch {
        nodes: vec![node("s", "seq3", &[])],
        ..Patch::new_v2()
    }
    .compile()
    .unwrap();
    assert_eq!(
        seq.resolve_module_event("s").unwrap(),
        seq.resolve_module_telemetry("s").unwrap()
    );
    assert!(Patch::default()
        .compile()
        .unwrap()
        .resolve_module_telemetry("s")
        .is_err());
}
#[test]
fn one_to_four_routes_all_lanes_and_clock_then_reset_priority_without_guard() {
    let mut e = patch("switch_1to4", "out2", 0.0).compile().unwrap();
    let h = e.resolve_module_telemetry("s").unwrap();
    assert_eq!(one(&mut e), PortFrame::mono(0.0));
    assert_eq!(one(&mut e).channels, 3);
    note(&mut e, 0, true);
    assert_eq!(one(&mut e).values, [0.0; 16]);
    assert_eq!(&one(&mut e).values[..3], &[2.0, -3.0, 7.0]);
    for _ in 0..20 {
        one(&mut e);
    }
    assert_eq!(e.module_telemetry(h).unwrap().step, 2); // Held high never repeats.
    note(&mut e, 0, false);
    one(&mut e);
    one(&mut e);
    note(&mut e, 0, true);
    note(&mut e, 1, true);
    one(&mut e);
    one(&mut e);
    assert_eq!(e.module_telemetry(h).unwrap().step, 1);
    pulse(&mut e, 0);
    assert_eq!(e.module_telemetry(h).unwrap().step, 2); // Still inside seq3's 1ms guard duration.
    pulse(&mut e, 0);
    pulse(&mut e, 0);
    pulse(&mut e, 0);
    assert_eq!(e.module_telemetry(h).unwrap().step, 1);
}
#[test]
fn four_to_one_unselected_inputs_determine_channels_and_raw_mono_is_zero_padded() {
    let mut p = patch("switch_4to1", "out", 0.0);
    p.nodes[0].params.insert("steps".into(), 2.0);
    p.nodes[3].params.insert("channels".into(), 1.0);
    p.nodes
        .push(node("wide", "poly_constant", &[("channels", 16.0)]));
    p.cables.push(cable("wide", "out", "s", "in4"));
    let mut e = p.compile().unwrap();
    one(&mut e);
    let out = one(&mut e);
    assert_eq!(out.channels, 16);
    assert_eq!(out.values[0], 2.0);
    assert_eq!(&out.values[1..], &[0.0; 15]);
    pulse(&mut e, 0);
    assert_eq!(one(&mut e).values, [0.0; 16]); // Unpatched selected input is silence.
    p.nodes[0].params.insert("declick".into(), 1.0);
    let mut smooth = p.compile().unwrap();
    one(&mut smooth);
    let out = one(&mut smooth);
    assert_eq!(out.channels, 16);
    assert_eq!(out.values, [0.2; 16]);
}
#[test]
fn adoption_shrink_wraps_on_next_sample_and_rate_or_kind_changes_start_fresh() {
    let p = patch("switch_1to4", "out1", 1.0);
    let mut old = p.compile().unwrap();
    pulse(&mut old, 0);
    pulse(&mut old, 0);
    pulse(&mut old, 0);
    let old_h = old.resolve_module_telemetry("s").unwrap();
    assert_eq!(old.module_telemetry(old_h).unwrap().step, 4);
    let mut p2 = p.clone();
    p2.nodes[0].params.insert("steps".into(), 2.0);
    let mut new = p2.compile().unwrap();
    new.adopt_state_from(&old);
    let h = new.resolve_module_telemetry("s").unwrap();
    assert_eq!(new.module_telemetry(h).unwrap().step, 4);
    one(&mut new);
    assert_eq!(new.module_telemetry(h).unwrap().step, 1);
    p2.sample_rate = 16000;
    let mut rate = p2.compile().unwrap();
    rate.adopt_state_from(&old);
    let h = rate.resolve_module_telemetry("s").unwrap();
    assert_eq!(rate.module_telemetry(h).unwrap().step, 1);
    let mut kind = patch("switch_4to1", "out", 1.0).compile().unwrap();
    kind.adopt_state_from(&old);
    let h = kind.resolve_module_telemetry("s").unwrap();
    assert_eq!(kind.module_telemetry(h).unwrap().step, 1);
}
#[test]
fn detector_adoption_retains_high_and_rewiring_rearms_only_after_a_fresh_low() {
    let mut p = patch("switch_1to4", "out2", 0.0);
    let mut old = p.compile().unwrap();
    pulse(&mut old, 0);
    let mut same = p.compile().unwrap();
    same.adopt_state_from(&old);
    one(&mut same);
    let h = same.resolve_module_telemetry("s").unwrap();
    assert_eq!(same.module_telemetry(h).unwrap().step, 2);
    p.nodes.push(node("high", "constant", &[("value", 10.0)]));
    p.cables[0] = cable("high", "out", "s", "clock");
    let mut changed = p.compile().unwrap();
    changed.adopt_state_from(&old);
    let h = changed.resolve_module_telemetry("s").unwrap();
    one(&mut changed); // Cleared edge arms low.
    one(&mut changed);
    assert_eq!(changed.module_telemetry(h).unwrap().step, 3);
    one(&mut changed);
    assert_eq!(changed.module_telemetry(h).unwrap().step, 3);
}
#[test]
fn switch_chains_keep_one_sample_per_cable_and_callback_partition_invariance() {
    let mut p = patch("switch_1to4", "out1", 0.0);
    p.nodes.push(node("t", "switch_4to1", &[]));
    p.cables.push(cable("s", "out1", "t", "in1"));
    p.output = Some("t".into());
    p.output_port = "out".into();
    let mut a = p.compile().unwrap();
    let mut expected = [PortFrame::default(); 259];
    a.process_poly_block(&mut expected);
    assert_eq!(expected[0], PortFrame::mono(0.0));
    assert_eq!(expected[1], PortFrame::mono(0.0));
    assert_eq!(&expected[2].values[..3], &[2.0, -3.0, 7.0]);
    let mut b = p.compile().unwrap();
    let mut actual = [PortFrame::default(); 259];
    for range in [0..1, 1..4, 4..131, 131..259] {
        b.process_poly_block(&mut actual[range]);
    }
    assert_eq!(actual, expected);
}
