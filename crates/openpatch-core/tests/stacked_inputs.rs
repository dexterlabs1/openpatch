use openpatch_core::{Cable, Engine, Node, Patch, PortFrame};

fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        score: None,
        id: id.into(),
        kind: kind.into(),
        bypassed: false,
        params: params
            .iter()
            .map(|(key, value)| (key.to_string(), *value))
            .collect(),
    }
}

fn cable(id: Option<&str>, from: &str, to: &str, input: &str) -> Cable {
    Cable {
        input_order: None,
        output_order: None,
        color: None,
        id: id.map(str::to_string),
        from: from.into(),
        output: "out".into(),
        to: to.into(),
        input: input.into(),
    }
}

fn one(engine: &mut Engine) -> PortFrame {
    let mut frames = [PortFrame::default()];
    engine.process_poly_block(&mut frames);
    frames[0]
}

fn stacked() -> Patch {
    Patch {
        nodes: vec![
            node("a", "constant", &[("value", 1.0)]),
            node("b", "constant", &[("value", 2.0)]),
            node("c", "constant", &[("value", 3.0)]),
            node("out", "gain", &[]),
        ],
        cables: vec![
            cable(Some("a_route"), "a", "out", "signal"),
            cable(Some("b_route"), "b", "out", "signal"),
        ],
        output: Some("out".into()),
        ..Patch::new_v2()
    }
}

#[test]
fn mixed_width_broadcast_sparse_and_zero_sources_sum_by_channel() {
    let mut patch = Patch {
        nodes: vec![
            node("mono", "poly_constant", &[("channels", 1.0), ("v1", 2.0)]),
            node(
                "wide",
                "poly_constant",
                &[("channels", 3.0), ("v1", 1.0), ("v2", 2.0), ("v3", 3.0)],
            ),
            node(
                "short",
                "poly_constant",
                &[("channels", 2.0), ("v1", 4.0), ("v2", 5.0)],
            ),
            node("zero", "poly_constant", &[("channels", 0.0)]),
            node("out", "gain", &[]),
        ],
        cables: vec![
            cable(None, "mono", "out", "signal"),
            cable(None, "wide", "out", "signal"),
            cable(None, "short", "out", "signal"),
            cable(None, "zero", "out", "signal"),
        ],
        output: Some("out".into()),
        ..Patch::new_v2()
    };
    let mut engine = patch.compile().unwrap();
    assert_eq!(one(&mut engine).values[0], 0.0);
    let frame = one(&mut engine);
    assert_eq!(frame.channels, 3);
    assert_eq!(&frame.values[..4], &[7.0, 9.0, 5.0, 0.0]);
    patch.cables.retain(|c| c.from == "zero");
    let mut zero_only = patch.compile().unwrap();
    one(&mut zero_only);
    assert_eq!(one(&mut zero_only).values[0], 0.0);
}

#[test]
fn multiple_zero_channel_cables_leave_cv_normaling_active() {
    let patch = Patch {
        nodes: vec![
            node("signal", "constant", &[("value", 2.0)]),
            node("zero_a", "poly_constant", &[("channels", 0.0)]),
            node("zero_b", "poly_constant", &[("channels", 0.0)]),
            node("out", "gain", &[]),
        ],
        cables: vec![
            cable(None, "signal", "out", "signal"),
            cable(None, "zero_a", "out", "cv"),
            cable(None, "zero_b", "out", "cv"),
        ],
        output: Some("out".into()),
        ..Patch::new_v2()
    };
    let mut engine = patch.compile().unwrap();
    one(&mut engine);
    assert_eq!(one(&mut engine).values[0], 2.0);
}

#[test]
fn stacked_width_shrinks_and_regrows_after_compatible_adoption() {
    let mut patch = Patch {
        nodes: vec![
            node(
                "wide",
                "poly_constant",
                &[("channels", 3.0), ("v1", 1.0), ("v2", 2.0), ("v3", 3.0)],
            ),
            node("mono", "poly_constant", &[("channels", 1.0), ("v1", 2.0)]),
            node("out", "gain", &[]),
        ],
        cables: vec![
            cable(Some("wide_route"), "wide", "out", "signal"),
            cable(Some("mono_route"), "mono", "out", "signal"),
        ],
        output: Some("out".into()),
        ..Patch::new_v2()
    };
    let mut wide = patch.compile().unwrap();
    one(&mut wide);
    assert_eq!(&one(&mut wide).values[..3], &[3.0, 4.0, 5.0]);

    patch.nodes[0].params.insert("channels".into(), 1.0);
    let mut narrow = patch.compile().unwrap();
    narrow.adopt_state_from(&wide);
    let delayed = one(&mut narrow);
    assert_eq!(delayed.channels, 3);
    assert_eq!(&delayed.values[..3], &[3.0, 4.0, 5.0]);
    let settled = one(&mut narrow);
    assert_eq!(settled.channels, 1);
    assert_eq!(settled.values[0], 3.0);

    patch.nodes[0].params.insert("channels".into(), 3.0);
    let mut regrown = patch.compile().unwrap();
    regrown.adopt_state_from(&narrow);
    assert_eq!(one(&mut regrown).channels, 1);
    let restored = one(&mut regrown);
    assert_eq!(restored.channels, 3);
    assert_eq!(&restored.values[..3], &[3.0, 4.0, 5.0]);
}

