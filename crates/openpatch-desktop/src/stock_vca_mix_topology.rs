/// Paint the native Stock VCA Mix signal path in a compact 270×40 logical band.
/// The four labeled outputs remain distinct until their real master sum; the
/// master level and optional master-CV factor apply only after that sum.
fn stock_vca_mix_topology(painter: &egui::Painter, rect: Rect, zoom: f32) {
    let point = |x: f32, y: f32| rect.min + vec2(x, y) * zoom;
    let ink = Color32::from_rgb(34, 36, 35);
    let rule = Stroke::new(0.9 * zoom, ink);
    let small_font = |size| crate::panel_skin::print_font(painter.ctx(), size * zoom);

    // Each named strip output enters the summing bus once. No channel feeds
    // another channel, and the outputs are not normalized or clamped here.
    let branch_y = [18.0, 22.0, 26.0, 30.0];
    painter.text(
        point(28.0, 7.0),
        Align2::CENTER_CENTER,
        "OUT 1–4",
        small_font(8.2),
        ink,
    );
    for y in branch_y {
        painter.circle_filled(point(24.0, y), 1.1 * zoom, ink);
        painter.line_segment([point(24.0, y), point(61.0, y)], rule);
    }
    painter.line_segment([point(61.0, branch_y[0]), point(61.0, branch_y[3])], rule);

    // Four output branches join at this explicit native sum.
    let center_y = 23.0;
    let arrow = |start: Pos2, end: Pos2| {
        painter.line_segment([start, end], rule);
        let head = vec2(3.2, 2.1) * zoom;
        painter.line_segment([end, end - vec2(head.x, -head.y)], rule);
        painter.line_segment([end, end - vec2(head.x, head.y)], rule);
    };
    arrow(point(61.0, center_y), point(71.0, center_y));
    let sum = Rect::from_center_size(point(81.0, center_y), vec2(20.0, 21.0) * zoom);
    painter.rect_filled(sum, 2.0 * zoom, Color32::from_rgb(245, 241, 228));
    painter.rect_stroke(sum, 2.0 * zoom, rule, egui::StrokeKind::Inside);
    painter.text(
        sum.center(),
        Align2::CENTER_CENTER,
        "Σ",
        small_font(12.0),
        ink,
    );

    let master = Rect::from_center_size(point(129.0, center_y), vec2(58.0, 21.0) * zoom);
    arrow(point(91.0, center_y), point(100.0, center_y));
    painter.rect_filled(master, 2.0 * zoom, Color32::from_rgb(245, 241, 228));
    painter.rect_stroke(master, 2.0 * zoom, rule, egui::StrokeKind::Inside);
    painter.text(
        master.center(),
        Align2::CENTER_CENTER,
        "× MIX LEVEL",
        small_font(8.5),
        ink,
    );

    let cv_factor = Rect::from_center_size(point(183.0, center_y), vec2(28.0, 21.0) * zoom);
    arrow(point(158.0, center_y), point(169.0, center_y));
    painter.rect_filled(cv_factor, 2.0 * zoom, Color32::from_rgb(245, 241, 228));
    painter.rect_stroke(cv_factor, 2.0 * zoom, rule, egui::StrokeKind::Inside);
    painter.text(
        cv_factor.center(),
        Align2::CENTER_CENTER,
        "× h",
        small_font(9.0),
        ink,
    );
    arrow(point(197.0, center_y), point(213.0, center_y));
    painter.text(
        point(235.0, center_y),
        Align2::CENTER_CENTER,
        "MIX",
        small_font(9.0),
        ink,
    );
}
