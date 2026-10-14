//! Shared Studio Rack materials. These functions only paint: rack controls own
//! their hit regions and parameter behavior independently of the hardware skin.
use eframe::egui::{self, pos2, vec2, Color32, FontId, Pos2, Rect, Stroke};

pub(crate) const INK: Color32 = Color32::from_rgb(32, 34, 35);

/// Restrained silkscreen accents identify signal families on the same metal.
pub(crate) fn family_accent(kind: &str) -> Color32 {
    match kind {
        "oscillator" | "analog_vco" | "multi_vco" | "white_noise" | "drum_voice" => {
            Color32::from_rgb(151, 93, 57)
        }
        "lowpass" | "ladder_vcf" | "svf_filter" => Color32::from_rgb(68, 113, 121),
        "adsr" | "lfo" | "multi_lfo" | "gate" | "seq3" | "score_sequencer" => {
            Color32::from_rgb(116, 96, 136)
        }
        "gain"
        | "analog_vca"
        | "mixer"
        | "mix6"
        | "poly_sum"
        | "cv_mix"
        | "vca_mix"
        | "fade"
        | "crossfade"
        | "stereo_delay"
        | "stereo_reverb"
        | "sidechain_compressor" => Color32::from_rgb(78, 116, 96),
        "output" | "audio_out" | "audio_stereo" | "midi_cv" => Color32::from_rgb(66, 104, 133),
        _ => Color32::from_rgb(112, 112, 105),
    }
}

