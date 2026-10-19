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
        "modal_resonator" | "colored_noise" | "macro_voice" | "rack_plaits" => {
            Color32::from_rgb(151, 93, 57)
        }
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
        "studio_mixer" => "PERFORMANCE CONSOLE",
        "bass_glitch" => "GRANULAR STUTTER",
        "diffuse_reverb" => "DENSE MODULATED REVERB",
        "tape_delay" => "SATURATED ECHO",
        "stereo_ensemble" => "SIX-VOICE STEREO CHORUS",
        "stereo_delay" => "MODULATED ECHO",
        "stereo_reverb" => "STEREO REVERBERATOR",
        "sidechain_compressor" => "SIDECHAIN COMPRESSOR",
        "drum_voice" => "PERCUSSION SYNTHESIZER",
        "svf_filter" => "LOW / BAND / HIGH",
        "analog_vco" => "ANALOG OSCILLATOR",
        "oscillator" => "SINE OSCILLATOR",
        "multi_vco" => "MULTI-WAVE OSCILLATOR",
        "macro_voice" => "8 ENGINE POLYPHONIC SYNTH",
        "rack_plaits" => "ORIGINAL RACK PLUGIN · PLAITS",
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
    // Anodized champagne metal, shared by every native faceplate. The room
    // lighting pass still owns reflected light; screen emission is separate.
    painter.rect_filled(rect, 0.0, Color32::from_rgb(132, 131, 121));
    let face = rect.shrink(1.0 * zoom);
    let mut mesh = egui::Mesh::default();
    for (point, color) in [
        (face.left_top(), Color32::from_rgb(232, 227, 213)),
        (face.right_top(), Color32::from_rgb(218, 213, 200)),
        (face.right_bottom(), Color32::from_rgb(201, 197, 184)),
        (face.left_bottom(), Color32::from_rgb(220, 215, 202)),
    ] {
        mesh.colored_vertex(point, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
    if zoom >= 0.65 {
        for row in 1..(rect.height() / (4.0 * zoom)) as usize {
            let y = rect.top() + row as f32 * 4.0 * zoom;
            painter.line_segment(
                [pos2(face.left(), y), pos2(face.right(), y)],
                Stroke::new(0.35 * zoom, Color32::from_white_alpha(9)),
            );
        }
    }
    painter.line_segment(
        [face.left_top(), face.right_top()],
        Stroke::new(0.6 * zoom, Color32::from_rgb(237, 234, 223)),
    );
    for x in [rect.left() + 8.0 * zoom, rect.right() - 8.0 * zoom] {
        for y in [rect.top() + 8.0 * zoom, rect.bottom() - 8.0 * zoom] {
            let center = pos2(x, y);
            painter.circle_filled(
                center + vec2(0.5, 0.8) * zoom,
                4.0 * zoom,
                Color32::from_black_alpha(70),
            );
            painter.circle_filled(center, 3.6 * zoom, Color32::from_rgb(101, 100, 90));
            painter.circle_stroke(
                center,
                3.0 * zoom,
                Stroke::new(0.8 * zoom, Color32::from_rgb(194, 190, 173)),
            );
            painter.line_segment(
                [
                    center + vec2(-1.7, 1.4) * zoom,
                    center + vec2(1.7, -1.4) * zoom,
                ],
                Stroke::new(1.0 * zoom, Color32::from_rgb(34, 36, 34)),
            );
        }
    }
}

pub(crate) fn luminous_accent(kind: &str) -> Color32 {
    match kind {
        "lowpass" | "ladder_vcf" | "stock_vcf" | "svf_filter" => Color32::from_rgb(126, 225, 174),
        "adsr" | "adsr_eg" | "lfo" | "multi_lfo" | "gate" | "seq3" | "score_sequencer"
        | "random_values" | "sh_asr" => Color32::from_rgb(195, 161, 239),
        "stereo_delay"
        | "stereo_reverb"
        | "sidechain_compressor"
        | "audio_stereo"
        | "output"
        | "audio_out" => Color32::from_rgb(126, 197, 243),
        "oscillator" | "analog_vco" | "multi_vco" | "macro_voice" | "drum_voice"
        | "modal_resonator" | "colored_noise" | "white_noise" => Color32::from_rgb(243, 177, 105),
        _ => Color32::from_rgb(143, 214, 179),
    }
}

/// Reflective cap and emissive 270-degree position scale. The ring is a setting
/// indicator; it deliberately does not imply measured audio activity.
#[allow(clippy::too_many_arguments)]
pub(crate) fn parameter_rotary(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    value: f32,
    active: bool,
    accent: Color32,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    let value = fader_value(value);
    let ring = (radius - 1.0).max(8.0) * zoom;
    let body = ring * 0.80;
    painter.circle_filled(
        center + vec2(0.7, 2.0) * zoom,
        body + 2.0 * zoom,
        Color32::from_black_alpha(80),
    );
    painter.circle_filled(center, body + zoom, Color32::from_rgb(17, 23, 24));
    for band in (0..8).rev() {
        let t = band as f32 / 7.0;
        painter.circle_filled(
            center - vec2(0.2, 0.45) * zoom,
            body * (0.88 + 0.12 * t),
            Color32::from_gray((34.0 + 28.0 * t) as u8),
        );
    }
    painter.circle_filled(center, body * 0.85, Color32::from_rgb(34, 39, 40));
    painter.circle_stroke(
        center,
        body * 0.84,
        Stroke::new(0.65 * zoom, Color32::from_rgb(65, 70, 68)),
    );
    let start = std::f32::consts::PI * 0.75;
    let sweep = std::f32::consts::PI * 1.5;
    for tick in 0..=24 {
        let (sin, cos) = (start + sweep * tick as f32 / 24.0).sin_cos();
        let direction = vec2(cos, sin);
        painter.line_segment(
            [
                center + direction * (ring + 1.5 * zoom),
                center + direction * (ring + if tick % 6 == 0 { 4.0 } else { 2.7 } * zoom),
            ],
            Stroke::new(0.65 * zoom, Color32::from_rgb(108, 108, 98)),
        );
    }
    let emission = crate::lighting::next(painter);
    let muted = Color32::from_rgb(69, 77, 76);
    for segment in 0..48 {
        let t = segment as f32 / 48.0;
        let a = start + sweep * t;
        let b = a + sweep / 48.0 * 0.72;
        let points = [
            center + vec2(a.cos(), a.sin()) * ring,
            center + vec2(b.cos(), b.sin()) * ring,
        ];
        if t <= value {
            painter.line_segment(points, Stroke::new(4.8 * zoom, accent.gamma_multiply(0.09)));
            painter.line_segment(points, Stroke::new(2.6 * zoom, accent.gamma_multiply(0.32)));
            painter.line_segment(points, Stroke::new(1.35 * zoom, accent));
        } else {
            painter.line_segment(points, Stroke::new(1.1 * zoom, muted));
        }
    }
    let angle = start + sweep * value;
    let direction = vec2(angle.cos(), angle.sin());
    painter.line_segment(
        [
            center + direction * body * 0.45,
            center + direction * body * 0.86,
        ],
        Stroke::new(2.0 * zoom, Color32::from_rgb(248, 230, 190)),
    );
    if active {
        painter.circle_stroke(
            center,
            ring + 5.0 * zoom,
            Stroke::new(0.7 * zoom, accent.gamma_multiply(0.6)),
        );
    }
    lighting.preserve(painter, emission);
}

/// Native rotary widget for bespoke panels. Uses stable caller scopes, relative
/// dragging, fine modifiers and keyboard edits; exact entry stays with the panel.
pub(crate) struct StudioDial<'a> {
    value: &'a mut f32,
    range: std::ops::RangeInclusive<f32>,
    size: egui::Vec2,
    color: Color32,
    zoom: f32,
    lighting: &'a crate::lighting::RoomLighting,
    logarithmic: bool,
}
impl<'a> StudioDial<'a> {
    pub(crate) fn new(
        value: &'a mut f32,
        range: std::ops::RangeInclusive<f32>,
        size: egui::Vec2,
        color: Color32,
        zoom: f32,
        lighting: &'a crate::lighting::RoomLighting,
    ) -> Self {
        Self {
            value,
            range,
            size,
            color,
            zoom,
            lighting,
            logarithmic: false,
        }
    }
    pub(crate) fn logarithmic(mut self, logarithmic: bool) -> Self {
        self.logarithmic = logarithmic;
        self
    }
}
// Some native event loops deliver an entire short drag in one repaint. egui's
// Response then has neither an active drag nor a click when release is outside
// the knob. Recover only a complete primary gesture that began on this control.
pub(crate) fn completed_pointer_drag(ui: &egui::Ui, response: &egui::Response) -> Option<egui::Vec2> {
    if !response.enabled() || response.dragged() || response.drag_stopped() {
        return None;
    }
    let clip = ui.clip_rect();
    let (from, delta) = ui.input(|i| {
        let mut start = None;
        let mut movement = None;
        for event in &i.events {
            if let egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, .. } = event {
                if *pressed {
                    start = (response.rect.contains(*pos) && clip.contains(*pos)).then_some(*pos);
                } else if let Some(from) = start.take() {
                    let delta = *pos - from;
                    if delta.length() >= 6.0 { movement = Some((from, delta)); }
                }
            }
        }
        movement
    })?;
    (ui.ctx().layer_id_at(from) == Some(response.layer_id)).then_some(delta)
}

