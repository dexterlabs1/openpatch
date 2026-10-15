use super::*;

fn module(kind: &str) -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"m","kind":kind}]})).unwrap()
}

fn normalized(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    shapes
        .iter()
        .map(|clipped| {
            let mut clipped = clipped.clone();
            egui::epaint::shape_transform::adjust_colors(&mut clipped.shape, |color| {
                // Retain alpha while discarding ambient RGB. This compares the
                // complete geometry, text meshes, order and clips of every shape.
                if *color != Color32::PLACEHOLDER {
                    *color = Color32::from_rgba_premultiplied(0, 0, 0, color.a());
                }
            });
            format!("{clipped:?}")
        })
        .collect()
}

#[test]
fn darkroom_every_catalog_panel_preserves_geometry_alpha_order_and_targets() {
    let catalog = openpatch_core::catalog();
    for entry in catalog["modules"].as_array().unwrap() {
        let kind = entry["kind"].as_str().unwrap();
        let mut preview = ModulePreview::new(entry);
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let rect = Rect::from_min_size(pos2(12.0, 12.0), vec2(panel_width(kind), PANEL_HEIGHT));
        let mut draw = |amount| {
            preview.renderer.set_room_brightness(amount);
            let mut targets = Vec::new();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let result = preview.renderer.render_modules(
                            ui,
                            &preview.patch,
                            &catalog,
                            rect,
                            48_000,
                            0,
                            RenderMode::Preview,
                        );
                        targets = result.jack_targets;
                        assert!(result.action.is_none());
                    });
                },
            );
            (output.shapes, format!("{targets:?}"))
        };
        let _ = draw(1.0); // font atlas/layout warmup
        let full = draw(1.0);
        let dark = draw(0.1);
        let black = draw(0.0);
        assert_eq!(normalized(&full.0), normalized(&dark.0), "{kind}");
        assert_eq!(normalized(&full.0), normalized(&black.0), "{kind}");
        assert_eq!(full.1, dark.1, "{kind}");
        assert_eq!(full.1, black.1, "{kind}");
        assert_ne!(
            format!("{:?}", full.0),
            format!("{:?}", dark.0),
            "{kind} must dim"
        );
    }
}

#[test]
fn darkroom_live_lamps_survive_while_inactive_and_missing_lamps_dim() {
    // Every lamp family; each target is a lens, not a reflective label or bezel.
    for (kind, x, y, radius, count, index, color) in [
        (
            "poly_viz",
            55.0,
            120.0,
            3.0,
            16,
            0,
            Color32::from_rgb(65, 174, 102),
        ),
        (
            "push",
            49.0,
            265.0,
            4.4,
            2,
            0,
            Color32::from_rgb(238, 177, 65),
        ),
        (
            "pulses",
            132.0,
            100.0,
            2.8,
            10,
            0,
            Color32::from_rgb(238, 177, 65),
        ),
        (
            "rescale",
            176.5,
            294.0,
            2.2,
            2,
            0,
            Color32::from_rgb(242, 199, 59),
        ),
    ] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        rack.set_room_brightness(0.1);
        let patch = module(kind);
        let mut lights = ModuleLightState {
            count,
            input_channels: Some(1),
            ..Default::default()
        };
        let lens = |shapes: &[egui::epaint::ClippedShape]| {
            shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle)
                        if circle.center.distance(pos2(24.0 + x, 12.0 + y)) < 0.01
                            && (circle.radius - radius).abs() < 0.01 =>
                    {
                        Some(circle.fill)
                    }
                    _ => None,
                })
                .expect("lamp lens")
        };
        let missing = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1;
        lights.brightness[index][0] = 0.0;
        rack.module_lights.insert("m".into(), lights);
        let inactive = frame_output(&ctx, &mut rack, &patch, 0.1, vec![]).1;
        lights.brightness[index][0] = 1.0;
        rack.module_lights.insert("m".into(), lights);
        let active = frame_output(&ctx, &mut rack, &patch, 0.2, vec![]).1;
        assert_eq!(lens(&active.shapes), color, "{kind}");
        assert!(lens(&inactive.shapes).r() < color.r() / 3, "{kind}");
        assert!(lens(&missing.shapes).r() < color.r() / 3, "{kind}");
        assert_eq!(
            active.shapes.len(),
            inactive.shapes.len() + 2,
            "exactly two halo rings: {kind}"
        );
        // A later UI time alone cannot make an unavailable lamp flash.
        rack.module_lights.clear();
        let later = frame_output(&ctx, &mut rack, &patch, 100.0, vec![]).1;
        assert_eq!(lens(&missing.shapes), lens(&later.shapes), "{kind}");
    }
}

