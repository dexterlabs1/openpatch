//! Independent acceptance tests for the functional v2 routing modules.
//! Expected voltages below follow the public equations, not private DSP helpers.
use openpatch_core::{catalog, Cable, Node, Patch, PortFrame};

fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        params: params
            .iter()
            .map(|(name, value)| ((*name).into(), *value))
            .collect(),
    }
}
fn source(id: &str, values: &[f32]) -> Node {
    let mut result = node(id, "poly_constant", &[("channels", values.len() as f32)]);
    for (index, value) in values.iter().enumerate() {
        result.params.insert(format!("v{}", index + 1), *value);
    }
    result
}
fn wire(from: &str, port: &str, to: &str, input: &str) -> Cable {
    Cable {
        from: from.into(),
        output: port.into(),
        to: to.into(),
        input: input.into(),
    }
}
fn patch(nodes: Vec<Node>, cables: Vec<Cable>, output: &str, port: &str) -> Patch {
    Patch {
        nodes,
        cables,
        output: Some(output.into()),
        output_port: port.into(),
        ..Patch::new_v2()
    }
}
fn render(patch: &Patch, count: usize) -> Vec<PortFrame> {
    let mut output = vec![PortFrame::default(); count];
    patch.compile().unwrap().process_poly_block(&mut output);
    output
}
fn settled(patch: &Patch) -> PortFrame {
    render(patch, 16)[15]
}
fn assert_frame(frame: &PortFrame, expected: &[f32]) {
    assert_eq!(usize::from(frame.channels), expected.len(), "{frame:?}");
    for (actual, expected) in frame.values.iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 0.000_001,
            "{actual} != {expected}"
        );
    }
    assert!(
        frame.values[expected.len()..]
            .iter()
            .all(|value| *value == 0.0),
        "inactive lanes must be zero: {frame:?}"
    );
}
fn mid_side(left: &[f32], right: &[f32], params: &[(&str, f32)]) -> Patch {
    patch(
        vec![
            source("l", left),
            source("r", right),
            node("ms", "mid_side", params),
        ],
        vec![
            wire("l", "out", "ms", "left"),
            wire("r", "out", "ms", "right"),
        ],
        "ms",
        "left",
    )
}
fn select(patch: &Patch, output: &str) -> PortFrame {
    let mut patch = patch.clone();
    patch.output_port = output.into();
    settled(&patch)
}

#[test]
fn stereo_encoder_decoder_is_identity_and_internal_normals_add_no_cable_delay() {
    let p = mid_side(&[8.0, -6.0, 2.0], &[-4.0, 2.0, 2.0], &[]);
    assert_frame(&select(&p, "mid"), &[2.0, -2.0, 2.0]);
    assert_frame(&select(&p, "side"), &[6.0, -4.0, 0.0]);
    assert_frame(&select(&p, "left"), &[8.0, -6.0, 2.0]);
    assert_frame(&select(&p, "right"), &[-4.0, 2.0, 2.0]);
    let frames = render(&p, 3);
    assert_frame(&frames[0], &[0.0]);
    assert_frame(&frames[1], &[8.0, -6.0, 2.0]);
    assert_eq!(frames[1], frames[2]); // Encoder -> internal decoder has no hidden edge.
}

#[test]
fn widths_collapse_expand_and_decode_negative_side_swaps_left_and_right() {
    let collapsed = mid_side(&[8.0, -6.0], &[-4.0, 2.0], &[("encode_width", 0.0)]);
    assert_frame(&select(&collapsed, "side"), &[0.0, 0.0]);
    assert_frame(&select(&collapsed, "left"), &[2.0, -2.0]);
    assert_frame(&select(&collapsed, "right"), &[2.0, -2.0]);
    let collapsed = mid_side(&[8.0], &[-4.0], &[("decode_width", 0.0)]);
    assert_frame(&select(&collapsed, "side"), &[6.0]);
    assert_frame(&select(&collapsed, "left"), &[2.0]);
    assert_frame(&select(&collapsed, "right"), &[2.0]);
    let expanded = mid_side(
        &[8.0],
        &[-4.0],
        &[("encode_width", 2.0), ("decode_width", 2.0)],
    );
    assert_frame(&select(&expanded, "left"), &[26.0]);
    assert_frame(&select(&expanded, "right"), &[-22.0]);

    let inverted = patch(
        vec![
            source("m", &[2.0, -2.0]),
            source("s", &[-6.0, 4.0]),
            node("ms", "mid_side", &[]),
        ],
        vec![
            wire("m", "out", "ms", "mid"),
            wire("s", "out", "ms", "side"),
        ],
        "ms",
        "left",
    );
    assert_frame(&select(&inverted, "left"), &[-4.0, 2.0]);
    assert_frame(&select(&inverted, "right"), &[8.0, -6.0]);
}

