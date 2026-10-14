//! Shared Studio Rack materials. These functions only paint: rack controls own
//! their hit regions and parameter behavior independently of the hardware skin.
use eframe::egui::{self, pos2, vec2, Color32, FontId, Pos2, Rect, Stroke};

pub(crate) const INK: Color32 = Color32::from_rgb(32, 34, 35);

/// Restrained silkscreen accents identify signal families on the same metal.
pub(crate) fn family_accent(kind: &str) -> Color32 {
    match kind {
        "oscillator" | "analog_vco" | "multi_vco" | "white_noise" => Color32::from_rgb(151, 93, 57),
        "lowpass" | "ladder_vcf" => Color32::from_rgb(68, 113, 121),
        "adsr" | "lfo" | "multi_lfo" | "gate" | "seq3" => Color32::from_rgb(116, 96, 136),
        "gain" | "analog_vca" | "mixer" | "mix6" | "cv_mix" | "vca_mix" | "fade" | "crossfade" => {
            Color32::from_rgb(78, 116, 96)
        }
        "output" | "audio_out" | "audio_stereo" | "midi_cv" => Color32::from_rgb(66, 104, 133),
        _ => Color32::from_rgb(112, 112, 105),
    }
}

pub(crate) fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "barlow-panel".into(),
        egui::FontData::from_static(include_bytes!(
            "../assets/fonts/BarlowSemiCondensed-SemiBold.ttf"
        ))
        .into(),
    );
    let mut panel_fonts = vec!["barlow-panel".into()];
    panel_fonts.extend(
        fonts.families[&egui::FontFamily::Proportional]
            .iter()
            .cloned(),
    );
    fonts
        .families
        .insert(egui::FontFamily::Name("Panel Print".into()), panel_fonts);
    ctx.set_fonts(fonts);
}

pub(crate) fn print_font(ctx: &egui::Context, size: f32) -> FontId {
    let family = egui::FontFamily::Name("Panel Print".into());
    // Standalone headless UI tests may intentionally use egui's default fonts.
    if ctx.fonts(|fonts| fonts.definitions().families.contains_key(&family)) {
        FontId::new(size, family)
    } else {
        FontId::proportional(size)
    }
}

