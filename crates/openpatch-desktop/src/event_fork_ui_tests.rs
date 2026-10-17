use super::*;

fn fork_patch(params: Value, cables: Value) -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"fork","kind":"event_fork","params":params}],"cables":cables})).unwrap()
}
fn fork_text(shape: &egui::Shape, texts: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => children.iter().for_each(|s| fork_text(s, texts)),
        egui::Shape::Text(text) => texts.push((
            text.galley.text().into(),
            text.galley.rect.translate(text.pos.to_vec2()),
        )),
        _ => {}
    }
}
#[test]
fn event_fork_shared_preview_keeps_controls_ports_print_and_seated_screws_clear_at_five_scales() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "event_fork")
        .unwrap();
    assert_eq!(panel_width("event_fork"), 300.0);
    assert_eq!(PANEL_HEIGHT, 396.0);
    assert_eq!(
        module["inputs"],
        json!(["gate1", "gate2", "probability1_cv", "probability2_cv"])
    );
    assert_eq!(module["outputs"], json!(["a1", "b1", "a2", "b2"]));
    for name in ["probability1", "probability2"] {
        assert_eq!(module["params"][name]["default"], 0.5);
    }
    for zoom in [0.25, 0.42, 0.85, 1.0, 1.4] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut preview = ModulePreview::new(module);
        let bounds = Rect::from_min_size(pos2(24.0, 12.0), preview.size() * zoom);
        let mut output = None;
        for _ in 0..2 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .show(ctx, |ui| preview.show(ui, bounds, &catalog));
                },
            ));
        }
        let mut texts = Vec::new();
        for shape in &output.unwrap().shapes {
            fork_text(&shape.shape, &mut texts);
        }
        for expected in [
            "EVENT FORK",
            "CHANNEL 1",
            "CHANNEL 2",
            "PROBABILITY",
            "50.0%",
            "TOGGLE",
            "LATCH",
            "DIRECT",
            "GATED",
            "SEED",
            "G2 ← G1 NORMAL",
            "GATE 1",
            "GATE 2",
            "P CV",
            "A1",
            "B1",
            "A2",
            "B2",
        ] {
            assert!(
                texts.iter().any(|(text, _)| text == expected),
                "{zoom}: missing {expected}"
            );
        }
        for (text, printed) in &texts {
            assert!(
                bounds.expand(0.5).contains_rect(*printed),
                "{zoom}: {text} clips {printed:?}"
            );
        }
        let local_targets = ["probability1", "probability2"]
            .into_iter()
            .flat_map(|name| {
                let (center, radius) = event_fork_control(name);
                let GenericParameterHardware::Rotary { target_half, .. } =
                    generic_parameter_hardware("event_fork", name, radius, false)
                else {
                    panic!("probability must use a rotary")
                };
                [
                    Rect::from_center_size(center, Vec2::splat(target_half * 2.0)),
                    Rect::from_center_size(event_fork_value_center(name), vec2(100.0, 19.0)),
                ]
            })
            .chain(
                ["toggle1", "toggle2", "latch1", "latch2"]
                    .into_iter()
                    .map(|n| event_fork_selector(n).unwrap().0),
            )
            .chain([event_fork_seed_rect()])
            .collect::<Vec<_>>();
        let targets = local_targets
            .into_iter()
            .map(|r| Rect::from_min_size(bounds.min + r.min.to_vec2() * zoom, r.size() * zoom))
            .collect::<Vec<_>>();
        let screws = [20.0, 400.0]
            .into_iter()
            .flat_map(|x| {
                [21.0, 699.0]
                    .map(|y| bounds.min + vec2(x * 300.0 / 420.0, y * PANEL_HEIGHT / 720.0) * zoom)
            })
            .collect::<Vec<_>>();
        for (i, target) in targets.iter().enumerate() {
            assert!(bounds.contains_rect(*target));
            for other in &targets[i + 1..] {
                assert!(
                    !target.intersects(*other),
                    "{zoom}: control targets overlap"
                );
            }
            for screw in &screws {
                assert!(
                    !target.intersects(Rect::from_center_size(*screw, Vec2::splat(9.8 * zoom))),
                    "{zoom}: control covers a screw"
                );
            }
        }
        let mut rims = Vec::new();
        for input in [true, false] {
            for index in 0..4 {
                let center = bounds.min
                    + port_position("event_fork", input, index, 4, 300.0).to_vec2() * zoom;
                let radius =
                    if input { 10.3 } else { 13.0 } * port_material_scale("event_fork") * zoom;
                let rim = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
                assert!(bounds.contains_rect(rim), "{zoom}: port clips");
                for target in &targets {
                    assert!(!target.intersects(rim), "{zoom}: control overlaps a port");
                }
                for (text, printed) in &texts {
                    assert!(
                        !printed.intersects(rim),
                        "{zoom}: {text} overlaps {input}/{index}"
                    );
                }
                for screw in &screws {
                    assert!(
                        center.distance(*screw) > radius + 4.9 * zoom,
                        "{zoom}: port covers a seated screw"
                    );
                }
                rims.push(rim);
            }
        }
        for (i, rim) in rims.iter().enumerate() {
            for other in &rims[i + 1..] {
                assert!(!rim.intersects(*other), "{zoom}: ports overlap");
            }
        }
    }
}
#[test]
fn event_fork_probability_drag_is_one_native_undo_gesture() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let patch = fork_patch(json!({}), json!([]));
    let mut rack = Rack::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let from = pos2(24.0, 12.0) + event_fork_control("probability2").0.to_vec2();
    let press = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(from), button(from, true)],
        true,
        true,
        17,
    )
    .0;
    assert!(press.is_none(), "press does not jump");
    let mut groups = Vec::new();
    for (time, distance) in [(1.1, 20.0), (1.2, 35.0)] {
        let Some(RackAction::Command(command, args)) = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            time,
            vec![egui::Event::PointerMoved(from - vec2(0.0, distance))],
            true,
            true,
            17,
        )
        .0
        else {
            panic!("drag must issue native parameter edit")
        };
        assert_eq!(command, "set_parameter");
        assert_eq!(args["id"], "fork");
        assert_eq!(args["parameter"], "probability2");
        assert_eq!(args["expected_revision"], 17);
        assert!(args["value"].as_f64().unwrap() > 0.5);
        groups.push(args["history_group"].as_str().unwrap().to_owned());
    }
    assert_eq!(groups[0], groups[1]);
    frame(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![button(from - vec2(0.0, 35.0), false)],
    );
}
#[test]
fn event_fork_normal_guide_follows_physical_gate2_cable_and_toggle_guide_follows_document() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    for (cables, params, normal, choice) in [
        (json!([]), json!({}), "G2 ← G1 NORMAL", "A ← CHOICE → B"),
        (
            json!([{"from":"source","output":"out","to":"fork","input":"gate2"}]),
            json!({"toggle1":1}),
            "G2 INDEPENDENT",
            "A STAY · B SWITCH",
        ),
    ] {
        let patch = fork_patch(params, cables);
        assert_eq!(
            event_fork_gate2_normalled(&patch.nodes[0], &patch),
            patch.cables.is_empty()
        );
        let mut rack = Rack::default();
        let (_, output) = frame_revision_state(&ctx, &mut rack, &patch, 1.0, vec![], true, true, 4);
        let mut texts = Vec::new();
        for shape in &output.shapes {
            fork_text(&shape.shape, &mut texts);
        }
        assert!(texts.iter().any(|(text, _)| text == normal));
        assert!(texts.iter().any(|(text, _)| text == choice));
        assert!(rack.module_lights.is_empty());
    }
}
#[test]
fn event_fork_four_lamps_follow_actual_output_component_and_stay_dark_without_telemetry() {
    let ctx = egui::Context::default();
    let patch = fork_patch(json!({}), json!([]));
    let node = &patch.nodes[0];
    let mut rack = Rack::default();
    let lenses = |rack: &Rack| {
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for index in 0..4 {
                    rack.paint_output_signal_light(
                        ui.painter(),
                        node,
                        index,
                        event_fork_port(false, index),
                    );
                }
            });
        });
        output
            .shapes
            .into_iter()
            .filter_map(|s| match s.shape {
                egui::Shape::Circle(c)
                    if (c.radius - crate::lighting::lamp_radius(2.2)).abs() < 0.01 =>
                {
                    Some((c.center, c.fill))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let dark = lenses(&rack);
    assert_eq!(dark.len(), 4);
    let mut state = ModuleLightState {
        count: 4,
        ..Default::default()
    };
    state.brightness = std::array::from_fn(|index| {
        if index == 1 || index == 2 {
            [0.8, 0.0]
        } else {
            [0.0, 1.0]
        }
    });
    rack.module_lights.insert(node.id.clone(), state);
    let active = lenses(&rack);
    for index in 0..4 {
        assert_eq!(active[index].0, dark[index].0);
        assert_eq!(
            active[index].1 == dark[index].1,
            index == 0 || index == 3,
            "only actual output volts light {index}"
        );
    }
    rack.module_lights.get_mut(&node.id).unwrap().count = 0;
    assert_eq!(
        lenses(&rack),
        dark,
        "unpublished state must not illuminate outputs"
    );
}

#[test]
fn event_fork_seed_uses_guarded_shared_precision_entry() {
    let ctx = egui::Context::default();
    let patch = fork_patch(json!({}), json!([]));
    let mut rack = Rack::default();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 11);
    let at = event_fork_seed_rect().center() + vec2(24.0, 12.0);
    for (time, pressed) in [(1.0, true), (1.1, false)] {
        assert!(frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            time,
            vec![egui::Event::PointerMoved(at), button(at, pressed)],
            true,
            true,
            11
        )
        .0
        .is_none());
    }
    let edit = rack
        .editing
        .as_mut()
        .expect("seed opens shared exact entry");
    assert_eq!(edit.kind, "event_fork");
    assert_eq!(edit.parameter, "seed");
    assert_eq!(edit.revision, 11);
    assert_eq!(edit.step, Some(1.0));
    edit.text = "16777215".into();
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
        11,
    )
    .0;
    assert!(
        matches!(action, Some(RackAction::Command("set_parameter", args)) if args["parameter"] == "seed" && args["value"] == 16777215.0 && args["expected_revision"] == 11)
    );
}
