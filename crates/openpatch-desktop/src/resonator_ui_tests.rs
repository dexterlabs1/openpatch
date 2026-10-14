use super::*;

fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"res","kind":"modal_resonator"}]}))
        .unwrap()
}

#[test]
fn every_resonator_value_opens_guarded_exact_entry_with_correct_units() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "modal_resonator")
        .unwrap();
    let patch = patch();
    let params = ordered_parameters(&patch.nodes[0], Some(module));
    assert_eq!(
        params.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        [
            "frequency",
            "stiffness",
            "decay",
            "damping",
            "strike_position",
            "pickup_position",
            "strike_strength",
            "level"
        ]
    );
    for (index, (name, spec)) in params.into_iter().enumerate() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
        let readout = pos2(24.0, 12.0) + resonator_control_rects(index).1.center().to_vec2();
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(readout), button(readout, true)],
            true,
            true,
            7,
        );
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![button(readout, false)],
            true,
            true,
            7,
        );
        let edit = rack
            .editing
            .as_mut()
            .expect("single click opens exact entry");
        assert_eq!(edit.parameter, name);
        assert_eq!(edit.revision, 7);
        assert_eq!(edit.surface, ParameterEditSurface::Window);
        let percent = matches!(
            name,
            "stiffness" | "damping" | "strike_position" | "pickup_position"
        );
        assert_eq!(edit.display_scale, if percent { 100.0 } else { 1.0 });
        assert_eq!(
            edit.min,
            spec["min"].as_f64().unwrap() as f32 * edit.display_scale
        );
        let target = if name == "frequency" {
            440.0
        } else if name == "decay" {
            3.0
        } else {
            0.5
        };
        edit.text = (target * edit.display_scale).to_string();
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            true,
            true,
            7,
        )
        .0;
        assert!(
            matches!(action, Some(RackAction::Command("set_parameter", args))
            if args["id"] == "res" && args["parameter"] == name && args["value"] == json!(target)
            && args["expected_revision"] == 7),
            "{name} exact entry must publish raw values with revision guard"
        );
    }
}

