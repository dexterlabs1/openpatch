use super::*;

fn primitives(output: &egui::FullOutput) -> Vec<String> {
    fn visit(shape: &egui::Shape, result: &mut Vec<String>) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, result);
                }
            }
            egui::Shape::Text(text) => result.push(format!(
                "text:{:?}:{:?}:{}:{:?}:{:?}:{:?}",
                text.pos,
                text.galley.size(),
                text.galley.text(),
                text.galley.job.sections,
                text.fallback_color,
                text.override_text_color
            )),
            shape => result.push(format!("{shape:?}")),
        }
    }
    let mut result = vec![];
    for shape in &output.shapes {
        visit(&shape.shape, &mut result);
    }
    result
}

fn texts(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    fn visit(shape: &egui::Shape, result: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, result);
                }
            }
            egui::Shape::Text(text) if !text.galley.text().is_empty() => result.push((
                text.galley.text().into(),
                Rect::from_min_size(text.pos, text.galley.size()),
            )),
            _ => {}
        }
    }
    let mut result = vec![];
    for shape in &output.shapes {
        visit(&shape.shape, &mut result);
    }
    result
}

#[test]
fn stock_mix_shared_panel_matches_browser_and_keeps_text_controls_and_ports_separate() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "stock_mix")
        .unwrap();
    assert_eq!(title("stock_mix"), "STOCK MIX");
    assert_eq!(title("mix6"), "MIX 6");
    assert_eq!(panel_width("stock_mix"), 240.0);
    assert_eq!(
        crate::panel_skin::family_accent("stock_mix"),
        crate::panel_skin::family_accent("mix6")
    );
    assert_eq!(
        module["inputs"],
        json!(["in1", "in2", "in3", "in4", "in5", "in6"])
    );
    assert_eq!(module["outputs"], json!(["out"]));
    for scale in [0.25, 0.42, 0.7, 1.0, 1.75] {
        let mut surfaces = vec![];
        for browser in [false, true] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let size = vec2(240.0, PANEL_HEIGHT);
            let bounds = Rect::from_min_size(pos2(24.0, 12.0), size * scale);
            let zoom = (bounds.width() / size.x).min(bounds.height() / size.y);
            let fitted = Rect::from_center_size(bounds.center(), size * zoom);
            let mut preview = ModulePreview::new(module);
            let mut patch = preview.patch.clone();
            patch.nodes[0].id = "mix".into();
            let mut rack = Rack {
                zoom,
                pan: Vec2::ZERO,
                ..Default::default()
            };
            rack.positions.insert("mix".into(), Pos2::ZERO);
            let mut output = egui::FullOutput::default();
            for _ in 0..2 {
                output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 900.0))),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE)
                            .show(ctx, |ui| {
                                if browser {
                                    preview.show(ui, bounds, &catalog);
                                } else {
                                    let mut child =
                                        ui.new_child(egui::UiBuilder::new().max_rect(fitted));
                                    child.set_clip_rect(ui.clip_rect().intersect(bounds));
                                    let rendered = rack.render_modules(
                                        &mut child,
                                        &patch,
                                        &catalog,
                                        fitted,
                                        48000,
                                        0,
                                        RenderMode::Interactive {
                                            drag_interactive: false,
                                            allow_manual_press: false,
                                        },
                                    );
                                    assert!(rendered.action.is_none());
                                    assert_eq!(
                                        (
                                            rendered.inputs.len(),
                                            rendered.outputs.len(),
                                            rendered.jack_targets.len()
                                        ),
                                        (6, 1, 7)
                                    );
                                    for i in 0..6 {
                                        let expected = fitted.min
                                            + vec2(
                                                60.0 + 60.0 * (i % 3) as f32,
                                                270.0 + 52.0 * (i / 3) as f32,
                                            ) * zoom;
                                        assert!(
                                            rendered.inputs
                                                [&("mix".into(), format!("in{}", i + 1))]
                                                .distance(expected)
                                                < 0.001
                                        );
                                    }
                                    assert!(
                                        rendered.outputs[&("mix".into(), "out".into())]
                                            .distance(fitted.min + vec2(120.0, 374.0) * zoom)
                                            < 0.001
                                    );
                                    for name in ["average", "invert"] {
                                        assert!(
                                            ctx.read_response(
                                                child.id().with(("knob", "mix", name))
                                            )
                                            .is_none(),
                                            "Flags must not become continuous faders"
                                        );
                                    }
                                    let level = ctx
                                        .read_response(child.id().with(("knob", "mix", "level")))
                                        .unwrap()
                                        .rect;
                                    assert!(bounds.contains_rect(level));
                                }
                            });
                    },
                );
            }
            let text = texts(&output);
            for required in [
                "STOCK MIX",
                "LEVEL",
                "Average",
                "Invert",
                "IN1",
                "IN2",
                "IN3",
                "IN4",
                "IN5",
                "IN6",
                "OUT",
            ] {
                assert!(
                    text.iter().any(|(s, _)| s == required),
                    "{scale} browser={browser} missing {required}"
                );
            }
            for (i, (label, rect)) in text.iter().enumerate() {
                assert!(
                    bounds.expand(0.5).contains_rect(*rect),
                    "{scale}: {label} clipped"
                );
                for (other, other_rect) in &text[i + 1..] {
                    assert!(
                        !rect
                            .shrink(0.2 * zoom)
                            .intersects(other_rect.shrink(0.2 * zoom)),
                        "{scale}: {label} overlaps {other}"
                    );
                }
            }
            surfaces.push(primitives(&output));
        }
        assert_eq!(surfaces[0], surfaces[1], "Rack/browser mismatch at {scale}");
    }
}

