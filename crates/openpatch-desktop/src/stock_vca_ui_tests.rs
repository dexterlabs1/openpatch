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
    let mut patch = patch();
    patch.nodes[0].params.insert("level".into(), 0.5);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let start = at(stock_vca_knob_rect().center());
    let pressed = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(start), button(start, true)],
        true,
        true,
        29,
    ).0;
    assert!(pressed.is_none(), "pressing a saved level knob must not jump");
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
        panic!("relative rotary drag")
    };
    assert_eq!(args["parameter"], "level");
    assert_eq!(args["expected_revision"], 29);
    assert!(args["history_group"]
        .as_str()
        .unwrap()
        .starts_with("rack-fader-"));
    assert!((args["value"].as_f64().unwrap() - 0.7).abs() < 1e-6);
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
fn stock_vca_rotary_fine_drag_returns_to_normal_speed_without_a_jump() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = patch();
    patch.nodes[0].params.insert("level".into(), 0.5);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let center = at(stock_vca_knob_rect().center());
    let fine = egui::Modifiers { ctrl: true, ..Default::default() };
    let pressed = frame_revision_state(&ctx, &mut rack, &patch, 1.0,
        vec![egui::Event::PointerMoved(center), egui::Event::PointerButton {
            pos:center, button:PointerButton::Primary, pressed:true, modifiers:fine,
        }], true, true, 29).0;
    assert!(pressed.is_none());
    let carrier = |modifiers| egui::Event::Key { key:egui::Key::F12,
        physical_key:None, pressed:false, repeat:false, modifiers };
    let fine_end = center - vec2(0.0, 15.0);
    let fine_edit = frame_revision_state(&ctx, &mut rack, &patch, 1.1,
        vec![egui::Event::PointerMoved(fine_end), carrier(fine)], true, true, 29).0;
    let Some(RackAction::Command("set_parameter", args)) = fine_edit else {
        panic!("fine rotary drag must edit level")
    };
    assert!((args["value"].as_f64().unwrap() - 0.51).abs() < 1e-6);
    let group = args["history_group"].clone();
    assert!(group.is_string());
    patch.nodes[0].params.insert("level".into(), args["value"].as_f64().unwrap() as f32);
    assert!(frame_revision_state(&ctx, &mut rack, &patch, 1.2,
        vec![egui::Event::PointerMoved(fine_end), carrier(egui::Modifiers::NONE)],
        true, true, 29).0.is_none(), "stationary modifier release cannot jump");
    let normal_end = fine_end - vec2(0.0, 15.0);
    let resumed = frame_revision_state(&ctx, &mut rack, &patch, 1.3,
        vec![egui::Event::PointerMoved(normal_end)], true, true, 29).0;
    let Some(RackAction::Command("set_parameter", args)) = resumed else {
        panic!("normal motion must continue the same gesture")
    };
    assert!((args["value"].as_f64().unwrap() - 0.61).abs() < 1e-6);
    assert_eq!(args["history_group"], group);
    patch.nodes[0].params.insert("level".into(), args["value"].as_f64().unwrap() as f32);
    assert!(frame_revision_state(&ctx, &mut rack, &patch, 1.4,
        vec![button(normal_end, false)], true, true, 29).0.is_none());
}

#[test]
fn stock_vca_rotary_double_click_restores_full_saved_level() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = patch();
    patch.nodes[0].params.insert("level".into(), 0.25);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let center = at(stock_vca_knob_rect().center());
    let mut actions = vec![];
    for (time, pressed) in [(1.0, true), (1.1, false), (1.2, true), (1.3, false)] {
        actions.push(frame_revision_state(&ctx, &mut rack, &patch, time,
            vec![egui::Event::PointerMoved(center), button(center, pressed)],
            true, true, 29).0);
    }
    assert!(actions[..3].iter().all(Option::is_none));
    let Some(RackAction::Command("set_parameter", args)) = actions.pop().unwrap() else {
        panic!("double click must restore full level")
    };
    assert_eq!(args["parameter"], "level");
    assert_eq!(args["value"], 1.0);
    assert_eq!(args["expected_revision"], 29);
}

