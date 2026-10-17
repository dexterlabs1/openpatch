use super::*;

fn mid_side_atlas_labels(shape: &egui::Shape, result: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                mid_side_atlas_labels(child, result);
            }
        }
        egui::Shape::Text(text) => result.push((
            text.galley.text().into(),
            text.galley.rect.translate(text.pos.to_vec2()),
        )),
        _ => {}
    }
}

#[test]
fn mid_side_actual_panels_explain_distinct_source_laws_without_live_state() {
    for (kind, law, normal) in [
        (
            "mid_side",
            "E,D = clamp(W × CV / 10, 0, 2) · NORMAL 10 V",
            "M / S NORMAL: ENCODER · INDEPENDENT",
        ),
        (
            "stock_mid_side",
            "E,D = max(W + CV / 5, 0) · NORMAL 0 V · UNCAPPED",
            "M / S NORMAL: ENCODER · INDEPENDENT",
        ),
    ] {
        for params in [json!({}), json!({"encode_width":0,"decode_width":2})] {
            let patch: Patch = serde_json::from_value(json!({
                "version":2,"nodes":[{"id":"matrix","kind":kind,"params":params}]
            }))
            .unwrap();
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut rack = Rack::default();
            let (_, output) =
                frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
            let mut texts = Vec::new();
            for shape in &output.shapes {
                mid_side_atlas_labels(&shape.shape, &mut texts);
            }
            for expected in [
                "ENCODER",
                "DECODER",
                "M = (L + R) / 2",
                "S = (L − R) / 2 × E",
                "L = M + S × D",
                "R = M − S × D",
                law,
                normal,
            ] {
                assert!(
                    texts.iter().any(|(label, _)| label == expected),
                    "{kind}: missing {expected} with params {params}"
                );
            }
            assert!(rack.module_lights.is_empty());
            assert!(rack.editing.is_none());
        }
    }
}

#[test]
fn mid_side_source_guides_clear_socket_rims_at_browser_and_rack_scales() {
    let catalog = openpatch_core::catalog();
    for kind in ["mid_side", "stock_mid_side"] {
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|module| module["kind"] == kind)
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
            let mut texts = Vec::new();
            for shape in &output.shapes {
                mid_side_atlas_labels(&shape.shape, &mut texts);
            }
            for (label, text_rect) in texts {
                if !(label.contains(" = ") || label.starts_with("M / S NORMAL:")) {
                    continue;
                }
                assert!(
                    bounds.expand(0.5).contains_rect(text_rect),
                    "{kind}/{zoom}: {label}"
                );
                for input in [true, false] {
                    let ports = module[if input { "inputs" } else { "outputs" }]
                        .as_array()
                        .unwrap();
                    for index in 0..ports.len() {
                        let at = bounds.min
                            + port_position(kind, input, index, ports.len(), 300.0).to_vec2()
                                * zoom;
                        assert!(
                            text_rect.distance_to_pos(at)
                                >= ((if input { 10.3 } else { 13.0 }) * port_material_scale(kind)
                                    - 0.1)
                                    * zoom,
                            "{kind}/{zoom}: {label} overlaps socket rim"
                        );
                    }
                }
            }
        }
    }
}
