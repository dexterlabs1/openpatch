//! Geometry and actual egui paint-list checks; no window or synthetic screenshot.
use super::*;

fn painted_fader(
    normalized: f32,
    active: bool,
    zoom: f32,
    pixels_per_point: f32,
    room: f32,
) -> (egui::Context, egui::FullOutput, Rect) {
    let ctx = egui::Context::default();
    let mut input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 800.0))),
        ..Default::default()
    };
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .native_pixels_per_point = Some(pixels_per_point);
    let track = parameter_fader_track(pos2(300.0, 300.0), 24.0, zoom);
    let output = ctx.run(input, |ctx| {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("generic-fader-paint"),
        ));
        let mut lighting = crate::lighting::RoomLighting::default();
        lighting.set(room);
        let start = lighting.begin(&painter);
        parameter_fader(
            &painter,
            track,
            normalized,
            active,
            family_accent("stock_vcf"),
            zoom,
        );
        lighting.finish(&painter, start);
    });
    assert!((output.pixels_per_point - pixels_per_point).abs() < 1e-6);
    (ctx, output, track)
}

#[test]
fn generic_fader_geometry_scales_once_and_has_vertical_endpoints() {
    for zoom in [0.25, 0.5, 1.0, 2.0, 4.0] {
        let center = pos2(200.0, 150.0);
        let track = parameter_fader_track(center, 24.0, zoom);
        assert_eq!(track.center(), center);
        assert_eq!(track.width(), 4.0 * zoom);
        assert_eq!(track.height(), 38.0 * zoom);
        let bottom = parameter_fader_thumb(track, 0.0, zoom);
        let middle = parameter_fader_thumb(track, 0.5, zoom);
        let top = parameter_fader_thumb(track, 1.0, zoom);
        assert_eq!(bottom.center(), pos2(center.x, track.bottom()));
        assert_eq!(middle.center(), center);
        assert_eq!(top.center(), pos2(center.x, track.top()));
        assert_eq!(middle.size(), vec2(20.0, 8.0) * zoom);
        assert!(top.center().y < middle.center().y && middle.center().y < bottom.center().y);
        let short = parameter_fader_track(center, 4.0, zoom);
        assert_eq!(short.height(), 20.0 * zoom);
    }
}

#[test]
fn generic_fader_values_are_bounded_and_never_poison_paint_geometry() {
    let track = parameter_fader_track(pos2(100.0, 100.0), 24.0, 1.0);
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -10.0] {
        assert_eq!(
            parameter_fader_thumb(track, value, 1.0),
            parameter_fader_thumb(track, 0.0, 1.0)
        );
    }
    assert_eq!(
        parameter_fader_thumb(track, 10.0, 1.0),
        parameter_fader_thumb(track, 1.0, 1.0)
    );
    for value in [f32::NAN, f32::INFINITY, -10.0, 0.5, 10.0] {
        let (ctx, output, _) = painted_fader(value, true, 1.0, 2.0, 1.0);
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        assert!(!primitives.is_empty());
        for primitive in primitives {
            let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                panic!("passive fader unexpectedly requires a paint callback");
            };
            assert!(mesh.vertices.iter().all(|v| v.pos.is_finite()));
            assert!(mesh.is_valid());
        }
    }
}

#[test]
fn generic_fader_actual_mesh_stays_inside_control_cell_at_zoom_and_pixel_scales() {
    for zoom in [0.25, 0.5, 1.0, 2.0, 4.0] {
        for pixels_per_point in [1.0, 1.5, 2.0] {
            for normalized in [0.0, 0.5, 1.0] {
                let (ctx, output, track) =
                    painted_fader(normalized, true, zoom, pixels_per_point, 1.0);
                let pixel_fringe = 2.0 / output.pixels_per_point;
                // Includes tick strokes, thumb, physical shadow and egui's AA
                // fringe. The original 24-point radius is a logical layout cell.
                let bounds = Rect::from_center_size(track.center(), vec2(48.0, 48.0) * zoom)
                    .expand(pixel_fringe);
                let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
                let mut vertices = 0;
                for primitive in primitives {
                    let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                        panic!("unexpected paint callback");
                    };
                    for vertex in mesh.vertices {
                        assert!(
                            bounds.contains(vertex.pos),
                            "zoom {zoom}, dpi {pixels_per_point}: {:?} outside {bounds:?}",
                            vertex.pos
                        );
                        vertices += 1;
                    }
                }
                assert!(vertices > 20, "fader paint unexpectedly empty");
            }
        }
    }
}

#[test]
fn generic_fader_configured_fill_follows_value_and_all_hardware_dims_in_darkroom() {
    let accent = family_accent("stock_vcf");
    for value in [0.0, 0.5, 1.0] {
        let (_, output, track) = painted_fader(value, false, 1.0, 1.0, 1.0);
        let configured: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect) if rect.fill == accent => Some(rect.rect),
                _ => None,
            })
            .collect();
        if value == 0.0 {
            assert!(configured.is_empty());
        } else {
            assert_eq!(configured.len(), 1);
            assert_eq!(configured[0].bottom(), track.bottom());
            assert_eq!(configured[0].height(), track.height() * value);
        }
    }
    for active in [false, true] {
        let (normal_ctx, normal, _) = painted_fader(0.75, active, 1.0, 2.0, 1.0);
        let (dark_ctx, dark, _) = painted_fader(0.75, active, 1.0, 2.0, 0.1);
        let normal = normal_ctx.tessellate(normal.shapes, normal.pixels_per_point);
        let dark = dark_ctx.tessellate(dark.shapes, dark.pixels_per_point);
        assert_eq!(normal.len(), dark.len());
        let mut normal_energy = 0_u64;
        let mut dark_energy = 0_u64;
        for (normal, dark) in normal.iter().zip(&dark) {
            let (egui::epaint::Primitive::Mesh(a), egui::epaint::Primitive::Mesh(b)) =
                (&normal.primitive, &dark.primitive)
            else {
                panic!("unexpected paint callback");
            };
            assert_eq!(a.indices, b.indices);
            assert_eq!(a.vertices.len(), b.vertices.len());
            for (a, b) in a.vertices.iter().zip(&b.vertices) {
                assert_eq!(a.pos, b.pos);
                assert_eq!(a.color.a(), b.color.a());
                for (normal, dark) in a.color.to_array()[..3].iter().zip(&b.color.to_array()[..3]) {
                    assert!(dark <= normal);
                    normal_energy += u64::from(*normal);
                    dark_energy += u64::from(*dark);
                }
            }
        }
        assert!(normal_energy > 0);
        // A parameter setting never becomes an emissive signal or a protected
        // telemetry meter, including its active interaction highlight.
        assert!(dark_energy * 5 < normal_energy);
    }
}