#[test]
fn stock_vca_rotary_drag_is_one_real_session_undo_and_redo() {
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    let mut initial = patch();
    initial.nodes[0].params.insert("level".into(), 0.5);
    session.replace_patch(initial, Some(0)).unwrap();
    let baseline = session.snapshot()["history"]["undo_steps"].as_u64().unwrap();
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    frame_revision_state(&ctx, &mut rack, session.patch(), 0.0, vec![], true, true, session.revision());
    let center = at(stock_vca_knob_rect().center());
    assert!(frame_revision_state(&ctx, &mut rack, session.patch(), 1.0,
        vec![egui::Event::PointerMoved(center), button(center, true)],
        true, true, session.revision()).0.is_none());
    let mut group = None;
    for (time, pixels) in [(1.1, 15.0), (1.2, 30.0)] {
        let action = frame_revision_state(&ctx, &mut rack, session.patch(), time,
            vec![egui::Event::PointerMoved(center - vec2(0.0, pixels))],
            true, true, session.revision()).0;
        let Some(RackAction::Command("set_parameter", args)) = action else {
            panic!("each rotary movement must emit an edit")
        };
        assert_eq!(args["expected_revision"], session.revision());
        assert_eq!(args["parameter"], "level");
        if let Some(previous) = &group {
            assert_eq!(&args["history_group"], previous);
        } else {
            group = Some(args["history_group"].clone());
        }
        session.execute("set_parameter", args).unwrap();
    }
    frame_revision_state(&ctx, &mut rack, session.patch(), 1.3,
        vec![button(center - vec2(0.0, 30.0), false)], true, true, session.revision());
    assert!((session.patch().nodes[0].params["level"] - 0.7).abs() < 1e-6);
    assert_eq!(session.snapshot()["history"]["undo_steps"].as_u64(), Some(baseline + 1));
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.patch().nodes[0].params["level"], 0.5);
    session.execute("redo", json!({})).unwrap();
    assert!((session.patch().nodes[0].params["level"] - 0.7).abs() < 1e-6);
}

