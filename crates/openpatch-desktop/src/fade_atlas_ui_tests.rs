use super::*;

fn fade_atlas_shapes(
    shape: &egui::Shape,
    labels: &mut Vec<(String, Rect)>,
    curves: &mut Vec<Vec<Pos2>>,
) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                fade_atlas_shapes(child, labels, curves);
            }
        }
        egui::Shape::Text(t) => labels.push((
            t.galley.text().into(),
            t.galley.rect.translate(t.pos.to_vec2()),
        )),
        egui::Shape::Path(path)
            if path.points.len() == 49
                && [
                    Color32::from_rgb(229, 183, 103),
                    Color32::from_rgb(109, 192, 177),
                ]
                .iter()
                .any(|color| path.stroke.color == egui::epaint::ColorMode::Solid(*color)) =>
        {
            curves.push(path.points.clone())
        }
        _ => {}
    }
}

#[test]
fn fade_preview_print_and_readouts_clear_controls_and_socket_rims_at_all_scales() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "fade")
        .unwrap();
    for zoom in [0.25, 0.42, 1.0] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut preview = ModulePreview::new(module);
        assert_eq!(preview.size(), vec2(240.0, 396.0));
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
        let mut curves = Vec::new();
        for shape in &output.shapes {
            fade_atlas_shapes(&shape.shape, &mut labels, &mut curves);
        }
        assert_eq!(curves.len(), 2, "configured A and B gain curves");
        for (label, text_rect) in &labels {
            assert!(
                bounds.expand(0.5).contains_rect(*text_rect),
                "{zoom}: {label} escapes panel"
            );
            for input in [true, false] {
                let ports = module[if input { "inputs" } else { "outputs" }]
                    .as_array()
                    .unwrap();
                for index in 0..ports.len() {
                    let at = bounds.min
                        + port_position("fade", input, index, ports.len(), 240.0).to_vec2() * zoom;
                    assert!(
                        text_rect.distance_to_pos(at)
                            >= (13.0 * port_material_scale("fade") - 0.1) * zoom,
                        "{zoom}: {label} touches socket rim"
                    );
                }
            }
            for name in ["position", "cv_amount"] {
                let (at, radius) = atlas_fade_control(name);
                let target = Rect::from_center_size(
                    bounds.min + at.to_vec2() * zoom,
                    Vec2::splat((radius + 3.0) * 2.0 * zoom),
                );
                assert!(
                    !text_rect.shrink(0.1 * zoom).intersects(target),
                    "{zoom}: {label} crosses {name} knob target"
                );
            }
        }
        for name in ["position", "cv_amount"] {
            let center = bounds.min + atlas_fade_value_center(name).to_vec2() * zoom;
            assert!(
                labels
                    .iter()
                    .any(|(_, r)| r.center().distance(center) < zoom),
                "{zoom}: missing actual {name} readout"
            );
        }
        for (index, (label, text_rect)) in labels.iter().enumerate() {
            for (other, other_rect) in labels.iter().skip(index + 1) {
                assert!(
                    !text_rect
                        .shrink(0.2 * zoom)
                        .intersects(other_rect.shrink(0.2 * zoom)),
                    "{zoom}: {label} overlaps {other}"
                );
            }
        }
    }
}

#[test]
fn fade_rendered_gain_law_tracks_saved_law_and_never_invents_live_cv() {
    for law in [0, 1] {
        let mut previous = None;
        for amount in [-1.0, 1.0] {
            let patch: Patch = serde_json::from_value(json!({
                "version":2,"nodes":[{"id":"fade","kind":"fade",
                    "params":{"position":0.25,"cv_amount":amount,"law":law}}]
            }))
            .unwrap();
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut rack = Rack::default();
            let (_, output) =
                frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 19);
            let mut labels = Vec::new();
            let mut curves = Vec::new();
            for shape in &output.shapes {
                fade_atlas_shapes(&shape.shape, &mut labels, &mut curves);
            }
            for expected in [
                "CONFIGURED OUT GAINS · CV = 0",
                "COMPLEMENT: A ↔ B",
                "x = clamp(POS + CV / 10 × AMOUNT, 0, 1)",
            ] {
                assert!(
                    labels.iter().any(|(label, _)| label == expected),
                    "missing {expected}"
                );
            }
            assert!(rack.module_lights.is_empty());
            assert_eq!(curves.len(), 2);
            let midpoint_gain = if law == 0 { 0.5 } else { 0.5_f32.sqrt() };
            for curve in &curves {
                let top = curve[0].y.min(curve[48].y);
                let bottom = curve[0].y.max(curve[48].y);
                let visible_gain = (bottom - curve[24].y) / (bottom - top);
                assert!((visible_gain - midpoint_gain).abs() < 0.0001);
            }
            if let Some(previous) = previous {
                assert_eq!(
                    curves, previous,
                    "CV amount cannot turn a saved-position guide into a live display"
                );
            }
            previous = Some(curves);
            rack.request_precision_control_for_smoke("fade", "cv_amount");
            frame_revision_state(&ctx, &mut rack, &patch, 0.1, vec![], true, true, 19);
            let edit = rack
                .editing
                .as_ref()
                .expect("saved signed amount remains editable offline");
            assert_eq!((edit.min, edit.max, edit.revision), (-100.0, 100.0, 19));
            assert_eq!(edit.text.parse::<f32>().unwrap(), amount * 100.0);
        }
    }
}
