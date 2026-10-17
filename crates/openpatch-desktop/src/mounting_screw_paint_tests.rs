use super::*;

#[test]
fn mounting_screw_remains_finite_seated_and_bounded_at_preview_scales() {
    for zoom in [0.25, 0.5, 0.65, 1.0, 2.0, 4.0] {
        let ctx = egui::Context::default();
        let center = pos2(100.0, 100.0);
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            mounting_screw(&ctx.debug_painter(), center, zoom);
        });
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        assert!(!primitives.is_empty());
        let bounds = Rect::from_center_size(center, vec2(12.0, 12.0) * zoom + vec2(2.0, 2.0));
        for primitive in primitives {
            let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                panic!("mounting screw requires no GPU callback");
            };
            assert!(mesh.is_valid());
            assert!(mesh
                .vertices
                .iter()
                .all(|v| v.pos.is_finite() && bounds.contains(v.pos)));
            // Reflective black hardware never contains additive/emissive colors.
            assert!(mesh.vertices.iter().all(|v| v.color.r() <= v.color.a()
                && v.color.g() <= v.color.a()
                && v.color.b() <= v.color.a()));
        }
    }
}

#[test]
#[ignore = "writes the requested headless material review contact sheet"]
fn mounting_screw_material_review() {
    let destination = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../native-test-output/black-screws");
    std::fs::create_dir_all(&destination).unwrap();
    let ctx = egui::Context::default();
    let size = vec2(720.0, 300.0);
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..Default::default()
        },
        |ctx| {
            let painter = ctx.debug_painter();
            painter.rect_filled(
                Rect::from_min_size(Pos2::ZERO, size),
                0.0,
                Color32::from_gray(27),
            );
            for (row, room) in [1.0, 0.25].into_iter().enumerate() {
                let y = 20.0 + row as f32 * 140.0;
                for (column, zoom) in [0.4, 1.0, 3.0, 8.0].into_iter().enumerate() {
                    let x = 15.0 + column as f32 * 175.0;
                    let rect = Rect::from_min_size(pos2(x, y), vec2(160.0, 100.0));
                    let mut lighting = crate::lighting::RoomLighting::default();
                    lighting.set(room);
                    let start = lighting.begin(&painter);
                    painter.rect_filled(rect, 0.0, Color32::from_rgb(238, 232, 216));
                    mounting_screw(&painter, rect.center(), zoom);
                    lighting.finish(&painter, start);
                    painter.text(
                        pos2(x, y + 105.0),
                        egui::Align2::LEFT_TOP,
                        format!("{zoom}x / room {room}"),
                        FontId::proportional(12.0),
                        Color32::WHITE,
                    );
                }
            }
        },
    );
    let mut capture = crate::headless_review::CpuCapture::default();
    capture.observe(&output);
    capture.save(
        &ctx,
        output,
        size,
        destination.join("material-contact-sheet.png"),
    );
}