/// Permanent faceplate print: independent of instance IDs and live values.
pub(crate) fn model_caption(kind: &str) -> &str {
    match kind {
        "score_sequencer" => "PATTERNS & AUTOMATION",
        "stereo_delay" => "MODULATED ECHO",
        "stereo_reverb" => "STEREO REVERBERATOR",
        "sidechain_compressor" => "SIDECHAIN COMPRESSOR",
        "drum_voice" => "PERCUSSION SYNTHESIZER",
        "svf_filter" => "LOW / BAND / HIGH",
        "analog_vco" => "ANALOG OSCILLATOR",
        "oscillator" => "SINE OSCILLATOR",
        "multi_vco" => "MULTI-WAVE OSCILLATOR",
        "lowpass" => "LOW-PASS FILTER",
        "ladder_vcf" => "TRANSISTOR FILTER",
        "analog_vca" => "TRANSCONDUCTANCE VCA",
        "gain" => "AMPLIFIER",
        "adsr" => "ENVELOPE GENERATOR",
        "lfo" | "multi_lfo" => "LOW FREQUENCY",
        "gate" => "PULSE GENERATOR",
        "seq3" => "THREE-ROW SEQUENCER",
        "constant" => "VOLTAGE SOURCE",
        "poly_constant" => "POLYPHONIC SOURCE",
        "output" | "audio_out" => "AUDIO OUTPUT",
        "audio_stereo" => "STEREO OUTPUT",
        "midi_cv" => "MIDI INTERFACE",
        "white_noise" => "NOISE GENERATOR",
        "mixer" => "SIGNAL SUMMER",
        "mix6" => "SIX-CHANNEL MIXER",
        "vca_mix" => "VOLTAGE CONTROLLED MIXER",
        "cv_mix" => "BIPOLAR MIXER",
        "fade" | "crossfade" => "SIGNAL CROSSFADER",
        "attenuverter" | "attenuverter8" => "BIPOLAR ATTENUATOR",
        "mutes10" => "TEN-CHANNEL MUTES",
        "mid_side" => "STEREO MATRIX",
        "octave" => "PITCH TRANSPOSE",
        "quantizer" => "PITCH QUANTIZER",
        "sample_hold" => "SAMPLE & HOLD",
        "mult" => "SIGNAL MULTIPLE",
        "poly_split" => "POLYPHONIC SPLITTER",
        "poly_merge" => "POLYPHONIC MERGER",
        "poly_sum" => "POLYPHONIC SUMMER",
        "switch_1to4" | "switch_4to1" => "SEQUENTIAL SWITCH",
        "logic" => "BOOLEAN LOGIC",
        "compare" => "VOLTAGE COMPARATOR",
        "process" => "SIGNAL PROCESSOR",
        "rescale" => "VOLTAGE SCALER",
        "pulses" | "push" => "MANUAL GATES",
        "gates" => "GATE PROCESSOR",
        _ => "SIGNAL UTILITY",
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
    fonts.font_data.insert(
        "barlow-heading".into(),
        egui::FontData::from_static(include_bytes!("../assets/fonts/Barlow-SemiBold.ttf")).into(),
    );
    let mut heading_fonts = vec!["barlow-heading".into()];
    heading_fonts.extend(
        fonts.families[&egui::FontFamily::Proportional]
            .iter()
            .cloned(),
    );
    fonts.families.insert(
        egui::FontFamily::Name("Panel Heading".into()),
        heading_fonts,
    );
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

/// Broader hardware lettering for the model and primary controls; fine print
/// keeps the condensed family so dense utility panels remain legible.
pub(crate) fn heading_font(ctx: &egui::Context, size: f32) -> FontId {
    let family = egui::FontFamily::Name("Panel Heading".into());
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
    // The rolled perimeter catches light independently of the brushed face.
    painter.rect_filled(rect, 1.0, Color32::from_rgb(78, 80, 79));
    painter.rect_filled(
        rect.shrink(0.8 * zoom),
        1.0,
        Color32::from_rgb(213, 214, 208),
    );
    painter.rect_filled(
        rect.shrink(1.7 * zoom),
        0.5,
        Color32::from_rgb(180, 180, 175),
    );
    let face = rect.shrink(3.0 * zoom);
    let mut mesh = egui::epaint::Mesh::default();
    for (fraction, left, right) in [
        (0.0, [238, 237, 231], [222, 223, 218]),
        (0.14, [227, 226, 220], [216, 217, 212]),
        (0.60, [224, 223, 217], [213, 214, 209]),
        (1.0, [215, 215, 209], [201, 203, 198]),
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
    painter.line_segment(
        [face.left_top(), face.right_top()],
        Stroke::new(0.8 * zoom, Color32::from_rgb(255, 254, 246)),
    );
    painter.line_segment(
        [face.left_top(), face.left_bottom()],
        Stroke::new(0.8 * zoom, Color32::from_white_alpha(155)),
    );
    painter.line_segment(
        [face.left_bottom(), face.right_bottom()],
        Stroke::new(0.9 * zoom, Color32::from_black_alpha(65)),
    );
    // Fixed, irregular brushing avoids a ruled-paper pattern. Omit subpixel
    // texture at small scales identically in the rack and library.
    if zoom >= 0.65 {
        for grain in 0..180 {
            let y = face.top() + (2.0 + grain as f32 * 2.2 + (grain * 7 % 5) as f32 * 0.13) * zoom;
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
                    0.45 * zoom,
                    if grain % 3 == 0 {
                        Color32::from_black_alpha(5)
                    } else {
                        Color32::from_white_alpha(16)
                    },
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

/// A nearly flat graphite crown, distinct from the knurled skirt below it.
fn shaded_disc(painter: &egui::Painter, center: Pos2, radius: f32) {
    let mut mesh = egui::epaint::Mesh::default();
    mesh.colored_vertex(
        center - vec2(radius * 0.14, radius * 0.20),
        Color32::from_rgb(53, 57, 59),
    );
    const SEGMENTS: u32 = 40;
    for segment in 0..SEGMENTS {
        let angle = segment as f32 * std::f32::consts::TAU / SEGMENTS as f32;
        let direction = vec2(angle.cos(), angle.sin());
        let light = ((-direction.x - direction.y) * 0.5 + 0.5).clamp(0.0, 1.0);
        let gray = (28.0 + 22.0 * light) as u8;
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
    for (offset, grow, alpha) in [(vec2(2.0, 4.0), 3.0, 30), (vec2(1.2, 2.8), 1.6, 60)] {
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
    // The shoulder follows the same upper-left light as the panel and cap.
    // A directional mesh avoids uniformly bright concentric outlines.
    let mut shoulder = egui::epaint::Mesh::default();
    shoulder.colored_vertex(center, Color32::from_gray(105));
    for segment in 0..40_u32 {
        let a = segment as f32 * std::f32::consts::TAU / 40.0;
        let direction = vec2(a.cos(), a.sin());
        let light = ((-direction.x - direction.y) * 0.5 + 0.5).clamp(0.0, 1.0);
        let gray = (47.0 + 124.0 * light) as u8;
        shoulder.colored_vertex(
            center + direction * radius * 0.83,
            Color32::from_rgb(gray, gray, gray),
        );
        shoulder.add_triangle(0, segment + 1, (segment + 1) % 40 + 1);
    }
    painter.add(egui::Shape::mesh(shoulder));
    painter.circle_filled(
        center + vec2(0.0, 0.7) * zoom,
        radius * 0.78,
        Color32::from_rgb(10, 13, 14),
    );
    shaded_disc(painter, center, radius * 0.74);
    painter.circle_stroke(
        center,
        radius * 0.75,
        Stroke::new(0.8 * zoom, Color32::from_rgb(13, 17, 16)),
    );
    let arc: Vec<_> = (0..=20)
        .map(|i| {
            let a = (-175.0 + i as f32 * 6.4).to_radians();
            center + vec2(a.cos(), a.sin()) * radius * 0.79
        })
        .collect();
    painter.add(egui::Shape::line(
        arc,
        Stroke::new(
            0.85 * zoom,
            Color32::from_rgba_unmultiplied(233, 235, 230, 195),
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

/// Material for native ComboBox/DragValue controls embedded in a panel.
/// Geometry, font sizes, and selection/active colors remain owned by the caller.
pub(crate) fn control_style(ui: &mut egui::Ui, zoom: f32) {
    let visuals = ui.visuals_mut();
    visuals.override_text_color = Some(INK);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(190, 191, 183);
    visuals.widgets.inactive.weak_bg_fill = visuals.widgets.inactive.bg_fill;
    visuals.widgets.inactive.bg_stroke = Stroke::new(0.6 * zoom, Color32::from_rgb(153, 155, 148));
    visuals.widgets.inactive.fg_stroke.color = INK;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(207, 208, 198);
    visuals.widgets.hovered.weak_bg_fill = visuals.widgets.hovered.bg_fill;
    visuals.widgets.hovered.bg_stroke = Stroke::new(0.8 * zoom, Color32::from_rgb(112, 115, 108));
    visuals.widgets.hovered.fg_stroke.color = INK;
    // Keep the existing active fill and selection palette as interaction cues.
    visuals.widgets.active.weak_bg_fill = visuals.widgets.active.bg_fill;
    visuals.widgets.active.fg_stroke.color = INK;
    visuals.text_edit_bg_color = Some(Color32::from_rgb(207, 208, 198));
    visuals.window_fill = Color32::from_rgb(220, 218, 208);
    visuals.extreme_bg_color = Color32::from_rgb(190, 191, 183);
    if !ui.is_enabled() {
        let visuals = ui.visuals_mut();
        visuals.disabled_alpha = 1.0;
        visuals.widgets.noninteractive = visuals.widgets.inactive;
    }
}