#[test]
fn stock_mix_real_flag_targets_emit_strict_guarded_edits_and_respect_disabled_rack() {
    for (parameter, x) in [("average", 67.5), ("invert", 172.5)] {
        for enabled in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let mut session = openpatch::control::Session::new(dir.path()).unwrap();
            session
                .replace_patch(
                    serde_json::from_value(
                        json!({"version":2,"nodes":[{"id":"mix","kind":"stock_mix"}]}),
                    )
                    .unwrap(),
                    None,
                )
                .unwrap();
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let revision = session.revision();
            frame_revision_state(
                &ctx,
                &mut rack,
                session.patch(),
                0.0,
                vec![],
                true,
                enabled,
                revision,
            );
            let at = pos2(24.0 + x, 12.0 + 212.0);
            let mut action = None;
            for (time, pressed) in [(0.1, true), (0.2, false)] {
                action = frame_revision_state(
                    &ctx,
                    &mut rack,
                    session.patch(),
                    time,
                    vec![egui::Event::PointerMoved(at), button(at, pressed)],
                    true,
                    enabled,
                    revision,
                )
                .0
                .or(action);
            }
            if enabled {
                let RackAction::Command("set_parameter", args) = action.unwrap() else {
                    panic!("Expected actual flag edit")
                };
                assert_eq!(args["parameter"], parameter);
                assert_eq!(args["value"], 1.0);
                assert_eq!(args["expected_revision"], revision);
                session.execute("set_parameter", args).unwrap();
                assert_eq!(session.patch().nodes[0].params[parameter], 1.0);
                session.execute("undo", json!({})).unwrap();
                assert_eq!(
                    session.patch().nodes[0]
                        .params
                        .get(parameter)
                        .copied()
                        .unwrap_or(0.0),
                    0.0
                );
            } else {
                assert!(action.is_none());
            }
        }
    }
}

#[test]
fn stock_mix_real_level_readout_opens_bounded_precision_control() {
    let ctx = egui::Context::default();
    let patch: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"mix","kind":"stock_mix"}]}))
            .unwrap();
    let mut rack = Rack::default();
    let mut value_rect = Rect::NOTHING;
    for frame in 0..4 {
        let events = if frame >= 2 {
            let at = value_rect.center();
            vec![egui::Event::PointerMoved(at), button(at, frame == 2)]
        } else {
            vec![]
        };
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                time: Some(frame as f64 * 0.1),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        assert!(rack
                            .show(ui, &patch, &openpatch_core::catalog(), 48000, 7)
                            .is_none());
                        value_rect = ctx
                            .read_response(ui.id().with(("value", "mix", "level")))
                            .unwrap()
                            .rect;
                        assert!(
                            value_rect.bottom() < 12.0 + 199.0,
                            "Level hit target must remain above flags"
                        );
                    });
            },
        );
    }
    let edit = rack
        .editing
        .as_mut()
        .expect("Actual level readout opens editor");
    assert_eq!(edit.parameter, "level");
    assert_eq!(edit.revision, 7);
    assert_eq!(edit.raw_max, 1.0);
    edit.text = "0.25".into();
    assert_eq!(parsed_parameter_result(edit).unwrap(), 0.25);
    edit.text = "1.01".into();
    assert!(parsed_parameter_result(edit).is_err());
}

#[test]
fn six_mix_mcp_flag_anchors_follow_actual_controls_and_configuration() {
    use crate::construction_motion::MotionMode;
    for kind in ["mix6", "stock_mix"] {
        for (parameter, x) in [("average", 67.5), ("invert", 172.5)] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let before: Patch =
                serde_json::from_value(json!({"version":2,"nodes":[{"id":"mix","kind":kind}]}))
                    .unwrap();
            let mut after = before.clone();
            after.nodes[0].params.insert(parameter.into(), 1.0);
            frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
            let now = std::time::Instant::now();
            let receipt = json!({"plan_id":21,"completed_steps":1,"last_revision":10,"current_revision":10,"last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":21,"step_index":0,"before_revision":9,"revision":10,"label":"Configure mixer","new_modules":[],"new_cables":[],"parameter_changes":[{"id":"mix","kind":kind,"parameter":parameter,"old_effective":0.0,"new_effective":1.0}]}});
            rack.observe_construction_transition_at(
                &before,
                &after,
                9,
                10,
                &receipt,
                MotionMode::Full,
                now,
            );
            rack.set_construction_clock_override(Some(now + std::time::Duration::from_millis(150)));
            let (action, output) =
                frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10);
            assert!(action.is_none());
            let anchor = &rack.construction_anchors[&("mix".into(), parameter.into())];
            assert_eq!((&*anchor.old_text, &*anchor.new_text), ("Off", "On"));
            assert_eq!(anchor.rect.center(), pos2(24.0 + x, 12.0 + 212.0));
            let expected = if parameter == "average" {
                "Σ/N"
            } else {
                "− OUT"
            };
            assert!(texts(&output).iter().any(|(text, _)| text == expected));
        }
    }
}
