use openpatch_core::*;
fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        bypassed: false,
        params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
    }
}
fn cable(from: &str, to: &str, input: &str) -> Cable {
    Cable {
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
fn settled(p: &Patch) -> PortFrame {
    let mut e = p.compile().unwrap();
    one(&mut e);
    one(&mut e)
}
#[test]
fn complete_catalog_validation_and_legacy_compatibility() {
    let catalog = catalog();
    for (kind, inputs, outputs) in [
        (
            "vca_mix",
            vec![
                "in1", "in2", "in3", "in4", "cv1", "cv2", "cv3", "cv4", "mix_cv",
            ],
            vec!["out1", "out2", "out3", "out4", "mix"],
        ),
        ("fade", vec!["a", "b", "cv"], vec!["out", "complement"]),
    ] {
        let m = catalog["modules"]
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
        p.nodes[0].params.insert("unknown".into(), 0.0);
        assert!(p.compile().is_err());
    }
    for (kind, name, value) in [
        ("vca_mix", "level1", 1.415),
        ("vca_mix", "mix_level", 2.01),
        ("vca_mix", "channel_exp", 0.5),
        ("vca_mix", "mix_exp", 0.5),
        ("fade", "position", 1.01),
        ("fade", "cv_amount", -1.01),
        ("fade", "law", 0.5),
    ] {
        let mut p = patch(kind, if kind == "fade" { "out" } else { "mix" });
        p.nodes[0].params.insert(name.into(), value);
        assert!(p.compile().is_err());
    }
    let legacy = Patch {
        nodes: vec![node("old", "crossfade", &[])],
        output: Some("old".into()),
        ..Patch::default()
    };
    assert!(legacy.compile().is_ok());
}
#[test]
fn every_strip_contributes_to_mix_and_master_does_not_affect_strip_outputs() {
    let mut p = patch("vca_mix", "mix");
    p.nodes[0].params.insert("mix_level".into(), 2.0);
    for i in 1..=4 {
        p.nodes
            .push(node(&format!("v{i}"), "constant", &[("value", i as f32)]));
        p.cables
            .push(cable(&format!("v{i}"), "s", &format!("in{i}")));
    }
    assert_eq!(settled(&p), PortFrame::mono(20.0));
    p.nodes.push(node("mute", "constant", &[("value", 0.0)]));
    p.cables.push(cable("mute", "s", "mix_cv"));
    assert_eq!(settled(&p), PortFrame::mono(0.0));
    for i in 1..=4 {
        p.output_port = format!("out{i}");
        assert_eq!(settled(&p), PortFrame::mono(i as f32));
    }
}
#[test]
fn mixer_outputs_share_signal_width_and_cv_width_does_not_create_voices() {
    let mut p = patch("vca_mix", "out1");
    p.nodes.push(node("mono", "constant", &[("value", 3.0)]));
    p.nodes
        .push(node("wide", "poly_constant", &[("channels", 16.0)]));
    for channel in 1..=16 {
        p.nodes[2].params.insert(format!("v{channel}"), 0.0);
    }
    p.cables = vec![cable("mono", "s", "in1"), cable("wide", "s", "in4")];
    assert_eq!(settled(&p).values, [3.0; 16]);
    for port in ["out1", "out2", "out3", "out4", "mix"] {
        p.output_port = port.into();
        assert_eq!(settled(&p).channels, 16);
    }
    p.cables[1] = cable("wide", "s", "cv1");
    p.output_port = "out1".into();
    assert_eq!(settled(&p), PortFrame::mono(0.0)); // CV has16 lanes but only one signal voice.
    p.nodes[2].params.insert("channels".into(), 0.0);
    assert_eq!(settled(&p), PortFrame::mono(3.0));
}
#[test]
fn mono_cv_broadcasts_short_poly_cv_zero_fills_and_zero_channels_normal_unity() {
    let mut p = patch("vca_mix", "mix");
    p.nodes.push(node(
        "signal",
        "poly_constant",
        &[("channels", 3.0), ("v1", 2.0), ("v2", 4.0), ("v3", 6.0)],
    ));
    p.nodes.push(node("cv", "constant", &[("value", 5.0)]));
    p.cables = vec![cable("signal", "s", "in1"), cable("cv", "s", "cv1")];
    assert_eq!(&settled(&p).values[..3], &[1.0, 2.0, 3.0]);
    p.nodes[2] = node(
        "cv",
        "poly_constant",
        &[("channels", 2.0), ("v1", 10.0), ("v2", 15.0)],
    );
    assert_eq!(&settled(&p).values[..3], &[2.0, 6.0, 0.0]);
    p.nodes[2].params.insert("channels".into(), 0.0);
    assert_eq!(&settled(&p).values[..3], &[2.0, 4.0, 6.0]);
}
#[test]
fn fade_signal_and_cv_lane_policies_apply_to_both_outputs() {
    let mut p = patch("fade", "out");
    p.nodes[0].params.insert("position".into(), 0.25);
    p.nodes[0].params.insert("cv_amount".into(), 1.0);
    p.nodes.push(node("a", "constant", &[("value", 6.0)]));
    p.nodes.push(node(
        "b",
        "poly_constant",
        &[("channels", 3.0), ("v1", 2.0), ("v2", 4.0), ("v3", 8.0)],
    ));
    p.nodes.push(node(
        "cv",
        "poly_constant",
        &[("channels", 2.0), ("v1", 0.0), ("v2", 10.0)],
    ));
    p.cables = vec![
        cable("a", "s", "a"),
        cable("b", "s", "b"),
        cable("cv", "s", "cv"),
    ];
    assert_eq!(&settled(&p).values[..3], &[5.0, 4.0, 6.5]);
    p.output_port = "complement".into();
    assert_eq!(&settled(&p).values[..3], &[3.0, 6.0, 7.5]);
    p.cables.remove(1);
    assert_eq!(settled(&p).channels, 1); // Two CV lanes do not create another signal voice.
}
#[test]
fn fade_bypass_uses_independent_direct_widths_and_mixer_bypass_freezes_meters() {
    let mut p = patch("fade", "out");
    p.nodes[0].bypassed = true;
    p.nodes.push(node("a", "constant", &[("value", 6.0)]));
    p.nodes.push(node(
        "b",
        "poly_constant",
        &[("channels", 3.0), ("v1", 2.0), ("v2", 4.0), ("v3", 8.0)],
    ));
    p.cables = vec![cable("a", "s", "a"), cable("b", "s", "b")];
    assert_eq!(settled(&p), PortFrame::mono(6.0));
    p.output_port = "complement".into();
    let out = settled(&p);
    assert_eq!(out.channels, 3);
    assert_eq!(&out.values[..3], &[2.0, 4.0, 8.0]);
    p.cables.pop();
    assert_eq!(settled(&p), PortFrame::mono(0.0));
    let mut p = patch("vca_mix", "mix");
    p.nodes.push(node("v", "constant", &[("value", 0.25)]));
    p.cables.push(cable("v", "s", "in1"));
    let mut old = p.compile().unwrap();
    for _ in 0..512 {
        one(&mut old);
    }
    let h = old.resolve_module_lights("s").unwrap();
    let lights = old.module_lights(h).unwrap();
    assert!(lights.brightness[0][0] > 0.0);
    p.nodes[0].bypassed = true;
    for port in ["out1", "out2", "out3", "out4", "mix"] {
        p.output_port = port.into();
        let mut muted = p.compile().unwrap();
        muted.adopt_state_from(&old);
        let h = muted.resolve_module_lights("s").unwrap();
        for _ in 0..513 {
            assert_eq!(one(&mut muted), PortFrame::mono(0.0));
        }
        assert_eq!(muted.module_lights(h).unwrap(), lights);
    }
}
#[test]
fn meter_handles_adopt_envelopes_divider_and_new_parameters_but_identity_reset_is_fresh() {
    let mut p = patch("vca_mix", "mix");
    p.nodes.push(node("v", "constant", &[("value", 0.25)]));
    p.cables.push(cable("v", "s", "in1"));
    let mut old = p.compile().unwrap();
    for _ in 0..511 {
        one(&mut old);
    }
    let old_h = old.resolve_module_lights("s").unwrap();
    assert_eq!(old.module_lights(old_h).unwrap().brightness[0][0], 0.0);
    p.nodes[0].params.insert("level1".into(), 0.5);
    let mut edited = p.compile().unwrap();
    edited.adopt_state_from(&old);
    let h = edited.resolve_module_lights("s").unwrap();
    assert_eq!(one(&mut edited), PortFrame::mono(0.0625));
    assert!(edited.module_lights(h).unwrap().brightness[0][0] > 0.4);
    assert_eq!(edited.module_lights(h).unwrap().count, 4);
    assert!(edited
        .module_lights(h)
        .unwrap()
        .brightness
        .iter()
        .all(|b| b[1] == 0.0));
    assert!(!edited.module_lights(h).unwrap().manual);
    assert!(edited.resolve_module_control("s").is_err());
    assert!(edited.resolve_module_event("s").is_err());
    assert!(!edited.reset_module_state(old_h));
    assert!(edited.reset_module_state(h));
    assert_eq!(edited.module_lights(h).unwrap().brightness, [[0.0; 2]; 8]);
    p.sample_rate = 16000;
    let mut rate = p.compile().unwrap();
    rate.adopt_state_from(&old);
    let h = rate.resolve_module_lights("s").unwrap();
    assert_eq!(rate.module_lights(h).unwrap().brightness, [[0.0; 2]; 8]);
    assert!(patch("fade", "out")
        .compile()
        .unwrap()
        .resolve_module_lights("s")
        .is_err());
}
#[test]
fn named_mix_and_fade_outputs_keep_one_sample_cables_and_partition_invariance() {
    for (kind, port, input) in [
        ("vca_mix", "out1", "in1"),
        ("vca_mix", "mix", "in1"),
        ("fade", "out", "a"),
        ("fade", "complement", "a"),
    ] {
        let mut p = patch(kind, port);
        p.nodes.push(node("v", "gate", &[("frequency", 100.0)]));
        p.cables.push(cable("v", "s", input));
        let mut expected = [PortFrame::default(); 1027];
        p.compile().unwrap().process_poly_block(&mut expected);
        assert_eq!(expected[0], PortFrame::mono(0.0));
        assert!(expected[1].values[0] > 0.0);
        let mut actual = [PortFrame::default(); 1027];
        let mut e = p.compile().unwrap();
        for range in [0..1, 1..4, 4..131, 131..643, 643..1027] {
            e.process_poly_block(&mut actual[range]);
        }
        assert_eq!(actual, expected);
    }
}
