use openpatch_core::*;
fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        score: None,
        id: id.into(),
        kind: kind.into(),
        bypassed: false,
        params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
    }
}
fn cable(from: &str, to: &str, input: &str) -> Cable {
    Cable {
        input_order: None,
        output_order: None,
        color: None,
        id: None,
        from: from.into(),
        to: to.into(),
        input: input.into(),
        output: "out".into(),
    }
}
fn patch(kind: &str, port: &str) -> Patch {
    Patch {
        sample_rate: 8000,
        nodes: vec![node("s", kind, &[])],
        output: Some("s".into()),
        output_port: port.into(),
        ..Patch::new_v2()
    }
}
fn one(e: &mut Engine) -> PortFrame {
    let mut out = [PortFrame::default()];
    e.process_poly_block(&mut out);
    out[0]
}
#[test]
fn exact_ports_full_config_catalog_and_version_one_rejection() {
    let c = catalog();
    for (kind, inputs, outputs) in [
        (
            "logic",
            vec!["a", "b"],
            vec!["not_a", "not_b", "or", "nor", "and", "nand", "xor", "xnor"],
        ),
        (
            "compare",
            vec!["a", "b"],
            vec![
                "max",
                "min",
                "clip",
                "lim",
                "clip_gate",
                "lim_gate",
                "greater",
                "less",
            ],
        ),
        (
            "process",
            vec!["slew_cv", "signal", "gate"],
            vec!["sh1", "sh2", "track_hold", "hold_track", "slew", "glide"],
        ),
        ("rescale", vec!["signal"], vec!["out"]),
    ] {
        let m = c["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        assert_eq!(m["inputs"], serde_json::json!(inputs));
        assert_eq!(m["outputs"], serde_json::json!(outputs));
        assert_eq!(m["min_patch_version"], 2);
        for meta in m["output_metadata"].as_array().unwrap() {
            assert_eq!(meta["maximum_channels"], 16);
            assert_eq!(meta["units"], "V");
        }
        let mut p = patch(kind, outputs[0]);
        assert!(p.compile().is_ok());
        p.version = 1;
        assert!(p.compile().is_err());
        p.version = 2;
        p.nodes[0].params.insert("button".into(), 1.0);
        assert!(p.compile().is_err());
    }
    for (name, value) in [
        ("gain", 1.01),
        ("offset", 11.0),
        ("multiplier", 2.0),
        ("reflect_min", 0.5),
        ("reflect_max", 0.5),
    ] {
        let mut p = patch("rescale", "out");
        p.nodes[0].params.insert(name.into(), value);
        assert!(p.compile().is_err());
    }
    let mut p = patch("compare", "max");
    p.nodes[0].params.insert("b_offset".into(), -11.0);
    assert!(p.compile().is_err());
}
#[test]
fn action_resolution_targets_strict_kind_and_release_works_during_bypass() {
    for (kind, port, press, release, wrong) in [
        (
            "logic",
            "not_b",
            ModuleAction::BPress,
            ModuleAction::BRelease,
            ModuleAction::GatePress,
        ),
        (
            "process",
            "sh1",
            ModuleAction::GatePress,
            ModuleAction::GateRelease,
            ModuleAction::BPress,
        ),
    ] {
        let mut p = patch(kind, port);
        let mut e = p.compile().unwrap();
        let h = e.resolve_module_action("s", press).unwrap();
        let meter = e.resolve_module_control("s").unwrap();
        assert_eq!(h, meter);
        assert!(e.resolve_module_action("s", wrong).is_err());
        assert!(!e.handle_module_event(h, wrong));
        assert!(e.handle_module_event(h, press));
        one(&mut e);
        assert!(e.module_control_state(meter).unwrap().pressed);
        assert!(e.handle_module_event(h, press));
        assert!(e.release_module_controls(meter));
        one(&mut e);
        assert!(!e.module_control_state(meter).unwrap().pressed);
        let mut other = p.compile().unwrap();
        assert!(!other.release_module_controls(meter));
        assert_eq!(other.module_control_state(meter), None);
        p.nodes[0].bypassed = true;
        let mut bypassed = p.compile().unwrap();
        assert!(bypassed.resolve_module_action("s", press).is_err());
        let h = bypassed.resolve_module_action("s", release).unwrap();
        assert!(bypassed.handle_module_event(h, release));
        assert!(bypassed.release_module_controls(h));
        assert!(!bypassed.module_control_state(h).unwrap().pressed);
    }
    let mut seq = patch("seq3", "cv1").compile().unwrap();
    let h = seq.resolve_module_event("s").unwrap();
    assert!(seq
        .resolve_module_action("s", ModuleAction::BPress)
        .is_err());
    assert!(!seq.handle_module_event(h, ModuleAction::BPress));
    assert!(!seq.release_module_controls(h));
}
#[test]
fn ordered_offline_manual_events_are_sample_accurate_and_invalid_targets_preflight() {
    let mut p = patch("logic", "not_b");
    let events = [
        TimedModuleEvent {
            frame: 2,
            id: "s".into(),
            action: ModuleAction::BPress,
        },
        TimedModuleEvent {
            frame: 4,
            id: "s".into(),
            action: ModuleAction::BRelease,
        },
    ];
    let audio = render_audio_with_performance_events(&p, 0.001, &[], &events).unwrap();
    assert_eq!(
        audio.samples,
        [10.0, 10.0, 0.0, 0.0, 10.0, 10.0, 10.0, 10.0]
    );
    p = patch("process", "sh1");
    p.nodes.push(node("v", "constant", &[("value", 3.0)]));
    p.cables.push(cable("v", "s", "signal"));
    let events = [
        TimedModuleEvent {
            frame: 2,
            id: "s".into(),
            action: ModuleAction::GatePress,
        },
        TimedModuleEvent {
            frame: 4,
            id: "s".into(),
            action: ModuleAction::GateRelease,
        },
    ];
    let audio = render_audio_with_performance_events(&p, 0.001, &[], &events).unwrap();
    assert_eq!(audio.samples, [0.0, 0.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0]);
    let bad = [TimedModuleEvent {
        frame: 2,
        id: "s".into(),
        action: ModuleAction::BPress,
    }];
    assert!(render_audio_with_performance_events(&p, 0.001, &[], &bad).is_err());
}
#[test]
fn process_adopts_gate_history_across_rewire_and_cable_low_creates_natural_falling_edge() {
    let mut p = patch("process", "hold_track");
    p.nodes.push(node("v", "constant", &[("value", 3.0)]));
    p.nodes.push(node("g", "constant", &[("value", 10.0)]));
    p.nodes.push(node("h", "constant", &[("value", 10.0)]));
    p.cables = vec![cable("v", "s", "signal"), cable("g", "s", "gate")];
    let mut old = p.compile().unwrap();
    one(&mut old);
    one(&mut old);
    p.nodes[1].params.insert("value".into(), 9.0);
    let mut changed = p.compile().unwrap();
    changed.adopt_state_from(&old);
    one(&mut changed);
    assert_eq!(one(&mut changed), PortFrame::mono(9.0));
    p.cables[1] = cable("h", "s", "gate");
    let mut rewired = p.compile().unwrap();
    rewired.adopt_state_from(&changed);
    assert_eq!(one(&mut rewired), PortFrame::mono(9.0)); // Falling captures current input rather than reset-held 3V.
    p.output_port = "sh2".into();
    let mut shifted = p.compile().unwrap();
    shifted.adopt_state_from(&changed);
    one(&mut shifted);
    assert_eq!(one(&mut shifted), PortFrame::mono(3.0)); // Fresh high shifts the original raw sample.
}
#[test]
fn runtime_light_handles_adopt_dividers_freeze_on_bypass_and_clear_on_identity_reset() {
    let mut p = patch("logic", "not_a");
    let mut old = p.compile().unwrap();
    for _ in 0..31 {
        one(&mut old);
    }
    let h = old.resolve_module_lights("s").unwrap();
    assert_eq!(
        old.module_lights(h).unwrap().brightness,
        [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]
    );
    let mut edited = p.compile().unwrap();
    edited.adopt_state_from(&old);
    let h = edited.resolve_module_lights("s").unwrap();
    one(&mut edited);
    assert_eq!(edited.module_lights(h).unwrap().brightness[0], [1.0, 0.0]);
    let c = edited
        .resolve_module_action("s", ModuleAction::BPress)
        .unwrap();
    edited.handle_module_event(c, ModuleAction::BPress);
    one(&mut edited);
    assert!(edited.module_lights(h).unwrap().manual);
    p.nodes[0].bypassed = true;
    let mut muted = p.compile().unwrap();
    muted.adopt_state_from(&edited);
    let h = muted.resolve_module_lights("s").unwrap();
    let lights = muted.module_lights(h).unwrap();
    assert!(!lights.manual);
    for _ in 0..100 {
        one(&mut muted);
    }
    assert_eq!(muted.module_lights(h).unwrap(), lights);
    assert!(muted.reset_module_state(h));
    assert_eq!(
        muted.module_lights(h).unwrap().brightness,
        [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]
    );
    for kind in ["compare", "rescale"] {
        let port = if kind == "compare" { "max" } else { "out" };
        let mut e = patch(kind, port).compile().unwrap();
        let h = e.resolve_module_lights("s").unwrap();
        assert!(e.resolve_module_control("s").is_err());
        assert!(e.reset_module_state(h));
    }
}
#[test]
fn rescale_bypass_is_raw_direct_frame_and_signal_callbacks_are_partition_invariant() {
    let mut p = patch("rescale", "out");
    p.nodes[0].bypassed = true;
    p.nodes.push(node(
        "v",
        "poly_constant",
        &[("channels", 3.0), ("v1", 2.0), ("v2", -3.0), ("v3", 12.0)],
    ));
    p.cables.push(cable("v", "s", "signal"));
    let mut e = p.compile().unwrap();
    assert_eq!(one(&mut e), PortFrame::mono(0.0));
    let out = one(&mut e);
    assert_eq!(&out.values[..3], &[2.0, -3.0, 12.0]);
    for (kind, port) in [
        ("logic", "or"),
        ("compare", "clip"),
        ("process", "slew"),
        ("rescale", "out"),
    ] {
        let mut p = patch(kind, port);
        p.nodes.push(node("v", "gate", &[("frequency", 100.0)]));
        p.cables.push(cable(
            "v",
            "s",
            if matches!(kind, "logic" | "compare") {
                "a"
            } else {
                "signal"
            },
        ));
        let mut expected = [PortFrame::default(); 259];
        p.compile().unwrap().process_poly_block(&mut expected);
        let mut actual = [PortFrame::default(); 259];
        let mut e = p.compile().unwrap();
        for range in [0..1, 1..4, 4..131, 131..259] {
            e.process_poly_block(&mut actual[range]);
        }
        assert_eq!(expected, actual);
    }
}
