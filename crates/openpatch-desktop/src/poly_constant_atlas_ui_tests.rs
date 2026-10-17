use super::*;

fn poly_constant_atlas_shapes(
    shape: &egui::Shape,
    labels: &mut Vec<(String, Rect)>,
    fields: &mut Vec<Rect>,
) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                poly_constant_atlas_shapes(child, labels, fields);
            }
        }
        egui::Shape::Text(text) => labels.push((
            text.galley.text().into(),
            text.galley.rect.translate(text.pos.to_vec2()),
        )),
        egui::Shape::Rect(rect) => fields.push(rect.rect),
        _ => {}
    }
}

#[test]
fn poly_constant_preview_bank_text_fields_and_socket_clear_at_all_scales() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "poly_constant")
        .unwrap();
    assert_eq!(panel_width("poly_constant"), 360.0);
    assert_eq!(
        port_position("poly_constant", false, 0, 1, 360.0),
        pos2(180.0, 374.0)
    );
    for zoom in [0.25, 0.42, 1.0] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut preview = ModulePreview::new(module);
        assert_eq!(preview.size(), vec2(360.0, 396.0));
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
        let mut fields = Vec::new();
        for shape in &output.shapes {
            poly_constant_atlas_shapes(&shape.shape, &mut labels, &mut fields);
        }
        let socket = bounds.min + vec2(180.0, 374.0) * zoom;
        let rim = 13.0 * port_material_scale("poly_constant") * zoom;
        for (label, text_rect) in &labels {
            assert!(
                bounds.expand(0.5).contains_rect(*text_rect),
                "{zoom}: {label} escapes panel"
            );
            assert!(
                text_rect.distance_to_pos(socket) >= rim - 0.1 * zoom,
                "{zoom}: {label} touches output rim"
            );
        }
        let mut targets = Vec::new();
        for channel in 1..=16 {
            let name = format!("v{channel}");
            let center = bounds.min + atlas_poly_constant_control(&name).0.to_vec2() * zoom;
            let caption = format!("{channel:02}");
            assert!(
                labels.iter().any(|(label, rect)| label == &caption
                    && rect.center().distance(center - vec2(31.0, 0.0) * zoom) < zoom),
                "{zoom}: missing channel {channel} caption"
            );
            let value_center =
                bounds.min + atlas_poly_constant_value_center(&name).to_vec2() * zoom;
            let field = fields
                .iter()
                .find(|rect| {
                    rect.center().distance(value_center) < 0.1 * zoom
                        && (rect.width() - 62.0 * zoom).abs() < 0.1 * zoom
                })
                .unwrap_or_else(|| panic!("{zoom}: missing real {name} value field"));
            assert!(bounds.contains_rect(*field));
            assert!(field.distance_to_pos(socket) >= rim - 0.1 * zoom);
            let voltage = labels
                .iter()
                .find(|(label, rect)| {
                    label.contains('V') && rect.center().distance(value_center) < zoom
                })
                .unwrap_or_else(|| panic!("{zoom}: missing {name} voltage readout"));
            assert!(field.expand(0.5).contains_rect(voltage.1));
            let knob_target = Rect::from_center_size(center, vec2(42.0, 42.0) * zoom);
            let value_target = Rect::from_center_size(value_center, vec2(62.0, 19.0) * zoom);
            assert!(
                !knob_target.intersects(value_target),
                "{name}: knob and exact-entry targets overlap"
            );
            for previous in &targets {
                assert!(!knob_target.intersects(*previous));
                assert!(!value_target.intersects(*previous));
            }
            targets.extend([knob_target, value_target]);
        }
    }
}

#[test]
fn poly_constant_inactive_saved_voltage_remains_editable_without_live_state() {
    for channels in [0, 1, 16] {
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"bank","kind":"poly_constant",
                "params":{"channels":channels,"v16":-7.25}}]
        }))
        .unwrap();
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut rack = Rack::default();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 31);
        assert!(rack.module_lights.is_empty());
        assert!(rack.sequence_states.is_empty());
        rack.request_precision_control_for_smoke("bank", "v16");
        let (action, _) =
            frame_revision_state(&ctx, &mut rack, &patch, 0.1, vec![], true, true, 31);
        assert!(action.is_none());
        let edit = rack.editing.as_ref().expect("saved inactive cell can open");
        assert_eq!((&*edit.parameter, edit.revision), ("v16", 31));
        assert_eq!(edit.text.parse::<f32>().unwrap(), -7.25);
        assert_eq!((edit.min, edit.max), (-16.0, 16.0));
        assert!(rack.module_lights.is_empty());
        assert_eq!(patch.nodes[0].params["v16"], -7.25);
    }
}
