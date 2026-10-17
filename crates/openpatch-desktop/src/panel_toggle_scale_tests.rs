use super::*;

#[test]
fn saved_mode_button_geometry_does_not_expand_at_small_zoom() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    for zoom in [0.25, 0.35, 0.5, 0.71, 1.0, 2.0, 4.0] {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for (index, label) in ["LFM", "SOFT", "Average", "Invert"].into_iter().enumerate() {
                    let rect = Rect::from_min_size(
                        pos2(20.0, 20.0 + index as f32 * 32.0 * zoom),
                        vec2(95.0, 26.0) * zoom,
                    );
                    let response = crate::panel_skin::toggle_button(ui, rect, label, true, zoom);
                    assert!(
                        rect.expand(0.6).contains_rect(response.rect),
                        "{label} button grows beyond its panel at {zoom}: {:?} vs {rect:?}",
                        response.rect
                    );
                }
            });
        });
    }
}

#[test]
fn saved_mode_labels_stay_single_line_inside_their_scaled_controls() {
    let catalog = openpatch_core::catalog();
    let patch = Patch {
        version: 2,
        nodes: ["multi_vco", "multi_lfo", "mix6"]
            .into_iter()
            .map(|kind| Node {
                id: kind.into(),
                kind: kind.into(),
                content: None,
                score: None,
                bypassed: false,
                params: Default::default(),
            })
            .collect(),
        ..Patch::default()
    };
    for zoom in [0.25, 0.35, 0.5, 0.71, 1.0, 2.0, 4.0] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut rack = Rack {
            zoom,
            pan: Vec2::ZERO,
            ..Rack::default()
        };
        for (index, node) in patch.nodes.iter().enumerate() {
            rack.positions
                .insert(node.id.clone(), pos2(index as f32 * 320.0, 0.0));
        }
        let canvas = Rect::from_min_size(pos2(12.0, 12.0), vec2(3580.0, 1760.0));
        let mut paint = |time| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(3600.0, 1800.0))),
                    focused: true,
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let result = rack.render_modules(
                            ui,
                            &patch,
                            &catalog,
                            canvas,
                            patch.sample_rate,
                            0,
                            RenderMode::Interactive {
                                drag_interactive: true,
                                allow_manual_press: false,
                            },
                        );
                        assert!(result.action.is_none());
                    });
                },
            )
        };
        let _ = paint(0.0);
        let output = paint(0.1);
        for (node_index, x, y, label) in [
            (0, 20.0, 279.0, "Linear FM"),
            (0, 155.0, 279.0, "Soft sync"),
            (1, 20.0, 279.0, "±5 V"),
            (1, 155.0, 279.0, "Invert"),
            (2, 20.0, 199.0, "Average"),
            (2, 125.0, 199.0, "Invert"),
        ] {
            let width = if node_index == 2 { 95.0 } else { 125.0 };
            let height = if node_index == 2 { 26.0 } else { 25.0 };
            let control = Rect::from_min_size(
                canvas.min + vec2(node_index as f32 * 320.0 + x, y) * zoom,
                vec2(width, height) * zoom,
            );
            let text = output
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text)
                        if text.galley.text() == label
                            && (text.pos.x - control.center().x).abs() < width * zoom =>
                    {
                        Some(text)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing {label} at {zoom}"));
            assert_eq!(
                text.galley.rows.len(),
                1,
                "{label} must never stack vertically at zoom {zoom}"
            );
            let printed = Rect::from_min_size(text.pos, text.galley.size());
            assert!(
                control.expand(0.6).contains_rect(printed),
                "{label} escapes its scaled control at {zoom}: {printed:?} vs {control:?}"
            );
        }
    }
}
