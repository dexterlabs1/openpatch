use super::*;

fn manual_atlas_labels(shape: &egui::Shape, labels: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                manual_atlas_labels(child, labels);
            }
        }
        egui::Shape::Text(t) => labels.push((
            t.galley.text().into(),
            t.galley.rect.translate(t.pos.to_vec2()),
        )),
        _ => {}
    }
}

#[test]
fn manual_atlas_actual_preview_print_clears_other_text_and_socket_rims() {
    let catalog = openpatch_core::catalog();
    for kind in ["logic", "gates", "pulses", "push"] {
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        for zoom in [0.25, 0.42, 1.0] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut preview = ModulePreview::new(module);
            let bounds = Rect::from_min_size(pos2(24.0, 12.0), preview.size() * zoom);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 800.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| preview.show(ui, bounds, &catalog));
                },
            );
            let mut labels = Vec::new();
            for shape in &output.shapes {
                manual_atlas_labels(&shape.shape, &mut labels);
            }
            for (index, (label, text_rect)) in labels.iter().enumerate() {
                assert!(
                    bounds.expand(0.5).contains_rect(*text_rect),
                    "{kind} {zoom}: {label} escapes panel"
                );
                for input in [true, false] {
                    let ports = module[if input { "inputs" } else { "outputs" }]
                        .as_array()
                        .unwrap();
                    for port in 0..ports.len() {
                        let at = bounds.min
                            + port_position(kind, input, port, ports.len(), panel_width(kind))
                                .to_vec2()
                                * zoom;
                        assert!(
                            text_rect.distance_to_pos(at)
                                >= (13.0 * port_material_scale(kind) - 0.1) * zoom,
                            "{kind} {zoom}: {label} touches socket {input}/{port}"
                        );
                    }
                }
                for (other, other_rect) in labels.iter().skip(index + 1) {
                    assert!(
                        !text_rect
                            .shrink(0.2 * zoom)
                            .intersects(other_rect.shrink(0.2 * zoom)),
                        "{kind} {zoom}: {label} overlaps {other}"
                    );
                }
            }
            for expected in if kind == "pulses" {
                vec!["1 ms", "HELD", "AUDIO OFFLINE"]
            } else if kind == "logic" {
                vec!["HIGH ≥1 V · OUTPUTS 0 / 10 V", "B · INPUT OR BUTTON"]
            } else if kind == "gates" {
                vec!["LENGTH", "RESET", "TIME = LENGTH × 2^CV"]
            } else {
                vec![
                    "(PUSH OR CV) XOR HOLD",
                    "MOMENTARY",
                    "TOGGLE LATCH",
                    "GATE RISE → 1 ms / 10 V TRIG",
                    "HOLD: PERSISTENT LATCH",
                    "AUDIO OFFLINE",
                ]
            } {
                assert!(
                    labels.iter().any(|(label, _)| label == expected),
                    "missing {kind} guide: {expected}"
                );
            }
        }
    }
}

// Select a physical role by its exact panel center AND calibrated lens radius.
// Port lenses, bezels and unrelated circles cannot satisfy this selector.
fn manual_atlas_lens(output: &egui::FullOutput, at: Pos2, radius: f32) -> Color32 {
    let mut colors = Vec::new();
    fn visit(shape: &egui::Shape, at: Pos2, radius: f32, colors: &mut Vec<Color32>) {
        match shape {
            egui::Shape::Vec(children) => {
                for child in children {
                    visit(child, at, radius, colors);
                }
            }
            egui::Shape::Circle(c)
                if c.center.distance(at) < 0.01 && (c.radius - radius).abs() < 0.01 =>
            {
                colors.push(c.fill)
            }
            _ => {}
        }
    }
    for shape in &output.shapes {
        visit(&shape.shape, at, radius, &mut colors);
    }
    assert_eq!(colors.len(), 1, "one actual observed lamp lens at {at:?}");
    colors[0]
}

