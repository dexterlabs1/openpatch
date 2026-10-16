use super::*;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"echo","kind":"stock_delay"}]}))
        .unwrap()
}
fn key(key: egui::Key, shift: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: if shift {
            egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::NONE
        },
    }
}
fn at(local: Pos2) -> Pos2 {
    local + vec2(24.0, 12.0)
}
fn open(index: usize) -> (egui::Context, Rack, Patch) {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let pos = at(stock_delay_value_rect(index).center());
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(pos), button(pos, true)],
        true,
        true,
        29,
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(pos, false)],
        true,
        true,
        29,
    );
    assert_eq!(
        rack.editing.as_ref().unwrap().parameter,
        STOCK_DELAY_PARAMS[index]
    );
    frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![], true, true, 29);
    (ctx, rack, patch)
}
#[test]
fn stock_delay_time_percent_and_signed_tone_entry_round_trip_with_bounds() {
    let patch = patch();
    for (index, text, expected) in [
        (0, "1 ms", 0.0),
        (0, "10 s", 1.0),
        (0, "0.5 s", 500.0_f32.log10() / 4.0),
        (0, "1000/4 ms", 250.0_f32.log10() / 4.0),
        (1, "35%", 0.35),
        (2, "-100%", 0.0),
        (2, "0%", 0.5),
        (2, "100%", 1.0),
        (3, "73%", 0.73),
        (4, "-60%", -0.6),
        (5, "80%", 0.8),
        (6, "-25%", -0.25),
        (7, "100%", 1.0),
    ] {
        let mut edit = stock_delay_edit(&patch.nodes[0], index, 29);
        edit.text = text.into();
        let raw = parameter_edit_raw_value(&edit, parsed_parameter_result(&edit).unwrap());
        assert!((raw - expected).abs() < 1e-6, "{index}: {text} => {raw}");
    }
    for index in 0..8 {
        let mut edit = stock_delay_edit(&patch.nodes[0], index, 29);
        for text in [
            "nan",
            "inf",
            "1/0",
            "100000",
            if index == 0 {
                "0.999 ms"
            } else if index == 2 || index >= 4 {
                "-100.01%"
            } else {
                "-0.01%"
            },
        ] {
            edit.text = text.into();
            assert!(parsed_parameter_result(&edit).is_err(), "{index}: {text}");
        }
    }
    let mut edit = stock_delay_edit(&patch.nodes[0], 0, 29);
    edit.text = "10.001 s".into();
    assert!(parsed_parameter_result(&edit).is_err());
}
#[test]
fn stock_delay_all_eight_controls_drag_with_revision_and_shared_undo_gesture() {
    for (index, name) in STOCK_DELAY_PARAMS.iter().enumerate() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = patch();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
        let start = at(stock_delay_slider_rect(index).center());
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(start), button(start, true)],
            true,
            true,
            29,
        );
        let end = start
            + if index < 4 {
                vec2(0.0, -25.0)
            } else {
                vec2(-20.0, 0.0)
            };
        let Some(RackAction::Command("set_parameter", first)) = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![egui::Event::PointerMoved(end)],
            true,
            true,
            29,
        )
        .0
        else {
            panic!("no drag for {name}")
        };
        let next = end
            + if index < 4 {
                vec2(0.0, -8.0)
            } else {
                vec2(5.0, 0.0)
            };
        let Some(RackAction::Command("set_parameter", second)) = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![egui::Event::PointerMoved(next)],
            true,
            true,
            30,
        )
        .0
        else {
            panic!("no continuation for {name}")
        };
        assert_eq!(first["parameter"], *name);
        assert_eq!(first["id"], "echo");
        assert_eq!(first["expected_revision"], 29);
        assert_eq!(second["expected_revision"], 30);
        assert!(first["history_group"]
            .as_str()
            .unwrap()
            .starts_with("rack-stock-delay-slider-"));
        assert_eq!(first["history_group"], second["history_group"]);
        let value = first["value"].as_f64().unwrap();
        assert!(
            if index < 4 { value > 0.5 } else { value < 0.0 },
            "{name}: {value}"
        );
    }
}
#[test]
fn stock_delay_readout_pointer_entry_applies_units_and_tab_visits_all_eight_controls() {
    for (index, text, expected) in [
        (0, "0.25 s", 250.0_f32.log10() / 4.0),
        (1, "73%", 0.73),
        (2, "-50%", 0.25),
        (3, "40%", 0.4),
        (4, "-60%", -0.6),
        (5, "80%", 0.8),
        (6, "-25%", -0.25),
        (7, "100%", 1.0),
    ] {
        let (ctx, mut rack, patch) = open(index);
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.3,
            vec![egui::Event::Text(text.into())],
            true,
            true,
            29,
        );
        let Some(RackAction::Command("set_parameter", args)) = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.4,
            vec![key(egui::Key::Enter, false)],
            true,
            true,
            29,
        )
        .0
        else {
            panic!("entry {index}")
        };
        assert_eq!(args["parameter"], STOCK_DELAY_PARAMS[index]);
        assert_eq!(args["expected_revision"], 29);
        assert!((args["value"].as_f64().unwrap() - f64::from(expected)).abs() < 1e-6);
    }
    for (index, backwards, next) in [(0, false, 1), (3, false, 4), (7, false, 0), (0, true, 7)] {
        let (ctx, mut rack, patch) = open(index);
        assert_eq!(
            rack.precision_controls
                .iter()
                .map(|control| control.edit.parameter.as_str())
                .collect::<Vec<_>>(),
            STOCK_DELAY_PARAMS
        );
        let Some(RackAction::ParameterEditAndNavigate { arguments, forward }) =
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.3,
                vec![key(egui::Key::Tab, backwards)],
                true,
                true,
                29,
            )
            .0
        else {
            panic!("Tab {index}")
        };
        assert_eq!(forward, !backwards);
        assert_eq!(arguments["parameter"], STOCK_DELAY_PARAMS[index]);
        rack.complete_parameter_navigation(Ok(30), !backwards);
        assert_eq!(
            rack.precision_next.as_ref().unwrap().parameter,
            STOCK_DELAY_PARAMS[next]
        );
    }
}
#[test]
fn stock_delay_controls_and_eight_jacks_fit_without_overlapping_targets() {
    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(panel_width("stock_delay"), PANEL_HEIGHT));
    assert_eq!(bounds.width(), 360.0);
    let controls = (0..8)
        .flat_map(|index| {
            [
                stock_delay_slider_rect(index),
                stock_delay_value_rect(index),
            ]
        })
        .collect::<Vec<_>>();
    for (index, control) in controls.iter().enumerate() {
        assert!(bounds.contains_rect(*control));
        for other in &controls[index + 1..] {
            assert!(!control.intersects(*other));
        }
    }
    let ports = (0..6)
        .map(|index| port_position("stock_delay", true, index, 6, bounds.width()))
        .chain((0..2).map(|index| port_position("stock_delay", false, index, 2, bounds.width())))
        .collect::<Vec<_>>();
    for (index, port) in ports.iter().enumerate() {
        assert!(bounds.shrink(14.0).contains(*port));
        for control in &controls {
            assert!(!control.expand(14.0).contains(*port));
        }
        for other in &ports[index + 1..] {
            assert!(port.distance(*other) > 32.0);
        }
    }
}
fn texts(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                texts(child, out);
            }
        }
        egui::Shape::Text(text) if !text.galley.text().is_empty() => out.push((
            text.galley.text().into(),
            Rect::from_min_size(text.pos, text.galley.size()),
        )),
        _ => {}
    }
}
#[test]
fn stock_delay_shared_browser_and_rack_labels_fit_at_zoom_and_200_percent_preview() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "stock_delay")
        .unwrap();
    for scale in [0.42, 0.85, 1.0, 2.0] {
        let mut surfaces = vec![];
        for browser in [false, true] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut rack = Rack {
                zoom: scale,
                ..Rack::default()
            };
            let mut preview = ModulePreview::new(module);
            let bounds = Rect::from_min_size(pos2(24.0, 12.0), vec2(360.0, PANEL_HEIGHT) * scale);
            let mut drawn = vec![];
            for _ in 0..2 {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 1000.0))),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE)
                            .show(ctx, |ui| {
                                if browser {
                                    preview.show(ui, bounds, &catalog);
                                } else {
                                    rack.show(ui, &patch(), &catalog, 48000, 29);
                                }
                            });
                    },
                );
                drawn.clear();
                for shape in output.shapes {
                    texts(&shape.shape, &mut drawn);
                }
            }
            for required in [
                "DELAY",
                "TIME",
                "FEEDBACK",
                "TONE",
                "MIX",
                "CV DEPTH",
                "TIME CV",
                "FDBK CV",
                "TONE CV",
                "MIX CV",
                "IN",
                "CLK",
                "WET",
                "500.0 ms",
                "50%",
                "+0%",
            ] {
                assert!(
                    drawn.iter().any(|(label, _)| label == required),
                    "{scale} browser={browser} missing {required}"
                );
            }
            for (index, (label, rect)) in drawn.iter().enumerate() {
                assert!(
                    bounds.expand(0.5).contains_rect(*rect),
                    "{scale}: {label} outside {bounds:?}: {rect:?}"
                );
                for (other, other_rect) in &drawn[index + 1..] {
                    assert!(
                        !rect
                            .shrink(0.2 * scale)
                            .intersects(other_rect.shrink(0.2 * scale)),
                        "{scale}: {label} overlaps {other}"
                    );
                }
            }
            assert!(preview.renderer.editing.is_none());
            surfaces.push(drawn);
        }
        assert_eq!(surfaces[0].len(), surfaces[1].len());
        for ((a, ar), (b, br)) in surfaces[0].iter().zip(&surfaces[1]) {
            assert_eq!(a, b);
            assert!(
                ar.min.distance(br.min) < 0.01 && ar.max.distance(br.max) < 0.01,
                "{scale} rack/browser {a}"
            );
        }
    }
}

