// Grain Weather shares one face, native parameter gestures and sockets in the
// rack and browser. Captions follow the playback mode; the guide states the
// configured buffer and what TRIG does, while only engine telemetry lights the
// output and freeze lamps.
fn grain_weather_mode(node: &Node) -> i32 {
    node.params
        .get("mode")
        .copied()
        .unwrap_or(0.0)
        .round()
        .clamp(0.0, 3.0) as i32
}
fn grain_weather_quality(node: &Node) -> i32 {
    node.params
        .get("quality")
        .copied()
        .unwrap_or(0.0)
        .round()
        .clamp(0.0, 3.0) as i32
}
fn grain_weather_control(name: &str) -> (Pos2, f32) {
    match name {
        "position" => (pos2(52.0, 140.0), 25.0),
        "size" => (pos2(147.0, 140.0), 25.0),
        "pitch" => (pos2(243.0, 140.0), 25.0),
        "density" => (pos2(338.0, 140.0), 25.0),
        "texture" => (pos2(43.0, 236.0), 19.0),
        "blend" => (pos2(119.0, 236.0), 19.0),
        "spread" => (pos2(195.0, 236.0), 19.0),
        "feedback" => (pos2(271.0, 236.0), 19.0),
        "reverb" => (pos2(347.0, 236.0), 19.0),
        _ => unreachable!("Grain Weather continuous control"),
    }
}
fn grain_weather_value_center(name: &str) -> Pos2 {
    let (center, _) = grain_weather_control(name);
    pos2(center.x, if center.y < 200.0 { 186.0 } else { 272.0 })
}
fn grain_weather_value_width(name: &str) -> f32 {
    let (center, _) = grain_weather_control(name);
    if center.y < 200.0 {
        84.0
    } else {
        66.0
    }
}
fn grain_weather_selector(name: &str) -> Option<(Rect, &'static str, &'static [&'static str])> {
    let (x, width, label, choices): (_, _, _, &[&str]) = match name {
        "mode" => (
            16.0,
            124.0,
            "MODE",
            &["GRANULAR", "STRETCH", "LOOPING DELAY", "SPECTRAL"],
        ),
        "quality" => (
            150.0,
            124.0,
            "QUALITY",
            &["16-BIT STEREO", "16-BIT MONO", "8-BIT STEREO", "8-BIT MONO"],
        ),
        "freeze" => (284.0, 90.0, "FREEZE", &["RECORD", "FROZEN"]),
        _ => return None,
    };
    Some((
        Rect::from_min_size(pos2(x, 66.0), vec2(width, 24.0)),
        label,
        choices,
    ))
}
fn grain_weather_port(input: bool, index: usize) -> Pos2 {
    if input {
        if index < 4 {
            // Audio, trigger and freeze along the bottom edge, clear of the
            // panel screws.
            pos2(38.0 + index as f32 * 60.0, 376.0)
        } else {
            // Six CV inputs under the controls they modulate.
            pos2(40.0 + (index - 4) as f32 * 62.0, 330.0)
        }
    } else {
        pos2(278.0 + index as f32 * 60.0, 376.0)
    }
}
fn grain_weather_port_label(port: &str) -> &str {
    match port {
        "left" => "IN L",
        "right" => "IN R",
        "trigger" => "TRIG",
        "freeze" => "FREEZE",
        "position_cv" => "POSITION",
        "size_cv" => "SIZE",
        "pitch_cv" => "V/OCT",
        "density_cv" => "DENSITY",
        "texture_cv" => "TEXTURE",
        "blend_cv" => "BLEND",
        other => port_label(other),
    }
}
fn grain_weather_output_label(port: &str) -> &str {
    match port {
        "left" => "OUT L",
        _ => "OUT R",
    }
}
fn grain_weather_caption(node: &Node, name: &str) -> &'static str {
    let mode = grain_weather_mode(node);
    match (name, mode) {
        ("position", 2) => "DELAY",
        ("position", _) => "POSITION",
        ("size", 1) => "WINDOW",
        ("size", 3) => "WARP",
        ("size", _) => "SIZE",
        ("pitch", _) => "PITCH",
        ("density", 0) => "DENSITY",
        ("density", 3) => "REFRESH",
        ("density", _) => "DIFFUSION",
        ("texture", 0) => "TEXTURE",
        ("texture", 3) => "QUANTIZE",
        ("texture", _) => "FILTER",
        ("blend", _) => "BLEND",
        ("spread", _) => "SPREAD",
        ("feedback", _) => "FEEDBACK",
        _ => "REVERB",
    }
}
fn paint_grain_weather_control_print(
    painter: &egui::Painter,
    rect: Rect,
    node: &Node,
    name: &str,
    zoom: f32,
) {
    let (center, radius) = grain_weather_control(name);
    painter.text(
        rect.min + vec2(center.x, center.y - radius - 13.0) * zoom,
        Align2::CENTER_CENTER,
        grain_weather_caption(node, name),
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
}
fn grain_weather_semitones(value: f32) -> String {
    if value.abs() < 0.05 {
        "0 st".into()
    } else if value < 0.0 {
        format!("−{:.1} st", -value)
    } else {
        format!("+{value:.1} st")
    }
}
fn grain_weather_value(node: &Node, name: &str, value: f32) -> String {
    match name {
        "pitch" => grain_weather_semitones(value),
        "density" if grain_weather_mode(node) == 0 => {
            // The source splits DENSITY at the centre: random seeding above,
            // regular seeding below, no grains in the 0.47..0.53 dead band.
            if value > 0.53 {
                format!("Random {:.0}%", ((value - 0.53) * 2.12).min(1.0) * 100.0)
            } else if value < 0.47 {
                format!("Even {:.0}%", ((0.47 - value) * 2.12).min(1.0) * 100.0)
            } else {
                "No grains".into()
            }
        }
        _ => format!("{:.0}%", value * 100.0),
    }
}
/// Buffer length the source memory holds at each quality setting.
fn grain_weather_buffer(quality: i32) -> (&'static str, &'static str) {
    match quality {
        1 => ("1.9 S", "MONO"),
        2 => ("4.1 S", "STEREO"),
        3 => ("7.4 S", "MONO"),
        _ => ("1.0 S", "STEREO"),
    }
}
fn grain_weather_guide(node: &Node) -> String {
    let quality = grain_weather_quality(node);
    let (length, channels) = grain_weather_buffer(quality);
    match grain_weather_mode(node) {
        0 => {
            let grains = [32, 40, 46, 57][quality as usize];
            format!("UP TO {grains} GRAINS FROM A {length} {channels} BUFFER · TRIG ADDS A GRAIN")
        }
        1 => format!("PITCH SHIFT AND TIME STRETCH OVER A {length} {channels} BUFFER · TRIG RESTARTS"),
        2 => format!("LOOPING DELAY OVER A {length} {channels} BUFFER · TRIG TAPS THE TIME"),
        _ => "PHASE VOCODER · FREEZE HOLDS THE SPECTRUM · TRIG HELD GLITCHES".into(),
    }
}
fn grain_weather_freeze_lamp(lights: Option<&ModuleLightState>) -> f32 {
    lights
        .filter(|state| state.count > 2)
        .map_or(0.0, |state| state.brightness[2][1].clamp(0.0, 1.0))
}
fn paint_grain_weather_configuration(
    ui: &mut egui::Ui,
    rect: Rect,
    node: &Node,
    zoom: f32,
    lights: Option<&ModuleLightState>,
    lighting: &crate::lighting::RoomLighting,
) {
    let guide = Rect::from_min_size(
        rect.min + vec2(16.0, 284.0) * zoom,
        vec2(358.0, 16.0) * zoom,
    );
    ui.painter().text(
        guide.center(),
        Align2::CENTER_CENTER,
        grain_weather_guide(node),
        crate::panel_skin::print_font(ui.ctx(), 8.5 * zoom),
        INK,
    );
    ui.interact(guide, ui.id().with(("grain-weather-guide", &node.id)), Sense::hover())
        .on_hover_text("Configured buffer and playback from the pinned Clouds source. The live input is recorded continuously unless FREEZE is on or the freeze gate is high. GRANULAR plays overlapping grains, STRETCH pitch-shifts and time-stretches with WSOLA, LOOPING DELAY is a delay that loops when frozen, SPECTRAL is a phase vocoder. Captions follow the mode. Position, size, density, texture and blend CV add voltage/5; V/OCT adds 12 semitones per volt. Mode and quality changes clear the buffer, as the source does. The kernel runs at 32 kHz; lamps show actual output volts and the actual freeze state.");
    // Measured freeze state beside the FREEZE selector caption.
    let center = rect.min + vec2(366.0, 54.0) * zoom;
    let strength = grain_weather_freeze_lamp(lights);
    ui.painter()
        .circle_filled(center, 3.0 * zoom, Color32::from_gray(78));
    lighting.circle(
        ui.painter(),
        center,
        2.2 * zoom,
        Color32::from_rgb(120, 196, 238).gamma_multiply(0.12 + strength * 0.88),
        strength,
        0.12,
    );
}
