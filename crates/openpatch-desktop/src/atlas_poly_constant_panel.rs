// The bank shows saved voltages, including channels outside the configured bus.
// Output width belongs to the channels control; it does not disable these cells.
fn atlas_poly_constant_control(name: &str) -> (Pos2, f32) {
    let channel = name
        .strip_prefix('v')
        .and_then(|number| number.parse::<usize>().ok())
        .filter(|number| (1..=16).contains(number))
        .unwrap_or(1)
        - 1;
    (
        pos2(
            [50.0, 137.0, 223.0, 310.0][channel % 4],
            117.0 + (channel / 4) as f32 * 67.0,
        ),
        18.0,
    )
}

fn atlas_poly_constant_value_center(name: &str) -> Pos2 {
    atlas_poly_constant_control(name).0 + vec2(0.0, 31.0)
}

fn paint_atlas_poly_constant_print(painter: &egui::Painter, rect: Rect, name: &str, zoom: f32) {
    let Some(channel) = name
        .strip_prefix('v')
        .and_then(|number| number.parse::<usize>().ok())
        .filter(|number| (1..=16).contains(number))
    else {
        return;
    };
    let center = atlas_poly_constant_control(name).0;
    painter.text(
        rect.min + (center.to_vec2() - vec2(31.0, 0.0)) * zoom,
        Align2::CENTER_CENTER,
        format!("{channel:02}"),
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
}