#[test]
fn stock_delay_held_slider_keeps_node_and_history_identity_across_graph_edits() {
    for change in ["remove", "insert", "reorder"] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"earlier","kind":"stock_delay"},
            {"id":"held","kind":"stock_delay"}
        ]}))
        .unwrap();
        rack.positions.insert("earlier".into(), pos2(0.0, 0.0));
        rack.positions.insert("held".into(), pos2(390.0, 0.0));
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
        let start = at(stock_delay_slider_rect(0).center() + vec2(390.0, 0.0));
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(start), button(start, true)],
            true,
            true,
            29,
        );
        let moved = start + vec2(0.0, -15.0);
        let before = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![egui::Event::PointerMoved(moved)],
            true,
            true,
            29,
        )
        .0;
        let Some(RackAction::Command("set_parameter", before)) = before else {
            panic!("initial drag {change}")
        };
        assert_eq!(before["id"], "held");
        let history = before["history_group"].clone();
        assert!(history.is_string());
        match change {
            "remove" => {
                patch.nodes.remove(0);
            }
            "insert" => {
                let mut inserted = patch.nodes[0].clone();
                inserted.id = "inserted".into();
                patch.nodes.insert(0, inserted);
                rack.positions.insert("inserted".into(), pos2(780.0, 0.0));
            }
            "reorder" => patch.nodes.swap(0, 1),
            _ => unreachable!(),
        }
        let after = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![egui::Event::PointerMoved(moved + vec2(0.0, -15.0))],
            true,
            true,
            30,
        )
        .0;
        let Some(RackAction::Command("set_parameter", after)) = after else {
            panic!("held slider lost after {change}")
        };
        assert_eq!(
            after["id"], "held",
            "active slider transferred after {change}"
        );
        assert_eq!(after["parameter"], "time", "wrong parameter after {change}");
        assert_eq!(after["expected_revision"], 30);
        assert_eq!(
            after["history_group"], history,
            "gesture history changed after {change}"
        );
        assert!(after["value"].as_f64().unwrap() > before["value"].as_f64().unwrap());
    }
}

