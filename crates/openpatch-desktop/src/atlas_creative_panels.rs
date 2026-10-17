// Native functional guides for the original macro/drum voices and compressor.
fn atlas_creative_kind(kind: &str) -> bool {
    matches!(kind, "macro_voice" | "drum_voice" | "sidechain_compressor")
}
fn atlas_creative_control(kind: &str, name: &str) -> (Pos2, f32) {
    match kind {
        "macro_voice" => {
            let (column, row) = match name {
                "frequency" => (0, 0),
                "harmonics" => (1, 0),
                "timbre" => (2, 0),
                "morph" => (0, 1),
                "decay" => (1, 1),
                _ => (2, 1),
            };
            (
                pos2(
                    50.0 + 100.0 * column as f32,
                    if row == 0 { 143.0 } else { 245.0 },
                ),
                if row == 0 { 25.0 } else { 22.0 },
            )
        }
        "drum_voice" => {
            let (column, row) = match name {
                "mode" => (0, 0),
                "pitch" => (1, 0),
                "decay" => (2, 0),
                "tone" => (0, 1),
                "drive" => (1, 1),
                _ => (2, 1),
            };
            (
                pos2(
                    60.0 + 120.0 * column as f32,
                    if row == 0 { 112.0 } else { 253.0 },
                ),
                if row == 0 { 29.0 } else { 27.0 },
            )
        }
        _ => match name {
            "makeup_db" => (pos2(42.0, 297.0), 18.0),
            "threshold_db" => (pos2(70.0, 103.0), 25.0),
            "ratio" => (pos2(210.0, 103.0), 25.0),
            "knee_db" => (pos2(350.0, 103.0), 25.0),
            "attack_ms" => (pos2(70.0, 210.0), 23.0),
            "release_ms" => (pos2(210.0, 210.0), 23.0),
            _ => (pos2(350.0, 210.0), 23.0),
        },
    }
}
fn atlas_creative_value_center(kind: &str, name: &str) -> Pos2 {
    let (c, _) = atlas_creative_control(kind, name);
    pos2(
        c.x,
        match kind {
            "macro_voice" => {
                if c.y < 200.0 {
                    189.0
                } else {
                    285.0
                }
            }
            "drum_voice" => {
                if c.y < 200.0 {
                    164.0
                } else {
                    298.0
                }
            }
            _ => {
                if name == "makeup_db" {
                    329.0
                } else if c.y < 200.0 {
                    150.0
                } else {
                    250.0
                }
            }
        },
    )
}
fn paint_atlas_creative_control_print(
    painter: &egui::Painter,
    rect: Rect,
    kind: &str,
    name: &str,
    zoom: f32,
) {
    let (c, _) = atlas_creative_control(kind, name);
    let y = match kind {
        "macro_voice" => {
            if c.y < 200.0 {
                97.0
            } else {
                205.0
            }
        }
        "drum_voice" => {
            if c.y < 200.0 {
                65.0
            } else {
                205.0
            }
        }
        _ => {
            if name == "makeup_db" {
                267.0
            } else if c.y < 200.0 {
                57.0
            } else {
                170.0
            }
        }
    };
    painter.text(
        rect.min + vec2(c.x, y) * zoom,
        Align2::CENTER_CENTER,
        parameter_label(name),
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
}
fn paint_atlas_creative_configuration(
    ui: &mut egui::Ui,
    rect: Rect,
    node: &Node,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    if node.kind == "sidechain_compressor" {
        paint_atlas_compressor_guide(ui, rect, node, zoom, lighting);
        return;
    }
    let painter = ui.painter();
    let p = |x, y| rect.min + vec2(x, y) * zoom;
    let width = rect.width() / zoom;
    let macro_voice = node.kind == "macro_voice";
    let y = if macro_voice { 301.0 } else { 189.0 };
    let half_height = if macro_voice { 7.0 } else { 8.0 };
    let band = Rect::from_min_max(p(14.0, y - half_height), p(width - 14.0, y + half_height));
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout { width },
        zoom,
    };
    let start = crate::lighting::next(painter);
    draw.screen(band, lighting);
    let ink = Color32::from_rgb(195, 200, 180);
    let blocks: Vec<(&str, f32)> = if macro_voice {
        vec![
            ("ENGINE", 45.0),
            ("LPG", 111.0),
            ("DC", 167.0),
            ("OUT / AUX", 240.0),
        ]
    } else {
        let source = match node.params.get("mode").copied().unwrap_or(0.0) as i32 {
            1 => "NOISE",
            2 => "METAL",
            _ => "SINE",
        };
        vec![
            (source, 49.0),
            ("DRIVE", 126.0),
            ("ENV", 199.0),
            ("LEVEL → OUT", 285.0),
        ]
    };
    for (i, (label, x)) in blocks.iter().enumerate() {
        painter.text(
            p(*x, y),
            Align2::CENTER_CENTER,
            *label,
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            ink,
        );
        if i + 1 < blocks.len() {
            let next = blocks[i + 1].1;
            let a = *x + if i == 0 && !macro_voice { 23.0 } else { 25.0 };
            let b = next - if i == 2 { 39.0 } else { 22.0 };
            if b > a {
                painter.arrow(
                    p(a, y),
                    vec2(b - a, 0.0) * zoom,
                    Stroke::new(0.8 * zoom, ink),
                );
            }
        }
    }
    lighting.preserve(painter, start);
    if !macro_voice {
        let decay = atlas_voice_parameter(node, "decay", 0.2, 0.005, 4.0);
        painter.text(
            p(264.0, 24.0),
            Align2::CENTER_CENTER,
            format!("ENV T60 {:.0} ms", decay * 1000.0),
            crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
            INK,
        );
    }
    ui.interact(band,ui.id().with(("atlas-creative-guide",&node.id)),Sense::hover()).on_hover_text(if macro_voice {
        "Configured signal path, not a sampled waveform. PITCH + FM is exponential 1 V/oct; harmonic/timbre/morph CV adds CV/5 then clamps. The selected engine feeds a one-pole LPG filter, amplitude/level stages and a 5 Hz DC blocker. With trigger channels, the envelope decays and controls amplitude and cutoff; with no trigger channels, amplitude is continuous. LEVEL CV normals to 10 V and clamps to 0–10 V. OUT/AUX share the envelope; voices follow pitch/trigger width. This is Plaits-inspired, not a bit-identical Plaits port."
    } else {
        "Configured drum path, not a live waveform. A Schmitt trigger (0.1/1 V) resets the envelope and samples ACCENT, normal 10 V and clamped to 0–10 V. Kick uses a sine with a tone-dependent pitch sweep, Clap uses filtered noise/bursts, Hi-hat uses six pulse oscillators plus noise. The source passes through normalized tanh drive then exponential amplitude, LEVEL and accent. ENV T60 is the amplitude envelope's time to −60 dB, not measured audio decay. Only channel one is processed."
    });
}
