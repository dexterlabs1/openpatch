use super::*;
fn labels(shape: &egui::Shape, texts: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                labels(child, texts);
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
fn voltage_family_labels_clear_named_socket_rims_and_parameter_targets() {
    let catalog = openpatch_core::catalog();
    for kind in [
        "attenuverter",
        "crossfade",
        "rescale",
        "quantizer",
        "stock_quantizer",
    ] {
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        for zoom in [0.25, 0.42, 1.0, 1.4] {
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
            let mut texts = Vec::new();
            for s in &output.shapes {
                labels(&s.shape, &mut texts);
            }
            for (label, rect) in &texts {
                assert!(
                    bounds.expand(0.5).contains_rect(*rect),
                    "{kind}/{zoom}: {label} outside panel {rect:?}"
                );
                for input in [true, false] {
                    let ports = module[if input { "inputs" } else { "outputs" }]
                        .as_array()
                        .unwrap();
                    for (index, port) in ports.iter().enumerate() {
                        let at = bounds.min
                            + port_position(kind, input, index, ports.len(), panel_width(kind))
                                .to_vec2()
                                * zoom;
                        let nearest = pos2(
                            at.x.clamp(rect.left(), rect.right()),
                            at.y.clamp(rect.top(), rect.bottom()),
                        );
                        assert!(
                            nearest.distance(at) >= (13.0 * port_material_scale(kind) - 0.1) * zoom,
                            "{kind}/{zoom}: {label} touches {port} rim"
                        );
                    }
                }
                // Text within actual choice/value fields is intentional. All
                // printed guides and captions must clear the rotary targets.
                for name in module["params"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                {
                    if matches!(
                        name,
                        "scale_mask" | "multiplier" | "reflect_min" | "reflect_max" | "law"
                    ) {
                        continue;
                    }
                    let (center, radius) = match kind {
                        "rescale" => atlas_rescale_control(name),
                        "quantizer" | "stock_quantizer" => atlas_quantizer_control(kind),
                        _ => atlas_voltage_control(kind, name),
                    };
                    let target = Rect::from_center_size(
                        bounds.min + center.to_vec2() * zoom,
                        Vec2::splat((radius + 3.0) * 2.0 * zoom),
                    );
                    assert!(
                        !rect.shrink(0.1 * zoom).intersects(target),
                        "{kind}/{zoom}: {label} crosses {name} rotary target"
                    );
                }
            }
        }
    }
}
#[test]
fn stock_quantizer_configured_guide_follows_effective_saved_bank() {
    let ctx = egui::Context::default();
    let patch:Patch=serde_json::from_value(json!({"version":2,"nodes":[{"id":"q","kind":"stock_quantizer","params":{"scale_mask":4095}}]})).unwrap();
    let mut rack = Rack::default();
    rack.set_stock_quantizer_states(BTreeMap::from([("q".into(), 0)]));
    let (_, output) = frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 8);
    let mut texts = Vec::new();
    for s in output.shapes {
        labels(&s.shape, &mut texts);
    }
    assert!(texts.iter().any(|(t, _)| t == "EMPTY → CHROMATIC"));
    assert!(!texts.iter().any(|(t, _)| t == "12 NOTES ENABLED"));
}
