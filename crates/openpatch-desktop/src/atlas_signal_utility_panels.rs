fn atlas_signal_utility_kind(kind: &str) -> bool {
    matches!(kind, "fade" | "compare" | "process" | "gate" | "gates")
}
fn atlas_signal_utility_control(kind: &str, name: &str) -> (Pos2, f32) {
    match (kind, name) {
        ("fade", _) => atlas_fade_control(name),
        ("gates", _) => (pos2(78.0, 189.0), 31.0),
        ("compare", _) => (pos2(120.0, 125.0), 31.0),
        ("process", _) => (pos2(120.0, 116.0), 31.0),
        ("gate", "frequency") => (pos2(75.0, 125.0), 31.0),
        _ => (pos2(75.0, 248.0), 27.0),
    }
}
fn atlas_signal_utility_value_center(kind: &str, name: &str) -> Pos2 {
    match (kind, name) {
        ("fade", _) => atlas_fade_value_center(name),
        ("gates", _) => pos2(78.0, 234.0),
        ("compare", _) => pos2(120.0, 170.0),
        ("process", _) => pos2(120.0, 161.0),
        ("gate", "frequency") => pos2(75.0, 170.0),
        _ => pos2(75.0, 290.0),
    }
}
fn paint_atlas_signal_utility_print(
    painter: &egui::Painter,
    rect: Rect,
    kind: &str,
    name: &str,
    zoom: f32,
) {
    if kind == "fade" {
        paint_atlas_fade_print(painter, rect, name, zoom);
        return;
    }
    let (label, y) = match (kind, name) {
        ("gates", _) => ("LENGTH", 140.0),
        ("compare", _) => ("B OFFSET", 76.0),
        ("process", _) => ("SLEW", 70.0),
        ("gate", "frequency") => ("FREQUENCY", 76.0),
        _ => ("DUTY", 205.0),
    };
    painter.text(
        rect.min + vec2(atlas_signal_utility_control(kind, name).0.x, y) * zoom,
        Align2::CENTER_CENTER,
        label,
        crate::panel_skin::print_font(painter.ctx(), 11.0 * zoom),
        INK,
    );
}
fn paint_atlas_signal_utility_configuration(
    painter: &egui::Painter,
    node: &Node,
    rect: Rect,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    let p = |x, y| rect.min + vec2(x, y) * zoom;
    let text = |x, y, label: &str, size| {
        painter.text(
            p(x, y),
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            INK,
        );
    };
    match node.kind.as_str() {
        "fade" => paint_atlas_fade_configuration(painter, node, rect, zoom, lighting),
        "gates" => {
            text(120.0, 61.0, "TIME = LENGTH × 2^CV", 8.5);
            text(120.0, 309.0, "LENGTH CV CLAMP: −32…+32 V", 8.5);
        }
        "compare" => {
            text(120.0, 310.0, "B′ = B + OFFSET · CLIP TO ±|B′|", 8.5);
            text(120.0, 323.0, "LIM = A − CLIP · GATES 0 / 10 V", 8.5);
        }
        "process" => {
            text(120.0, 248.0, "GATE: ≥2 V HIGH · ≤0.1 V LOW", 8.5);
            text(120.0, 310.0, "0 ms/V = INSTANT · CV IGNORED", 8.5);
            text(120.0, 323.0, "OTHER: ms/V × 2^(SLEW CV)", 8.5);
        }
        "gate" => {
            let frequency = node
                .params
                .get("frequency")
                .copied()
                .unwrap_or(2.0)
                .clamp(0.05, 100.0);
            let period = 1.0 / frequency;
            let label = if period >= 1.0 {
                format!("PERIOD {:.2} s", period)
            } else {
                format!("PERIOD {:.1} ms", period * 1000.0)
            };
            text(75.0, 190.0, &label, 9.0);
            let start = crate::lighting::next(painter);
            let draw = AtlasVoicePaint {
                painter,
                origin: rect.min,
                layout: AtlasVoiceLayout { width: 150.0 },
                zoom,
            };
            draw.screen(
                Rect::from_min_max(p(18.0, 318.0), p(132.0, 339.0)),
                lighting,
            );
            let duty = node
                .params
                .get("duty")
                .copied()
                .unwrap_or(0.5)
                .clamp(0.01, 0.99);
            let edge = 26.0 + 98.0 * duty;
            painter.add(egui::Shape::line(
                vec![
                    p(26.0, 323.0),
                    p(edge, 323.0),
                    p(edge, 335.0),
                    p(124.0, 335.0),
                ],
                Stroke::new(1.4 * zoom, Color32::from_rgb(229, 183, 103)),
            ));
            lighting.preserve(painter, start);
            text(75.0, 309.0, "CONFIGURED · 0 / 10 V", 8.5);
        }
        _ => {}
    }
}