#[test]
fn resonator_slider_drag_groups_one_gesture_and_is_keyboard_adjustable() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = patch();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    // Click away from the thumb before dragging; the initial jump must join
    // the same undo group as the rest of this gesture.
    let rail = resonator_control_rects(1).0.translate(vec2(24.0, 12.0));
    let start = pos2(
        rail.left() + 5.0 + (rail.width() - 10.0) * 0.4,
        rail.center().y,
    );
    let grouped = |action| match action {
        Some(RackAction::Command("set_parameter", args)) => {
            assert_eq!(args["parameter"], "stiffness");
            (
                args["history_group"]
                    .as_str()
                    .expect("one grouped slider gesture")
                    .to_owned(),
                args["value"].as_f64().unwrap() as f32,
            )
        }
        _ => panic!("expected slider edit"),
    };
    let (press_group, pressed_value) = grouped(frame(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    ));
    patch.nodes[0]
        .params
        .insert("stiffness".into(), pressed_value);
    let (group, first) = grouped(frame(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![egui::Event::PointerMoved(start + vec2(15.0, 0.0))],
    ));
    patch.nodes[0].params.insert("stiffness".into(), first);
    let (same, second) = grouped(frame(
        &ctx,
        &mut rack,
        &patch,
        1.2,
        vec![egui::Event::PointerMoved(start + vec2(30.0, 0.0))],
    ));
    assert_eq!(group, same);
    assert_eq!(press_group, group);
    assert!(second > first && first > 0.2);
    patch.nodes[0].params.insert("stiffness".into(), second);
    frame(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![button(start + vec2(30.0, 0.0), false)],
    );
    let keyboard = frame(
        &ctx,
        &mut rack,
        &patch,
        1.4,
        vec![egui::Event::Key {
            key: egui::Key::ArrowRight,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(
        matches!(keyboard, Some(RackAction::Command("set_parameter", args))
        if args["parameter"] == "stiffness" && args["value"].as_f64().unwrap() > f64::from(second)
        && args.get("history_group").is_none())
    );
    frame(
        &ctx,
        &mut rack,
        &patch,
        2.0,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    let (other, _) = grouped(frame(
        &ctx,
        &mut rack,
        &patch,
        2.1,
        vec![egui::Event::PointerMoved(start + vec2(15.0, 0.0))],
    ));
    assert_ne!(other, group);
}

#[test]
fn resonator_controls_and_named_patch_targets_stay_separate() {
    let width = panel_width("modal_resonator");
    assert_eq!(width, 480.0);
    let panel = Rect::from_min_size(Pos2::ZERO, vec2(width, PANEL_HEIGHT));
    let sockets = [(true, 3), (false, 1)]
        .into_iter()
        .flat_map(|(input, count)| {
            (0..count).map(move |index| {
                Rect::from_center_size(
                    port_position("modal_resonator", input, index, count, width),
                    vec2(36.0, 36.0),
                )
            })
        })
        .collect::<Vec<_>>();
    for (index, socket) in sockets.iter().enumerate() {
        assert!(panel.contains_rect(*socket));
        for other in &sockets[index + 1..] {
            assert!(!socket.intersects(*other));
        }
    }
    for index in 0..8 {
        let (slider, value) = resonator_control_rects(index);
        assert!(panel.contains_rect(slider) && panel.contains_rect(value));
        assert!(!slider.intersects(value));
        for socket in &sockets {
            assert!(!socket.intersects(slider) && !socket.intersects(value));
        }
        for other in index + 1..8 {
            let (next_slider, next_value) = resonator_control_rects(other);
            assert!(!slider.intersects(next_slider) && !value.intersects(next_value));
        }
    }
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"res","kind":"modal_resonator"}, {"id":"source","kind":"constant"}, {"id":"sink","kind":"audio_out"}
    ]})).unwrap();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let source = fixture_jack(&rack, &patch, "source", "out", false);
    for (index, port) in ["pitch", "strike", "excite"].into_iter().enumerate() {
        let input = fixture_jack(&rack, &patch, "res", port, true);
        assert_eq!(input, pos2(84.0 + index as f32 * 95.0, 375.0));
        let action = drag(&ctx, &mut rack, &patch, 1.0 + index as f64, source, input);
        assert!(
            matches!(action,Some(RackAction::Command("connect",args)) if args["to"] == "res" && args["input"] == port)
        );
    }
    let output = fixture_jack(&rack, &patch, "res", "out", false);
    let sink = fixture_jack(&rack, &patch, "sink", "signal", true);
    assert!(
        matches!(drag(&ctx,&mut rack,&patch,5.0,output,sink),Some(RackAction::Command("connect",args)) if args["from"] == "res" && args["output"] == "out")
    );
    assert!(rack.editing.is_none());
}

#[test]
fn resonator_plot_is_static_configuration_and_reacts_to_model_parameters() {
    fn labels(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().into()),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    labels(shape, out);
                }
            }
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = patch();
    let first = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1;
    let mut text = Vec::new();
    for shape in &first.shapes {
        labels(&shape.shape, &mut text);
    }
    for label in [
        "NATURAL MODES",
        "CONFIG · 48 kHz · ZERO CV",
        "± FORCE × PICKUP · LOG Hz",
        "RT60  2.00 → 0.36 s",
    ] {
        assert!(text.iter().any(|t| t == label), "missing {label}: {text:?}");
    }
    let stems = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::LineSegment { points, stroke }
                    if stroke.width == 1.6 && points[0].x > 330.0 && points[0].y > 130.0 =>
                {
                    Some(format!("{points:?}:{stroke:?}"))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let before = stems(&first);
    assert!(!before.is_empty());
    let later = frame_output(&ctx, &mut rack, &patch, 30.0, vec![]).1;
    assert_eq!(
        before,
        stems(&later),
        "configuration must not animate as if audio were live"
    );
    patch.nodes[0].params.insert("pickup_position".into(), 0.5);
    let changed = frame_output(&ctx, &mut rack, &patch, 31.0, vec![]).1;
    assert_ne!(
        before,
        stems(&changed),
        "pickup mode shapes must alter signed modal stems"
    );
}
