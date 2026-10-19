use super::*;

fn view_atlas_labels(shape: &egui::Shape, labels: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                view_atlas_labels(child, labels);
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
fn view_atlas_actual_preview_print_clears_text_and_socket_rims() {
    let catalog = openpatch_core::catalog();
    for kind in ["scope", "audio_stereo", "poly_viz", "notes", "blank"] {
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
                    view_atlas_labels(&shape.shape, &mut labels);
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
fn narrow_blank_heading_and_mounts_remain_inside_the_actual_spacer() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "blank")
        .unwrap();
    for hp in [3, 4, 5, 7, 10] {
        for zoom in [0.25, 0.42, 1.0] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut preview = ModulePreview::new(module);
            preview.patch.nodes[0].content =
                Some(serde_json::from_value(json!({"type":"blank","width_hp":hp})).unwrap());
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
                view_atlas_labels(&s.shape, &mut texts);
            }
            assert!(!texts.is_empty());
            for (text, rect) in texts {
                assert!(
                    bounds.expand(0.5).contains_rect(rect),
                    "{hp}HP {zoom}: {text} clips"
                );
            }
            fn mounts(shape: &egui::Shape, out: &mut Vec<(Pos2, f32)>) {
                match shape {
                    egui::Shape::Vec(v) => {
                        for s in v {
                            mounts(s, out);
                        }
                    }
                    // The outer captive washer is present even when fine screw detail
                    // is omitted at thumbnail scales.
                    egui::Shape::Circle(c) if c.fill == Color32::from_rgb(45, 46, 43) => {
                        out.push((c.center, c.radius))
                    }
                    _ => {}
                }
            }
            let mut bolts = Vec::new();
            for s in &output.shapes {
                mounts(&s.shape, &mut bolts);
            }
            assert_eq!(bolts.len(), 4);
            for (at, radius) in bolts {
                assert!(
                    bounds.contains_rect(Rect::from_center_size(
                        at,
                        vec2(radius * 2.0, radius * 2.0)
                    )),
                    "{hp}HP mount leaves face"
                );
            }
        }
    }
}
