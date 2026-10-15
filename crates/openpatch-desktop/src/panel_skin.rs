//! Shared Studio Rack materials. These functions only paint: rack controls own
//! their hit regions and parameter behavior independently of the hardware skin.
use eframe::egui::{self, pos2, vec2, Color32, FontId, Pos2, Rect, Stroke};

pub(crate) const INK: Color32 = Color32::from_rgb(32, 34, 35);

const UNICODE_FALLBACK_FONTS: [(&str, &[u8]); 3] = [
    (
        "noto-cjk-jp-regular",
        include_bytes!("../assets/fonts/NotoSansCJKjp-Regular.otf"),
    ),
    (
        "noto-symbols-2-regular",
        include_bytes!("../assets/fonts/NotoSansSymbols2-Regular.ttf"),
    ),
    (
        "noto-music-regular",
        include_bytes!("../assets/fonts/NotoMusic-Regular.ttf"),
    ),
];

/// Restrained silkscreen accents identify signal families on the same metal.
pub(crate) fn family_accent(kind: &str) -> Color32 {
    match kind {
        "oscillator" | "analog_vco" | "multi_vco" | "white_noise" | "drum_voice" => {
            Color32::from_rgb(151, 93, 57)
        }
        "modal_resonator" | "colored_noise" => Color32::from_rgb(151, 93, 57),
        "lowpass" | "ladder_vcf" | "stock_vcf" | "svf_filter" => Color32::from_rgb(68, 113, 121),
        "adsr" | "adsr_eg" | "lfo" | "multi_lfo" | "gate" | "seq3" | "score_sequencer"
        | "random_values" | "sh_asr" => Color32::from_rgb(116, 96, 136),
        "gain"
        | "analog_vca"
        | "stock_vca"
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
        "stock_vcf" => "POLYPHONIC LP / HP FILTER",
        "stock_vca" => "POLYPHONIC GAIN",
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
    // Keep every existing Latin/UI face first. Missing glyphs use unmodified
    // static Regular CJK, symbol and musical-notation faces on every platform.
    for (name, bytes) in UNICODE_FALLBACK_FONTS {
        fonts
            .font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
        for family in fonts.families.values_mut() {
            family.push(name.into());
        }
    }
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
    // Warm, matte metal: a simple face and a crisp edge keep the controls clear.
    // Room illumination belongs to the caller, including bespoke panel layouts.
    painter.rect_filled(
        rect.translate(vec2(1.0, 1.5) * zoom),
        1.0,
        Color32::from_black_alpha(38),
    );
    painter.rect_filled(rect, 1.0, Color32::from_rgb(139, 140, 133));
    let face = rect.shrink(1.0 * zoom);
    painter.rect_filled(face, 0.0, Color32::from_rgb(223, 220, 210));
    painter.line_segment(
        [face.left_top(), face.right_top()],
        Stroke::new(0.6 * zoom, Color32::from_rgb(237, 234, 223)),
    );
}

/// The normalized parameter's travel in screen points. `radius` is a logical,
/// unscaled layout radius; `center` is already in screen coordinates.
pub(crate) fn parameter_fader_track(center: Pos2, radius: f32, zoom: f32) -> Rect {
    let half_height = (radius - 5.0).max(10.0) * zoom;
    Rect::from_center_size(center, vec2(4.0 * zoom, 2.0 * half_height))
}

fn fader_value(normalized: f32) -> f32 {
    if normalized.is_finite() {
        normalized.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Shared thumb geometry for paint and the caller's interaction region.
/// Full scale is at the top; zero is at the bottom.
pub(crate) fn parameter_fader_thumb(track: Rect, normalized: f32, zoom: f32) -> Rect {
    let y = egui::lerp(track.bottom()..=track.top(), fader_value(normalized));
    Rect::from_center_size(pos2(track.center().x, y), vec2(20.0, 8.0) * zoom)
}

/// Passive parameter hardware, not a signal meter: configured position and
/// interaction accents dim with the faceplate under the caller's room lighting.
pub(crate) fn parameter_fader(
    painter: &egui::Painter,
    track: Rect,
    normalized: f32,
    active: bool,
    accent: Color32,
    zoom: f32,
) {
    let thumb = parameter_fader_thumb(track, normalized, zoom);
    for step in 0..=4 {
        let y = egui::lerp(track.bottom()..=track.top(), step as f32 / 4.0);
        let outer = if step % 2 == 0 { 11.0 } else { 8.0 } * zoom;
        for side in [-1.0, 1.0] {
            painter.line_segment(
                [
                    pos2(track.center().x + side * 5.0 * zoom, y),
                    pos2(track.center().x + side * outer, y),
                ],
                Stroke::new(0.8 * zoom, Color32::from_rgb(114, 113, 104)),
            );
        }
    }
    painter.rect_filled(
        track.expand(0.8 * zoom),
        1.5 * zoom,
        Color32::from_rgb(146, 145, 134),
    );
    painter.rect_filled(track, 1.0 * zoom, Color32::from_rgb(51, 56, 53));
    let fill = Rect::from_min_max(
        pos2(track.left() + zoom, thumb.center().y),
        pos2(track.right() - zoom, track.bottom()),
    );
    if fill.height() > 0.0 {
        painter.rect_filled(fill, 0.0, accent);
    }
    painter.rect_filled(
        thumb.translate(vec2(0.6, 0.7) * zoom),
        1.4 * zoom,
        Color32::from_black_alpha(45),
    );
    painter.rect_filled(thumb, 1.4 * zoom, Color32::from_rgb(235, 231, 216));
    painter.rect_stroke(
        thumb,
        1.4 * zoom,
        Stroke::new(
            if active { 1.4 } else { 0.9 } * zoom,
            if active {
                accent
            } else {
                Color32::from_rgb(72, 77, 70)
            },
        ),
        egui::StrokeKind::Inside,
    );
    painter.line_segment(
        [
            pos2(thumb.left() + 3.0 * zoom, thumb.center().y),
            pos2(thumb.right() - 3.0 * zoom, thumb.center().y),
        ],
        Stroke::new(1.2 * zoom, accent),
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
    // A bounded numeric entry field, with a small pencil mark that distinguishes
    // an editable setting from the surrounding silkscreen labels.
    let available = (max_width - 25.0 * zoom).max(1.0);
    let mut galley = painter.layout_no_wrap(text.to_owned(), FontId::monospace(11.0 * zoom), INK);
    if galley.size().x > available {
        let size = 11.0 * zoom * available / galley.size().x;
        galley = painter.layout_no_wrap(text.to_owned(), FontId::monospace(size), INK);
    }
    let width = (galley.size().x + 25.0 * zoom)
        .max(48.0 * zoom)
        .min(max_width);
    let rect = Rect::from_center_size(center, vec2(width, 17.0 * zoom));
    painter.rect_filled(rect, 2.0 * zoom, Color32::from_rgb(237, 233, 222));
    painter.rect_stroke(
        rect,
        2.0 * zoom,
        Stroke::new(0.8 * zoom, Color32::from_rgb(143, 148, 135)),
        egui::StrokeKind::Inside,
    );
    let icon_center = pos2(rect.right() - 7.0 * zoom, center.y);
    let icon = Color32::from_rgb(90, 101, 89);
    painter.line_segment(
        [
            icon_center + vec2(-2.5, 2.0) * zoom,
            icon_center + vec2(2.0, -2.5) * zoom,
        ],
        Stroke::new(1.2 * zoom, icon),
    );
    painter.line_segment(
        [
            icon_center + vec2(-3.0, 3.0) * zoom,
            icon_center + vec2(2.5, 3.0) * zoom,
        ],
        Stroke::new(0.7 * zoom, icon),
    );
    let text_rect = Rect::from_min_max(
        rect.min + vec2(4.0, 1.0) * zoom,
        rect.max - vec2(15.0, 1.0) * zoom,
    );
    painter
        .with_clip_rect(painter.clip_rect().intersect(text_rect))
        .galley(
            pos2(
                text_rect.center().x - galley.size().x * 0.5,
                center.y - galley.size().y * 0.5,
            ),
            galley,
            INK,
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_fonts_resolve_notes_unicode_without_replacement_glyphs() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let mut missing = Vec::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            for family in [
                egui::FontFamily::Proportional,
                egui::FontFamily::Monospace,
                egui::FontFamily::Name("Panel Print".into()),
                egui::FontFamily::Name("Panel Heading".into()),
            ] {
                let font = FontId::new(16.0, family);
                for character in "→日本語café🎛𝄞".chars() {
                    if !ctx.fonts_mut(|fonts| fonts.has_glyph(&font, character)) {
                        missing.push(format!(
                            "{:?}: {character} U+{:04X}",
                            font.family, character as u32
                        ));
                    }
                }
            }
        });
        assert!(missing.is_empty(), "missing actual glyphs: {missing:?}");
    }

    #[test]
    fn bundled_unicode_fallback_faces_parse_as_regular_and_have_real_glyphs() {
        let mut definitions = egui::FontDefinitions::empty();
        for (name, bytes) in UNICODE_FALLBACK_FONTS {
            // OS/2.usWeightClass is 400 for each unmodified static Regular face.
            // In particular, the CJK fallback is not a variable font's Thin default.
            let count = usize::from(u16::from_be_bytes(bytes[4..6].try_into().unwrap()));
            let table = bytes[12..12 + count * 16]
                .as_chunks::<16>()
                .0
                .iter()
                .find(|table| &table[..4] == b"OS/2")
                .unwrap();
            let offset = u32::from_be_bytes(table[8..12].try_into().unwrap()) as usize;
            assert_eq!(
                u16::from_be_bytes(bytes[offset + 4..offset + 6].try_into().unwrap()),
                400,
                "{name}"
            );
            definitions
                .font_data
                .insert(name.into(), egui::FontData::from_static(bytes).into());
            definitions
                .families
                .insert(egui::FontFamily::Name(name.into()), vec![name.into()]);
        }
        let ctx = egui::Context::default();
        ctx.set_fonts(definitions);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            for ((name, _), sample) in UNICODE_FALLBACK_FONTS
                .iter()
                .zip(["→日本語café", "🎛", "𝄞♫"])
            {
                let font = FontId::new(16.0, egui::FontFamily::Name((*name).into()));
                for character in sample.chars() {
                    assert!(
                        ctx.fonts_mut(|fonts| fonts.has_glyph(&font, character)),
                        "{name} lacks {character}"
                    );
                }
                // Exercise actual outline rasterization as well as cmap lookup,
                // including the CJK font's OTF/CFF outlines.
                let galley = ctx
                    .fonts_mut(|fonts| fonts.layout_no_wrap(sample.into(), font, Color32::WHITE));
                for row in &galley.rows {
                    for glyph in &row.glyphs {
                        assert!(
                            glyph.uv_rect.max[0] > glyph.uv_rect.min[0]
                                && glyph.uv_rect.max[1] > glyph.uv_rect.min[1],
                            "{name} has no rasterized outline for {}",
                            glyph.chr
                        );
                    }
                }
            }
        });
    }

    #[test]
    fn installed_unicode_fallbacks_preserve_existing_face_priority() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let defaults = egui::FontDefinitions::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            ctx.fonts(|fonts| {
                let definitions = fonts.definitions();
                for (family, original_faces) in &defaults.families {
                    assert!(definitions.families[family].starts_with(original_faces));
                }
                for (family, first) in [
                    ("Panel Print", "barlow-panel"),
                    ("Panel Heading", "barlow-heading"),
                ] {
                    let faces = &definitions.families[&egui::FontFamily::Name(family.into())];
                    assert_eq!(faces[0], first);
                    assert!(
                        faces[1..].starts_with(&defaults.families[&egui::FontFamily::Proportional])
                    );
                }
                let fallback_names: Vec<_> = UNICODE_FALLBACK_FONTS
                    .iter()
                    .map(|(name, _)| (*name).to_owned())
                    .collect();
                for faces in definitions.families.values() {
                    assert!(faces.ends_with(&fallback_names));
                }
            });
        });
    }
}

#[cfg(test)]
#[path = "generic_fader_paint_tests.rs"]
mod generic_fader_paint_tests;