pub(crate) fn metal(painter: &egui::Painter, rect: Rect, zoom: f32) {
    painter.rect_filled(
        rect.translate(vec2(1.8, 2.4) * zoom),
        1.0,
        Color32::from_black_alpha(115),
    );
    painter.rect_filled(rect, 1.0, Color32::from_rgb(90, 90, 87));
    let face = rect.shrink(1.5 * zoom);
    let mut mesh = egui::epaint::Mesh::default();
    for (fraction, left, right) in [
        (0.0, [242, 240, 234], [231, 229, 223]),
        (0.14, [226, 224, 218], [219, 217, 211]),
        (0.60, [220, 218, 212], [214, 212, 206]),
        (1.0, [212, 210, 204], [203, 202, 197]),
    ] {
        let y = egui::lerp(face.y_range(), fraction);
        mesh.colored_vertex(
            pos2(face.left(), y),
            Color32::from_rgb(left[0], left[1], left[2]),
        );
        mesh.colored_vertex(
            pos2(face.right(), y),
            Color32::from_rgb(right[0], right[1], right[2]),
        );
    }
    for row in 0..3 {
        let i = row * 2;
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i + 1, i + 3, i + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
    // Brushing is deterministic and sparse; omit subpixel texture in thumbnails.
    if zoom >= 0.65 {
        for grain in 0..56 {
            let y = face.top() + (6.0 + grain as f32 * 6.8) * zoom;
            if y >= face.bottom() {
                break;
            }
            let inset = (3 + (grain * 7) % 13) as f32 * zoom;
            painter.line_segment(
                [
                    pos2(face.left() + inset, y),
                    pos2(face.right() - 3.0 * zoom, y),
                ],
                Stroke::new(
                    0.55 * zoom,
                    Color32::from_white_alpha(if grain % 3 == 0 { 13 } else { 7 }),
                ),
            );
        }
    }
    for (a, b, color) in [
        (
            rect.left_top(),
            rect.right_top(),
            Color32::from_rgb(252, 250, 244),
        ),
        (
            rect.left_top(),
            rect.left_bottom(),
            Color32::from_rgb(245, 243, 237),
        ),
        (
            rect.right_top(),
            rect.right_bottom(),
            Color32::from_rgb(78, 79, 77),
        ),
        (
            rect.left_bottom(),
            rect.right_bottom(),
            Color32::from_rgb(103, 104, 101),
        ),
    ] {
        painter.line_segment([a, b], Stroke::new(zoom.max(0.5), color));
    }
}

/// A smooth, directional cap rather than overlapping flat circles.
fn shaded_disc(painter: &egui::Painter, center: Pos2, radius: f32) {
    let mut mesh = egui::epaint::Mesh::default();
    mesh.colored_vertex(
        center - vec2(radius * 0.18, radius * 0.24),
        Color32::from_rgb(76, 79, 80),
    );
    const SEGMENTS: u32 = 40;
    for segment in 0..SEGMENTS {
        let angle = segment as f32 * std::f32::consts::TAU / SEGMENTS as f32;
        let direction = vec2(angle.cos(), angle.sin());
        let light = ((-direction.x - direction.y) * 0.5 + 0.5).clamp(0.0, 1.0);
        let gray = (24.0 + 29.0 * light) as u8;
        mesh.colored_vertex(
            center + direction * radius,
            Color32::from_rgb(gray, gray + 1, gray + 2),
        );
        mesh.add_triangle(0, segment + 1, (segment + 1) % SEGMENTS + 1);
    }
    painter.add(egui::Shape::mesh(mesh));
}

pub(crate) fn knob(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    normalized: f32,
    active: bool,
    zoom: f32,
) {
    let angle = |value: f32| (-225.0 + value * 270.0).to_radians();
    for step in 0..=10 {
        let a = angle(step as f32 / 10.0);
        let direction = vec2(a.cos(), a.sin());
        painter.line_segment(
            [
                center + direction * (radius + 5.0 * zoom),
                center + direction * (radius + if step % 5 == 0 { 9.5 } else { 8.0 } * zoom),
            ],
            Stroke::new(
                if step % 5 == 0 { 1.15 } else { 0.8 } * zoom,
                Color32::from_rgb(81, 84, 74),
            ),
        );
    }
    for (offset, grow, alpha) in [(vec2(2.0, 4.0), 3.0, 25), (vec2(1.2, 2.8), 1.6, 49)] {
        painter.circle_filled(
            center + offset * zoom,
            radius + grow * zoom,
            Color32::from_black_alpha(alpha),
        );
    }
    painter.circle_filled(
        center,
        radius + 1.6 * zoom,
        Color32::from_rgb(161, 163, 161),
    );
    painter.circle_stroke(
        center,
        radius + 1.5 * zoom,
        Stroke::new(0.7 * zoom, Color32::from_rgb(247, 246, 238)),
    );
    painter.circle_filled(center, radius, Color32::from_rgb(23, 25, 27));
    for step in 0..32 {
        let a = step as f32 * std::f32::consts::TAU / 32.0;
        let direction = vec2(a.cos(), a.sin());
        let shade = if direction.x + direction.y < 0.0 {
            92
        } else {
            47
        };
        painter.line_segment(
            [
                center + direction * radius * 0.85,
                center + direction * radius * 0.97,
            ],
            Stroke::new(0.85 * zoom, Color32::from_gray(shade)),
        );
    }
    shaded_disc(painter, center, radius * 0.81);
    painter.circle_stroke(
        center,
        radius * 0.82,
        Stroke::new(0.8 * zoom, Color32::from_rgb(13, 17, 16)),
    );
    let arc: Vec<_> = (0..=20)
        .map(|i| {
            let a = (-175.0 + i as f32 * 6.4).to_radians();
            center + vec2(a.cos(), a.sin()) * radius * 0.85
        })
        .collect();
    painter.add(egui::Shape::line(
        arc,
        Stroke::new(
            0.85 * zoom,
            Color32::from_rgba_unmultiplied(214, 217, 218, 150),
        ),
    ));
    if active {
        painter.circle_stroke(
            center,
            radius + 2.3 * zoom,
            Stroke::new(1.2 * zoom, Color32::from_rgb(217, 157, 60)),
        );
    }
    let a = angle(normalized.clamp(0.0, 1.0));
    let direction = vec2(a.cos(), a.sin());
    painter.line_segment(
        [
            center + direction * radius * 0.25,
            center + direction * radius * 0.72,
        ],
        Stroke::new(2.2 * zoom, Color32::from_rgb(247, 241, 215)),
    );
}

pub(crate) fn socket(painter: &egui::Painter, center: Pos2, connected: bool, zoom: f32) {
    painter.circle_filled(
        center + vec2(0.7, 1.8) * zoom,
        11.1 * zoom,
        Color32::from_black_alpha(80),
    );
    let corners: Vec<_> = (0..6)
        .map(|i| {
            let a = (30.0 + i as f32 * 60.0).to_radians();
            center + vec2(a.cos(), a.sin()) * 10.3 * zoom
        })
        .collect();
    painter.add(egui::Shape::convex_polygon(
        corners,
        Color32::from_rgb(158, 160, 160),
        Stroke::new(0.8 * zoom, Color32::from_rgb(85, 87, 88)),
    ));
    painter.circle_filled(center, 8.9 * zoom, Color32::from_rgb(230, 229, 222));
    painter.circle_stroke(
        center,
        8.9 * zoom,
        Stroke::new(0.9 * zoom, Color32::from_rgb(252, 250, 244)),
    );
    painter.circle_filled(
        center + vec2(0.15, 0.4) * zoom,
        7.25 * zoom,
        Color32::from_rgb(64, 67, 68),
    );
    painter.circle_filled(center, 5.9 * zoom, Color32::from_rgb(12, 17, 16));
    painter.circle_stroke(
        center,
        6.5 * zoom,
        Stroke::new(0.65 * zoom, Color32::from_rgb(116, 119, 119)),
    );
    if connected {
        painter.circle_stroke(
            center,
            8.1 * zoom,
            Stroke::new(0.8 * zoom, Color32::from_rgb(163, 135, 86)),
        );
    }
}

pub(crate) fn readout(
    painter: &egui::Painter,
    center: Pos2,
    text: &str,
    max_width: f32,
    zoom: f32,
) {
    // A real recessed display, with a bounded galley even for long units.
    let available = (max_width - 10.0 * zoom).max(1.0);
    let mut galley = painter.layout_no_wrap(text.to_owned(), FontId::monospace(11.0 * zoom), INK);
    if galley.size().x > available {
        let size = 11.0 * zoom * available / galley.size().x;
        galley = painter.layout_no_wrap(text.to_owned(), FontId::monospace(size), INK);
    }
    let width = (galley.size().x + 12.0 * zoom)
        .max(42.0 * zoom)
        .min(max_width);
    let rect = Rect::from_center_size(center, vec2(width, 17.0 * zoom));
    painter.rect_filled(rect, 2.0 * zoom, Color32::from_rgb(190, 191, 183));
    painter.rect_stroke(
        rect,
        2.0 * zoom,
        Stroke::new(0.6 * zoom, Color32::from_rgb(153, 155, 148)),
        egui::StrokeKind::Inside,
    );
    painter.line_segment(
        [
            rect.left_top() + vec2(2.0, 0.0) * zoom,
            rect.right_top() - vec2(2.0, 0.0) * zoom,
        ],
        Stroke::new(0.8 * zoom, Color32::from_black_alpha(75)),
    );
    painter.line_segment(
        [
            rect.left_bottom() + vec2(2.0, 0.5) * zoom,
            rect.right_bottom() + vec2(-2.0, 0.5) * zoom,
        ],
        Stroke::new(0.9 * zoom, Color32::from_white_alpha(170)),
    );
    painter
        .with_clip_rect(painter.clip_rect().intersect(rect))
        .galley(center - galley.size() * 0.5, galley, INK);
}

pub(crate) fn port_label(
    painter: &egui::Painter,
    center: Pos2,
    label: &str,
    output: bool,
    zoom: f32,
) {
    let color = if output {
        Color32::from_rgb(245, 242, 233)
    } else {
        INK
    };
    let galley = painter.layout_no_wrap(
        label.to_owned(),
        print_font(painter.ctx(), 10.5 * zoom),
        color,
    );
    if output {
        let plate = Rect::from_center_size(center, galley.size() + vec2(8.0, 2.0) * zoom);
        painter.rect_filled(plate, 1.5 * zoom, INK);
    }
    painter.galley(center - galley.size() * 0.5, galley, color);
}