#[test]
fn stock_delay_deleted_held_slider_never_transfers_drag_to_remaining_filter() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"held","kind":"stock_delay"},
        {"id":"remaining","kind":"stock_delay"}
    ]}))
    .unwrap();
    rack.positions.insert("held".into(), pos2(0.0, 0.0));
    rack.positions.insert("remaining".into(), pos2(390.0, 0.0));
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let start = at(stock_delay_slider_rect(0).center());
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(start), button(start, true)],
        true,
        true,
        29,
    );
    let before = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![egui::Event::PointerMoved(start + vec2(0.0, -15.0))],
        true,
        true,
        29,
    )
    .0;
    assert!(
        matches!(before, Some(RackAction::Command("set_parameter", args)) if args["id"] == "held" && args["parameter"] == "time")
    );
    patch.nodes.remove(0);
    let after = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.2,
        vec![egui::Event::PointerMoved(start + vec2(0.0, -30.0))],
        true,
        true,
        30,
    )
    .0;
    assert!(
        after.is_none(),
        "a deleted slider's drag must not edit the remaining node"
    );
    let released = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![button(start + vec2(0.0, -30.0), false)],
        true,
        true,
        30,
    )
    .0;
    assert!(released.is_none());
}

#[test]
fn stock_delay_clock_lamp_uses_actual_dsp_and_emits_with_room_lights_off() {
    let patch = patch();
    let mut engine = patch.compile().unwrap();
    engine.process_block(&mut vec![0.0; 25000]);
    let lights = engine
        .module_lights(engine.resolve_module_lights("echo").unwrap())
        .unwrap();
    assert_eq!(lights.count, 1);
    assert!(lights.brightness[0][0] > 0.1);
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    rack.set_module_states(BTreeMap::from([("echo".into(), lights)]), BTreeMap::new());
    fn emitted_core(shape: &egui::Shape, center: Pos2) -> usize {
        match shape {
            egui::Shape::Vec(shapes) => {
                shapes.iter().map(|shape| emitted_core(shape, center)).sum()
            }
            egui::Shape::Circle(circle)
                if circle.center.distance(center) < 0.01 && (circle.radius - 1.98).abs() < 0.01 =>
            {
                1
            }
            _ => 0,
        }
    }
    for room in [1.0, 0.0] {
        rack.set_room_brightness(room);
        let (_, output) = frame_output(&ctx, &mut rack, &patch, 1.0, vec![]);
        assert_eq!(
            output
                .shapes
                .iter()
                .map(|shape| emitted_core(&shape.shape, at(pos2(154.0, 366.0))))
                .sum::<usize>(),
            1,
            "room={room}"
        );
    }
    rack.set_module_states(BTreeMap::new(), BTreeMap::new());
    let (_, output) = frame_output(&ctx, &mut rack, &patch, 2.0, vec![]);
    assert_eq!(
        output
            .shapes
            .iter()
            .map(|shape| emitted_core(&shape.shape, at(pos2(154.0, 366.0))))
            .sum::<usize>(),
        0,
        "Saved time and connected jacks must not invent a clock light"
    );
}
