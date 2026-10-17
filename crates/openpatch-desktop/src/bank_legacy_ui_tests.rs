use super::*;

fn bank_legacy_labels(shape: &egui::Shape, labels: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                bank_legacy_labels(child, labels);
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
fn bank_legacy_actual_preview_print_clears_text_and_socket_rims() {
    let catalog = openpatch_core::catalog();
    for kind in [
        "attenuverter8",
        "mutes10",
        "cv_mix",
        "oscillator",
        "constant",
        "octave",
    ] {
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
                    bank_legacy_labels(&shape.shape, &mut labels);
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
                if kind == "oscillator" && !extreme {
                    assert!(labels.iter().any(|(t, _)| t == "PHASE"));
                    assert!(labels.iter().any(|(t, _)| t == "HELD"));
                    assert!(!labels.iter().any(|(t, _)| t == "ONE CYCLE"));
                }
            }
        }
    }
}

#[test]
fn bank_normalled_volts_and_connected_zero_percent_are_distinct_in_actual_editor() {
    for (kind, parameter, input) in [
        ("attenuverter8", "gain8", "in3"),
        ("cv_mix", "level3", "in3"),
    ] {
        for connected in [false, true] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut rack = Rack::default();
            let mut patch:Patch=serde_json::from_value(json!({"version":2,"nodes":[{"id":"bank","kind":kind,"params":{parameter:0.5}},{"id":"zero","kind":"constant","params":{"value":0}}]})).unwrap();
            if connected {
                patch.cables.push(serde_json::from_value(json!({"id":"source","from":"zero","output":"out","to":"bank","input":input})).unwrap());
            }
            patch.compile().unwrap();
            let (_, out) = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]);
            let mut labels = Vec::new();
            for shape in &out.shapes {
                bank_legacy_labels(&shape.shape, &mut labels);
            }
            let expected = if connected { "+50.0%" } else { "+5.0 V" };
            assert!(
                labels.iter().any(|(t, _)| t == expected),
                "{kind}: actual displayed coefficient must retain its topology-dependent units"
            );
            let local = atlas_bank_value_center(kind, parameter);
            let at = pos2(24.0, 12.0) + local.to_vec2();
            frame(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![egui::Event::PointerMoved(at), button(at, true)],
            );
            frame(&ctx, &mut rack, &patch, 1.1, vec![button(at, false)]);
            let editor = rack
                .editing
                .as_ref()
                .expect("independent numeric well opens");
            assert_eq!(editor.parameter, parameter);
            assert_eq!(editor.display_scale, if connected { 100.0 } else { 10.0 });
            assert_eq!(editor.display_unit, if connected { "%" } else { "V" });
            assert_eq!(
                parameter_edit_raw_value(editor, if connected { -25.0 } else { -2.5 }),
                -0.25
            );
        }
    }
}
