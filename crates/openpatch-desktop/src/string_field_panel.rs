// String Field shares one face, native parameter gestures and sockets in the
// rack and browser. Captions follow the resonator model; the guide states the
// configured model and voices, while only engine telemetry lights the output
// and strum lamps.
fn string_field_model(node: &Node) -> i32 {
    node.params
        .get("model")
        .copied()
        .unwrap_or(0.0)
        .round()
        .clamp(0.0, 5.0) as i32
}
fn string_field_polyphony(node: &Node) -> i32 {
    node.params
        .get("polyphony")
        .copied()
        .unwrap_or(1.0)
        .round()
        .clamp(1.0, 4.0) as i32
}
const STRING_FIELD_COLUMNS: [f32; 5] = [43.0, 119.0, 195.0, 271.0, 347.0];
fn string_field_control(name: &str) -> (Pos2, f32) {
    let (column, large) = match name {
        "frequency" => (0, true),
        "structure" => (1, true),
        "brightness" => (2, true),
        "damping" => (3, true),
        "position" => (4, true),
        "frequency_amount" => (0, false),
        "structure_amount" => (1, false),
        "brightness_amount" => (2, false),
        "damping_amount" => (3, false),
        "position_amount" => (4, false),
        _ => unreachable!("String Field continuous control"),
    };
    if large {
        (pos2(STRING_FIELD_COLUMNS[column], 142.0), 24.0)
    } else {
        (pos2(STRING_FIELD_COLUMNS[column], 234.0), 16.0)
    }
}
fn string_field_value_center(name: &str) -> Pos2 {
    let (center, _) = string_field_control(name);
    pos2(center.x, if center.y < 200.0 { 185.0 } else { 266.0 })
}
fn string_field_value_width(name: &str) -> f32 {
    let (center, _) = string_field_control(name);
    if center.y < 200.0 {
        72.0
    } else {
        62.0
    }
}
fn string_field_selector(name: &str) -> Option<(Rect, &'static str, &'static [&'static str])> {
    let (x, width, label, choices): (_, _, _, &[&str]) = match name {
        "polyphony" => (
            16.0,
            112.0,
            "POLYPHONY",
            &["1 VOICE", "2 VOICES", "3 VOICES", "4 VOICES"],
        ),
        "model" => (
            140.0,
            234.0,
            "MODEL",
            &[
                "MODAL",
                "SYMPATHETIC STRINGS",
                "INHARMONIC STRING",
                "FM VOICE",
                "CHORD STRINGS",
                "STRING AND REVERB",
            ],
        ),
        _ => return None,
    };
    Some((
        Rect::from_min_size(pos2(x, 66.0), vec2(width, 24.0)),
        label,
        choices,
    ))
}
fn string_field_port(input: bool, index: usize) -> Pos2 {
    if input {
        if index < 3 {
            // Exciter, strum and V/OCT along the bottom edge, clear of the
            // panel screws.
            pos2(38.0 + index as f32 * 60.0, 376.0)
        } else {
            // Five CV inputs under the controls they modulate.
            pos2(STRING_FIELD_COLUMNS[index - 3], 330.0)
        }
    } else {
        pos2(278.0 + index as f32 * 60.0, 376.0)
    }
}
fn string_field_port_label(port: &str) -> &str {
    match port {
        "in" => "IN",
        "strum" => "STRUM",
        "v_oct" => "V/OCT",
        "frequency_cv" => "FREQ",
        "structure_cv" => "STRUCT",
        "brightness_cv" => "BRIGHT",
        "damping_cv" => "DAMP",
        "position_cv" => "POS",
        other => port_label(other),
    }
}
fn string_field_output_label(port: &str) -> &str {
    match port {
        "odd" => "ODD",
        _ => "EVEN",
    }
}
fn string_field_caption(node: &Node, name: &str) -> &'static str {
    let model = string_field_model(node);
    match (name, model) {
        ("frequency", _) => "FREQUENCY",
        ("structure", 1) => "INTERVALS",
        ("structure", 2 | 5) => "STIFFNESS",
        ("structure", 3) => "RATIO",
        ("structure", 4) => "CHORD",
        ("structure", _) => "STRUCTURE",
        ("brightness", 3) => "INDEX",
        ("brightness", _) => "BRIGHTNESS",
        ("damping", 3) => "DECAY",
        ("damping", _) => "DAMPING",
        ("position", 3) => "FEEDBACK",
        ("position", _) => "POSITION",
        _ => "AMOUNT",
    }
}
fn paint_string_field_control_print(
    painter: &egui::Painter,
    rect: Rect,
    node: &Node,
    name: &str,
    zoom: f32,
) {
    let (center, radius) = string_field_control(name);
    painter.text(
        rect.min + vec2(center.x, center.y - radius - 13.0) * zoom,
        Align2::CENTER_CENTER,
        string_field_caption(node, name),
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
}
/// Note name of the frequency knob with V/OCT at 0 V: MIDI note 12 (C0) plus
/// the knob's semitones, and the cents away from that note.
fn string_field_note(value: f32) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let midi = 12.0 + value;
    let nearest = midi.round();
    let cents = ((midi - nearest) * 100.0).round() as i32;
    let note = nearest as i32;
    let name = format!("{}{}", NAMES[note.rem_euclid(12) as usize], note / 12 - 1);
    if cents.abs() < 5 {
        name
    } else if cents < 0 {
        format!("{name} −{}¢", -cents)
    } else {
        format!("{name} +{cents}¢")
    }
}
/// Chord the structure knob selects in the chord-strings model; one and two
/// voices play chords, three and four voices play intervals.
fn string_field_chord(polyphony: i32, structure: f32) -> &'static str {
    let index = (structure * 10.0).round().clamp(0.0, 10.0) as usize;
    if polyphony <= 2 {
        [
            "Octave", "Fifth", "Sus 4", "Minor", "Minor 7", "Minor 9", "Minor 11", "Six nine",
            "Major 9", "Major 7", "Major",
        ][index]
    } else {
        [
            "Octave down",
            "Second",
            "Minor third",
            "Major third",
            "Fourth",
            "Fifth",
            "Sixth",
            "Minor 7th",
            "Major 7th",
            "Octave",
            "Two octaves",
        ][index]
    }
}
fn string_field_signed_percent(value: f32) -> String {
    let percent = (value * 100.0).round();
    if percent == 0.0 {
        "0%".into()
    } else if percent < 0.0 {
        format!("−{:.0}%", -percent)
    } else {
        format!("+{percent:.0}%")
    }
}
fn string_field_value(node: &Node, name: &str, value: f32) -> String {
    match name {
        "frequency" => string_field_note(value),
        "structure" if string_field_model(node) == 4 => {
            string_field_chord(string_field_polyphony(node), value).into()
        }
        // FM feedback is negative below the centre.
        "position" if string_field_model(node) == 3 => {
            string_field_signed_percent((value - 0.5) * 2.0)
        }
        name if name.ends_with("_amount") => string_field_signed_percent(value),
        _ => format!("{:.0}%", value * 100.0),
    }
}
fn string_field_guide(node: &Node) -> String {
    let polyphony = string_field_polyphony(node);
    let strings = 8 / polyphony;
    let body = match string_field_model(node) {
        0 => format!("MODAL BODY OF UP TO {} MODES", 64 / polyphony - 4 - (64 / polyphony - 4) % 2),
        1 => format!("{strings} SYMPATHETIC STRINGS"),
        2 => "INHARMONIC STRING".into(),
        3 => "TWO-OPERATOR FM".into(),
        4 => format!("{strings} STRINGS TUNED TO A CHORD"),
        _ => "STRING WITH STEREO REVERB".into(),
    };
    if polyphony == 1 {
        format!("{body} · 1 VOICE")
    } else {
        format!("{body} · {polyphony} VOICES · ODD AND EVEN VOICES SPLIT")
    }
}
fn string_field_strum_lamp(lights: Option<&ModuleLightState>) -> f32 {
    lights
        .filter(|state| state.count > 2)
        .map_or(0.0, |state| state.brightness[2][1].clamp(0.0, 1.0))
}
fn paint_string_field_configuration(
    ui: &mut egui::Ui,
    rect: Rect,
    node: &Node,
    zoom: f32,
    lights: Option<&ModuleLightState>,
    lighting: &crate::lighting::RoomLighting,
) {
    let guide = Rect::from_min_size(
        rect.min + vec2(16.0, 282.0) * zoom,
        vec2(358.0, 16.0) * zoom,
    );
    ui.painter().text(
        guide.center(),
        Align2::CENTER_CENTER,
        string_field_guide(node),
        crate::panel_skin::print_font(ui.ctx(), 8.5 * zoom),
        INK,
    );
    ui.interact(guide, ui.id().with(("string-field-guide", &node.id)), Sense::hover())
        .on_hover_text("Resonator from the pinned Rings source. With nothing in IN, a built-in exciter plays each strum. With STRUM unpatched, the module strums by itself when V/OCT changes by more than about half a semitone, or on each new note it hears at IN when only IN is patched; with neither patched it stays silent. With V/OCT unpatched, FREQUENCY sets the note alone; with it patched, FREQUENCY moves in semitones. Each strum moves to the next voice so earlier notes ring on. With one voice the outputs carry two sets of partials; with more, odd and even voices go to separate outputs, and a single patched output carries both. CV inputs add through their AMOUNT knobs. Captions follow the model. The strum lamp and output lamps show the module's actual state.");
    // Measured strum beside the STRUM jack.
    let center = rect.min + vec2(122.0, 364.0) * zoom;
    let strength = string_field_strum_lamp(lights);
    ui.painter()
        .circle_filled(center, 3.0 * zoom, Color32::from_gray(78));
    lighting.circle(
        ui.painter(),
        center,
        2.2 * zoom,
        Color32::from_rgb(238, 196, 120).gamma_multiply(0.12 + strength * 0.88),
        strength,
        0.12,
    );
}
