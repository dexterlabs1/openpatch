fn atlas_poly_kind(kind: &str) -> bool {
    matches!(
        kind,
        "poly_sum" | "poly_split" | "poly_merge" | "poly_constant"
    )
}
fn paint_atlas_poly_count(
    painter: &egui::Painter,
    screen: Rect,
    text: &str,
    available: bool,
    zoom: f32,
    font_size: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    if available {
        lighting.screen_glow(painter, screen, zoom);
    }
    painter.rect_filled(screen, 2.0 * zoom, Color32::from_rgb(15, 24, 18));
    painter.rect_stroke(
        screen,
        2.0 * zoom,
        Stroke::new(1.2 * zoom, Color32::from_rgb(126, 121, 94)),
        egui::StrokeKind::Outside,
    );
    let start = crate::lighting::next(painter);
    painter.text(
        screen.center(),
        Align2::CENTER_CENTER,
        text,
        crate::panel_skin::print_font(painter.ctx(), font_size * zoom),
        if available {
            Color32::from_rgb(155, 220, 197)
        } else {
            Color32::from_rgb(135, 145, 138)
        },
    );
    if available {
        lighting.preserve(painter, start);
    }
}
fn paint_atlas_poly_guide(painter: &egui::Painter, rect: Rect, kind: &str, zoom: f32) {
    let (x, y, text) = match kind {
        "poly_sum" => (75.0, 78.0, "OUT = LEVEL × Σ IN"),
        "poly_split" => (150.0, 148.0, "CH 1–16 → OUT 1–16 · MISSING = 0 V"),
        "poly_merge" => (150.0, 66.0, "FIRST LANE OF IN 1–16 → POLY CH 1–16"),
        _ => return,
    };
    painter.text(
        rect.min + vec2(x, y) * zoom,
        Align2::CENTER_CENTER,
        text,
        crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
        INK,
    );
}
fn paint_atlas_poly_sum_print(painter: &egui::Painter, rect: Rect, zoom: f32) {
    painter.text(
        rect.min + vec2(75.0, 97.0) * zoom,
        Align2::CENTER_CENTER,
        "LEVEL",
        crate::panel_skin::print_font(painter.ctx(), 11.0 * zoom),
        INK,
    );
}
