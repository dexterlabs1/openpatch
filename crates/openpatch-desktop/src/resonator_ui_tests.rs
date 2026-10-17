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
fn resonator_rotary_drag_groups_one_gesture_and_is_keyboard_adjustable() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
    let start = resonator_control_rects(1).0.center() + vec2(24.0, 12.0);
    let grouped = |action| match action {
        Some(RackAction::Command("set_parameter", args)) => {
            assert_eq!(args["parameter"], "stiffness");
            (
                args["history_group"]
                    .as_str()
                    .expect("one grouped rotary gesture")
                    .to_owned(),
                args["value"].as_f64().unwrap() as f32,
                args["expected_revision"].as_u64().unwrap(),
            )
        }
        _ => panic!("expected rotary edit"),
    };
    assert!(frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(start), button(start, true)],
        true,
        true,
        7,
    )
    .0
    .is_none());
    let (group, first, revision) = grouped(
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![egui::Event::PointerMoved(start - vec2(0.0, 15.0))],
            true,
            true,
            7,
        )
        .0,
    );
    assert_eq!(revision, 7);
    patch.nodes[0].params.insert("stiffness".into(), first);
    let (same, second, revision) = grouped(
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![egui::Event::PointerMoved(start - vec2(0.0, 30.0))],
            true,
            true,
            8,
        )
        .0,
    );
    assert_eq!(group, same);
    assert_eq!(revision, 8);
    assert!(second > first && first > 0.2);
    patch.nodes[0].params.insert("stiffness".into(), second);
    assert!(frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![button(start - vec2(0.0, 30.0), false)],
        true,
        true,
        9,
    )
    .0
    .is_none());
    let keyboard = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.4,
        vec![egui::Event::Key {
            key: egui::Key::ArrowUp,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        true,
        true,
        9,
    )
    .0;
    let Some(RackAction::Command("set_parameter", keyboard)) = keyboard else {
        panic!("focused rotary must accept ArrowUp")
    };
    assert_eq!(keyboard["parameter"], "stiffness");
    assert!(keyboard["value"].as_f64().unwrap() > f64::from(second));
    assert_eq!(keyboard["expected_revision"], 9);
    assert!(keyboard.get("history_group").is_none());
    patch.nodes[0].params.insert(
        "stiffness".into(),
        keyboard["value"].as_f64().unwrap() as f32,
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        2.0,
        vec![egui::Event::PointerMoved(start), button(start, true)],
        true,
        true,
        10,
    );
    let (other, _, revision) = grouped(
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            2.1,
            vec![egui::Event::PointerMoved(start - vec2(0.0, 15.0))],
            true,
            true,
            10,
        )
        .0,
    );
    assert_ne!(other, group);
    assert_eq!(revision, 10);
}

