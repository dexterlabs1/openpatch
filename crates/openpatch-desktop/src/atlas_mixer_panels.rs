// Six-input routing guides describe saved configuration, never audio activity.
// Both engines sum six inputs then apply level, optional averaging and polarity.
// Their lane transfer/arithmetic differences remain in the processors/tooltips.
fn paint_atlas_six_mix_print(painter: &egui::Painter, rect: Rect, node: &Node, zoom: f32) {
    let point = |x, y| rect.min + vec2(x, y) * zoom;
    let width = rect.width() / zoom;
    let text = |x, y, value: &str, size| {
        painter.text(
            point(x, y),
            Align2::CENTER_CENTER,
            value,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            INK,
        );
    };
    let center = width * 0.5;
    text(center, 65.0, "LEVEL", 11.0);
    // Endpoints of the shared 270-degree linear rotary scale.
    text(center - 43.0, 168.0, "0", 9.0);
    text(center + 43.0, 168.0, "1", 9.0);
    let average = node.params.get("average").copied().unwrap_or(0.0) >= 0.5;
    let invert = node.params.get("invert").copied().unwrap_or(0.0) >= 0.5;
    let y = 233.0;
    for (x1, x2) in [(42.0, 62.0), (90.0, 113.0), (157.0, 180.0)] {
        painter.arrow(
            point(x1, y),
            vec2(x2 - x1, 0.0) * zoom,
            Stroke::new(0.8 * zoom, MUTED),
        );
    }
    text(29.0, y, "1…6", 9.0);
    text(76.0, y, if average { "Σ/N" } else { "Σ" }, 11.0);
    text(135.0, y, "× LEVEL", 9.0);
    text(198.0, y, if invert { "− OUT" } else { "+ OUT" }, 9.0);
    text(center, 349.0, "N = INPUTS WITH CHANNELS", 8.0);
}
