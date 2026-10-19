//! Original installed Rack artwork. Files are rasterized once on the UI thread;
//! third-party panels never enter OpenPatch's native Studio skin.
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

struct Art {
    textures: BTreeMap<&'static str, egui::TextureHandle>,
}
fn texture(ctx: &egui::Context, path: &Path) -> Result<egui::TextureHandle, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let tree = resvg::usvg::Tree::from_data(&bytes, &resvg::usvg::Options::default())
        .map_err(|e| e.to_string())?;
    let scale = if path.file_stem().is_some_and(|s| s == "Plaits") {
        4.0
    } else {
        3.0
    };
    let width = (tree.size().width() * scale).ceil() as u32;
    let height = (tree.size().height() * scale).ceil() as u32;
    if width > 4096 || height > 4096 {
        return Err("Original panel artwork exceeds texture bounds".into());
    }
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).ok_or("Unable to allocate panel artwork")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let image =
        egui::ColorImage::from_rgba_premultiplied([width as usize, height as usize], pixmap.data());
    Ok(ctx.load_texture(
        format!("original-rack:{}", path.display()),
        image,
        egui::TextureOptions::LINEAR,
    ))
}
fn load(ctx: &egui::Context) -> Result<Art, String> {
    let path = std::env::var_os("OPENPATCH_RACK_HOST_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join("OpenPatch/rack-host.json")
        });
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let plugin = PathBuf::from(
        config["plugin"]
            .as_str()
            .ok_or("Missing Rack plugin path")?,
    );
    let components = PathBuf::from(config["system"].as_str().ok_or("Missing Rack resources")?)
        .join("res/ComponentLibrary");
    let mut textures = BTreeMap::new();
    textures.insert("Plaits", texture(ctx, &plugin.join("res/Plaits.svg"))?);
    textures.insert("Braids", texture(ctx, &plugin.join("res/Braids.svg"))?);
    for name in [
        "Rogan2SGray",
        "Rogan2SGray_fg",
        "Rogan2PSWhite",
        "Rogan2PSWhite_fg",
        "Rogan2PSGreen",
        "Rogan2PSGreen_fg",
        "Rogan2PSRed",
        "Rogan2PSRed_fg",
        "Rogan3PSWhite",
        "Rogan3PSWhite_fg",
        "Rogan1PSWhite",
        "Rogan1PSWhite_fg",
        "Trimpot",
        "Trimpot_bg",
        "PJ301M",
        "TL1105_0",
        "ScrewSilver",
    ] {
        textures.insert(name, texture(ctx, &components.join(format!("{name}.svg")))?);
    }
    Ok(Art { textures })
}
fn art(ctx: &egui::Context) -> Arc<Result<Art, String>> {
    let id = egui::Id::new("original-rack-plaits-artwork");
    if let Some(cached) = ctx.data(|d| d.get_temp::<Arc<Result<Art, String>>>(id)) {
        return cached;
    }
    let loaded = Arc::new(load(ctx));
    ctx.data_mut(|d| d.insert_temp(id, loaded.clone()));
    loaded
}
pub(crate) fn panel_rect(rect: Rect) -> Rect {
    Rect::from_center_size(
        rect.center(),
        Vec2::new(rect.width(), rect.width() * 380.0 / 180.0),
    )
}
pub(crate) fn point(rect: Rect, x: f32, y: f32) -> Pos2 {
    let r = panel_rect(rect);
    r.min + Vec2::new(x / 60.96 * r.width(), y / 128.69331 * r.height())
}
pub(crate) fn piece_rect(rect: Rect, x: f32, y: f32, size: f32) -> Rect {
    Rect::from_min_size(point(rect, x, y), Vec2::splat(size / 60.96 * rect.width()))
}
pub(crate) fn piece(painter: &egui::Painter, rect: Rect, name: &'static str, angle: f32) {
    let assets = art(painter.ctx());
    let Ok(assets) = assets.as_ref() else { return };
    let Some(texture) = assets.textures.get(name) else {
        return;
    };
    let mut mesh = egui::Mesh::with_texture(texture.id());
    let rotation = egui::emath::Rot2::from_angle(angle);
    for (position, uv) in [
        (rect.left_top(), egui::pos2(0.0, 0.0)),
        (rect.right_top(), egui::pos2(1.0, 0.0)),
        (rect.right_bottom(), egui::pos2(1.0, 1.0)),
        (rect.left_bottom(), egui::pos2(0.0, 1.0)),
    ] {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: rect.center() + rotation * (position - rect.center()),
            uv,
            color: Color32::WHITE,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}
pub(crate) fn background(painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(rect, 0.0, Color32::from_rgb(42, 44, 43));
    let assets = art(painter.ctx());
    if let Err(error) = assets.as_ref() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "PLAITS\nOriginal panel unavailable",
            egui::FontId::proportional(10.0),
            Color32::LIGHT_GRAY,
        );
        let _ = error;
        return;
    }
    piece(painter, panel_rect(rect), "Plaits", 0.0);
    for (x, y) in [(5.08, 0.0), (50.8, 0.0), (5.08, 123.61), (50.8, 123.61)] {
        piece(painter, piece_rect(rect, x, y, 5.08), "ScrewSilver", 0.0);
    }
    for x in [3.31381, 14.75983, 26.20655, 37.65257, 49.0986] {
        for y in [92.48067, 107.08103] {
            piece(painter, piece_rect(rect, x, y, 8.0264), "PJ301M", 0.0);
        }
    }
}