#[test]
fn stock_vca_accepted_level_edit_animates_angular_setting_beside_real_meter() {
    use crate::construction_motion::{Marker, MotionMode};
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut before = patch();
    before.nodes[0].params.insert("level".into(), 0.25);
    let mut after = before.clone();
    after.nodes[0].params.insert("level".into(), 0.75);
    frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
    let now = std::time::Instant::now();
    let receipt = json!({"plan_id":23,"completed_steps":1,"last_revision":10,"current_revision":10,
        "last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":23,
        "step_index":0,"before_revision":9,"revision":10,"label":"Set level",
        "new_modules":[],"new_cables":[],"parameter_changes":[{"id":"amplifier",
        "kind":"stock_vca","parameter":"level","old_effective":0.25,"new_effective":0.75}]}});
    rack.observe_construction_transition_at(&before, &after, 9, 10, &receipt, MotionMode::Full, now);
    rack.set_construction_clock_override(Some(now + std::time::Duration::from_millis(150)));
    let (action, output) = frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10);
    assert!(action.is_none());
    let anchor = &rack.construction_anchors[&("amplifier".into(), "level".into())];
    assert_eq!((&*anchor.old_text, &*anchor.new_text), ("25%", "75%"));
    let Some(Marker::Rotary { center, radius, old, new }) = anchor.marker.as_ref() else {
        panic!("accepted level edit needs angular cue")
    };
    assert!(center.distance(at(stock_vca_knob_rect().center())) < 0.001);
    assert!((*radius - 30.0 * rack.zoom).abs() < 0.001);
    assert_eq!((*old, *new), (0.25, 0.75));
    let mut labels = vec![];
    for shape in &output.shapes { texts(&shape.shape, &mut labels); }
    assert!(labels.iter().any(|(label, _)| label == "75%"));
    assert!(labels.iter().any(|(label, _)| label == "— TELEMETRY OFFLINE"));
    assert!(!rack.module_lights.contains_key("amplifier"), "cue cannot fabricate gain telemetry");
}
#[test]
fn stock_vca_controls_lanes_and_ports_fit_without_overlap() {
    let bounds = Rect::from_min_size(Pos2::ZERO, vec2(panel_width("stock_vca"), PANEL_HEIGHT));
    let mut controls = vec![
        stock_vca_knob_rect(),
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
    for scale in [0.25, 0.35, 0.5, 1.0] {
        let mut surfaces = vec![];
        for browser in [false, true] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut preview = ModulePreview::new(module);
            let size = vec2(panel_width("stock_vca"), PANEL_HEIGHT);
            let bounds = Rect::from_min_size(pos2(24.0, 12.0), size * scale);
            // Match preview.show's exact fitted geometry at fractional scales.
            let fitted_scale = (bounds.width() / size.x).min(bounds.height() / size.y);
            let fitted = Rect::from_center_size(bounds.center(), size * fitted_scale);
            let mut rack = Rack {
                zoom: fitted_scale,
                pan: Vec2::ZERO,
                ..Rack::default()
            };
            rack.positions.insert("amplifier".into(), Pos2::ZERO);
            let rack_patch = patch();
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
                    ctx.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().frame(egui::Frame::NONE).show(
                                ctx,
                                |ui| {
                                    let mut child =
                                        ui.new_child(egui::UiBuilder::new().max_rect(fitted));
                                    child.set_clip_rect(ui.clip_rect().intersect(bounds));
                                    let rendered = rack.render_modules(
                                        &mut child,
                                        &rack_patch,
                                        &catalog,
                                        fitted,
                                        48_000,
                                        0,
                                        RenderMode::Interactive {
                                            drag_interactive: false,
                                            allow_manual_press: false,
                                        },
                                    );
                                    assert!(rendered.action.is_none());
                                },
                            );
                        },
                    )
                };
                drawn.clear();
                primitives.clear();
                // Compare complete module primitive streams. Rack rails are host
                // chrome, so render_modules isolates the same surface as preview.
                fn primitives_in(shape: &egui::Shape, result: &mut Vec<String>) {
                    match shape {
                        egui::Shape::Vec(shapes) => {
                            for shape in shapes {
                                primitives_in(shape, result)
                            }
                        }
                        egui::Shape::Text(t) => result.push(format!(
                            "text:{:?}:{:?}:{}:{:?}:{:?}:{:?}:{:?}:{}:{}",
                            t.pos,
                            t.galley.size(),
                            t.galley.text(),
                            t.galley.job.sections,
                            t.fallback_color,
                            t.override_text_color,
                            t.underline,
                            t.opacity_factor,
                            t.angle,
                        )),
                        egui::Shape::Rect(r) => result.push(format!("rect:{r:?}")),
                        egui::Shape::Circle(c) => result.push(format!("circle:{c:?}")),
                        egui::Shape::LineSegment { points, stroke } => {
                            result.push(format!("line:{points:?}:{stroke:?}"))
                        }
                        egui::Shape::Mesh(mesh) => {
                            result.push(format!("mesh:{:?}:{:?}", mesh.vertices, mesh.indices))
                        }
                        other => result.push(format!("shape:{other:?}")),
                    }
                }
                fn count_faces(shape: &egui::Shape) -> usize {
                    match shape {
                        egui::Shape::Vec(shapes) => shapes.iter().map(count_faces).sum(),
                        egui::Shape::Circle(circle) if circle.fill == Color32::from_rgb(56, 59, 57) => 1,
                        _ => 0,
                    }
                }
                let rotary_faces: usize = output.shapes.iter().map(|shape| count_faces(&shape.shape)).sum();
                assert_eq!(rotary_faces, 1, "{scale} browser={browser}: actual level rotary face");
                for s in output.shapes {
                    texts(&s.shape, &mut drawn);
                    primitives_in(&s.shape, &mut primitives);
                }
            }
            for label in [
                "VCA",
                "LEVEL",
                "0–100%",
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
            let local = stock_vca_knob_rect();
            let hit = Rect::from_min_max(
                fitted.min + local.min.to_vec2() * fitted_scale,
                fitted.min + local.max.to_vec2() * fitted_scale,
            );
            for (label, text_rect) in &drawn {
                assert!(!hit.intersects(*text_rect),
                    "{scale} browser={browser}: {label:?} overlaps saved level rotary target");
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
fn stock_vca_held_rotary_keeps_node_and_history_identity_across_graph_edits() {
    for change in ["remove", "insert", "reorder"] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"earlier","kind":"stock_vca"},
            {"id":"held","kind":"stock_vca","params":{"level":0.5}}
        ]}))
        .unwrap();
        rack.positions.insert("earlier".into(), pos2(0.0, 0.0));
        rack.positions.insert("held".into(), pos2(330.0, 0.0));
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
        let start = at(stock_vca_knob_rect().center() + vec2(330.0, 0.0));
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
            panic!("held rotary lost after {change}")
        };
        assert_eq!(
            after["id"], "held",
            "active rotary transferred after {change}"
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
fn stock_vca_deleted_held_rotary_never_transfers_drag_to_remaining_filter() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"held","kind":"stock_vca","params":{"level":0.5}},
        {"id":"remaining","kind":"stock_vca"}
    ]}))
    .unwrap();
    rack.positions.insert("held".into(), pos2(0.0, 0.0));
    rack.positions.insert("remaining".into(), pos2(330.0, 0.0));
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let start = at(stock_vca_knob_rect().center());
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
        "a deleted rotary's drag must not edit the remaining node"
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
            brightness: [[1.0, 0.0]; openpatch_core::MAX_MODULE_LIGHTS],
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

