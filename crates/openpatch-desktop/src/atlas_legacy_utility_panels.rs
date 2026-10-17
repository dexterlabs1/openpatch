// Configured guides for legacy scalar modules. These are not engine telemetry.
// The legacy oscillator/DC values are internal signal units, not scaled volts.
fn atlas_legacy_utility_kind(kind: &str) -> bool {
    matches!(kind, "oscillator" | "constant" | "octave")
}
fn atlas_legacy_utility_control(kind: &str, name: &str) -> (Pos2, f32) {
    match (kind, name) {
        ("oscillator", "frequency") => (pos2(60.0, 117.0), 31.0),
        ("oscillator", _) => (pos2(60.0, 225.0), 29.0),
        ("constant", _) => (pos2(60.0, 128.0), 32.0),
        _ => (pos2(90.0, 118.0), 32.0),
    }
}
fn atlas_legacy_utility_value_center(kind: &str, name: &str) -> Pos2 {
    match (kind, name) {
        ("oscillator", "frequency") => pos2(60.0, 164.0),
        ("oscillator", _) => pos2(60.0, 272.0),
        ("constant", _) => pos2(60.0, 176.0),
        _ => pos2(90.0, 166.0),
    }
}
fn paint_atlas_legacy_utility_print(
    painter: &egui::Painter,
    rect: Rect,
    kind: &str,
    name: &str,
    zoom: f32,
) {
    let (x, y, label) = match (kind, name) {
        ("oscillator", "frequency") => (60.0, 70.0, "FREQUENCY"),
        ("oscillator", _) => (60.0, 180.0, "AMPLITUDE"),
        ("constant", _) => (60.0, 80.0, "VALUE"),
        _ => (90.0, 70.0, "OCTAVES"),
    };
    painter.text(
        rect.min + vec2(x, y) * zoom,
        Align2::CENTER_CENTER,
        label,
        crate::panel_skin::print_font(painter.ctx(), 10.5 * zoom),
        INK,
    );
}
fn paint_atlas_legacy_utility_configuration(
    painter: &egui::Painter,
    node: &Node,
    rect: Rect,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    let p = |x, y| {
        let y = if node.kind == "octave" && y >= 200.0 {
            y - 7.0
        } else {
            y
        };
        rect.min + vec2(x, y) * zoom
    };
    let text = |x, y, label: &str, size, color| {
        painter.text(
            p(x, y),
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    };
    let quiet = Color32::from_rgb(133, 143, 123);
    let ink = Color32::from_rgb(195, 200, 180);
    let grid = Color32::from_rgb(45, 59, 44);
    let amber = Color32::from_rgb(229, 183, 103);
    let width = match node.kind.as_str() {
        "oscillator" => 210.0,
        "constant" => 120.0,
        _ => 180.0,
    };
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout { width },
        zoom,
    };
    let start = crate::lighting::next(painter);
    match node.kind.as_str() {
        "oscillator" => {
            // Fixed ±1 amplitude axis; one cycle is shown regardless of Hz.
            // Zero frequency is shown separately: phase is held, not reset.
            draw.screen(
                Rect::from_min_max(p(119.0, 83.0), p(196.0, 285.0)),
                lighting,
            );
            text(157.5, 99.0, "CONFIGURED", 8.0, ink);
            text(157.5, 114.0, "FM = 0 Hz", 8.0, quiet);
            let amplitude = atlas_voice_parameter(node, "amplitude", 0.2, 0.0, 1.0);
            let frequency = atlas_voice_parameter(node, "frequency", 220.0, 0.0, 20000.0);
            for y in [151.0, 189.0, 227.0] {
                painter.line_segment([p(129.0, y), p(187.0, y)], Stroke::new(0.6 * zoom, grid));
            }
            for x in [129.0, 158.0, 187.0] {
                painter.line_segment([p(x, 151.0), p(x, 227.0)], Stroke::new(0.6 * zoom, grid));
            }
            text(157.5, 137.0, "+1", 8.0, quiet);
            text(157.5, 241.0, "−1", 8.0, quiet);
            if frequency > 0.0 {
                let points = (0..=64)
                    .map(|i| {
                        let phase = f64::from(i) / 64.0;
                        p(
                            129.0 + 58.0 * phase as f32,
                            189.0
                                - (amplitude * (phase * std::f64::consts::TAU).sin() * 38.0) as f32,
                        )
                    })
                    .collect();
                painter.add(egui::Shape::line(points, Stroke::new(1.6 * zoom, amber)));
                text(157.5, 260.0, "ONE CYCLE", 8.0, ink);
            } else {
                text(157.5, 182.0, "PHASE", 8.0, amber);
                text(157.5, 196.0, "HELD", 8.0, amber);
                text(157.5, 260.0, "0 Hz", 8.0, ink);
            }
            text(157.5, 274.0, "RAW UNITS", 8.0, quiet);
            text(105.0, 291.0, "Hz = CLAMP(FREQ + FM, 0, .45fs)", 8.0, INK);
        }
        "constant" => {
            text(60.0, 199.0, "OUT = VALUE", 9.0, INK);
            draw.screen(
                Rect::from_min_max(p(14.0, 215.0), p(106.0, 293.0)),
                lighting,
            );
            let value = atlas_voice_parameter(node, "value", 0.0, -16.0, 16.0);
            for y in [232.0, 254.0, 276.0] {
                painter.line_segment([p(23.0, y), p(97.0, y)], Stroke::new(0.6 * zoom, grid));
            }
            text(60.0, 223.0, "+16", 8.0, quiet);
            text(60.0, 285.0, "−16", 8.0, quiet);
            let y = 254.0 - value as f32 / 16.0 * 22.0;
            painter.line_segment([p(23.0, y), p(97.0, y)], Stroke::new(1.8 * zoom, amber));
            text(60.0, 309.0, "CONFIGURED DC", 8.0, INK);
            text(60.0, 325.0, "INTERNAL UNITS", 8.0, INK);
        }
        "octave" => {
            text(90.0, 190.0, "OUT = PITCH + OCT + ROUND(CV)", 8.0, INK);
            draw.screen(
                Rect::from_min_max(p(18.0, 211.0), p(162.0, 289.0)),
                lighting,
            );
            text(90.0, 222.0, "CONFIGURED · CV = 0 V", 8.0, ink);
            let octaves = atlas_voice_parameter(node, "octaves", 0.0, -4.0, 4.0).round();
            // Fixed x: ±2 V pitch; fixed y: ±6 V output. No auto-fit or clipping
            // is implied by the plot: the actual pitch arithmetic is unbounded
            // musically (with the Core's ±1,000,000 safety limit).
            painter.line_segment(
                [p(32.0, 256.0), p(148.0, 256.0)],
                Stroke::new(0.6 * zoom, grid),
            );
            painter.line_segment(
                [p(90.0, 235.0), p(90.0, 277.0)],
                Stroke::new(0.6 * zoom, grid),
            );
            painter.line_segment(
                [
                    p(32.0, 256.0 - ((-2.0 + octaves) * 3.5) as f32),
                    p(148.0, 256.0 - ((2.0 + octaves) * 3.5) as f32),
                ],
                Stroke::new(1.8 * zoom, amber),
            );
            text(90.0, 282.0, "PITCH −2…+2 V · OUT ±6 V", 8.0, quiet);
            text(90.0, 298.0, "1 V/OCT · CV HALF TIES AWAY", 8.0, INK);
        }
        _ => {}
    }
    lighting.preserve(painter, start);
}
