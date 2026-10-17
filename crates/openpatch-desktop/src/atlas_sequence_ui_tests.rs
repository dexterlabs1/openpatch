use super::*;
fn collect(shape: &egui::Shape, texts: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(v) => {
            for s in v {
                collect(s, texts)
            }
        }
        egui::Shape::Text(t) => texts.push((
            t.galley.text().into(),
            t.galley.rect.translate(t.pos.to_vec2()),
        )),
        _ => {}
    }
}
#[test]
fn sequence_atlas_actual_print_clears_all_named_socket_rims() {
    let catalog = openpatch_core::catalog();
    for kind in ["seq3", "stock_seq3"] {
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
            let panel = Rect::from_min_size(pos2(24.0, 12.0), preview.size() * zoom);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1300.0, 800.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| preview.show(ui, panel, &catalog));
                },
            );
            let mut texts = vec![];
            for s in output.shapes {
                collect(&s.shape, &mut texts);
            }
            for (label, bounds) in texts {
                assert!(
                    panel.expand(0.5).contains_rect(bounds),
                    "{kind}/{zoom}: {label} outside panel {bounds:?}"
                );
                if ["CV1", "CV2", "CV3"].contains(&label.as_str())
                    && bounds.center().x < panel.left() + 220.0 * zoom
                {
                    for name in ["tempo", "steps", "tempo_amount", "steps_amount"] {
                        let well = Rect::from_center_size(
                            panel.min + atlas_sequence_value_center(name).to_vec2() * zoom,
                            vec2(78.0, 14.0) * zoom,
                        );
                        assert!(
                            !bounds.intersects(well),
                            "{kind}/{zoom}: {label} touches {name} value well"
                        );
                    }
                }
                for input in [true, false] {
                    let ports = module[if input { "inputs" } else { "outputs" }]
                        .as_array()
                        .unwrap();
                    for (i, port) in ports.iter().enumerate() {
                        let at = panel.min
                            + port_position(kind, input, i, ports.len(), panel_width(kind))
                                .to_vec2()
                                * zoom;
                        let nearest = pos2(
                            at.x.clamp(bounds.left(), bounds.right()),
                            at.y.clamp(bounds.top(), bounds.bottom()),
                        );
                        let rim = if input { 10.3 } else { 13.0 } * port_material_scale(kind);
                        assert!(
                            nearest.distance(at) >= (rim - 0.1) * zoom,
                            "{kind}/{zoom}: {label} touches {port} rim {bounds:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn sequence_saved_and_runtime_buttons_never_emit_commands_from_preview() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "seq3")
        .unwrap();
    let node: Node = serde_json::from_value(json!({"id":"seq","kind":"seq3"})).unwrap();
    for interactive in [false, true] {
        for local in [
            pos2(61.0, 277.0),
            pos2(154.0, 277.0),
            pos2(47.5, 311.5),
            pos2(238.0, 73.0),
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack {
                performance_available: true,
                ..Default::default()
            };
            let at = local + vec2(24.0, 12.0);
            let mut accepted = None;
            for frame_index in 0..3 {
                let events = match frame_index {
                    1 => vec![egui::Event::PointerMoved(at), button(at, true)],
                    2 => vec![button(at, false)],
                    _ => vec![],
                };
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 600.0))),
                        focused: true,
                        time: Some(frame_index as f64 * 0.1),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE)
                            .show(ctx, |ui| {
                                accepted = rack.sequence_panel(
                                    ui,
                                    &node,
                                    Some(module),
                                    Rect::from_min_size(pos2(24.0, 12.0), vec2(780.0, 396.0)),
                                    37,
                                    interactive,
                                );
                            });
                    },
                );
                if !interactive {
                    assert!(accepted.is_none(), "preview clicked {local:?}");
                }
            }
            if interactive {
                assert!(
                    matches!(accepted,Some(RackAction::Command(_,args)) if args["expected_revision"]==37),
                    "real control at {local:?} must remain usable"
                );
            }
        }
    }
}