#[test]
fn darkroom_meters_and_sequence_indicators_follow_published_state() {
    for kind in [
        "poly_sum",
        "vca_mix",
        "seq3",
        "switch_1to4",
        "switch_4to1",
        "gates",
        "logic",
        "compare",
    ] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        rack.set_room_brightness(0.0);
        let patch = module(kind);
        let missing = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1;
        rack.module_lights.insert(
            "m".into(),
            ModuleLightState {
                count: 16,
                input_channels: Some(1),
                brightness: [[1.0, 1.0]; 16],
                ..Default::default()
            },
        );
        rack.sequence_states.insert(
            "m".into(),
            ModuleTelemetry {
                step: 1,
                running: true,
                length: 4,
            },
        );
        let active = frame_output(&ctx, &mut rack, &patch, 0.1, vec![]).1;
        // Published signal must add bounded halos/visible emission even with
        // all reflective surfaces at zero illumination.
        if !kind.starts_with("switch_") {
            assert!(active.shapes.len() > missing.shapes.len(), "{kind}");
        }
        let bright = active
            .shapes
            .iter()
            .filter(|clipped| match &clipped.shape {
                egui::Shape::Circle(shape) => {
                    shape.fill.r().max(shape.fill.g()).max(shape.fill.b()) > 40
                        || shape
                            .stroke
                            .color
                            .r()
                            .max(shape.stroke.color.g())
                            .max(shape.stroke.color.b())
                            > 40
                }
                egui::Shape::Rect(shape) => {
                    shape.fill.r().max(shape.fill.g()).max(shape.fill.b()) > 40
                        || shape
                            .stroke
                            .color
                            .r()
                            .max(shape.stroke.color.g())
                            .max(shape.stroke.color.b())
                            > 40
                }
                _ => false,
            })
            .count();
        assert!(bright > 0, "{kind}");
    }
}

