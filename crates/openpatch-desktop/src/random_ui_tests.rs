use super::*;

const PARAMS: [&str; 9] = [
    "rate",
    "probability",
    "spread",
    "shape",
    "rate_cv",
    "probability_cv",
    "spread_cv",
    "shape_cv",
    "unipolar",
];
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"random","kind":"random"}]})).unwrap()
}
fn key(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}
fn open_value(index: usize) -> (egui::Context, Rack, Patch) {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let position = random_value_center(index) + vec2(24.0, 12.0);
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(position), button(position, true)],
        true,
        true,
        29,
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(position, false)],
        true,
        true,
        29,
    );
    assert_eq!(
        rack.editing.as_ref().expect("numeric target").parameter,
        PARAMS[index]
    );
    frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![], true, true, 29);
    (ctx, rack, patch)
}
fn collect_text(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                collect_text(child, out);
            }
        }
        egui::Shape::Text(text) if !text.galley.text().is_empty() => out.push((
            text.galley.text().into(),
            Rect::from_min_size(text.pos, text.galley.size()),
        )),
        _ => {}
    }
}
fn texts(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    let mut result = vec![];
    for shape in &output.shapes {
        collect_text(&shape.shape, &mut result);
    }
    result
}

#[test]
fn random_all_nine_numeric_controls_apply_in_displayed_units_with_revision_guard() {
    for (index, text, expected) in [
        (0, "1/2 Hz", -1.0),
        (1, "35%", 0.35),
        (2, "60%", 0.6),
        (3, "25%", 0.25),
        (4, "-75%", -0.75),
        (5, "50%", 0.5),
        (6, "-100%", -1.0),
        (7, "100%", 1.0),
        (8, "1", 1.0),
    ] {
        let (ctx, mut rack, patch) = open_value(index);
        let edit = rack.editing.as_mut().unwrap();
        assert_eq!(edit.revision, 29);
        assert_eq!(
            edit.display_unit,
            if index == 0 {
                "Hz"
            } else if index == 8 {
                ""
            } else {
                "%"
            }
        );
        edit.text = text.into();
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.3,
            vec![key(egui::Key::Enter, true)],
            true,
            true,
            29,
        )
        .0;
        let Some(RackAction::Command("set_parameter", args)) = action else {
            panic!("numeric apply for {}", PARAMS[index]);
        };
        assert_eq!(args["id"], "random");
        assert_eq!(args["parameter"], PARAMS[index]);
        assert_eq!(args["expected_revision"], 29);
        assert!((args["value"].as_f64().unwrap() as f32 - expected).abs() < 1e-6);
        assert!(rack.editing.is_none());
    }
}

#[test]
fn random_numeric_rate_bounds_notes_and_percent_limits_reject_invalid_or_stale_edits() {
    let (ctx, mut rack, patch) = open_value(0);
    let edit = rack.editing.as_mut().unwrap();
    assert_eq!((edit.min, edit.max), (0.002, 2000.0));
    for (text, hz) in [("0.002 Hz", 0.002_f32), ("2000 Hz", 2000.0), ("A4", 440.0)] {
        edit.text = text.into();
        let raw = parameter_edit_raw_value(edit, parsed_parameter_result(edit).unwrap());
        assert!((raw - hz.log2()).abs() < 1e-6);
    }
    for text in ["0", "-1", "0.00199 Hz", "2000.01 Hz", "nan", "inf", "1/0"] {
        edit.text = text.into();
        assert!(parsed_parameter_result(edit).is_err(), "{text}");
    }
    edit.text = "4 Hz".into();
    assert!(frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![key(egui::Key::Enter, true)],
        true,
        true,
        30,
    )
    .0
    .is_none());
    assert!(rack.editing.as_ref().unwrap().stale);
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.4,
        vec![egui::Event::WindowFocused(false)],
        false,
        true,
        30,
    );
    assert!(rack.editing.is_none());
    for (index, &name) in PARAMS.iter().enumerate().skip(1) {
        let (_, mut rack, _) = open_value(index);
        let edit = rack.editing.as_mut().unwrap();
        let invalid: &[&str] = if index == 8 {
            &["-1", "0.5", "2"]
        } else if index < 4 {
            &["-0.01%", "100.01%"]
        } else {
            &["-100.01%", "100.01%"]
        };
        for text in invalid {
            edit.text = (*text).into();
            assert!(parsed_parameter_result(edit).is_err(), "{}: {text}", name);
        }
    }
}

