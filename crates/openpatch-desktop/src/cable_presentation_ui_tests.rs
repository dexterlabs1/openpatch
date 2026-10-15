use super::*;
use crate::cable_presentation::CablePresentation;

fn fixture() -> (Patch, Arc<crate::audio::PortLightFrame>) {
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"p","kind":"constant","params":{"value":5}},
        {"id":"n","kind":"constant","params":{"value":-5}},
        {"id":"sum","kind":"gain"},{"id":"other","kind":"gain"}],
        "cables":[{"from":"p","to":"sum","input":"signal"},
            {"from":"n","to":"sum","input":"signal"},
            {"from":"p","to":"other","input":"signal"}],"output":"sum"}))
    .unwrap();
    let mut engine = patch.compile().unwrap();
    engine.set_port_lights_enabled(true);
    engine.process_block(&mut [0.0; 64]);
    let snapshot = crate::audio::offline_frame(&patch, &engine, 0).unwrap();
    (patch, snapshot)
}
fn rack(snapshot: Arc<crate::audio::PortLightFrame>) -> Rack {
    let mut rack = Rack::default();
    rack.restore_layout(&json!({"version":1,"zoom":1,"pan":[24,12],"positions":{
        "p":[0,0],"n":[150,0],"sum":[300,0],"other":[480,0]}}))
        .unwrap();
    rack.set_port_lights(Some(snapshot));
    rack
}
fn curves(output: &egui::FullOutput) -> Vec<egui::epaint::CubicBezierShape> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::CubicBezier(curve) => Some(curve.clone()),
            _ => None,
        })
        .collect()
}
fn without_curves(output: &egui::FullOutput) -> Vec<String> {
    output
        .shapes
        .iter()
        .filter(|shape| !matches!(shape.shape, egui::Shape::CubicBezier(_)))
        .map(|shape| format!("{shape:?}"))
        .collect()
}
fn lenses(output: &egui::FullOutput, zoom: f32) -> Vec<(Pos2, Color32)> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Circle(circle)
                if (circle.radius - crate::lighting::port_lamp_radius(zoom)).abs() < 0.001
                    && circle.fill.a() == 0
                    && circle.fill != Color32::TRANSPARENT =>
            {
                Some((circle.center, circle.fill))
            }
            _ => None,
        })
        .collect()
}
fn evaluate(p: [Pos2; 4], t: f32) -> Pos2 {
    let s = 1.0 - t;
    (p[0].to_vec2() * s.powi(3)
        + p[1].to_vec2() * 3.0 * s * s * t
        + p[2].to_vec2() * 3.0 * s * t * t
        + p[3].to_vec2() * t.powi(3))
    .to_pos2()
}

#[test]
fn cable_presentation_opacity_affects_every_body_layer_but_not_hardware_or_measured_light() {
    let (patch, snapshot) = fixture();
    let before = serde_json::to_value(&patch).unwrap();
    let ctx = egui::Context::default();
    let mut rack = rack(snapshot);
    frame_output(&ctx, &mut rack, &patch, 0.0, vec![]);
    let full = frame_output(&ctx, &mut rack, &patch, 0.1, vec![]).1;
    assert_eq!(curves(&full).len(), 9);
    for (time, opacity) in [(0.2, 0.25), (0.3, 0.0)] {
        rack.set_cable_presentation(CablePresentation {
            opacity,
            ..Default::default()
        });
        let (action, changed) = frame_output(&ctx, &mut rack, &patch, time, vec![]);
        assert!(action.is_none());
        assert_eq!(without_curves(&full), without_curves(&changed));
        if opacity == 0.0 {
            assert!(curves(&changed).is_empty(), "no residual black shadows");
        } else {
            for (original, changed) in curves(&full).iter().zip(curves(&changed)) {
                assert_eq!(original.points, changed.points);
                let egui::epaint::ColorMode::Solid(color) = original.stroke.color else {
                    panic!("solid cable")
                };
                let egui::epaint::ColorMode::Solid(dimmed) = changed.stroke.color else {
                    panic!("solid cable")
                };
                assert_eq!(dimmed, color.gamma_multiply(opacity));
            }
        }
    }
    assert_eq!(serde_json::to_value(&patch).unwrap(), before);
    let out = pos2(24.0, 12.0)
        + port_position("constant", false, 0, 1, panel_width("constant")).to_vec2();
    let revealed = frame_output(
        &ctx,
        &mut rack,
        &patch,
        0.4,
        vec![egui::Event::PointerMoved(out)],
    )
    .1;
    assert_eq!(
        curves(&revealed).len(),
        6,
        "hovered port reveals its two routes at zero opacity"
    );
}