#[test]
fn darkroom_scope_keeps_trace_clip_and_later_cable_occlusion() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch = serde_json::from_value(json!({"version":2,
        "nodes":[{"id":"m","kind":"scope"}],
        "cables":[{"id":"crossing","from":"m","output":"x","to":"m","input":"x"}]
    }))
    .unwrap();
    let mut state = openpatch_core::ScopeSnapshot {
        x_channels: 1,
        ..Default::default()
    };
    state.x[0].fill(openpatch_core::ScopePoint {
        min: -2.0,
        max: 2.0,
    });
    rack.scope_states.insert("m".into(), Arc::new(state));
    let full = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1;
    rack.set_room_brightness(0.0);
    let dark = frame_output(&ctx, &mut rack, &patch, 0.1, vec![]).1;
    let trace_color = scope_voice_color(Color32::from_rgb(86, 215, 222), 0);
    let traces = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .enumerate()
            .filter_map(|(index, clipped)| match &clipped.shape {
                egui::Shape::LineSegment { stroke, .. } if stroke.color == trace_color => {
                    Some((index, clipped.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let a = traces(&full);
    let b = traces(&dark);
    assert!(!a.is_empty());
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
    let trace_clip = Rect::from_min_max(pos2(65.0, 114.0), pos2(613.0, 224.0));
    assert!(b.iter().all(|(_, clipped)| clipped.clip_rect == trace_clip));
    let cable_index = dark
        .shapes
        .iter()
        .position(|clipped| matches!(clipped.shape, egui::Shape::CubicBezier(_)))
        .expect("crossing cable");
    assert!(
        b.last().unwrap().0 < cable_index,
        "cable stays after screen emission in original order"
    );
    assert_eq!(normalized(&full.shapes), normalized(&dark.shapes));
}

#[test]
fn darkroom_numeric_editor_and_selection_remain_readable_and_controls_keep_targets() {
    let patch = module("multi_vco");
    let mut recordings = Vec::new();
    for amount in [1.0, 0.0] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        rack.set_room_brightness(amount);
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let field = pos2(99.0, 169.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(field), button(field, true)],
        );
        frame(&ctx, &mut rack, &patch, 1.1, vec![button(field, false)]);
        assert_eq!(rack.editing.as_ref().unwrap().parameter, "frequency");
        let output = frame_output(&ctx, &mut rack, &patch, 1.2, vec![]).1;
        let editor: Vec<_> = output
            .shapes
            .iter()
            .filter(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => {
                    text.galley.text().contains("m / frequency")
                        || text.galley.text().contains("Apply")
                }
                _ => false,
            })
            .map(|shape| format!("{shape:?}"))
            .collect();
        assert!(!editor.is_empty());
        assert!(output.shapes.iter().any(|clipped| matches!(&clipped.shape,
            egui::Shape::Rect(rect) if rect.stroke.color == Color32::from_rgb(201,137,54))));
        rack.editing.as_mut().unwrap().text = "440".into();
        let action = frame(
            &ctx,
            &mut rack,
            &patch,
            1.3,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let Some(RackAction::Command(name, args)) = action else {
            panic!("numeric editor applies")
        };
        recordings.push((editor, name, args));
    }
    assert_eq!(recordings[0], recordings[1]);
}

#[test]
fn darkroom_hovered_connected_jack_keeps_patching_guidance_visible() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    rack.set_room_brightness(0.0);
    let patch: Patch = serde_json::from_value(json!({"version":2,
        "nodes":[{"id":"m","kind":"gain"}],
        "cables":[{"id":"trace","from":"m","output":"out","to":"m","input":"signal"}]
    }))
    .unwrap();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let jack = fixture_jack(&rack, &patch, "m", "out", false);
    let output = frame_output(
        &ctx,
        &mut rack,
        &patch,
        0.1,
        vec![egui::Event::PointerMoved(jack)],
    )
    .1;
    let color = cable_instance_color(&patch.cables[0]);
    assert!(output.shapes.iter().any(|clipped| matches!(&clipped.shape,
        egui::Shape::Circle(circle) if circle.center == jack
            && circle.stroke.color == color && (circle.radius-11.4).abs() < 0.01)));
}

#[test]
fn darkroom_active_inline_editors_keep_text_background_caret_and_typed_actions() {
    for (kind, parameter, local, typed, expected) in [
        (
            "seq3",
            "tempo",
            Rect::from_min_size(pos2(20.0, 99.0), vec2(175.0, 24.0)),
            "144",
            144.0,
        ),
        ("scope", "time", scope_control_rect(2), "0.125", 0.125),
    ] {
        let patch = module(kind);
        let mut results = Vec::new();
        for amount in [1.0, 0.0] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            rack.set_room_brightness(amount);
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let bounds = local.translate(vec2(24.0, 12.0));
            let target = bounds.center();
            frame(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![egui::Event::PointerMoved(target), button(target, true)],
            );
            frame(&ctx, &mut rack, &patch, 1.1, vec![button(target, false)]);
            assert!(
                ctx.memory(|memory| memory.focused()).is_some(),
                "inline editor focused"
            );
            let output = frame_output(&ctx, &mut rack, &patch, 1.2, vec![]).1;
            assert!(
                rack.editing.is_none(),
                "this must exercise the inline editor"
            );
            assert!(ctx.wants_keyboard_input());
            let mut text = 0;
            let mut backgrounds = 0;
            let mut carets = 0;
            let editor: Vec<_> = output
                .shapes
                .iter()
                .filter(|clipped| match &clipped.shape {
                    egui::Shape::Text(shape) if bounds.contains(shape.pos) => {
                        text += 1;
                        true
                    }
                    egui::Shape::Rect(shape) if bounds.expand(1.0).contains_rect(shape.rect) => {
                        backgrounds += 1;
                        true
                    }
                    egui::Shape::LineSegment { points, .. }
                        if points.iter().all(|p| bounds.contains(*p)) =>
                    {
                        carets += 1;
                        true
                    }
                    _ => false,
                })
                .map(|shape| format!("{shape:?}"))
                .collect();
            assert!(
                text > 0 && backgrounds > 0 && carets > 0,
                "{kind}: text {text}, backgrounds {backgrounds}, caret {carets}"
            );
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                1.3,
                vec![
                    egui::Event::Text(typed.into()),
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            let Some(RackAction::Command(name, args)) = action else {
                panic!("inline editor should type: {kind} {amount}")
            };
            assert_eq!(name, "set_parameter");
            assert_eq!(args["parameter"], parameter);
            assert!((args["value"].as_f64().unwrap() - expected).abs() < 0.0001);
            results.push((editor, name, args));
        }
        assert_eq!(
            results[0], results[1],
            "{kind} active editor stays at normal contrast"
        );
    }
}

#[test]
fn darkroom_construction_cable_accents_stay_visible_while_physical_curves_dim() {
    for reduced in [false, true] {
        let ctx = egui::Context::default();
        let mut lighting = crate::lighting::RoomLighting::default();
        lighting.set(0.0);
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            let start = lighting.begin(&painter);
            paint_motion_cable_curves(
                &painter,
                pos2(10.0, 10.0),
                pos2(200.0, 10.0),
                CABLES[0],
                1.0,
                false,
                0.0,
                Some(crate::cable_animation::Phase {
                    progress: 0.5,
                    settle: 0.0,
                    fade: 1.0,
                    reduced,
                }),
                &lighting,
            );
            lighting.finish(&painter, start);
        });
        let mut accents = 0;
        let mut curves = 0;
        for clipped in &output.shapes {
            match &clipped.shape {
                egui::Shape::Circle(circle) => {
                    assert!(circle.stroke.color.r() > 0 || circle.fill.r() > 0);
                    accents += 1;
                }
                egui::Shape::CubicBezier(curve) => {
                    let egui::epaint::ColorMode::Solid(color) = curve.stroke.color else {
                        panic!("solid curve")
                    };
                    assert_eq!([color.r(), color.g(), color.b()], [0, 0, 0]);
                    curves += 1;
                }
                _ => {}
            }
        }
        assert!(curves > 0);
        assert_eq!(accents, if reduced { 1 } else { 3 });
    }
}

