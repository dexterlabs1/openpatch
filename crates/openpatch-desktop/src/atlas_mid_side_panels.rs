// Static source-law guides. Width is configured here; no signal is measured.
fn atlas_mid_side_kind(kind: &str) -> bool {
    matches!(kind, "mid_side" | "stock_mid_side")
}
fn atlas_mid_side_control(name: &str) -> (Pos2, f32) {
    (
        pos2(if name == "encode_width" { 75.0 } else { 225.0 }, 133.0),
        31.0,
    )
}
fn atlas_mid_side_value_center(name: &str) -> Pos2 {
    pos2(atlas_mid_side_control(name).0.x, 182.0)
}
fn paint_atlas_mid_side_print(painter: &egui::Painter, rect: Rect, name: &str, zoom: f32) {
    let x = atlas_mid_side_control(name).0.x;
    for (y, label, size) in [
        (
            74.0,
            if name == "encode_width" {
                "ENCODER"
            } else {
                "DECODER"
            },
            10.0,
        ),
        (88.0, "WIDTH", 9.0),
    ] {
        painter.text(
            rect.min + vec2(x, y) * zoom,
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            INK,
        );
    }
}
fn paint_atlas_mid_side_configuration(
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
    let start = crate::lighting::next(painter);
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout { width: 300.0 },
        zoom,
    };
    draw.screen(
        Rect::from_min_max(p(18.0, 193.0), p(282.0, 216.0)),
        lighting,
    );
    let encoder = Color32::from_rgb(229, 183, 103);
    let decoder = Color32::from_rgb(109, 192, 177);
    for (x, y, label, color) in [
        (75.0, 198.0, "M = (L + R) / 2", encoder),
        (75.0, 210.0, "S = (L − R) / 2 × E", encoder),
        (225.0, 198.0, "L = M + S × D", decoder),
        (225.0, 210.0, "R = M − S × D", decoder),
    ] {
        text(x, y, label, 8.5, color);
    }
    lighting.preserve(painter, start);
    // routing::width multiplies its 10 V normal; StockMidSide adds a 0 V
    // normal. Neither guide substitutes knob position for measured CV.
    let stock = node.kind == "stock_mid_side";
    text(
        150.0,
        267.0,
        if stock {
            "E,D = max(W + CV / 5, 0) · NORMAL 0 V · UNCAPPED"
        } else {
            "E,D = clamp(W × CV / 10, 0, 2) · NORMAL 10 V"
        },
        8.0,
        INK,
    );
    // Each unpatched decoder input independently uses its encoder output.
    text(
        150.0,
        333.0,
        "M / S NORMAL: ENCODER · INDEPENDENT",
        8.0,
        MUTED,
    );
}