#[test]
fn stock_vca_rotary_keeps_focus_for_repeated_arrow_edits() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = patch();
    patch.nodes[0].params.insert("level".into(), 0.5);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let position = at(stock_vca_knob_rect().center());
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(position), button(position, true)],
        true,
        true,
        29,
    )
    .0;
    assert!(action.is_none(), "center click cannot jump the level rotary");
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(position, false)],
        true,
        true,
        30,
    );
    let knob_id = egui::Id::new(("stock-vca-knob", "amplifier", "level"));
    assert!(
        ctx.memory(|memory| memory.has_focus(knob_id)),
        "clicked rotary must retain focus"
    );
    for (step, (shift, expected)) in [(false, 0.49), (false, 0.48), (true, 0.38)]
        .iter()
        .enumerate()
    {
        let time = 2.0 + step as f64;
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            time,
            vec![key(egui::Key::ArrowDown, *shift)],
            true,
            true,
            30 + step as u64,
        )
        .0;
        let Some(RackAction::Command("set_parameter", args)) = action else {
            panic!("arrow edit {} lost rotary focus", step + 1)
        };
        assert_eq!(args["id"], "amplifier");
        assert_eq!(args["parameter"], "level");
        let actual = args["value"].as_f64().unwrap();
        assert!(
            (actual - expected).abs() < 1e-6,
            "step {step}: {actual} versus {expected}"
        );
        assert!(
            ctx.memory(|memory| memory.has_focus(knob_id)),
            "arrow edit {} must preserve rotary focus",
            step + 1
        );
        // Apply the emitted edit as the host does before the next key frame.
        patch.nodes[0].params.insert("level".into(), actual as f32);
        let mut release = key(egui::Key::ArrowDown, *shift);
        if let egui::Event::Key { pressed, .. } = &mut release {
            *pressed = false;
        }
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            time + 0.1,
            vec![release],
            true,
            true,
            31 + step as u64,
        );
    }
}

#[test]
fn stock_vca_focused_arrow_edits_do_not_pan_and_idle_arrows_still_pan() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = patch();
    patch.nodes[0].params.insert("level".into(), 0.5);
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let center = at(stock_vca_knob_rect().center());
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(center), button(center, true)],
        true,
        true,
        29,
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(center, false)],
        true,
        true,
        29,
    );
    let knob_id = egui::Id::new(("stock-vca-knob", "amplifier", "level"));
    assert!(ctx.memory(|memory| memory.has_focus(knob_id)));
    for (index, modifiers) in [egui::Modifiers::NONE, egui::Modifiers::CTRL]
        .into_iter()
        .enumerate()
    {
        let time = 2.0 + index as f64;
        let held_pan = rack.pan;
        let held_viewport = rack.viewport;
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            time,
            vec![egui::Event::Key {
                key: egui::Key::ArrowDown,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            true,
            true,
            30 + index as u64,
        )
        .0;
        let Some(RackAction::Command("set_parameter", args)) = action else {
            panic!("focused level rotary must accept arrow edit {index}")
        };
        assert_eq!(args["parameter"], "level");
        assert_eq!(args["expected_revision"], 30 + index as u64);
        let next = args["value"].as_f64().unwrap() as f32;
        assert!(next < patch.nodes[0].params["level"]);
        assert_eq!(rack.pan, held_pan, "focused arrow moved the rack camera");
        assert_eq!(rack.viewport, held_viewport);
        assert!(ctx.memory(|memory| memory.has_focus(knob_id)));
        patch.nodes[0].params.insert("level".into(), next);
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            time + 0.1,
            vec![egui::Event::Key {
                key: egui::Key::ArrowDown,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers,
            }],
            true,
            true,
            31 + index as u64,
        );
        assert_eq!(rack.pan, held_pan, "releasing the arrow moved the rack camera");
    }
    ctx.memory_mut(|memory| memory.surrender_focus(knob_id));
    frame_revision_state(&ctx, &mut rack, &patch, 4.0, vec![], true, true, 32);
    assert!(!ctx.memory(|memory| memory.has_focus(knob_id)));
    let idle_pan = rack.pan;
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        4.1,
        vec![key(egui::Key::ArrowRight, false)],
        true,
        true,
        32,
    );
    assert!(rack.pan.x < idle_pan.x, "idle right arrow must still pan the rack");
}