fn manual_atlas_cap(output: &egui::FullOutput, target: Rect) -> Color32 {
    let colors: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r)
                if r.rect.min.distance(target.min) < 0.01
                    && r.rect.max.distance(target.max) < 0.01 =>
            {
                Some(r.fill)
            }
            _ => None,
        })
        .collect();
    assert_eq!(colors.len(), 1, "one actual physical button cap");
    colors[0]
}

#[test]
fn manual_atlas_lamps_remain_observed_and_hold_latch_does_not_depress_button() {
    for (kind, x, y, radius, count, index, button_x, button_y, size) in [
        (
            "pulses",
            132.0,
            100.0,
            2.8,
            10,
            0,
            92.0,
            100.0,
            vec2(62.0, 21.0),
        ),
        (
            "push",
            131.0,
            265.0,
            4.4,
            2,
            1,
            131.0,
            209.0,
            vec2(72.0, 48.0),
        ),
    ] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let patch: Patch =
            serde_json::from_value(json!({"version":2,"nodes":[{"id":"manual","kind":kind}]}))
                .unwrap();
        let node = &patch.nodes[0];
        let saved_params = node.params.clone();
        let origin = pos2(24.0, 12.0);
        let mut rack = Rack {
            performance_available: true,
            ..Rack::default()
        };
        let render = |rack: &mut Rack| {
            ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let rect = Rect::from_min_size(origin, vec2(panel_width(kind), PANEL_HEIGHT));
                    let action = if kind == "pulses" {
                        rack.pulses_panel(ui, node, rect, false)
                    } else {
                        rack.push_panel(ui, node, rect, false)
                    };
                    assert!(action.is_none());
                });
            })
        };
        let absent = render(&mut rack);
        let mut labels = Vec::new();
        for shape in &absent.shapes {
            manual_atlas_labels(&shape.shape, &mut labels);
        }
        assert!(labels.iter().any(|(text, _)| text == "WAITING FOR ENGINE"));
        rack.module_controls.insert(
            "manual".into(),
            ModuleControlState {
                pressed: true,
                pressed_mask: 1 << index,
            },
        );
        let missing_lamps = render(&mut rack);
        let mut labels = Vec::new();
        for s in &missing_lamps.shapes {
            manual_atlas_labels(&s.shape, &mut labels);
        }
        assert!(labels.iter().any(|(t, _)| t == "LAMPS UNAVAILABLE"));
        rack.module_lights.insert(
            "manual".into(),
            ModuleLightState {
                count,
                ..Default::default()
            },
        );
        let zero = render(&mut rack);
        let mut labels = Vec::new();
        for shape in &zero.shapes {
            manual_atlas_labels(&shape.shape, &mut labels);
        }
        let expected = if kind == "pulses" {
            "10 V · RUNTIME CONTROLS"
        } else {
            "ENGINE · GATE / LATCH"
        };
        assert!(
            labels.iter().any(|(text, _)| text == expected),
            "observed zero must not be labeled unavailable"
        );
        let lamp_at = origin + vec2(x, y);
        let lens_radius = crate::lighting::lamp_radius(radius);
        assert_eq!(
            manual_atlas_lens(&absent, lamp_at, lens_radius),
            manual_atlas_lens(&zero, lamp_at, lens_radius),
            "a pressed mask cannot invent light"
        );
        let target = Rect::from_center_size(origin + vec2(button_x, button_y), size).shrink(1.6);
        let pressed_cap = manual_atlas_cap(&zero, target);
        rack.module_controls.insert(
            "manual".into(),
            ModuleControlState {
                pressed: false,
                pressed_mask: 0,
            },
        );
        rack.module_lights.get_mut("manual").unwrap().brightness[index][0] = 1.0;
        let lit = render(&mut rack);
        assert_ne!(
            manual_atlas_lens(&zero, lamp_at, lens_radius),
            manual_atlas_lens(&lit, lamp_at, lens_radius)
        );
        assert_ne!(
            pressed_cap,
            manual_atlas_cap(&lit, target),
            "observed lamp cannot hold a released physical button down"
        );
        assert!(rack.held_manual.is_none());
        assert_eq!(
            node.params, saved_params,
            "painting does not edit saved state"
        );
    }
}
