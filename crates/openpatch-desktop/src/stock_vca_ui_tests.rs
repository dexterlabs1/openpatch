use super::*;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"amplifier","kind":"stock_vca"}]}))
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
    let pos = at(if index == 0 {
        stock_vca_value_rect().center()
    } else {
        stock_vca_mode_rect().center()
    });
    if index == 0 {
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
    } else {
        rack.request_precision_control_for_smoke("amplifier", "response");
        frame_revision_state(&ctx, &mut rack, &patch, 1.1, vec![], true, true, 29);
    }
    assert_eq!(
        rack.editing.as_ref().unwrap().parameter,
        STOCK_VCA_PARAMS[index]
    );
    frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![], true, true, 29);
    (ctx, rack, patch)
}
#[test]
fn stock_vca_direct_entry_accepts_percent_and_integer_response_only() {
    for (index, text, expected) in [
        (0, "75/2%", 0.375),
        (0, "0%", 0.0),
        (0, "100%", 1.0),
        (1, "0", 0.0),
        (1, "1", 1.0),
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
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.4,
            vec![key(egui::Key::Enter, false)],
            true,
            true,
            29,
        )
        .0;
        let Some(RackAction::Command("set_parameter", args)) = action else {
            panic!("direct entry {text}")
        };
        assert_eq!(args["id"], "amplifier");
        assert_eq!(args["parameter"], STOCK_VCA_PARAMS[index]);
        assert_eq!(args["expected_revision"], 29);
        assert!((args["value"].as_f64().unwrap() - expected).abs() < 1e-6);
    }
    let patch = patch();
    for (index, text) in [
        (0, "100.01%"),
        (0, "-0.01"),
        (0, "nan"),
        (0, "inf"),
        (0, "1/0"),
        (1, "0.5"),
        (1, "2"),
        (1, "-1"),
    ] {
        let mut edit = stock_vca_edit(&patch.nodes[0], index, 29);
        edit.text = text.into();
        assert!(parsed_parameter_result(&edit).is_err(), "{text}");
    }
}
#[test]
fn stock_vca_tab_cycles_level_and_response_in_both_directions() {
    for (index, backwards, next) in [(0, false, 1), (1, false, 0), (0, true, 1), (1, true, 0)] {
        let (ctx, mut rack, patch) = open(index);
        assert_eq!(
            rack.precision_controls
                .iter()
                .map(|c| c.edit.parameter.as_str())
                .collect::<Vec<_>>(),
            STOCK_VCA_PARAMS
        );
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.3,
            vec![key(egui::Key::Tab, backwards)],
            true,
            true,
            29,
        )
        .0;
        let Some(RackAction::ParameterEditAndNavigate { arguments, forward }) = action else {
            panic!("Tab {index}")
        };
        assert_eq!(forward, !backwards);
        assert_eq!(arguments["parameter"], STOCK_VCA_PARAMS[index]);
        rack.complete_parameter_navigation(Ok(30), !backwards);
        assert_eq!(
            rack.precision_next.as_ref().unwrap().parameter,
            STOCK_VCA_PARAMS[next]
        );
    }
}
#[test]
fn stock_vca_actual_pointer_drag_and_mode_click_use_observed_revision() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let start = at(stock_vca_slider_rect().center());
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
    let end = start - vec2(0.0, 30.0);
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![egui::Event::PointerMoved(end)],
        true,
        true,
        29,
    )
    .0;
    let Some(RackAction::Command("set_parameter", args)) = action else {
        panic!("vertical drag")
    };
    assert_eq!(args["parameter"], "level");
    assert_eq!(args["expected_revision"], 29);
    assert!(args["history_group"]
        .as_str()
        .unwrap()
        .starts_with("rack-stock-vca-slider-"));
    assert!((args["value"].as_f64().unwrap() - (0.5 + 30.0 / 164.0)).abs() < 1e-6);
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.2,
        vec![button(end, false)],
        true,
        true,
        29,
    );
    for (index, offset) in [(0, 20.0), (1, 62.0)] {
        let pos = at(stock_vca_mode_rect().left_top() + vec2(offset, 11.0));
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            2.0 + index as f64,
            vec![egui::Event::PointerMoved(pos), button(pos, true)],
            true,
            true,
            30,
        );
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            2.1 + index as f64,
            vec![button(pos, false)],
            true,
            true,
            30,
        )
        .0;
        let Some(RackAction::Command("set_parameter", args)) = action else {
            panic!("mode click {index}")
        };
        assert_eq!(args["parameter"], "response");
        assert_eq!(args["value"], index);
        assert_eq!(args["expected_revision"], 30);
    }
}
#[test]
fn stock_vca_controls_lanes_and_ports_fit_without_overlap() {
    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(panel_width("stock_vca"), PANEL_HEIGHT));
    let mut controls = vec![
        stock_vca_slider_rect(),
        stock_vca_value_rect(),
        stock_vca_mode_rect(),
    ];
    controls.extend((0..16).map(stock_vca_gain_rect));
    for (i, a) in controls.iter().enumerate() {
        assert!(bounds.contains_rect(*a));
        for b in &controls[i + 1..] {
            assert!(!a.intersects(*b));
        }
    }
    let ports = (0..2)
        .map(|i| port_position("stock_vca", true, i, 2, bounds.width()))
        .chain([port_position("stock_vca", false, 0, 1, bounds.width())])
        .collect::<Vec<_>>();
    for (i, p) in ports.iter().enumerate() {
        assert!(bounds.shrink(14.0).contains(*p));
        for c in &controls {
            assert!(!c.expand(18.0).contains(*p));
        }
        for q in &ports[i + 1..] {
            assert!(p.distance(*q) > 36.0);
        }
    }
}
fn texts(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Text(t) => out.push((
            t.galley.text().into(),
            t.galley.rect.translate(t.pos.to_vec2()),
        )),
        egui::Shape::Vec(shapes) => {
            for s in shapes {
                texts(s, out)
            }
        }
        _ => {}
    }
}
#[test]
fn stock_vca_rack_browser_fit_and_shared_primitive_parity_at_all_scales() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "stock_vca")
        .unwrap();
    for scale in [0.42, 0.85, 1.0] {
        let mut surfaces = vec![];
        for browser in [false, true] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut rack = Rack {
                zoom: scale,
                ..Rack::default()
            };
            let mut preview = ModulePreview::new(module);
            let bounds = Rect::from_min_size(
                pos2(24.0, 12.0),
                vec2(panel_width("stock_vca"), PANEL_HEIGHT) * scale,
            );
            let mut drawn = vec![];
            let mut primitives = vec![];
            for _ in 0..2 {
                let output = if browser {
                    ctx.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default()
                                .frame(egui::Frame::NONE)
                                .show(ctx, |ui| preview.show(ui, bounds, &catalog));
                        },
                    )
                } else {
                    frame_output(&ctx, &mut rack, &patch(), 0.0, vec![]).1
                };
                drawn.clear();
                primitives.clear();
                // Match the full faceplate primitive stream (rack framing is outside bounds).
                fn primitives_in(shape: &egui::Shape, bounds: Rect, result: &mut Vec<String>) {
                    match shape {
                        egui::Shape::Vec(shapes) => {
                            for shape in shapes {
                                primitives_in(shape, bounds, result)
                            }
                        }
                        egui::Shape::Text(t) if bounds.expand(0.5).contains(t.pos) => result.push(
                            format!("text:{:?}:{}:{:?}", t.pos, t.galley.text(), t.galley.size()),
                        ),
                        egui::Shape::Rect(r) if bounds.contains_rect(r.rect) => {
                            result.push(format!("rect:{r:?}"))
                        }
                        egui::Shape::Circle(c) if bounds.contains(c.center) => {
                            result.push(format!("circle:{c:?}"))
                        }
                        egui::Shape::LineSegment { points, stroke }
                            if points.iter().all(|p| bounds.contains(*p)) =>
                        {
                            result.push(format!("line:{points:?}:{stroke:?}"))
                        }
                        _ => {}
                    }
                }
                for s in output.shapes {
                    texts(&s.shape, &mut drawn);
                    primitives_in(&s.shape, bounds, &mut primitives);
                }
            }
            for label in [
                "VCA",
                "POLYPHONIC GAIN",
                "LEVEL",
                "GAIN / VOICE",
                "0 — 100%",
                "100%",
                "EXP",
                "LIN",
                "— TELEMETRY OFFLINE",
                "CV",
                "IN",
                "OUT",
            ] {
                assert!(
                    drawn.iter().any(|(s, _)| s == label),
                    "{scale} browser={browser} missing {label}"
                );
            }
            for (i, (text, a)) in drawn.iter().enumerate() {
                assert!(
                    bounds.expand(0.5).contains_rect(*a),
                    "{scale} {text} outside {bounds:?}: {a:?}"
                );
                for (other, b) in &drawn[i + 1..] {
                    assert!(
                        !a.shrink(0.2 * scale).intersects(b.shrink(0.2 * scale)),
                        "{scale}: {text} overlaps {other}"
                    );
                }
            }
            surfaces.push(primitives);
            assert!(preview.renderer.editing.is_none());
        }
        assert_eq!(
            surfaces[0], surfaces[1],
            "rack/browser primitive mismatch at {scale}"
        );
    }
}
#[test]
fn stock_vca_gain_columns_use_active_telemetry_and_remain_emissive_in_darkness() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch();
    let mut lights = ModuleLightState {
        count: 16,
        input_channels: Some(5),
        ..Default::default()
    };
    for (i, gain) in [0.0, 0.0625, 0.5, 1.0, 1.0].iter().enumerate() {
        lights.brightness[i][0] = *gain;
    }
    rack.module_lights.insert("amplifier".into(), lights);
    let accent = Color32::from_rgb(86, 181, 137);
    for room in [1.0, 0.0] {
        rack.set_room_brightness(room);
        let (_, output) = frame_output(&ctx, &mut rack, &patch, 1.0, vec![]);
        let mut fills = vec![];
        fn gain_fills(s: &egui::Shape, accent: Color32, out: &mut Vec<Rect>) {
            match s {
                egui::Shape::Vec(ss) => {
                    for s in ss {
                        gain_fills(s, accent, out)
                    }
                }
                egui::Shape::Rect(r) if r.fill == accent => out.push(r.rect),
                _ => {}
            }
        }
        for shape in output.shapes {
            gain_fills(&shape.shape, accent, &mut fills);
        }
        assert_eq!(
            fills.len(),
            4,
            "exactly four positive gain columns at room={room}"
        );
        for (rect, index) in fills.iter().zip(1..5) {
            let expected = stock_vca_gain_rect(index);
            assert!((rect.height() - expected.height() * lights.brightness[index][0]).abs() < 1e-4);
        }
    }
    rack.module_lights.clear();
    let (_, output) = frame_output(&ctx, &mut rack, &patch, 2.0, vec![]);
    let mut drawn = vec![];
    for s in output.shapes {
        texts(&s.shape, &mut drawn);
    }
    assert!(drawn.iter().any(|(s, _)| s == "— TELEMETRY OFFLINE"));
}

