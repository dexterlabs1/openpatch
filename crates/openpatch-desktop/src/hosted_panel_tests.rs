use super::*;

fn fixture() -> (egui::Context, Rack, Patch) {
    let document = openpatch::project_document::ProjectDocument::decode(include_bytes!(
        "../../../examples/experimental-hosted-offset.json"
    ))
    .unwrap();
    let patch = document.state_carrier().unwrap().patch;
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    rack.reconcile_document(&patch, &document, &openpatch_core::catalog());
    frame_revision_state(&ctx, &mut rack, &patch, 0., vec![], true, true, 17);
    (ctx, rack, patch)
}
fn point(rack: &Rack, local: Pos2) -> Pos2 {
    rack.viewport.unwrap().min
        + rack.pan
        + (rack.positions["binary"].to_vec2() + local.to_vec2()) * rack.zoom
}
fn command(action: Option<RackAction>, expected: &str) -> Value {
    match action {
        Some(RackAction::Command(name, args)) if name == expected => args,
        _ => panic!("expected hosted {expected} interaction"),
    }
}

#[test]
fn hosted_example_native_and_hosted_panels_are_contiguous_without_overlap() {
    let (_, rack, patch) = fixture();
    let mut panels: Vec<_> = patch
        .nodes
        .iter()
        .map(|node| {
            (
                node.id.as_str(),
                Rect::from_min_size(
                    rack.positions[&node.id],
                    vec2(panel_width(&node.kind), PANEL_HEIGHT),
                ),
            )
        })
        .chain(rack.unresolved_panels.iter().map(|panel| {
            assert!(
                panel.hosted,
                "the example must retain its actual hosted panel"
            );
            (
                panel.id.as_str(),
                Rect::from_min_size(
                    rack.positions[&panel.id],
                    vec2(UNRESOLVED_WIDTH, PANEL_HEIGHT),
                ),
            )
        }))
        .collect();
    panels.sort_by(|a, b| a.1.min.x.total_cmp(&b.1.min.x));
    assert_eq!(
        panels.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        ["source", "binary", "amp"]
    );
    for pair in panels.windows(2) {
        let (left_id, left) = pair[0];
        let (right_id, right) = pair[1];
        assert!(
            !left.intersect(right).is_positive(),
            "example panels {left_id} {left:?} and {right_id} {right:?} overlap"
        );
        assert_eq!(left.min.y, right.min.y, "example panels must share a row");
        assert_eq!(left.max.x, right.min.x, "example panels must be contiguous");
    }
}

#[test]
fn hosted_shared_knob_has_hit_priority_grouped_history_and_revision_fence() {
    let (ctx, mut rack, patch) = fixture();
    let knob = point(&rack, pos2(55., 278.));
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.,
        vec![egui::Event::PointerMoved(knob), button(knob, true)],
        true,
        true,
        17,
    );
    let first = command(
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![egui::Event::PointerMoved(knob - vec2(0., 12.))],
            true,
            true,
            17,
        )
        .0,
        "set_hosted_parameter",
    );
    assert_eq!(first["id"], "binary");
    assert_eq!(first["parameter"], "offset");
    assert_eq!(first["expected_revision"], 17);
    assert!(first["value"].as_f64().unwrap() > 0.);
    let second = command(
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![egui::Event::PointerMoved(knob - vec2(0., 24.))],
            true,
            true,
            18,
        )
        .0,
        "set_hosted_parameter",
    );
    assert_eq!(first["history_group"], second["history_group"]);
    assert_eq!(second["expected_revision"], 18);
    assert!(second["value"].as_f64().unwrap() > first["value"].as_f64().unwrap());
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![button(knob - vec2(0., 24.), false)],
        true,
        true,
        18,
    );
    assert!(
        rack.take_dependency_focus().is_none(),
        "knob drag must not activate panel background"
    );
}

#[test]
fn hosted_persistent_option_submits_shared_command_and_disabled_panel_cannot_edit() {
    let (ctx, mut rack, patch) = fixture();
    let option = point(&rack, pos2(90., 328.));
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.,
        vec![egui::Event::PointerMoved(option), button(option, true)],
        true,
        true,
        17,
    );
    let args = command(
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![button(option, false)],
            true,
            true,
            17,
        )
        .0,
        "set_hosted_option",
    );
    assert_eq!(
        args,
        json!({"id":"binary","option":"offset_first","value":false,"expected_revision":17})
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        2.,
        vec![egui::Event::PointerMoved(option), button(option, true)],
        true,
        false,
        17,
    );
    assert!(frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        2.1,
        vec![button(option, false)],
        true,
        false,
        17
    )
    .0
    .is_none());
}

