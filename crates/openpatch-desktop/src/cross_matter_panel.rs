// Cross Matter shares one face, native parameter gestures and sockets in the
// rack and browser. Captions follow the selected mode; the guide describes the
// configured signal path, while only engine telemetry lights the output lamps.
fn cross_matter_mode(node: &Node) -> (bool, bool) {
    let shifter = node.params.get("mode").copied().unwrap_or(0.0).round() as i32 == 1;
    let internal = node.params.get("carrier").copied().unwrap_or(1.0).round() as i32 != 0;
    (shifter, internal)
}
fn cross_matter_control(name: &str) -> (Pos2, f32) {
    match name {
        "algorithm" => (pos2(180.0, 150.0), 32.0),
        "level1" => (pos2(62.0, 152.0), 24.0),
        "level2" => (pos2(298.0, 152.0), 24.0),
        "timbre" => (pos2(180.0, 250.0), 25.0),
        _ => unreachable!("Cross Matter continuous control"),
    }
}
fn cross_matter_value_center(name: &str) -> Pos2 {
    let (center, _) = cross_matter_control(name);
    pos2(center.x, if center.y < 200.0 { 197.0 } else { 288.0 })
}
fn cross_matter_selector(name: &str) -> Option<(Rect, &'static str, &'static [&'static str])> {
    let (x, width, label, choices): (_, _, _, &[&str]) = match name {
        "carrier" => (
            18.0,
            150.0,
            "CARRIER",
            &["EXTERNAL", "SINE", "TRIANGLE", "SAW"],
        ),
        "mode" => (
            186.0,
            156.0,
            "MODE",
            &["CROSS MODULATION", "FREQUENCY SHIFTER"],
        ),
        _ => return None,
    };
    Some((
        Rect::from_min_size(pos2(x, 66.0), vec2(width, 24.0)),
        label,
        choices,
    ))
}
fn cross_matter_port(input: bool, index: usize) -> Pos2 {
    if input {
        pos2(34.0 + index as f32 * 58.4, 338.0)
    } else {
        pos2(140.0 + index as f32 * 100.0, 378.0)
    }
}
fn cross_matter_port_label(port: &str) -> &str {
    match port {
        "carrier" => "CARRIER",
        "modulator" => "MODULATOR",
        "level1_cv" => "LEVEL 1",
        "level2_cv" => "LEVEL 2",
        "algorithm_cv" => "ALGORITHM",
        "timbre_cv" => "TIMBRE",
        "out" => "OUT",
        "aux" => "AUX",
        other => port_label(other),
    }
}
fn cross_matter_caption(node: &Node, name: &str) -> &'static str {
    let (shifter, internal) = cross_matter_mode(node);
    match (name, shifter) {
        ("algorithm", false) => "ALGORITHM",
        ("algorithm", true) => {
            if internal {
                "SHIFT"
            } else {
                "PHASE"
            }
        }
        ("timbre", false) => "TIMBRE",
        ("timbre", true) => "UP / DOWN",
        ("level1", false) => {
            if internal {
                "PITCH"
            } else {
                "LEVEL 1"
            }
        }
        ("level1", true) => "FEEDBACK",
        ("level2", false) => "LEVEL 2",
        _ => "DRY / WET",
    }
}
fn paint_cross_matter_control_print(
    painter: &egui::Painter,
    rect: Rect,
    node: &Node,
    name: &str,
    zoom: f32,
) {
    let (center, radius) = cross_matter_control(name);
    painter.text(
        rect.min + vec2(center.x, center.y - radius - 13.0) * zoom,
        Align2::CENTER_CENTER,
        cross_matter_caption(node, name),
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
}
fn cross_matter_algorithm(knob: f32) -> f32 {
    openpatch_core::cross_matter_effective_algorithm(knob)
}
fn cross_matter_algorithm_name(knob: f32) -> String {
    const NAMES: [&str; 7] = [
        "Crossfade",
        "Fold",
        "Analog ring",
        "Digital ring",
        "XOR",
        "Comparator",
        "Vocoder",
    ];
    let value = cross_matter_algorithm(knob);
    if value >= 0.725 {
        return NAMES[6].into();
    }
    // Blends name both neighbours by their first word.
    const SHORT: [&str; 7] = [
        "Crossfade",
        "Fold",
        "Analog",
        "Digital",
        "XOR",
        "Comparator",
        "Vocoder",
    ];
    if value > 0.675 {
        return format!("{} / {}", SHORT[5], SHORT[6]);
    }
    let position = (value * 8.0).min(5.999);
    let index = position as usize;
    let fraction = position - index as f32;
    if fraction < 0.2 {
        NAMES[index].into()
    } else if fraction > 0.8 {
        NAMES[index + 1].into()
    } else {
        format!("{} / {}", SHORT[index], SHORT[index + 1])
    }
}
fn cross_matter_hz(hz: f32) -> String {
    let magnitude = hz.abs();
    let sign = if hz < 0.0 { "−" } else { "+" };
    if magnitude >= 1000.0 {
        format!("{sign}{:.2} kHz", magnitude / 1000.0)
    } else if magnitude >= 100.0 {
        format!("{sign}{magnitude:.0} Hz")
    } else {
        format!("{sign}{magnitude:.1} Hz")
    }
}
/// Display for the native shift at zero CV (warps/dsp/modulator.cc mapping).
fn cross_matter_shift_hz(knob: f32) -> f32 {
    let direction = if knob >= 0.5 { 1.0 } else { -1.0 };
    let amount = 2.0 * (knob - 0.5).abs();
    let hz = if amount <= 0.4 {
        amount * amount * amount * 62.5
    } else {
        4.0 * (180.0 * (amount - 0.4) / 12.0).exp2()
    };
    hz * direction
}
fn cross_matter_value(node: &Node, name: &str, value: f32) -> String {
    let (shifter, internal) = cross_matter_mode(node);
    match name {
        "algorithm" if shifter && internal => cross_matter_hz(cross_matter_shift_hz(value)),
        "algorithm" if shifter => format!("{:.0}°", value * 360.0),
        "algorithm" => cross_matter_algorithm_name(value),
        "level1" if internal && !shifter => {
            // MIDI 36..96 from the knob; MIDI 69 = 440 Hz.
            let hz = 440.0 * ((60.0 * value + 36.0 - 69.0) / 12.0).exp2();
            let text = cross_matter_hz(hz);
            text.trim_start_matches('+').to_string()
        }
        _ => format!("{:.0}%", value * 100.0),
    }
}
fn cross_matter_guide(node: &Node) -> &'static str {
    let (shifter, internal) = cross_matter_mode(node);
    if shifter {
        return if internal {
            "MODULATOR SHIFTED BY THE BUILT-IN QUADRATURE CARRIER · AUX OPPOSITE SHIFT"
        } else {
            "MODULATOR SHIFTED BY THE CARRIER INPUT · AUX OPPOSITE SHIFT"
        };
    }
    let vocoder = cross_matter_algorithm(node.params.get("algorithm").copied().unwrap_or(0.17)) > 0.7;
    match (vocoder, internal) {
        (true, true) => "MODULATOR SHAPES THE BUILT-IN CARRIER · AUX CARRIER",
        (true, false) => "MODULATOR SHAPES THE CARRIER INPUT · AUX BOTH INPUTS",
        (false, true) => "BUILT-IN CARRIER × MODULATOR · AUX CARRIER",
        (false, false) => "CARRIER × MODULATOR · AUX BOTH INPUTS",
    }
}
fn paint_cross_matter_configuration(ui: &mut egui::Ui, rect: Rect, node: &Node, zoom: f32) {
    let guide = Rect::from_min_size(
        rect.min + vec2(18.0, 296.0) * zoom,
        vec2(324.0, 16.0) * zoom,
    );
    ui.painter().text(
        guide.center(),
        Align2::CENTER_CENTER,
        cross_matter_guide(node),
        crate::panel_skin::print_font(ui.ctx(), 8.5 * zoom),
        INK,
    );
    ui.interact(guide, ui.id().with(("cross-matter-guide", &node.id)), Sense::hover())
        .on_hover_text("Configured signal path from the pinned Warps source. ALGORITHM morphs through crossfade, fold, analog ring, digital ring, XOR, comparator and vocoder; TIMBRE is each algorithm's parameter. LEVEL 1 and LEVEL 2 set the input levels; with the built-in carrier LEVEL 1 sets its pitch and its CV input becomes 1 V/oct, while the carrier input modulates the carrier's frequency. Algorithm and timbre CV add voltage/5; a patched level CV scales its knob by voltage/5. In frequency-shifter mode the knobs become shift (or phase), up/down balance, feedback and dry/wet. The kernel runs at 96 kHz; output lamps show actual delayed output volts.");
}