/// Run explicitly with --ignored --nocapture; no audio/device/window is opened.
#[test]
#[ignore]
fn darkroom_dense_patch_cpu_and_shape_budget() {
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":(0..48)
        .map(|n|json!({"id":format!("m{n}"),"kind":"poly_viz"})).collect::<Vec<_>>()
    }))
    .unwrap();
    let mut results = Vec::new();
    for amount in [1.0, 0.1] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        rack.set_zoom(0.25);
        rack.set_room_brightness(amount);
        for (index, node) in patch.nodes.iter().enumerate() {
            rack.positions.insert(
                node.id.clone(),
                pos2((index % 16) as f32 * 180.0, (index / 16) as f32 * ROW),
            );
        }
        rack.module_lights = patch
            .nodes
            .iter()
            .map(|node| {
                (
                    node.id.clone(),
                    ModuleLightState {
                        count: 16,
                        input_channels: Some(16),
                        brightness: [[0.7, 0.3]; 16],
                        ..Default::default()
                    },
                )
            })
            .collect();
        for n in 0..10 {
            frame(&ctx, &mut rack, &patch, n as f64 / 60.0, vec![]);
        }
        let start = std::time::Instant::now();
        let mut shapes = 0;
        for n in 0..120 {
            shapes = frame_output(&ctx, &mut rack, &patch, (n + 10) as f64 / 60.0, vec![])
                .1
                .shapes
                .len();
        }
        results.push((start.elapsed().as_secs_f64() / 120.0 * 1000.0, shapes));
    }
    eprintln!("48 VIZ modules at 25%: lights on {:.3} ms / {} shapes; darkroom {:.3} ms / {} shapes; ratio {:.2}",results[0].0,results[0].1,results[1].0,results[1].1,results[1].0/results[0].0);
    assert!(
        results[1].1 <= results[0].1 + 48 * 32 * 2,
        "at most two halo rings per actual lamp"
    );
    assert!(
        results[1].0 < results[0].0 * 8.0 + 5.0,
        "bounded CPU pass, not a per-shape search over every lamp"
    );
}
