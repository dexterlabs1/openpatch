use super::*;

fn fixture(nodes: &[(&str, &str)], routes: &[(&str, &str)]) -> (Rack, Patch) {
    let patch: Patch = serde_json::from_value(json!({
        "nodes": nodes.iter().map(|(id, kind)| json!({"id":id,"kind":kind})).collect::<Vec<_>>(),
        "cables": routes.iter().map(|(from,to)| json!({"from":from,"to":to,"input":"in"})).collect::<Vec<_>>()
    })).unwrap();
    let mut rack = Rack::default();
    for (index, node) in patch.nodes.iter().enumerate() {
        rack.positions
            .insert(node.id.clone(), pos2(index as f32 * 900.0, ROW * 3.0));
    }
    (rack, patch)
}

fn assert_layout(rack: &Rack, patch: &Patch, proposal: &ModuleMove, rows: usize) {
    assert_eq!(
        proposal.before.len(),
        patch.nodes.len() + rack.unresolved_panels.len()
    );
    assert_eq!(
        proposal.before.keys().collect::<Vec<_>>(),
        proposal.after.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        proposal
            .after
            .values()
            .map(|point| (point.y / ROW) as usize)
            .collect::<BTreeSet<_>>()
            .len(),
        rows
    );
    let width = |id: &String| {
        patch
            .nodes
            .iter()
            .find(|node| &node.id == id)
            .map(module_width)
            .unwrap_or(UNRESOLVED_WIDTH)
    };
    for (id, position) in &proposal.after {
        assert!(position.is_finite());
        assert_eq!(position.x % HP, 0.0);
        assert_eq!(position.y % ROW, 0.0);
        for (other, other_position) in &proposal.after {
            if id >= other {
                continue;
            }
            assert!(
                position.y != other_position.y
                    || position.x + width(id) <= other_position.x
                    || other_position.x + width(other) <= position.x,
                "overlap: {id} / {other}"
            );
        }
    }
}

#[test]
fn organize_orders_connected_signal_chain_and_preserves_original_positions() {
    let (mut rack, patch) = fixture(
        &[
            ("afilter", "lowpass"),
            ("moutput", "output"),
            ("zsource", "oscillator"),
        ],
        &[("zsource", "afilter"), ("afilter", "moutput")],
    );
    rack.observed_revision = 19;
    let original = rack.positions.clone();
    let proposal = rack.organize(&patch, &Value::Null, Some(1)).unwrap();
    assert!(proposal.after["zsource"].x < proposal.after["afilter"].x);
    assert!(proposal.after["afilter"].x < proposal.after["moutput"].x);
    assert_eq!(proposal.revision, 19);
    assert_eq!(proposal.before, original);
    assert_eq!(rack.positions, original);
    assert_layout(&rack, &patch, &proposal, 1);
}

#[test]
fn organize_branches_feedback_and_disconnected_voices_are_deterministic() {
    let (rack, mut patch) = fixture(
        &[
            ("s", "oscillator"),
            ("a", "lowpass"),
            ("b", "gain"),
            ("c", "output"),
            ("x", "lfo"),
            ("y", "gain"),
        ],
        &[
            ("s", "a"),
            ("s", "b"),
            ("a", "c"),
            ("b", "c"),
            ("c", "a"),
            ("x", "y"),
        ],
    );
    let first = rack.organize(&patch, &Value::Null, Some(2)).unwrap();
    assert_layout(&rack, &patch, &first, 2);
    patch.nodes.reverse();
    patch.cables.reverse();
    assert_eq!(first, rack.organize(&patch, &Value::Null, Some(2)).unwrap());
    let widths = patch
        .nodes
        .iter()
        .map(|node| (node.id.clone(), module_width(node)))
        .collect();
    let edges = patch
        .cables
        .iter()
        .map(|cable| (cable.from.clone(), cable.to.clone()))
        .collect();
    let order = organize_signal_order(&widths, &edges);
    assert_eq!(&order[4..], &["x", "y"]);
    assert_eq!(order[0], "s");
}

#[test]
fn organize_respects_dynamic_widths_source_routes_and_exact_requested_rows() {
    let (mut rack, mut patch) = fixture(
        &[
            ("blank", "blank"),
            ("mixer", "studio_mixer"),
            ("output", "output"),
        ],
        &[],
    );
    patch.nodes[0].content = Some(openpatch_core::ModuleContent::Blank { width_hp: 37 });
    patch.nodes[1].params.insert("channels".into(), 13.0);
    rack.unresolved_panels.push(UnresolvedPanel {
        id: "source".into(),
        plugin: "Missing".into(),
        model: "Voice".into(),
        version: "1".into(),
        hosted: false,
        hosted_module: None,
        ports: vec![],
    });
    rack.positions.insert("source".into(), pos2(0.0, ROW * 9.0));
    rack.source_routes = serde_json::from_value(json!([
        {"from":"source","to":"mixer","input":"in"},
        {"from":"mixer","to":"output","input":"in"}
    ]))
    .unwrap();
    let one = rack.organize(&patch, &Value::Null, Some(1)).unwrap();
    assert!(one.after["source"].x < one.after["mixer"].x);
    assert!(one.after["mixer"].x < one.after["output"].x);
    for requested in 1..=8 {
        let proposal = rack
            .organize(&patch, &Value::Null, Some(requested))
            .unwrap();
        assert_layout(&rack, &patch, &proposal, requested.min(4));
    }
}

#[test]
fn organize_auto_adapts_to_viewport_and_refuses_locked_or_live_gestures() {
    let (mut rack, patch) = fixture(
        &[
            ("a", "oscillator"),
            ("b", "oscillator"),
            ("c", "oscillator"),
            ("d", "oscillator"),
        ],
        &[],
    );
    rack.viewport = Some(Rect::from_min_size(Pos2::ZERO, vec2(2400.0, 500.0)));
    let wide = rack.organize(&patch, &Value::Null, None).unwrap();
    rack.viewport = Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 1800.0)));
    let tall = rack.organize(&patch, &Value::Null, None).unwrap();
    assert!(
        tall.after.values().map(|p| p.y).fold(0.0, f32::max)
            > wide.after.values().map(|p| p.y).fold(0.0, f32::max)
    );
    assert_eq!(tall, rack.organize(&patch, &Value::Null, None).unwrap());
    rack.set_positions_locked(true);
    assert!(rack.organize(&patch, &Value::Null, None).is_err());
    rack.set_positions_locked(false);
    rack.dragging_cable = true;
    assert!(rack.organize(&patch, &Value::Null, None).is_err());
    rack.dragging_cable = false;
    assert!(rack.organize(&patch, &Value::Null, Some(0)).is_err());
    assert!(rack.organize(&patch, &Value::Null, Some(9)).is_err());
}
