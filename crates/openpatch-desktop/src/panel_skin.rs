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
        "modal_resonator" | "colored_noise" => Color32::from_rgb(151, 93, 57),
        "lowpass" | "ladder_vcf" | "svf_filter" => Color32::from_rgb(68, 113, 121),
        "adsr" | "adsr_eg" | "lfo" | "multi_lfo" | "gate" | "seq3" | "score_sequencer"
        | "random_values" | "sh_asr" => Color32::from_rgb(116, 96, 136),
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
        "output" | "audio_out" | "audio_stereo" | "midi_cv" | "midi_cc" | "midi_gate" => {
            Color32::from_rgb(66, 104, 133)
        }
        _ => Color32::from_rgb(112, 112, 105),
    }
}

/// Permanent faceplate print: independent of instance IDs and live values.
pub(crate) fn model_caption(kind: &str) -> &str {
    match kind {
        "notes" => "PATCH NOTES",
        "blank" => "RACK SPACE",
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
        "adsr_eg" => "POLYPHONIC DIGITAL ENVELOPE",
        "lfo" | "multi_lfo" => "LOW FREQUENCY",
        "gate" => "PULSE GENERATOR",
        "seq3" => "THREE-ROW SEQUENCER",
        "constant" => "VOLTAGE SOURCE",
        "poly_constant" => "POLYPHONIC SOURCE",
        "output" | "audio_out" => "AUDIO OUTPUT",
        "audio_stereo" => "STEREO OUTPUT",
        "midi_cv" => "MIDI INTERFACE",
        "midi_cc" => "SIXTEEN CONTROLLER OUTPUTS",
        "midi_gate" => "SIXTEEN NOTE GATES",
        "white_noise" => "NOISE GENERATOR",
        "colored_noise" => "SEVEN NOISE COLORS",
        "random_values" => "HELD RANDOM VOLTAGES",
        "sh_asr" => "EIGHT-STAGE SAMPLE & HOLD",
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
        "scope" => "POLYPHONIC SIGNAL MONITOR",
        "modal_resonator" => "STIFF STRING / BEAM · 16 MODES",
        "poly_viz" => "POLYPHONIC VOLTAGE DISPLAY",
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
    // A matte face keeps fine labels and controls in the foreground. Its broad,
    // low-contrast falloff still responds naturally to the rack's room lighting.
    painter.rect_filled(
        rect.translate(vec2(1.0, 1.5) * zoom),
        1.0,
        Color32::from_black_alpha(38),
    );
    painter.rect_filled(rect, 1.0, Color32::from_rgb(139, 140, 133));
    let face = rect.shrink(1.0 * zoom);
    let mut mesh = egui::epaint::Mesh::default();
    for (fraction, left, right) in [
        (0.0, [225, 223, 215], [222, 221, 213]),
        (1.0, [219, 218, 210], [216, 216, 208]),
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
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(egui::Shape::mesh(mesh));
    painter.line_segment(
        [face.left_top(), face.right_top()],
        Stroke::new(0.6 * zoom, Color32::from_rgb(239, 237, 229)),
    );
}

/// A matte graphite face with only a broad, subtle change in reflected light.
fn shaded_disc(painter: &egui::Painter, center: Pos2, radius: f32) {
    let mut mesh = egui::epaint::Mesh::default();
    mesh.colored_vertex(center, Color32::from_rgb(49, 52, 52));
    const SEGMENTS: u32 = 40;
    for segment in 0..SEGMENTS {
        let angle = segment as f32 * std::f32::consts::TAU / SEGMENTS as f32;
        let direction = vec2(angle.cos(), angle.sin());
        let light = ((-direction.x - direction.y) * 0.5 + 0.5).clamp(0.0, 1.0);
        let gray = (39.0 + 8.0 * light) as u8;
        mesh.colored_vertex(
            center + direction * radius,
            Color32::from_rgb(gray, gray + 3, gray + 3),
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
                Color32::from_rgb(92, 94, 86),
            ),
        );
    }
    // A quiet edge separates the control from its panel without a machined
    // shoulder, knurling or bright concentric rings competing with the pointer.
    painter.circle_filled(
        center + vec2(0.6, 1.2) * zoom,
        radius + 1.6 * zoom,
        Color32::from_black_alpha(30),
    );
    painter.circle_filled(
        center,
        radius + 1.6 * zoom,
        Color32::from_rgb(123, 126, 120),
    );
    shaded_disc(painter, center, radius);
    painter.circle_stroke(
        center,
        radius,
        Stroke::new(0.8 * zoom, Color32::from_rgb(30, 34, 33)),
    );
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
            center + direction * radius * 0.78,
        ],
        Stroke::new(2.2 * zoom, Color32::from_rgb(247, 244, 225)),
    );
}

pub(crate) fn socket(painter: &egui::Painter, center: Pos2, connected: bool, zoom: f32) {
    painter.circle_filled(
        center + vec2(0.7, 1.8) * zoom,
        11.1 * zoom,
        Color32::from_black_alpha(30),
    );
    let corners: Vec<_> = (0..6)
        .map(|i| {
            let a = (30.0 + i as f32 * 60.0).to_radians();
            center + vec2(a.cos(), a.sin()) * 10.3 * zoom
        })
        .collect();
    painter.add(egui::Shape::convex_polygon(
        corners,
        Color32::from_rgb(158, 160, 154),
        Stroke::new(0.8 * zoom, Color32::from_rgb(106, 109, 103)),
    ));
    painter.circle_filled(center, 8.9 * zoom, Color32::from_rgb(214, 214, 205));
    painter.circle_stroke(
        center,
        8.9 * zoom,
        Stroke::new(0.9 * zoom, Color32::from_rgb(229, 228, 218)),
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
    // A quiet numeric field with a bounded galley even for long units.
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
    painter.rect_filled(rect, 2.0 * zoom, Color32::from_rgb(204, 205, 196));
    painter.rect_stroke(
        rect,
        2.0 * zoom,
        Stroke::new(0.6 * zoom, Color32::from_rgb(165, 168, 157)),
        egui::StrokeKind::Inside,
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
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(204, 205, 196);
    visuals.widgets.inactive.weak_bg_fill = visuals.widgets.inactive.bg_fill;
    visuals.widgets.inactive.bg_stroke = Stroke::new(0.6 * zoom, Color32::from_rgb(165, 168, 157));
    visuals.widgets.inactive.fg_stroke.color = INK;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(216, 217, 207);
    visuals.widgets.hovered.weak_bg_fill = visuals.widgets.hovered.bg_fill;
    visuals.widgets.hovered.bg_stroke = Stroke::new(0.8 * zoom, Color32::from_rgb(116, 121, 109));
    visuals.widgets.hovered.fg_stroke.color = INK;
    // Keep the existing active fill and selection palette as interaction cues.
    visuals.widgets.active.weak_bg_fill = visuals.widgets.active.bg_fill;
    visuals.widgets.active.fg_stroke.color = INK;
    visuals.text_edit_bg_color = Some(Color32::from_rgb(216, 217, 207));
    visuals.window_fill = Color32::from_rgb(224, 222, 213);
    visuals.extreme_bg_color = Color32::from_rgb(204, 205, 196);
    if !ui.is_enabled() {
        let visuals = ui.visuals_mut();
        visuals.disabled_alpha = 1.0;
        visuals.widgets.noninteractive = visuals.widgets.inactive;
    }
}