#[test]
fn width_cv_is_multiplicative_clamped_and_does_not_create_voices() {
    for (cv, expected_side) in [
        (-10.0, 0.0),
        (0.0, 0.0),
        (5.0, 3.0),
        (10.0, 6.0),
        (16.0, 9.6),
    ] {
        let mut p = mid_side(&[8.0], &[-4.0], &[]);
        p.nodes.push(source("cv", &[cv]));
        p.cables.push(wire("cv", "out", "ms", "encode_cv"));
        assert_frame(&select(&p, "side"), &[expected_side]);
    }
    let mut p = mid_side(&[8.0], &[-4.0], &[("encode_width", 2.0)]);
    p.nodes.push(source("cv", &[16.0, 0.0, -16.0]));
    p.cables.push(wire("cv", "out", "ms", "encode_cv"));
    assert_frame(&select(&p, "side"), &[12.0]); // Effective width saturates at 2, CV count ignored.
    p.cables[2].input = "decode_cv".into();
    assert_frame(&select(&p, "left"), &[21.2]); // M2 + S12 * (1 * 16/10).
    p.nodes[2].params.insert("decode_width".into(), 0.5);
    assert_frame(&select(&p, "left"), &[11.6]); // M2 + S12 * (0.5 * 16/10).
}

#[test]
fn encoder_counts_signal_voices_and_broadcasts_mono_but_not_short_poly() {
    let p = mid_side(&[8.0, 4.0, -2.0], &[2.0], &[]);
    assert_frame(&select(&p, "mid"), &[5.0, 3.0, 0.0]);
    assert_frame(&select(&p, "side"), &[3.0, 1.0, -2.0]);
    assert_frame(&select(&p, "left"), &[8.0, 4.0, -2.0]);
    assert_frame(&select(&p, "right"), &[2.0, 2.0, 2.0]);
    let mut p = mid_side(&[8.0, 4.0, -2.0], &[2.0, 6.0], &[]);
    assert_frame(&select(&p, "mid"), &[5.0, 5.0, -1.0]);
    assert_frame(&select(&p, "right"), &[2.0, 6.0, 0.0]);
    p.nodes.push(source("cv", &[10.0, 5.0]));
    p.cables.push(wire("cv", "out", "ms", "encode_cv"));
    assert_frame(&select(&p, "side"), &[3.0, -0.5, 0.0]);
    p.nodes[3] = source("cv", &[]); // Explicit zero channels normals to 10V.
    assert_frame(&select(&p, "side"), &[3.0, -1.0, -1.0]);
}

#[test]
fn decoder_mid_and_side_normal_independently_and_external_inputs_set_its_count() {
    let mut p = mid_side(&[8.0, 4.0, -2.0], &[-4.0, 2.0, 6.0], &[]);
    p.nodes.push(source("external", &[1.0]));
    p.cables.push(wire("external", "out", "ms", "mid"));
    assert_frame(&select(&p, "left"), &[7.0, 2.0, -3.0]);
    assert_frame(&select(&p, "right"), &[-5.0, 0.0, 5.0]);
    p.cables[2].input = "side".into();
    assert_frame(&select(&p, "left"), &[3.0, 4.0, 3.0]);
    assert_frame(&select(&p, "right"), &[1.0, 2.0, 1.0]);
    p.nodes[3] = source("external", &[0.0]); // Connected 0V overrides S; not a normal.
    assert_frame(&select(&p, "left"), &[2.0, 3.0, 2.0]);
    p.nodes[3] = source("external", &[]); // A zero-channel cable restores the normal.
    assert_frame(&select(&p, "left"), &[8.0, 4.0, -2.0]);

    p.nodes[3] = source("external", &[1.0, 2.0]);
    p.nodes.push(source("external_m", &[5.0]));
    p.cables.push(wire("external_m", "out", "ms", "mid"));
    assert_frame(&select(&p, "left"), &[6.0, 7.0]); // Both overrides replace encoder's 3-voice count.
    assert_frame(&select(&p, "right"), &[4.0, 3.0]);
}