#[test]
fn resonator_rotary_ctrl_drag_uses_fine_increment() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch(), None).unwrap();
    frame_revision_state(
        &ctx,
        &mut rack,
        session.patch(),
        0.0,
        vec![],
        true,
        true,
        session.revision(),
    );
    let start = resonator_control_rects(1).0.center() + vec2(24.0, 12.0);
    let fine = egui::Modifiers {
        ctrl: true,
        ..Default::default()
    };
    let press = egui::Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: fine,
    };
    let baseline = session.snapshot()["history"]["undo_steps"]
        .as_u64()
        .unwrap();
    assert!(frame_revision_state(
        &ctx,
        &mut rack,
        session.patch(),
        1.0,
        vec![egui::Event::PointerMoved(start), press],
        true,
        true,
        session.revision()
    )
    .0
    .is_none());
    let carrier = || egui::Event::Key {
        key: egui::Key::F12,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: fine,
    };
    let moved = start - vec2(0.0, 15.0);
    let revision = session.revision();
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        session.patch(),
        1.1,
        vec![egui::Event::PointerMoved(moved), carrier()],
        true,
        true,
        revision,
    )
    .0;
    let Some(RackAction::Command("set_parameter", first)) = action else {
        panic!("fine rotary drag must edit stiffness")
    };
    assert_eq!(first["parameter"], "stiffness");
    assert_eq!(first["expected_revision"], revision);
    assert!((first["value"].as_f64().unwrap() - 0.21).abs() < 1e-6);
    assert!(first["history_group"].is_string());
    session.execute("set_parameter", first.clone()).unwrap();
    let next = start - vec2(0.0, 30.0);
    let revision = session.revision();
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        session.patch(),
        1.2,
        vec![egui::Event::PointerMoved(next), carrier()],
        true,
        true,
        revision,
    )
    .0;
    let Some(RackAction::Command("set_parameter", second)) = action else {
        panic!("second fine rotary move must edit stiffness")
    };
    assert_eq!(second["expected_revision"], revision);
    assert_eq!(second["history_group"], first["history_group"]);
    assert!((second["value"].as_f64().unwrap() - 0.22).abs() < 1e-6);
    session.execute("set_parameter", second.clone()).unwrap();
    let saved = session.patch().nodes[0].params["stiffness"];
    assert!(frame_revision_state(
        &ctx,
        &mut rack,
        session.patch(),
        1.3,
        vec![button(next, false)],
        true,
        true,
        session.revision()
    )
    .0
    .is_none());
    assert_eq!(
        session.snapshot()["history"]["undo_steps"]
            .as_u64()
            .unwrap(),
        baseline + 1
    );
    session.execute("undo", json!({})).unwrap();
    assert_eq!(
        session.patch().nodes[0]
            .params
            .get("stiffness")
            .copied()
            .unwrap_or(0.2),
        0.2
    );
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.patch().nodes[0].params["stiffness"], saved);
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
    for (parameter, value) in [
        ("pickup_position", 0.5),
        ("strike_position", 0.5),
        ("stiffness", 0.8),
    ] {
        let mut configured = patch.clone();
        configured.nodes[0].params.insert(parameter.into(), value);
        let changed = frame_output(&ctx, &mut rack, &configured, 31.0, vec![]).1;
        assert_ne!(
            before,
            stems(&changed),
            "{parameter} must alter the configured modal stems"
        );
    }
    let profile = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect)
                    if rect.rect.width() == 5.0 && rect.rect.bottom() == 300.0 =>
                {
                    Some(format!("{:?}", rect.rect))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let decay_before = profile(&first);
    assert!(!decay_before.is_empty());
    patch.nodes[0].params.insert("damping".into(), 0.9);
    let damped = frame_output(&ctx, &mut rack, &patch, 32.0, vec![]).1;
    assert_ne!(
        decay_before,
        profile(&damped),
        "loss changes the decay profile"
    );
    let modes = openpatch_core::modal_resonator_modes(&patch.nodes[0], 48000).unwrap();
    let expected = format!(
        "RT60  {:.2} → {:.2} s",
        modes[0].decay_seconds,
        modes[modes.len() - 1].decay_seconds
    );
    let mut damped_text = Vec::new();
    for shape in &damped.shapes {
        labels(&shape.shape, &mut damped_text);
    }
    assert!(damped_text.contains(&expected));
    for label in ["EXCITER / FORCE", "MODES / LOSS", "VELOCITY → OUT"] {
        assert!(damped_text.iter().any(|text| text == label));
    }
}

#[test]
fn resonator_printed_frequency_range_matches_effective_precision_limits() {
    for (rate, label, limit) in [(8000, "20–3600 Hz", 3600.0), (48000, "20–4000 Hz", 4000.0)] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch = patch();
        patch.sample_rate = rate;
        let (_, output) = frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 9);
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == label)),
            "{rate}: truthful range print"
        );
        let at = pos2(24.0, 12.0) + resonator_control_rects(0).1.center().to_vec2();
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(at), button(at, true)],
            true,
            true,
            9,
        );
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![button(at, false)],
            true,
            true,
            9,
        );
        let edit = rack.editing.as_ref().expect("frequency exact entry");
        assert_eq!(edit.max, limit);
        assert_eq!(edit.raw_max, limit);
    }
}
