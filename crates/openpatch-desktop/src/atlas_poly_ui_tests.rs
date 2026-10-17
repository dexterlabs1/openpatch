use super::*;

fn poly_labels(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                poly_labels(shape, out);
            }
        }
        egui::Shape::Text(t) => out.push((
            t.galley.text().into(),
            t.galley.rect.translate(t.pos.to_vec2()),
        )),
        _ => {}
    }
}

#[test]
fn poly_family_actual_print_clears_enlarged_rims_at_overview_zoom() {
    let catalog = openpatch_core::catalog();
    for kind in ["poly_sum", "poly_split", "poly_merge", "poly_constant"] {
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
            let mut labels = vec![];
            for shape in output.shapes {
                poly_labels(&shape.shape, &mut labels);
            }
            for (label, rect) in labels {
                assert!(
                    bounds.expand(0.5).contains_rect(rect),
                    "{kind}/{zoom}: {label} outside panel"
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
                        let radius = if input { 10.3 } else { 13.0 } * port_material_scale(kind);
                        assert!(
                            rect.distance_to_pos(at) >= (radius - 0.1) * zoom,
                            "{kind}/{zoom}: {label} touches {port}"
                        );
                    }
                }
            }
        }
    }
}
