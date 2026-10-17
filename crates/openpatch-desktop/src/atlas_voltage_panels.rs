// Source-backed configured transfer/gain guides; these are not live waveforms.
fn atlas_voltage_kind(kind: &str) -> bool {
    matches!(
        kind,
        "attenuverter" | "crossfade" | "rescale" | "quantizer" | "stock_quantizer"
    )
}
fn atlas_voltage_control(kind: &str, name: &str) -> (Pos2, f32) {
    match (kind, name) {
        ("attenuverter", "gain") => (pos2(45.0, 132.0), 29.0),
        ("attenuverter", _) => (pos2(135.0, 132.0), 29.0),
        ("crossfade", "law") => (pos2(105.0, 264.0), 10.0),
        _ => (pos2(90.0, 125.0), 32.0),
    }
}
fn atlas_voltage_value_center(kind: &str, name: &str) -> Pos2 {
    match (kind, name) {
        ("attenuverter", _) => pos2(atlas_voltage_control(kind, name).0.x, 177.0),
        ("crossfade", "law") => pos2(90.0, 288.0),
        _ => pos2(90.0, 171.0),
    }
}
fn paint_atlas_voltage_print(
    painter: &egui::Painter,
    rect: Rect,
    kind: &str,
    name: &str,
    zoom: f32,
) {
    let p = |x, y| rect.min + vec2(x, y) * zoom;
    let text = |x, y, label: &str, size, align| {
        painter.text(
            p(x, y),
            align,
            label,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            INK,
        );
    };
    if kind == "attenuverter" {
        let x = atlas_voltage_control(kind, name).0.x;
        text(
            x,
            76.0,
            if name == "gain" { "GAIN" } else { "OFFSET" },
            10.0,
            Align2::CENTER_CENTER,
        );
        text(
            x,
            88.0,
            if name == "gain" {
                "−1 TO +1 ×"
            } else {
                "−10 TO +10 V"
            },
            7.0,
            Align2::CENTER_CENTER,
        );
    } else if name == "law" {
        text(13.0, 264.0, "LAW", 8.0, Align2::LEFT_CENTER);
    } else {
        text(90.0, 74.0, "BLEND · A → B", 10.0, Align2::CENTER_CENTER);
    }
}
fn paint_atlas_voltage_configuration(
    painter: &egui::Painter,
    node: &Node,
    rect: Rect,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    let p = |x, y| rect.min + vec2(x, y) * zoom;
    let atten = node.kind == "attenuverter";
    let top = if atten { 206.0 } else { 189.0 };
    let bottom = if atten { 291.0 } else { 244.0 };
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout { width: 180.0 },
        zoom,
    };
    let start = crate::lighting::next(painter);
    draw.screen(Rect::from_min_max(p(18.0, top), p(162.0, bottom)), lighting);
    let ink = Color32::from_rgb(195, 200, 180);
    let amber = Color32::from_rgb(229, 183, 103);
    let cyan = Color32::from_rgb(109, 192, 177);
    let quiet = Color32::from_rgb(157, 169, 146);
    let text = |x, y, label: &str, size, color| {
        painter.text(
            p(x, y),
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    };
    let line = |a, b, color, thickness| {
        painter.line_segment([a, b], Stroke::new(thickness * zoom, color));
    };
    if atten {
        text(90.0, 218.0, "CONFIGURED TRANSFER", 8.0, ink);
        let graph = |x: f32, y: f32| p(39.0 + (x + 10.0) * 5.4, 257.0 - y * 1.05);
        for y in [-20.0, 0.0, 20.0] {
            line(
                graph(-10.0, y),
                graph(10.0, y),
                quiet.gamma_multiply(0.35),
                0.6,
            );
        }
        line(
            graph(0.0, -20.0),
            graph(0.0, 20.0),
            quiet.gamma_multiply(0.35),
            0.6,
        );
        let gain = node
            .params
            .get("gain")
            .copied()
            .unwrap_or(0.0)
            .clamp(-1.0, 1.0);
        let offset = node
            .params
            .get("offset")
            .copied()
            .unwrap_or(0.0)
            .clamp(-10.0, 10.0);
        line(
            graph(-10.0, -10.0 * gain + offset),
            graph(10.0, 10.0 * gain + offset),
            amber,
            1.6,
        );
        text(27.0, 236.0, "+20 V", 6.0, quiet);
        text(27.0, 278.0, "−20 V", 6.0, quiet);
        text(39.0, 285.0, "−10", 6.0, quiet);
        text(93.0, 285.0, "IN V", 6.0, quiet);
        text(147.0, 285.0, "+10", 6.0, quiet);
        lighting.preserve(painter, start);
        text(90.0, 196.0, "OUT = GAIN × IN + OFFSET", 7.5, INK);
        text(90.0, 306.0, "UNPATCHED IN = 10 V · MONO", 7.0, MUTED);
    } else {
        text(90.0, 198.0, "CONFIGURED A / B GAINS", 7.0, ink);
        let graph = |x: f32, y: f32| p(32.0 + x * 116.0, 230.0 - y * 23.0);
        line(
            graph(0.0, 0.0),
            graph(1.0, 0.0),
            quiet.gamma_multiply(0.4),
            0.6,
        );
        let power = node.params.get("law").copied().unwrap_or(0.0) >= 0.5;
        let curve = |b: bool| -> Vec<Pos2> {
            (0..=48)
                .map(|i| {
                    let x = i as f32 / 48.0;
                    let g = if b { x } else { 1.0 - x };
                    graph(x, if power { g.sqrt() } else { g })
                })
                .collect()
        };
        painter.add(egui::Shape::line(
            curve(false),
            Stroke::new(1.3 * zoom, amber),
        ));
        painter.add(egui::Shape::line(
            curve(true),
            Stroke::new(1.3 * zoom, cyan),
        ));
        let mix = node
            .params
            .get("mix")
            .copied()
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        line(
            graph(mix, 0.0),
            graph(mix, 1.0),
            ink.gamma_multiply(0.6),
            0.7,
        );
        text(25.0, 208.0, "1", 6.0, quiet);
        text(25.0, 230.0, "0", 6.0, quiet);
        text(40.0, 238.0, "A", 6.5, amber);
        text(140.0, 238.0, "B", 6.5, cyan);
        lighting.preserve(painter, start);
    }
}