#[test]
fn decoder_cv_broadcast_short_poly_and_zero_channel_normal_are_explicit() {
    let mut p = mid_side(&[8.0, 4.0, -2.0], &[-4.0, 2.0, 6.0], &[]);
    p.nodes.push(source("cv", &[5.0]));
    p.cables.push(wire("cv", "out", "ms", "decode_cv"));
    assert_frame(&select(&p, "left"), &[5.0, 3.5, 0.0]);
    p.nodes[3] = source("cv", &[10.0, 5.0]);
    assert_frame(&select(&p, "left"), &[8.0, 3.5, 2.0]);
    p.nodes[3] = source("cv", &[]);
    assert_frame(&select(&p, "left"), &[8.0, 4.0, -2.0]);
}

#[test]
fn mix6_levels_invert_and_average_connected_jacks_even_when_zero_volts() {
    let mut p = patch(
        vec![
            source("a", &[6.0]),
            source("zero", &[0.0]),
            node("mix", "mix6", &[]),
        ],
        vec![wire("a", "out", "mix", "in1")],
        "mix",
        "out",
    );
    assert_frame(&settled(&p), &[6.0]);
    p.nodes[2].params.insert("level".into(), 0.5);
    assert_frame(&settled(&p), &[3.0]);
    p.nodes[2].params.insert("invert".into(), 1.0);
    assert_frame(&settled(&p), &[-3.0]);
    p.nodes[2].params.insert("average".into(), 1.0);
    assert_frame(&settled(&p), &[-3.0]);
    p.cables.push(wire("zero", "out", "mix", "in6"));
    assert_frame(&settled(&p), &[-1.5]);
    p.nodes[1] = source("zero", &[]);
    assert_frame(&settled(&p), &[-3.0]); // Zero channels is disconnected, unlike zero volts.
    p.cables.clear();
    assert_frame(&settled(&p), &[0.0]);
}

#[test]
fn mix6_all_six_inputs_and_poly_lanes_are_independent() {
    let mut p = patch(vec![node("mix", "mix6", &[])], vec![], "mix", "out");
    for lane in 1..=6 {
        let id = format!("src{lane}");
        p.nodes.push(source(&id, &[lane as f32]));
        p.cables.push(wire(&id, "out", "mix", &format!("in{lane}")));
    }
    assert_frame(&settled(&p), &[21.0]);
    p.nodes[0].params.insert("average".into(), 1.0);
    assert_frame(&settled(&p), &[3.5]);
    let mut p = patch(
        vec![
            source("a", &[2.0, 4.0, 6.0]),
            source("b", &[1.0, -1.0]),
            source("c", &[10.0]),
            node("mix", "mix6", &[]),
        ],
        vec![
            wire("a", "out", "mix", "in1"),
            wire("b", "out", "mix", "in3"),
            wire("c", "out", "mix", "in6"),
        ],
        "mix",
        "out",
    );
    assert_frame(&settled(&p), &[13.0, 13.0, 16.0]);
    p.nodes[3].params.insert("average".into(), 1.0);
    assert_frame(&settled(&p), &[13.0 / 3.0, 13.0 / 3.0, 16.0 / 3.0]);
    // A connected short-poly jack still participates in the same per-jack
    // average divisor when it has no value at the third voice.
}