fn xfade_fixture() -> (egui::Context, Rack, Patch) {
    use openpatch::project_document::{hosted::HostedModule, DocumentNode};
    let mut document = openpatch::project_document::ProjectDocument::decode(include_bytes!(
        "../../../examples/experimental-hosted-offset.json"
    ))
    .unwrap();
    for node in &mut document.graph.nodes {
        if let DocumentNode::Hosted { module, .. } = node {
            let mut identity = module.identity.clone();
            identity.model = "Bogaudio-XFade".into();
            *module = HostedModule::xfade(identity, 0., 0.5, false).unwrap();
        }
    }
    for cable in &mut document.graph.cables {
        if cable.route.to == "binary" {
            cable.route.input = "a".into();
        }
    }
    let patch = document.state_carrier().unwrap().patch;
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    rack.reconcile_document(&patch, &document, &openpatch_core::catalog());
    frame_revision_state(&ctx, &mut rack, &patch, 0., vec![], true, true, 17);
    (ctx, rack, patch)
}

#[test]
fn hosted_xfade_rotaries_use_supported_ranges_and_shared_history() {
    for (name, center, initial) in [
        ("mix", pos2(55., 278.), 0.),
        ("curve", pos2(125., 278.), 0.5),
    ] {
        let (ctx, mut rack, patch) = xfade_fixture();
        let knob = point(&rack, center);
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.,
            vec![egui::Event::PointerMoved(knob), button(knob, true)],
            true,
            true,
            17,
        );
        let args = command(
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.1,
                vec![egui::Event::PointerMoved(knob - vec2(0., 12.))],
                true,
                true,
                17,
            )
            .0,
            "set_hosted_parameter",
        );
        assert_eq!(args["id"], "binary");
        assert_eq!(args["parameter"], name);
        assert_eq!(args["expected_revision"], 17);
        assert!(args["history_group"].is_string());
        let value = args["value"].as_f64().unwrap();
        assert!(value > initial && value <= 1., "{name}: {value}");
    }
}

#[test]
fn hosted_xfade_linear_is_a_parameter_and_disabled_panel_cannot_edit_it() {
    let (ctx, mut rack, patch) = xfade_fixture();
    let linear = point(&rack, pos2(90., 328.));
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.,
        vec![egui::Event::PointerMoved(linear), button(linear, true)],
        true,
        true,
        17,
    );
    let args = command(
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![button(linear, false)],
            true,
            true,
            17,
        )
        .0,
        "set_hosted_parameter",
    );
    assert_eq!(
        args,
        json!({"id":"binary","parameter":"linear","value":1.,"expected_revision":17})
    );
    assert!(rack.take_dependency_focus().is_none());
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        2.,
        vec![egui::Event::PointerMoved(linear), button(linear, true)],
        true,
        false,
        17,
    );
    assert!(frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        2.1,
        vec![button(linear, false)],
        true,
        false,
        17,
    )
    .0
    .is_none());
}

#[test]
fn hosted_xfade_shared_control_labels_fit_at_rack_and_thumbnail_scales() {
    fn labels(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Vec(children) => {
                for child in children {
                    labels(child, out);
                }
            }
            egui::Shape::Text(text) => out.push((
                text.galley.text().into(),
                Rect::from_min_size(text.pos, text.galley.size()),
            )),
            _ => {}
        }
    }
    for scale in [0.42, 0.85, 1.] {
        let (ctx, mut rack, patch) = xfade_fixture();
        rack.zoom = scale;
        let output = frame_revision_state(&ctx, &mut rack, &patch, 1., vec![], true, true, 17).1;
        let panel = Rect::from_min_size(
            point(&rack, Pos2::ZERO),
            vec2(UNRESOLVED_WIDTH, PANEL_HEIGHT) * scale,
        );
        let mut text = Vec::new();
        for shape in &output.shapes {
            labels(&shape.shape, &mut text);
        }
        let text: Vec<_> = text
            .into_iter()
            .filter(|(_, bounds)| panel.contains(bounds.center()))
            .collect();
        for expected in ["MIX", "CURVE", "○ LINEAR", "MIX CV", "A", "B", "OUT"] {
            let bounds = text
                .iter()
                .find_map(|(label, bounds)| (label == expected).then_some(bounds))
                .unwrap_or_else(|| panic!("missing XFade label {expected} at {scale}"));
            assert!(panel.contains_rect(*bounds), "{expected}: {bounds:?}");
        }
        for (label, _) in text {
            assert!(
                !["OFFSET", "SCALE", "● Offset first", "○ Disable limit"].contains(&label.as_str()),
                "XFade must not draw OFFSET controls: {label}"
            );
        }
    }
}