#[test]
fn random_rotaries_and_polarity_accept_keyboard_edits_through_the_live_controls() {
    for (index, &name) in PARAMS.iter().enumerate() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch = patch();
        // Put each continuous control in the middle so Home changes it.
        patch.nodes[0]
            .params
            .insert(name.into(), if index == 0 { 1.0 } else { 0.5 });
        if index == 8 {
            patch.nodes[0].params.insert("unipolar".into(), 0.0);
        }
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch.clone(), None).unwrap();
        let revision = session.revision();
        let original = patch.nodes[0].params[name];
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, revision);
        let position = random_parameter_position(index) + vec2(24.0, 12.0);
        // Clicking the selected polarity segment focuses it without changing it.
        let position = position
            - if index == 8 {
                vec2(30.0, 0.0)
            } else {
                Vec2::ZERO
            };
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(position), button(position, true)],
            true,
            true,
            revision,
        );
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![button(position, false)],
            true,
            true,
            revision,
        );
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![key(
                if index == 8 {
                    egui::Key::End
                } else {
                    egui::Key::Home
                },
                true,
            )],
            true,
            true,
            revision,
        )
        .0;
        let Some(RackAction::Command("set_parameter", args)) = action else {
            panic!("keyboard target {}", name);
        };
        let expected = if index == 8 {
            1.0
        } else if index == 0 {
            0.002_f32.log2()
        } else if index < 4 {
            0.0
        } else {
            -1.0
        };
        assert_eq!(args["parameter"], name);
        assert_eq!(args["expected_revision"], revision);
        assert!((args["value"].as_f64().unwrap() as f32 - expected).abs() < 1e-6);
        session.execute("set_parameter", args).unwrap();
        assert!((session.patch().nodes[0].params[name] - expected).abs() < 1e-6);
        session.execute("undo", json!({})).unwrap();
        assert_eq!(session.patch().nodes[0].params[name], original);
        assert!(rack.editing.is_none());
        assert!(rack.held_manual.is_none());
    }
}

#[test]
fn random_every_named_jack_routes_independently_through_actual_cable_gestures() {
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"random","kind":"random"}, {"id":"source","kind":"constant"},
        {"id":"sink","kind":"audio_stereo"}]}))
    .unwrap();
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let source = fixture_jack(&rack, &patch, "source", "out", false);
    for (index, name) in [
        "rate_cv",
        "shape_cv",
        "trigger",
        "external",
        "probability_cv",
        "spread_cv",
    ]
    .into_iter()
    .enumerate()
    {
        let input = fixture_jack(&rack, &patch, "random", name, true);
        let action = drag(&ctx, &mut rack, &patch, 1.0 + index as f64, source, input);
        assert!(
            matches!(action, Some(RackAction::Command("connect", ref args))
            if args["to"] == "random" && args["input"] == name),
            "{name}"
        );
    }
    let sink = fixture_jack(&rack, &patch, "sink", "left", true);
    for (index, name) in ["stepped", "linear", "smooth", "exponential", "trigger"]
        .into_iter()
        .enumerate()
    {
        let output = fixture_jack(&rack, &patch, "random", name, false);
        let action = drag(&ctx, &mut rack, &patch, 8.0 + index as f64, output, sink);
        assert!(
            matches!(action, Some(RackAction::Command("connect", ref args))
            if args["from"] == "random" && args["output"] == name),
            "{name}"
        );
    }
    assert!(rack.held_manual.is_none());
}

#[test]
fn random_seed_context_menu_opens_integer_editor_and_commits_with_revision() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let at = pos2(75.0, 45.0);
    let secondary = |pressed| egui::Event::PointerButton {
        pos: at,
        button: PointerButton::Secondary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(at), secondary(true)],
        true,
        true,
        29,
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![secondary(false)],
        true,
        true,
        29,
    );
    let output = frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![], true, true, 29).1;
    let target = texts(&output)
        .into_iter()
        .find(|(text, _)| text == "Edit random seed…")
        .expect("seed remains available in the module menu")
        .1
        .center();
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![egui::Event::PointerMoved(target), button(target, true)],
        true,
        true,
        29,
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.4,
        vec![button(target, false)],
        true,
        true,
        29,
    );
    frame_revision_state(&ctx, &mut rack, &patch, 1.5, vec![], true, true, 29);
    assert!(
        rack.precision_controls.is_empty(),
        "seed entry has no phantom panel traversal"
    );
    let edit = rack.editing.as_mut().expect("seed editor");
    assert_eq!(edit.parameter, "seed");
    for invalid in ["0", "16777216", "1.5"] {
        edit.text = invalid.into();
        assert!(parsed_parameter_result(edit).is_err());
    }
    edit.text = "16777215".into();
    assert_eq!(parsed_parameter_result(edit).unwrap(), 16_777_215.0_f32);
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.6,
        vec![key(egui::Key::Enter, true)],
        true,
        true,
        29,
    )
    .0;
    let Some(RackAction::Command("set_parameter", args)) = action else {
        panic!("seed editor did not emit set_parameter");
    };
    assert_eq!(args["parameter"], "seed");
    assert_eq!(args["value"].as_f64(), Some(16_777_215.0));
    assert_eq!(args["expected_revision"], 29);
}