#[test]
fn octave_shifts_are_exact_pitch_volts_and_cv_rounds_ties_away_from_zero() {
    for octaves in -4..=4 {
        let p = patch(
            vec![
                source("pitch", &[0.0, 1.0 / 12.0, -7.0 / 12.0]),
                node("oct", "octave", &[("octaves", octaves as f32)]),
            ],
            vec![wire("pitch", "out", "oct", "pitch")],
            "oct",
            "out",
        );
        assert_frame(
            &settled(&p),
            &[
                octaves as f32,
                octaves as f32 + 1.0 / 12.0,
                octaves as f32 - 7.0 / 12.0,
            ],
        );
    }
    let p = patch(
        vec![
            source("pitch", &[0.25; 8]),
            source("cv", &[-1.5, -0.5, -0.499, 0.0, 0.499, 0.5, 1.5, 16.0]),
            node("oct", "octave", &[("octaves", 4.0)]),
        ],
        vec![
            wire("pitch", "out", "oct", "pitch"),
            wire("cv", "out", "oct", "shift_cv"),
        ],
        "oct",
        "out",
    );
    assert_frame(
        &settled(&p),
        &[2.25, 3.25, 4.25, 4.25, 4.25, 5.25, 6.25, 20.25],
    );
}

#[test]
fn octave_primary_pitch_controls_count_while_cv_broadcasts_or_zero_fills() {
    let mut p = patch(
        vec![
            source("pitch", &[0.0, 0.25, -0.25]),
            source("cv", &[1.0]),
            node("oct", "octave", &[]),
        ],
        vec![
            wire("pitch", "out", "oct", "pitch"),
            wire("cv", "out", "oct", "shift_cv"),
        ],
        "oct",
        "out",
    );
    assert_frame(&settled(&p), &[1.0, 1.25, 0.75]);
    p.nodes[1] = source("cv", &[1.0, -1.0]);
    assert_frame(&settled(&p), &[1.0, -0.75, -0.25]);
    p.nodes[1] = source("cv", &[]);
    assert_frame(&settled(&p), &[0.0, 0.25, -0.25]);
    p.nodes[0] = source("pitch", &[]);
    p.nodes[1] = source("cv", &[2.0, 3.0, 4.0]);
    assert_frame(&settled(&p), &[2.0]); // CV cannot create extra pitch lanes.
}

#[test]
fn mult_all_eight_outputs_preserve_zero_mono_and_sixteen_channels_with_one_edge_delay() {
    let values: Vec<f32> = (0..16).map(|index| index as f32 - 7.5).collect();
    for values in [&[][..], &values[..1], &values[..]] {
        let mut p = patch(
            vec![source("source", values), node("mult", "mult", &[])],
            vec![wire("source", "out", "mult", "signal")],
            "mult",
            "out1",
        );
        for port in 1..=8 {
            p.output_port = format!("out{port}");
            let output = render(&p, 4);
            assert_frame(&output[0], &[]);
            for frame in &output[1..] {
                assert_frame(frame, values);
            }
        }
    }
}

#[test]
fn mult_added_cable_delays_both_values_and_channel_count_exactly_once() {
    let p = patch(
        vec![
            source("src", &[1.0, 2.0]),
            node("a", "mult", &[]),
            node("b", "mult", &[]),
        ],
        vec![
            wire("src", "out", "a", "signal"),
            wire("a", "out8", "b", "signal"),
        ],
        "b",
        "out4",
    );
    let output = render(&p, 5);
    assert_frame(&output[0], &[]);
    assert_frame(&output[1], &[]);
    for frame in &output[2..] {
        assert_frame(frame, &[1.0, 2.0]);
    }
}

#[test]
fn routing_feedback_is_independent_of_node_cable_order_and_callback_partition() {
    let p = patch(
        vec![
            node("oct", "octave", &[("octaves", 1.0)]),
            node("mult", "mult", &[]),
            node("mix", "mix6", &[("level", 0.5)]),
            node("ms", "mid_side", &[]),
        ],
        vec![
            wire("oct", "out", "mult", "signal"),
            wire("mult", "out8", "mix", "in1"),
            wire("mix", "out", "ms", "left"),
            wire("mix", "out", "ms", "right"),
            wire("ms", "left", "oct", "pitch"),
        ],
        "oct",
        "out",
    );
    let expected = render(&p, 517);
    assert_frame(&expected[0], &[1.0]);
    assert_frame(&expected[3], &[1.0]);
    assert_frame(&expected[4], &[1.5]); // Four cables in the feedback cycle.
    assert!(expected[516].values[0] > 1.9);
    let mut reordered = p;
    reordered.nodes.reverse();
    reordered.cables.reverse();
    for chunk in [1, 3, 64, 128, 129, 517] {
        let mut engine = reordered.compile().unwrap();
        let mut actual = vec![PortFrame::default(); expected.len()];
        for block in actual.chunks_mut(chunk) {
            engine.process_poly_block(block);
        }
        assert_eq!(actual, expected, "partition={chunk}");
    }
}