impl egui::Widget for StudioDial<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let (rect, mut response) = ui.allocate_exact_size(self.size, egui::Sense::click_and_drag());
        let min = *self.range.start();
        let max = *self.range.end();
        let span = (max - min).max(f32::EPSILON);
        let old = *self.value;
        let mut normalized = if self.logarithmic {
            ((*self.value - min).max(0.0)).ln_1p() / span.ln_1p()
        } else {
            (*self.value - min) / span
        };
        let position_id = response.id.with("studio-position");
        let completed_motion = completed_pointer_drag(ui, &response);
        if response.is_pointer_button_down_on()
            && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary))
        {
            ui.data_mut(|d| d.insert_temp(position_id, normalized));
        }
        if response.dragged() {
            normalized = ui
                .data(|d| d.get_temp::<f32>(position_id))
                .unwrap_or(normalized);
        }
        let initial_normalized = normalized;
        if response.is_pointer_button_down_on() || response.clicked() {
            response.request_focus();
        }
        if ui.is_enabled() {
            let modifiers = ui.input(|i| i.modifiers);
            let fine = if modifiers.command || modifiers.ctrl {
                if modifiers.shift {
                    0.01
                } else {
                    0.1
                }
            } else {
                1.0
            };
            if response.dragged() || completed_motion.is_some() {
                // Follow screen points: native integrations may provide a zero raw
                // mouse delta alongside real PointerMoved events.
                let motion = completed_motion.unwrap_or_else(|| response.drag_delta());
                normalized += (motion.x * 0.35 - motion.y) / (160.0 * self.zoom) * fine;
            }
            if response.has_focus() {
                ui.ctx().memory_mut(|m| {
                    m.set_focus_lock_filter(
                        response.id,
                        egui::EventFilter {
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            ..Default::default()
                        },
                    )
                });
                let step = if modifiers.shift { 0.1 } else { 0.01 } * fine;
                let (nudge, boundary) = ui.input_mut(|i| {
                    let up = i.consume_key(modifiers, egui::Key::ArrowUp)
                        || i.consume_key(modifiers, egui::Key::ArrowRight);
                    let down = i.consume_key(modifiers, egui::Key::ArrowDown)
                        || i.consume_key(modifiers, egui::Key::ArrowLeft);
                    let boundary = if i.consume_key(modifiers, egui::Key::Home) {
                        Some(0.0)
                    } else if i.consume_key(modifiers, egui::Key::End) {
                        Some(1.0)
                    } else {
                        None
                    };
                    (i32::from(up) - i32::from(down), boundary)
                });
                normalized = boundary.unwrap_or(normalized + nudge as f32 * step);
            }
        }
        normalized = normalized.clamp(0.0, 1.0);
        // Drawing, focus and preview must never quantize the stored value.
        if ui.is_enabled() && normalized != initial_normalized {
            *self.value = if self.logarithmic {
                min + (normalized * span.ln_1p()).exp_m1()
            } else {
                min + normalized * span
            };
            if *self.value != old {
                response.mark_changed();
            }
            ui.data_mut(|d| d.insert_temp(position_id, normalized));
        }
        if response.drag_stopped() || !ui.is_enabled() || !ui.input(|i| i.focused) {
            ui.data_mut(|d| d.remove::<f32>(position_id));
        }
        let radius = (rect.height().min(rect.width()) * 0.5 / self.zoom - 3.0).max(6.0);
        parameter_rotary(
            ui.painter(),
            rect.center(),
            radius,
            normalized,
            response.hovered() || response.has_focus(),
            self.color,
            self.zoom,
            self.lighting,
        );
        response.on_hover_cursor(egui::CursorIcon::ResizeVertical)
    }
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
    // Flat concentric marks keep the connection target legible without a
    // decorative nut, offset shadow or reflected-metal stack. Output surrounds
    // and actual signal emission are painted separately by the caller.
    painter.circle_filled(center, 10.3 * zoom, Color32::from_rgb(190, 191, 182));
    painter.circle_stroke(
        center,
        10.3 * zoom,
        Stroke::new(0.8 * zoom, Color32::from_rgb(106, 109, 103)),
    );
    painter.circle_filled(center, 5.9 * zoom, Color32::from_rgb(12, 17, 16));
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