#[test]
fn cable_presentation_brightness_scales_only_visible_emission_and_zero_cannot_resurrect_snapshot() {
    let (patch, snapshot) = fixture();
    let original = snapshot.ports.clone();
    let ctx = egui::Context::default();
    let mut rack = rack(snapshot.clone());
    frame_output(&ctx, &mut rack, &patch, 0.0, vec![]);
    let full = frame_output(&ctx, &mut rack, &patch, 0.1, vec![]).1;
    let colors = lenses(&full, 1.0);
    assert_eq!(colors.len(), 3);
    rack.set_cable_presentation(CablePresentation {
        plug_brightness: 0.25,
        ..Default::default()
    });
    let dimmed = frame_output(&ctx, &mut rack, &patch, 0.2, vec![]).1;
    assert_eq!(
        lenses(&dimmed, 1.0),
        colors
            .iter()
            .map(|(p, c)| (*p, c.gamma_multiply(0.25)))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        snapshot.ports, original,
        "presentation does not rewrite signed raw detector state"
    );
    rack.set_room_brightness(0.0);
    let dark = frame_output(&ctx, &mut rack, &patch, 0.3, vec![]).1;
    assert_eq!(lenses(&dark, 1.0), lenses(&dimmed, 1.0));
    rack.set_cable_presentation(CablePresentation {
        plug_brightness: 0.0,
        ..Default::default()
    });
    assert!(rack.port_lights.is_none());
    assert!(lenses(&frame_output(&ctx, &mut rack, &patch, 0.4, vec![]).1, 1.0).is_empty());
    rack.set_cable_presentation(CablePresentation::default());
    assert!(lenses(&frame_output(&ctx, &mut rack, &patch, 0.5, vec![]).1, 1.0).is_empty());
    rack.set_port_lights(Some(snapshot));
    assert_eq!(
        lenses(&frame_output(&ctx, &mut rack, &patch, 0.6, vec![]).1, 1.0),
        colors
    );
}