#[test]
fn random_actual_five_lamps_follow_engine_order_and_remain_live_in_the_darkroom() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    rack.set_room_brightness(0.0);
    let lens = |output: &egui::FullOutput, index: usize| {
        let center = pos2(44.0 + index as f32 * 68.0, 90.0);
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Circle(circle)
                    if circle.center.distance(center) < 0.01
                        && (circle.radius - crate::lighting::lamp_radius(2.5)).abs() < 0.01 =>
                {
                    Some(circle.fill)
                }
                _ => None,
            })
            .expect("actual module lamp lens")
    };
    let missing = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1;
    for index in 0..5 {
        let mut state = ModuleLightState {
            count: 5,
            ..Default::default()
        };
        state.brightness[index][0] = if matches!(index, 0 | 2) { 3.0 } else { 1.0 };
        rack.module_lights.insert("random".into(), state);
        let live = frame_output(&ctx, &mut rack, &patch, 1.0 + index as f64, vec![]).1;
        for other in 0..5 {
            if other == index {
                assert_eq!(lens(&live, other), Color32::from_rgb(207, 176, 83));
            } else {
                assert_eq!(lens(&live, other), lens(&missing, other));
            }
        }
    }
    rack.module_lights.clear();
    let later = frame_output(&ctx, &mut rack, &patch, 100.0, vec![]).1;
    for index in 0..5 {
        assert_eq!(lens(&later, index), lens(&missing, index));
    }
}

#[test]
fn random_shared_browser_and_rack_fit_labels_controls_and_jacks_at_all_supported_scales() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "random")
        .unwrap();
    for scale in [0.25, 0.42, 0.72, 1.0, 2.0] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut preview = ModulePreview::new(module);
        let bounds = Rect::from_min_size(pos2(24.0, 12.0), vec2(360.0, PANEL_HEIGHT) * scale);
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 1000.0))),
            ..Default::default()
        };
        let draw_preview = |preview: &mut ModulePreview| {
            ctx.run(input(), |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| preview.show(ui, bounds, &catalog));
            })
        };
        draw_preview(&mut preview);
        let browser = draw_preview(&mut preview);
        let fitted =
            Rect::from_center_size(bounds.center(), preview.size() * preview.renderer.zoom);
        let actual = ctx.run(input(), |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(fitted));
                    child.set_clip_rect(ui.clip_rect().intersect(bounds));
                    let result = preview.renderer.render_modules(
                        &mut child,
                        &preview.patch,
                        &catalog,
                        fitted,
                        48_000,
                        0,
                        RenderMode::Interactive {
                            drag_interactive: false,
                            allow_manual_press: false,
                        },
                    );
                    assert!(result.action.is_none());
                    assert_eq!(result.inputs.len(), 6);
                    assert_eq!(result.outputs.len(), 5);
                    for (_, position) in result.jack_targets {
                        assert!(bounds.contains(position));
                    }
                });
        });
        let drawn = texts(&browser);
        assert_eq!(drawn, texts(&actual), "shared faceplate at {scale}");
        for required in [
            "RANDOM",
            "RATE",
            "PROBABILITY",
            "SPREAD",
            "SHAPE",
            "RATE CV",
            "PROB CV",
            "SPREAD CV",
            "SHAPE CV",
            "RANGE",
            "Bipolar",
            "Offset",
            "TRIG",
            "EXT",
            "STEP",
            "LIN",
            "SMOOTH",
            "EXP",
        ] {
            assert!(
                drawn.iter().any(|(text, _)| text == required),
                "missing {required} at {scale}"
            );
        }
        for (index, (text, rect)) in drawn.iter().enumerate() {
            assert!(
                bounds.expand(0.5).contains_rect(*rect),
                "{scale}: {text} clips"
            );
            assert!(!text.eq_ignore_ascii_case("openpatch"));
            for (other, other_rect) in &drawn[index + 1..] {
                assert!(
                    !rect
                        .shrink(0.2 * scale)
                        .intersects(other_rect.shrink(0.2 * scale)),
                    "{scale}: {text} overlaps {other}"
                );
            }
        }
        assert!(preview.renderer.editing.is_none());
        assert!(preview.renderer.held_manual.is_none());
    }
}
