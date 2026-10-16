// Opt-in CPU rendering of the real shared panel renderer. No window, graphics
// context or audio device is opened. Used for focused design reviews on Mac.
use super::*;

#[test]
#[ignore = "writes a headless design review PNG to OPENPATCH_PANEL_PREVIEW"]
fn export_panel_design_preview() {
    let path = std::env::var("OPENPATCH_PANEL_PREVIEW").expect("set PNG output path");
    let catalog = openpatch_core::catalog();
    let defaults = "adsr_eg,adsr,lfo,analog_vca,lowpass,svf_filter";
    let selection = std::env::var("OPENPATCH_PANEL_KINDS").unwrap_or_else(|_| defaults.into());
    let kinds: Vec<_> = selection.split(',').collect();
    let mut previews: Vec<_> = kinds
        .iter()
        .map(|kind| {
            ModulePreview::new(
                catalog["modules"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|module| module["kind"] == *kind)
                    .unwrap(),
            )
        })
        .collect();
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let size = vec2(
        previews.iter().map(|p| p.size().x + 12.0).sum::<f32>() + 28.0,
        650.0,
    );
    let mut texture = None;
    let mut output = None;
    for frame in 0..2 {
        let result = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                time: Some(frame as f64),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        ui.painter()
                            .rect_filled(ui.max_rect(), 0.0, Color32::from_rgb(28, 31, 31));
                        for (scale, y) in [(1.0, 26.0), (0.42, 456.0)] {
                            let mut x = 20.0;
                            for preview in &mut previews {
                                let bounds =
                                    Rect::from_min_size(pos2(x, y), preview.size() * scale);
                                ui.push_id(
                                    (scale.to_bits(), preview.patch.nodes[0].kind.clone()),
                                    |ui| {
                                        preview.show(ui, bounds, &catalog);
                                    },
                                );
                                x = bounds.right() + 12.0;
                            }
                        }
                    });
            },
        );
        for (id, delta) in &result.textures_delta.set {
            assert_eq!(*id, egui::TextureId::Managed(0));
            let egui::ImageData::Color(image) = &delta.image;
            if let Some([x, y]) = delta.pos {
                let atlas: &mut egui::ColorImage = texture.as_mut().unwrap();
                for row in 0..image.size[1] {
                    for column in 0..image.size[0] {
                        atlas[(x + column, y + row)] = image[(column, row)];
                    }
                }
            } else {
                texture = Some((**image).clone());
            }
        }
        output = Some(result);
    }
    let output = output.unwrap();
    for clipped in &output.shapes {
        if let egui::Shape::Text(text) = &clipped.shape {
            assert!(
                !text.galley.text().contains("use of widget ID"),
                "simultaneous panel previews reused a widget identity"
            );
        }
    }
    let atlas = texture.unwrap();
    let mut image =
        image::RgbaImage::from_pixel(size.x as u32, size.y as u32, image::Rgba([28, 31, 31, 255]));
    for primitive in ctx.tessellate(output.shapes, output.pixels_per_point) {
        let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
            panic!("panel preview contains a GPU callback");
        };
        assert_eq!(mesh.texture_id, egui::TextureId::Managed(0));
        for indices in mesh.indices.as_chunks::<3>().0 {
            let v = std::array::from_fn(|i| mesh.vertices[indices[i] as usize]);
            raster_triangle(&mut image, &atlas, primitive.clip_rect, v);
        }
    }
    image.save(path).unwrap();
}

fn raster_triangle(
    image: &mut image::RgbaImage,
    atlas: &egui::ColorImage,
    clip: Rect,
    v: [egui::epaint::Vertex; 3],
) {
    let edge = |a: Pos2, b: Pos2, p: Pos2| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let area = edge(v[0].pos, v[1].pos, v[2].pos);
    if area.abs() < 1e-6 {
        return;
    }
    let bounds = Rect::from_min_max(
        pos2(
            v.iter().map(|v| v.pos.x).fold(f32::INFINITY, f32::min),
            v.iter().map(|v| v.pos.y).fold(f32::INFINITY, f32::min),
        ),
        pos2(
            v.iter().map(|v| v.pos.x).fold(f32::NEG_INFINITY, f32::max),
            v.iter().map(|v| v.pos.y).fold(f32::NEG_INFINITY, f32::max),
        ),
    )
    .intersect(clip)
    .intersect(Rect::from_min_size(
        Pos2::ZERO,
        vec2(image.width() as f32, image.height() as f32),
    ));
    for y in bounds.top().floor().max(0.0) as u32..bounds.bottom().ceil().max(0.0) as u32 {
        for x in bounds.left().floor().max(0.0) as u32..bounds.right().ceil().max(0.0) as u32 {
            let p = pos2(x as f32 + 0.5, y as f32 + 0.5);
            let w = [
                edge(v[1].pos, v[2].pos, p) / area,
                edge(v[2].pos, v[0].pos, p) / area,
                edge(v[0].pos, v[1].pos, p) / area,
            ];
            // Half-open edges keep adjacent mesh triangles from blending twice.
            if w.iter().any(|w| *w < 0.0) {
                continue;
            }
            if w.iter().enumerate().any(|(i, weight)| {
                if *weight != 0.0 {
                    return false;
                }
                let a = v[(i + 1) % 3].pos;
                let b = v[(i + 2) % 3].pos;
                let (a, b) = if area > 0.0 { (a, b) } else { (b, a) };
                b.y < a.y || (b.y == a.y && b.x > a.x)
            }) {
                continue;
            }
            let uv = v[0].uv.to_vec2() * w[0] + v[1].uv.to_vec2() * w[1] + v[2].uv.to_vec2() * w[2];
            let tx = (uv.x * atlas.size[0] as f32)
                .floor()
                .clamp(0.0, (atlas.size[0] - 1) as f32) as usize;
            let ty = (uv.y * atlas.size[1] as f32)
                .floor()
                .clamp(0.0, (atlas.size[1] - 1) as f32) as usize;
            let texel = atlas[(tx, ty)].to_array();
            let mut source = [0.0; 4];
            for channel in 0..4 {
                source[channel] = (0..3)
                    .map(|i| v[i].color[channel] as f32 * w[i])
                    .sum::<f32>()
                    * texel[channel] as f32
                    / 255.0;
            }
            let pixel = image.get_pixel_mut(x, y);
            for channel in 0..3 {
                pixel[channel] = (source[channel]
                    + pixel[channel] as f32 * (1.0 - source[3] / 255.0))
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
    }
}