#[test]
fn stock_vca_held_slider_keeps_node_and_history_identity_across_graph_edits() {
    for change in ["remove", "insert", "reorder"] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"earlier","kind":"stock_vca"},
            {"id":"held","kind":"stock_vca"}
        ]}))
        .unwrap();
        rack.positions.insert("earlier".into(), pos2(0.0, 0.0));
        rack.positions.insert("held".into(), pos2(330.0, 0.0));
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
        let start = at(stock_vca_slider_rect().center() + vec2(330.0, 0.0));
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
                rack.positions.insert("inserted".into(), pos2(660.0, 0.0));
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
        assert_eq!(
            after["parameter"], "level",
            "wrong parameter after {change}"
        );
        assert_eq!(after["expected_revision"], 30);
        assert_eq!(
            after["history_group"], history,
            "gesture history changed after {change}"
        );
        assert!(after["value"].as_f64().unwrap() > before["value"].as_f64().unwrap());
    }
}

#[test]
fn stock_vca_deleted_held_slider_never_transfers_drag_to_remaining_filter() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"held","kind":"stock_vca"},
        {"id":"remaining","kind":"stock_vca"}
    ]}))
    .unwrap();
    rack.positions.insert("held".into(), pos2(0.0, 0.0));
    rack.positions.insert("remaining".into(), pos2(330.0, 0.0));
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let start = at(stock_vca_slider_rect().center());
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
        matches!(before, Some(RackAction::Command("set_parameter", args)) if args["id"] == "held" && args["parameter"] == "level")
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
fn stock_vca_response_is_editable_from_actual_header_and_body_menus() {
    for menu_position in [pos2(90.0, 44.0), pos2(90.0, 400.0)] {
        for (label, response) in [("Exponential (fourth power)", 0), ("Linear", 1)] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch = patch();
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
            for (time, pressed) in [(1.0, true), (1.1, false)] {
                frame_revision_state(
                    &ctx,
                    &mut rack,
                    &patch,
                    time,
                    vec![
                        egui::Event::PointerMoved(menu_position),
                        egui::Event::PointerButton {
                            pos: menu_position,
                            button: PointerButton::Secondary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    true,
                    true,
                    29,
                );
            }
            let (_, output) =
                frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![], true, true, 29);
            let mut drawn = vec![];
            for shape in output.shapes {
                texts(&shape.shape, &mut drawn);
            }
            let target = drawn
                .iter()
                .find(|(text, _)| text == label)
                .unwrap_or_else(|| panic!("Missing {label} at {menu_position:?}"))
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
            let action = frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.4,
                vec![button(target, false)],
                true,
                true,
                29,
            )
            .0;
            let Some(RackAction::Command("set_parameter", args)) = action else {
                panic!("Menu click {label} at {menu_position:?}")
            };
            assert_eq!(args["id"], "amplifier");
            assert_eq!(args["parameter"], "response");
            assert_eq!(args["value"], response);
        }
    }
}