#[test]
fn cable_presentation_tension_keeps_endpoints_and_one_geometry_for_prefix_hover_and_loops() {
    for zoom in [0.25, 0.5, 1.0, 2.0, 4.0] {
        for (a, b) in [
            (pos2(40.0, 80.0), pos2(600.0, 300.0)),
            (pos2(600.0, 300.0), pos2(40.0, 80.0)),
            (pos2(10.0, 10.0), pos2(11.0, 10.0)),
            (pos2(10.0, 10.0), pos2(10.0, 10.0)),
        ] {
            for fan in [-20.0, 0.0, 20.0] {
                for tension in [0.0, 0.5, 1.0] {
                    let points = cable_curve_points(a, b, zoom, fan, tension);
                    assert_eq!((points[0], points[3]), (a, b));
                    assert!(points.iter().all(|p| p.is_finite()));
                    for t in [0.0, 0.1, 0.5, 0.9, 1.0] {
                        let point = evaluate(points, t);
                        assert!(cable_curve_near(points, point, (4.5 * zoom).max(3.0)));
                        if tension == 1.0 {
                            let offset = point - a.lerp(b, t);
                            assert!(offset.y >= -0.001 && offset.y <= 12.0 * zoom + 0.001);
                            assert!(
                                offset.x.abs() <= (a.distance(b) * 0.0225).min(1.8 * zoom) + 0.001
                            );
                        }
                        let prefix = cable_curve_prefix(points, t);
                        for u in [0.0, 0.2, 0.7, 1.0] {
                            assert!(evaluate(prefix, u).distance(evaluate(points, u * t)) < 0.001);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn cable_presentation_actual_draft_uses_selected_tension_and_full_opacity() {
    let (patch, snapshot) = fixture();
    let ctx = egui::Context::default();
    let mut rack = rack(snapshot);
    rack.set_cable_presentation(CablePresentation {
        opacity: 0.0,
        tension: 1.0,
        plug_brightness: 1.0,
    });
    frame_output(&ctx, &mut rack, &patch, 0.0, vec![]);
    rack.pending = Some(PendingCable::at_revision(
        CableEnd::Output {
            node: "p".into(),
            port: "out".into(),
        },
        &patch,
        0,
    ));
    let pointer = pos2(850.0, 510.0);
    let output = frame_output(
        &ctx,
        &mut rack,
        &patch,
        0.1,
        vec![egui::Event::PointerMoved(pointer)],
    )
    .1;
    let curves = curves(&output);
    assert_eq!(
        curves.len(),
        3,
        "only the full-opacity active draft remains"
    );
    let main = &curves[1];
    assert_eq!(main.points[3], pointer);
    assert_eq!(
        main.points,
        cable_curve_points(main.points[0], pointer, rack.zoom, 0.0, 1.0)
    );
    let middle = evaluate(main.points, 0.5);
    assert!(
        middle.y > main.points[0].lerp(pointer, 0.5).y,
        "taut draft retains a gravity bow"
    );
    for t in [0.2, 0.5, 0.8] {
        assert!(cable_curve_near(main.points, evaluate(main.points, t), 3.0));
    }
    let egui::epaint::ColorMode::Solid(color) = main.stroke.color else {
        panic!("solid")
    };
    assert!(color.a() > 150);
}

#[test]
fn cable_presentation_animated_tip_and_shadow_use_the_current_nondefault_geometry() {
    for tension in [0.0, 1.0] {
        for opacity in [0.0, 0.25, 1.0] {
            let ctx = egui::Context::default();
            let lighting = crate::lighting::RoomLighting::default();
            let from = pos2(80.0, 400.0);
            let to = pos2(700.0, 100.0);
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                paint_motion_cable_curves(
                    &ctx.layer_painter(egui::LayerId::background()),
                    from,
                    to,
                    CABLES[0],
                    1.0,
                    true,
                    20.0,
                    Some(crate::cable_animation::Phase {
                        progress: 0.4,
                        settle: 0.0,
                        fade: 1.0,
                        reduced: false,
                    }),
                    &lighting,
                    CablePresentation {
                        opacity,
                        tension,
                        plug_brightness: 1.0,
                    },
                );
            });
            let expected = evaluate(cable_curve_points(from, to, 1.0, 20.0, tension), 0.4);
            assert!(output.shapes.iter().any(
                |s| matches!(&s.shape,egui::Shape::Circle(c) if c.center.distance(expected)<0.001)
            ));
            if opacity == 0.0 {
                assert!(curves(&output).is_empty());
            } else {
                assert_eq!(curves(&output).len(), 6);
            }
        }
    }
}

#[test]
fn cable_presentation_high_tension_sag_is_monotonic_bounded_and_screen_scale_consistent() {
    for zoom in [0.25, 0.5, 1.0, 2.0, 4.0] {
        for (a, b) in [
            (pos2(0.0, 0.0), pos2(600.0, 120.0)),
            (pos2(600.0, 120.0), pos2(0.0, 0.0)),
            (pos2(0.0, 0.0), pos2(0.0, 300.0)),
            (pos2(0.0, 300.0), pos2(0.0, 0.0)),
            (pos2(0.0, 0.0), pos2(1.0, 0.0)),
            (pos2(0.0, 0.0), pos2(0.0, 0.0)),
        ] {
            let pan = vec2(20.0, 50.0);
            let from = (a.to_vec2() * zoom + pan).to_pos2();
            let to = (b.to_vec2() * zoom + pan).to_pos2();
            for fan in [-20.0, 0.0, 20.0] {
                let mut previous = f32::INFINITY;
                for tension in [0.5, 0.6, 0.7, 0.8, 0.9, 1.0] {
                    let points = cable_curve_points(from, to, zoom, fan, tension);
                    let sag = evaluate(points, 0.5).y - from.lerp(to, 0.5).y;
                    assert!(
                        sag >= -0.001 && sag <= previous + 0.001,
                        "zoom={zoom} tension={tension}"
                    );
                    previous = sag;
                    if tension == 1.0 {
                        if a != b {
                            assert!(sag > 0.0);
                        }
                        assert!(sag <= (from.distance(to) * 0.04125).min(12.0 * zoom) + 0.001);
                        // Taut chord progress remains monotonic: no short/reverse loops.
                        let direction = to - from;
                        let mut progress = -0.001;
                        for i in 0..=32 {
                            let point = evaluate(points, i as f32 / 32.0);
                            let projected = (point - from).dot(direction);
                            assert!(projected + 0.001 >= progress);
                            progress = projected;
                        }
                        let world = cable_curve_points(a, b, 1.0, fan, tension);
                        for (point, expected) in points.iter().zip(world) {
                            assert!(
                                point.distance((expected.to_vec2() * zoom + pan).to_pos2()) < 0.001
                            );
                        }
                    }
                }
                // Default is the exact original control construction.
                let sag = (from.distance(to) * 0.29 + 38.0 * zoom).clamp(45.0 * zoom, 195.0 * zoom);
                assert_eq!(
                    cable_curve_points(from, to, zoom, fan, 0.5),
                    [
                        from,
                        from + vec2(fan * zoom, sag),
                        to + vec2(fan * zoom, sag),
                        to
                    ]
                );
            }
        }
    }
}