pub(crate) fn port_position(input: bool, index: usize) -> Pos2 {
    let (x, y) = if input {
        match index {
            0 => (26.20655, 107.08103),
            1 => (3.31381, 107.08103),
            2 => (49.0986, 92.48067),
            3 => (14.75983, 92.48067),
            4 => (37.65257, 92.48067),
            5 => (26.20655, 92.48067),
            _ => (14.75983, 107.08103),
        }
    } else {
        (if index == 0 { 37.65257 } else { 49.0986 }, 107.08103)
    };
    point(
        Rect::from_min_size(Pos2::ZERO, Vec2::new(180.0, 396.0)),
        x + 4.0132,
        y + 4.0132,
    )
}

pub(crate) fn braids_rect(rect: Rect) -> Rect {
    Rect::from_center_size(
        rect.center(),
        Vec2::new(rect.width(), rect.width() * 380.0 / 240.0),
    )
}
pub(crate) fn braids_point(rect: Rect, x: f32, y: f32) -> Pos2 {
    braids_rect(rect).min + egui::vec2(x, y) * (rect.width() / 240.0)
}
pub(crate) fn braids_piece_rect(rect: Rect, x: f32, y: f32, size: f32) -> Rect {
    Rect::from_min_size(
        braids_point(rect, x, y),
        Vec2::splat(size * rect.width() / 240.0),
    )
}
pub(crate) fn braids_background(painter: &egui::Painter, rect: Rect) {
    painter.rect_filled(rect, 0.0, Color32::from_rgb(42, 44, 43));
    piece(painter, braids_rect(rect), "Braids", 0.0);
    for (x, y) in [(15., 0.), (210., 0.), (15., 365.), (210., 365.)] {
        piece(
            painter,
            braids_piece_rect(rect, x, y, 15.),
            "ScrewSilver",
            0.0,
        );
    }
    for x in [10., 47., 84., 122., 160., 205.] {
        piece(
            painter,
            braids_piece_rect(rect, x, 316., 24.),
            "PJ301M",
            0.0,
        );
    }
}
pub(crate) fn braids_port(input: bool, index: usize) -> Pos2 {
    let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(240., 396.));
    let x = if input {
        [47., 10., 84., 122., 160.][index.min(4)]
    } else {
        205.
    };
    braids_point(rect, x + 12., 328.)
}
