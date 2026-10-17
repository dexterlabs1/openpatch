// Complementary crossfade from core/mixing.rs. This display is saved configuration,
// not a per-voice CV measurement; the complementary output swaps positive gains.
fn atlas_fade_control(name: &str) -> (Pos2, f32) {
    if name == "position" {
        (pos2(65.0, 121.0), 31.0)
    } else {
        (pos2(175.0, 121.0), 26.0)
    }
}

fn atlas_fade_value_center(name: &str) -> Pos2 {
    pos2(atlas_fade_control(name).0.x, 165.0)
}

fn paint_atlas_fade_print(painter: &egui::Painter, rect: Rect, name: &str, zoom: f32) {
    let x = atlas_fade_control(name).0.x;
    for (y, label, size) in if name == "position" {
        [(70.0, "POSITION", 10.0), (82.0, "A → B · 0…1", 7.0)]
    } else {
        [(70.0, "CV AMOUNT", 10.0), (82.0, "−1…+1 ×", 7.0)]
    } {
        painter.text(
            rect.min + vec2(x, y) * zoom,
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            INK,
        );
    }
}

fn paint_atlas_fade_configuration(
    painter: &egui::Painter,
    node: &Node,
    rect: Rect,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    let p = |x, y| rect.min + vec2(x, y) * zoom;
    let text = |x, y, label: &str, size, color| {
        painter.text(
            p(x, y),
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    };
    text(
        120.0,
        185.0,
        "x = clamp(POS + CV / 10 × AMOUNT, 0, 1)",
        7.5,
        INK,
    );
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout { width: 240.0 },
        zoom,
    };
    let start = crate::lighting::next(painter);
    draw.screen(
        Rect::from_min_max(p(20.0, 198.0), p(220.0, 274.0)),
        lighting,
    );
    let ink = Color32::from_rgb(195, 200, 180);
    let amber = Color32::from_rgb(229, 183, 103);
    let cyan = Color32::from_rgb(109, 192, 177);
    let quiet = Color32::from_rgb(157, 169, 146);
    text(120.0, 208.0, "CONFIGURED OUT GAINS · CV = 0", 7.5, ink);
    let graph = |x: f32, gain: f32| p(46.0 + 148.0 * x, 252.0 - 32.0 * gain);
    for gain in [0.0, 0.5, 1.0] {
        painter.line_segment(
            [graph(0.0, gain), graph(1.0, gain)],
            Stroke::new(0.6 * zoom, quiet.gamma_multiply(0.3)),
        );
    }
    let sqrt = node.params.get("law").copied().unwrap_or(0.0) != 0.0;
    let gain = |x: f32| if sqrt { x.sqrt() } else { x };
    for (b, color) in [(false, amber), (true, cyan)] {
        let curve = (0..=48)
            .map(|i| {
                let x = i as f32 / 48.0;
                graph(x, gain(if b { x } else { 1.0 - x }))
            })
            .collect::<Vec<_>>();
        painter.add(egui::Shape::line(curve, Stroke::new(1.4 * zoom, color)));
    }
    let position = node
        .params
        .get("position")
        .copied()
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    painter.line_segment(
        [graph(position, 0.0), graph(position, 1.0)],
        Stroke::new(0.8 * zoom, ink.gamma_multiply(0.65)),
    );
    for (weight, color) in [(1.0 - position, amber), (position, cyan)] {
        painter.circle_filled(graph(position, gain(weight)), 2.0 * zoom, color);
    }
    text(34.0, 220.0, "1", 6.5, quiet);
    text(34.0, 252.0, "0", 6.5, quiet);
    text(46.0, 263.0, "A", 7.0, amber);
    text(120.0, 263.0, "COMPLEMENT: A ↔ B", 6.5, quiet);
    text(194.0, 263.0, "B", 7.0, cyan);
    lighting.preserve(painter, start);
    text(
        120.0,
        289.0,
        if sqrt {
            "SQUARE-ROOT LAW"
        } else {
            "LINEAR LAW"
        },
        9.0,
        INK,
    );
}