#[test]
fn catalog_names_defaults_ports_and_version_gates_match_persisted_contracts() {
    let catalog = catalog();
    for (kind, inputs, outputs) in [
        (
            "mid_side",
            vec!["left", "right", "mid", "side", "encode_cv", "decode_cv"],
            vec!["mid", "side", "left", "right"],
        ),
        (
            "mix6",
            vec!["in1", "in2", "in3", "in4", "in5", "in6"],
            vec!["out"],
        ),
        ("octave", vec!["pitch", "shift_cv"], vec!["out"]),
        (
            "mult",
            vec!["signal"],
            vec![
                "out1", "out2", "out3", "out4", "out5", "out6", "out7", "out8",
            ],
        ),
    ] {
        let entry = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        assert_eq!(entry["min_patch_version"], 2);
        assert_eq!(entry["model"], "functional");
        assert_eq!(entry["inputs"], serde_json::json!(inputs));
        assert_eq!(entry["outputs"], serde_json::json!(outputs));
        let defaults = entry["params"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, spec)| (name.clone(), spec["default"].as_f64().unwrap() as f32))
            .collect();
        let p = patch(
            vec![Node {
                id: "module".into(),
                kind: kind.into(),
                params: defaults,
            }],
            vec![],
            "module",
            outputs[0],
        );
        p.validate().unwrap();
        let json = serde_json::to_value(&p).unwrap();
        let restored: Patch = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&restored).unwrap(), json);
        assert_eq!(render(&p, 3), render(&restored, 3));
        let mut legacy = p;
        legacy.version = 1;
        assert!(legacy.validate().is_err());
    }
}

#[test]
fn invalid_controls_and_persisted_port_names_fail_validation() {
    for (kind, parameter, bad) in [
        ("mid_side", "encode_width", -0.1),
        ("mid_side", "decode_width", 2.01),
        ("mix6", "level", -0.1),
        ("mix6", "level", 1.01),
        ("mix6", "invert", 0.5),
        ("mix6", "average", 2.0),
        ("octave", "octaves", -5.0),
        ("octave", "octaves", 5.0),
        ("octave", "octaves", 0.5),
        ("mult", "gain", 1.0),
    ] {
        let output = match kind {
            "mid_side" => "mid",
            "mult" => "out1",
            _ => "out",
        };
        let mut p = patch(
            vec![node("module", kind, &[(parameter, bad)])],
            vec![],
            "module",
            output,
        );
        assert!(p.validate().is_err(), "{kind}.{parameter}={bad}");
        p.nodes[0].params.clear();
        p.nodes[0].params.insert("unknown".into(), 1.0);
        assert!(p.validate().is_err(), "unknown {kind} parameter accepted");
    }
    for (kind, port) in [
        ("mid_side", "out"),
        ("mix6", "out2"),
        ("octave", "pitch"),
        ("mult", "out9"),
        ("mult", "out01"),
    ] {
        let p = patch(vec![node("module", kind, &[])], vec![], "module", port);
        assert!(p.validate().is_err(), "invalid selected port {kind}.{port}");
    }
    let p = patch(
        vec![source("src", &[1.0]), node("mult", "mult", &[])],
        vec![wire("src", "out", "mult", "signal2")],
        "mult",
        "out1",
    );
    assert!(p.validate().is_err());
    let mut json = serde_json::to_value(&p).unwrap();
    json["nodes"][1]["extra_field"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Patch>(json).is_err());
}
