use openpatch_core::*;
fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        bypassed: false,
        id: id.into(),
        kind: kind.into(),
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
fn patch(kind: &str, port: &str, params: &[(&str, f32)]) -> Patch {
    Patch {
        nodes: vec![node("s", kind, params)],
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
fn catalog_ports_parameter_defaults_validation_and_legacy_rejection() {
    let c = catalog();
    for (kind, count, prefix, outcount) in [
        ("attenuverter8", 8, "gain", 8),
        ("mutes10", 10, "mute", 10),
        ("cv_mix", 3, "level", 1),
    ] {
        let m = c["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        assert_eq!(
            m["inputs"],
            serde_json::json!((1..=count).map(|i| format!("in{i}")).collect::<Vec<_>>())
        );
        assert_eq!(m["outputs"].as_array().unwrap().len(), outcount);
        assert_eq!(m["params"].as_object().unwrap().len(), count);
        for i in 1..=count {
            assert_eq!(m["params"][format!("{prefix}{i}")]["default"], 0.0);
        }
        assert_eq!(m["min_patch_version"], 2);
        for meta in m["output_metadata"].as_array().unwrap() {
            assert_eq!(meta["maximum_channels"], 16);
            assert_eq!(meta["units"], "V");
        }
        let mut p = patch(kind, if outcount == 1 { "out" } else { "out1" }, &[]);
        assert!(p.compile().is_ok());
        p.version = 1;
        assert!(p.compile().is_err());
        p.version = 2;
        for (name, value) in [
            (format!("{prefix}1"), 1.01),
            (format!("{prefix}0"), 0.0),
            ("unknown".into(), 0.0),
        ] {
            let mut bad = p.clone();
            bad.nodes[0].params.insert(name, value);
            assert!(bad.compile().is_err());
        }
        if kind == "mutes10" {
            p.nodes[0].params.insert("mute1".into(), 0.5);
            assert!(p.compile().is_err());
        }
    }
}
#[test]
fn attenuverters_default_to_silence_and_generate_independent_unpatched_offsets() {
    for row in 1..=8 {
        let mut p = patch("attenuverter8", &format!("out{row}"), &[]);
        assert_eq!(settled(&p), PortFrame::mono(0.0));
        p.nodes[0]
            .params
            .insert(format!("gain{row}"), if row % 2 == 0 { -0.5 } else { 1.0 });
        assert_eq!(
            settled(&p),
            PortFrame::mono(if row % 2 == 0 { -5.0 } else { 10.0 })
        );
    }
}
#[test]
fn attenuverter_normals_copy_full_input_not_prior_output_and_midchain_override() {
    let mut p = patch(
        "attenuverter8",
        "out8",
        &[("gain1", 0.0), ("gain2", -1.0), ("gain8", 0.5)],
    );
    p.nodes.push(node(
        "a",
        "poly_constant",
        &[("channels", 16.0), ("v1", 3.0), ("v16", -4.0)],
    ));
    p.cables.push(cable("a", "s", "in1"));
    let out = settled(&p);
    assert_eq!(out.channels, 16);
    assert_eq!(out.values[0], 1.5);
    assert_eq!(out.values[15], -2.0);
    p.nodes.push(node("b", "constant", &[("value", -6.0)]));
    p.cables.push(cable("b", "s", "in5"));
    assert_eq!(settled(&p), PortFrame::mono(-3.0));
    p.nodes[2] = node("b", "poly_constant", &[("channels", 0.0)]);
    let out = settled(&p);
    assert_eq!(out.channels, 16);
    assert_eq!(out.values[15], -2.0);
}
#[test]
fn mutes_inherit_first_voltage_as_mono_and_muted_rows_do_not_break_normaling() {
    let mut p = patch("mutes10", "out1", &[("mute1", 1.0)]);
    p.nodes.push(node(
        "v",
        "poly_constant",
        &[("channels", 3.0), ("v1", 2.0), ("v2", -4.0), ("v3", 8.0)],
    ));
    p.cables.push(cable("v", "s", "in1"));
    let out = settled(&p);
    assert_eq!(out.channels, 3);
    assert_eq!(out.values, [0.0; 16]);
    p.output_port = "out2".into();
    assert_eq!(settled(&p), PortFrame::mono(2.0));
    p.output_port = "out10".into();
    assert_eq!(settled(&p), PortFrame::mono(2.0));
    p.nodes.push(node("other", "constant", &[("value", -3.0)]));
    p.cables.push(cable("other", "s", "in6"));
    assert_eq!(settled(&p), PortFrame::mono(-3.0));
    p.nodes[0].params.insert("mute10".into(), 1.0);
    assert_eq!(settled(&p), PortFrame::mono(0.0));
    assert_eq!(
        settled(&patch("mutes10", "out10", &[])),
        PortFrame::mono(0.0)
    );
}
#[test]
fn cv_mix_offsets_broadcast_mono_and_zero_fill_short_poly_without_averaging() {
    assert_eq!(settled(&patch("cv_mix", "out", &[])), PortFrame::mono(0.0));
    assert_eq!(
        settled(&patch(
            "cv_mix",
            "out",
            &[("level1", 1.0), ("level2", 1.0), ("level3", 1.0)]
        )),
        PortFrame::mono(30.0)
    );
    let mut p = patch(
        "cv_mix",
        "out",
        &[("level1", 1.0), ("level2", -0.5), ("level3", 0.25)],
    );
    p.nodes.push(node(
        "poly",
        "poly_constant",
        &[("channels", 3.0), ("v1", 1.0), ("v2", 2.0), ("v3", 3.0)],
    ));
    p.nodes.push(node("mono", "constant", &[("value", 4.0)]));
    p.cables = vec![cable("poly", "s", "in1"), cable("mono", "s", "in2")];
    assert_eq!(&settled(&p).values[..3], &[1.5, 2.5, 3.5]); // -2V mono +2.5V normal on every lane.
    p.nodes[2] = node(
        "mono",
        "poly_constant",
        &[("channels", 2.0), ("v1", 4.0), ("v2", 6.0)],
    );
    assert_eq!(&settled(&p).values[..3], &[1.5, 1.5, 5.5]);
    p.nodes[2].params.insert("channels".into(), 0.0);
    assert_eq!(&settled(&p).values[..3], &[-1.5, -0.5, 0.5]); // Zero-channel means independent 10V normal.
}
#[test]
fn normaling_is_same_sample_inside_module_and_each_explicit_cable_delays_once() {
    let mut p = patch("attenuverter8", "out8", &[("gain8", 1.0)]);
    p.nodes.push(node("v", "constant", &[("value", 3.0)]));
    p.cables.push(cable("v", "s", "in1"));
    let mut e = p.compile().unwrap();
    assert_eq!(one(&mut e), PortFrame::mono(10.0));
    assert_eq!(one(&mut e), PortFrame::mono(3.0));
    let mut full = [PortFrame::default(); 259];
    p.compile().unwrap().process_poly_block(&mut full);
    let mut split = [PortFrame::default(); 259];
    let mut e = p.compile().unwrap();
    for range in [0..1, 1..4, 4..131, 131..259] {
        e.process_poly_block(&mut split[range]);
    }
    assert_eq!(full, split);
}