#[test]
fn stock_vca_bypassed_unavailable_meter_remains_explicitly_offline() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = patch();
    patch.nodes[0].bypassed = true;
    // Both missing and malformed publications are unavailable, including bypass.
    for lights in [
        None,
        Some(ModuleLightState {
            count: 16,
            input_channels: None,
            brightness: [[1.0, 0.0]; 16],
            ..Default::default()
        }),
    ] {
        rack.module_lights.clear();
        if let Some(lights) = lights {
            rack.module_lights.insert("amplifier".into(), lights);
        }
        for room in [1.0, 0.0] {
            rack.set_room_brightness(room);
            let (_, output) = frame_output(&ctx, &mut rack, &patch, 1.0, vec![]);
            let mut drawn = vec![];
            for shape in output.shapes {
                texts(&shape.shape, &mut drawn);
            }
            assert!(drawn.iter().any(|(text, _)| text == "BYPASS · — OFFLINE"));
            assert!(!drawn.iter().any(|(text, _)| text == "BYPASS · GAIN FROZEN"));
        }
    }
    rack.module_lights.insert(
        "amplifier".into(),
        ModuleLightState {
            count: 16,
            input_channels: Some(1),
            ..Default::default()
        },
    );
    let (_, output) = frame_output(&ctx, &mut rack, &patch, 2.0, vec![]);
    let mut drawn = vec![];
    for shape in output.shapes {
        texts(&shape.shape, &mut drawn);
    }
    assert!(drawn.iter().any(|(text, _)| text == "BYPASS · GAIN FROZEN"));
}
