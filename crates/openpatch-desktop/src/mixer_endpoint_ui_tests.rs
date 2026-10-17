use super::*;

fn mixer_endpoint_labels(shape: &egui::Shape, labels: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                mixer_endpoint_labels(child, labels);
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
fn mixer_endpoint_actual_preview_print_clears_text_and_socket_rims() {
    let catalog = openpatch_core::catalog();
    for kind in ["vca_mix", "mixer", "output", "audio_out"] {
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        for zoom in [0.25, 0.42, 1.0] {
            for extreme in [false, true] {
                let ctx = egui::Context::default();
                crate::panel_skin::install_fonts(&ctx);
                let mut preview = ModulePreview::new(module);
                for (name, spec) in module["params"].as_object().unwrap() {
                    let field = if extreme { "max" } else { "min" };
                    preview.patch.nodes[0]
                        .params
                        .insert(name.clone(), spec[field].as_f64().unwrap() as f32);
                }
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
                    mixer_endpoint_labels(&shape.shape, &mut labels);
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
            }
        }
    }
}

#[test]
fn vca_strip_meters_use_observation_not_saved_gain_and_missing_is_not_zero() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"mix","kind":"vca_mix"}]}))
            .unwrap();
    let render = |rack: &Rack, node: &Node| {
        ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                rack.mixing_panel(
                    ui,
                    node,
                    Rect::from_min_size(pos2(24.0, 12.0), vec2(390.0, 396.0)),
                )
            });
        })
    };
    let colors = |output: &egui::FullOutput| {
        let mut result = Vec::new();
        for channel in 0..4 {
            for segment in 0..12 {
                let target = Rect::from_min_size(
                    pos2(
                        24.0 + 37.5 + channel as f32 * 75.0 - 25.0 + segment as f32 * 4.2,
                        12.0 + 180.0,
                    ),
                    vec2(3.1, 6.0),
                );
                let target = Rect::from_center_size(
                    target.center(),
                    vec2(target.width() * 1.2, target.height()),
                );
                let fills: Vec<_> = output
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
                assert_eq!(fills.len(), 1, "one real strip meter segment");
                result.push(fills[0]);
            }
        }
        result
    };
    let absent = render(&rack, &patch.nodes[0]);
    patch.nodes[0]
        .params
        .insert("level1".into(), std::f32::consts::SQRT_2);
    patch.nodes[0].params.insert("mix_level".into(), 2.0);
    let configured = render(&rack, &patch.nodes[0]);
    assert_eq!(
        colors(&absent),
        colors(&configured),
        "saved gain cannot invent activity"
    );
    rack.module_lights.insert(
        "mix".into(),
        ModuleLightState {
            count: 4,
            ..Default::default()
        },
    );
    let zero = render(&rack, &patch.nodes[0]);
    assert_eq!(colors(&absent), colors(&zero));
    for (out, expected) in [
        (&absent, "VU UNAVAILABLE"),
        (&zero, "POST-CV VU · −24…0 dB"),
    ] {
        let mut labels = Vec::new();
        for s in &out.shapes {
            mixer_endpoint_labels(&s.shape, &mut labels);
        }
        assert!(labels.iter().any(|(s, _)| s == expected));
    }
    rack.module_lights.get_mut("mix").unwrap().brightness[0][0] = 0.05;
    let active = render(&rack, &patch.nodes[0]);
    let before = colors(&zero);
    let after = colors(&active);
    assert_ne!(before[0], after[0]);
    assert_eq!(
        &before[1..],
        &after[1..],
        "weak measured state must not light unrelated strips"
    );
}