#[test]
fn duplicate_routes_contribute_twice_and_feedback_delays_one_sample() {
    let mut patch = Patch {
        nodes: vec![
            node("source", "constant", &[("value", 1.0)]),
            node("out", "mixer", &[]),
        ],
        cables: vec![
            cable(Some("first"), "source", "out", "a"),
            cable(Some("second"), "source", "out", "a"),
            cable(Some("feedback"), "out", "out", "b"),
        ],
        output: Some("out".into()),
        ..Patch::new_v2()
    };
    let mut engine = patch.compile().unwrap();
    assert_eq!(
        [
            one(&mut engine).values[0],
            one(&mut engine).values[0],
            one(&mut engine).values[0]
        ],
        [0.0, 2.0, 4.0]
    );
    patch.version = 1;
    assert!(patch.compile().is_err());
}

#[test]
fn deterministic_f32_order_and_signed_cancellation() {
    let make = |order: &[&str]| Patch {
        nodes: vec![
            node(
                "positive",
                "poly_constant",
                &[("channels", 1.0), ("v1", 16.0)],
            ),
            node(
                "negative",
                "poly_constant",
                &[("channels", 1.0), ("v1", -16.0)],
            ),
            node(
                "small",
                "poly_constant",
                &[("channels", 1.0), ("v1", 0.0000001)],
            ),
            node("out", "poly_split", &[]),
        ],
        cables: order
            .iter()
            .map(|source| cable(None, source, "out", "signal"))
            .collect(),
        output: Some("out".into()),
        output_port: "out1".into(),
        ..Patch::new_v2()
    };
    let mut first = make(&["positive", "negative", "small"]).compile().unwrap();
    let mut second = make(&["positive", "small", "negative"]).compile().unwrap();
    one(&mut first);
    one(&mut second);
    assert_eq!(one(&mut first).values[0], 0.0000001);
    assert_eq!(one(&mut second).values[0], 0.0);
}

#[test]
fn id_allocation_reserves_explicit_ids_and_rejects_invalid_duplicates() {
    let mut patch = stacked();
    patch.cables = vec![
        cable(None, "a", "out", "signal"),
        cable(Some("cable_1"), "b", "out", "signal"),
        cable(None, "c", "out", "signal"),
    ];
    patch.assign_missing_cable_ids().unwrap();
    assert_eq!(
        patch
            .cables
            .iter()
            .map(|c| c.id.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("cable_2"), Some("cable_1"), Some("cable_3")]
    );
    let once = serde_json::to_string(&patch).unwrap();
    patch.assign_missing_cable_ids().unwrap();
    assert_eq!(serde_json::to_string(&patch).unwrap(), once);
    let mut invalid = patch.clone();
    invalid.cables[0].id = Some("bad space".into());
    assert!(invalid.compile().is_err());
    assert!(invalid.assign_missing_cable_ids().is_err());
    invalid.cables[0].id = Some("cable_1".into());
    assert!(invalid.compile().is_err());
    assert!(invalid.assign_missing_cable_ids().is_err());
    let legacy_json = r#"{"from":"a","to":"out","input":"signal"}"#;
    assert_eq!(serde_json::from_str::<Cable>(legacy_json).unwrap().id, None);
}

#[test]
fn route_history_survives_stack_edit_and_explicit_exclusion_is_fresh() {
    let patch = stacked();
    let mut old = patch.compile().unwrap();
    one(&mut old);
    let mut expanded = patch.clone();
    expanded
        .cables
        .push(cable(Some("c_route"), "c", "out", "signal"));
    let mut adopted = expanded.compile().unwrap();
    adopted.adopt_state_from(&old);
    assert_eq!(one(&mut adopted).values[0], 3.0);
    let mut excluded = expanded.compile().unwrap();
    let handle = excluded.resolve_cable_state("b_route").unwrap();
    assert!(excluded.resolve_cable_state("absent").is_err());
    excluded.adopt_state_from_excluding_routes(&old, &[], &[handle]);
    assert_eq!(one(&mut excluded).values[0], 1.0);
    let mut renamed = patch.clone();
    renamed.cables[1].id = Some("new_identity".into());
    let mut changed = renamed.compile().unwrap();
    changed.adopt_state_from(&old);
    assert_eq!(one(&mut changed).values[0], 1.0);

    let mut moved = patch.clone();
    moved.cables[1].from = "c".into();
    let mut rewired = moved.compile().unwrap();
    rewired.adopt_state_from(&old);
    assert_eq!(one(&mut rewired).values[0], 1.0);
}

#[test]
fn unique_idless_route_can_promote_to_explicit_identity() {
    let mut patch = stacked();
    patch.cables[0].id = None;
    patch.cables[1].id = None;
    let mut old = patch.compile().unwrap();
    one(&mut old);
    patch.assign_missing_cable_ids().unwrap();
    let mut promoted = patch.compile().unwrap();
    promoted.adopt_state_from(&old);
    assert_eq!(one(&mut promoted).values[0], 3.0);
}

#[test]
fn idless_direct_adoption_survives_node_and_cable_reorder() {
    let patch = Patch {
        nodes: vec![
            node("left", "constant", &[("value", 1.0)]),
            node("right", "constant", &[("value", 2.0)]),
            node("out", "audio_stereo", &[]),
        ],
        cables: vec![
            cable(None, "left", "out", "left"),
            cable(None, "right", "out", "right"),
        ],
        output: Some("out".into()),
        ..Patch::new_v2()
    };
    let mut old = patch.compile().unwrap();
    one(&mut old);
    let mut reordered = patch.clone();
    reordered.nodes.reverse();
    reordered.cables.reverse();
    let mut next = reordered.compile().unwrap();
    next.adopt_state_from(&old);
    let frame = one(&mut next);
    assert_eq!(&frame.values[..2], &[0.1, 0.2]);
}
