// Stock Noise's printed signal paths are source-backed process roles. They do
// not represent live samples, output activity, or a transfer-function plot.
const STOCK_NOISE_OUTPUTS: [&str; 7] = ["white", "pink", "red", "violet", "blue", "gray", "black"];
const STOCK_NOISE_PROCESSES: [&str; 7] = [
    "NORMAL DRAW",
    "VOSS 8 ROWS",
    "WHITE > FIXED LP",
    "WHITE > FIRST DIFFERENCE",
    "PINK > FIRST DIFFERENCE",
    "WHITE > FFT1024 EQUAL-LOUD",
    "UNIFORM U > 10U-5",
];
const STOCK_NOISE_TERMINAL_COLORS: [Color32; 7] = [
    Color32::from_rgb(218, 219, 199),
    Color32::from_rgb(219, 143, 170),
    Color32::from_rgb(225, 131, 110),
    Color32::from_rgb(184, 157, 214),
    Color32::from_rgb(136, 180, 218),
    Color32::from_rgb(173, 188, 183),
    Color32::from_rgb(164, 169, 155),
];

fn stock_noise_port_label(port: &str) -> &str {
    match port {
        "white" => "WHITE",
        "pink" => "PINK",
        "red" => "RED",
        "violet" => "VIOLET",
        "blue" => "BLUE",
        "gray" => "GRAY",
        "black" => "BLACK",
        other => other,
    }
}

fn stock_noise_row_y(index: usize) -> f32 {
    92.0 + index as f32 * 46.0
}

fn stock_noise_screen_rect(rect: Rect, zoom: f32, index: usize) -> Rect {
    Rect::from_min_max(
        rect.min + vec2(24.0, stock_noise_row_y(index) - 17.5) * zoom,
        rect.min + vec2(236.0, stock_noise_row_y(index) + 17.5) * zoom,
    )
}

fn paint_stock_noise_face(
    painter: &egui::Painter,
    rect: Rect,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    let point = |x: f32, y: f32| rect.min + vec2(x, y) * zoom;
    painter.text(
        point(24.0, 58.0),
        Align2::LEFT_CENTER,
        "PROCESS ROLES · SOURCE → OUTPUT",
        crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
        crate::panel_skin::INK,
    );
    // These are fixed processing descriptions, not sampled noise, spectral
    // promises or activity indicators. Each row routes only to its own jack.
    for index in 0..STOCK_NOISE_OUTPUTS.len() {
        let y = stock_noise_row_y(index);
        let screen = stock_noise_screen_rect(rect, zoom, index);
        lighting.screen_glow(painter, screen, zoom);
        painter.rect_filled(
            screen.expand(1.5 * zoom),
            2.5 * zoom,
            Color32::from_rgb(126, 121, 94),
        );
        let start = crate::lighting::next(painter);
        painter.rect_filled(screen, 2.0 * zoom, Color32::from_rgb(15, 24, 18));
        painter.rect_stroke(
            screen,
            2.0 * zoom,
            Stroke::new(zoom, Color32::from_rgb(5, 12, 8)),
            egui::StrokeKind::Inside,
        );
        let accent = STOCK_NOISE_TERMINAL_COLORS[index];
        painter.rect_filled(
            Rect::from_min_max(point(30.0, y - 11.0), point(32.0, y + 11.0)),
            0.0,
            accent,
        );
        painter.text(
            point(39.0, y - 9.0),
            Align2::LEFT_CENTER,
            stock_noise_port_label(STOCK_NOISE_OUTPUTS[index]),
            crate::panel_skin::print_font(painter.ctx(), 12.0 * zoom),
            accent,
        );
        painter.text(
            point(228.0, y - 9.0),
            Align2::RIGHT_CENTER,
            "OUT",
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            Color32::from_rgb(133, 143, 123),
        );
        painter.text(
            point(39.0, y + 9.0),
            Align2::LEFT_CENTER,
            STOCK_NOISE_PROCESSES[index],
            crate::panel_skin::print_font(painter.ctx(), 9.5 * zoom),
            Color32::from_rgb(195, 200, 180),
        );
        lighting.preserve(painter, start);
        // Passive printed routing ends before the enlarged metal rim.
        painter.line_segment(
            [point(240.0, y), point(248.0, y)],
            Stroke::new(zoom, crate::panel_skin::INK),
        );
        painter.line_segment(
            [point(245.0, y - 2.0), point(248.0, y)],
            Stroke::new(zoom, crate::panel_skin::INK),
        );
        painter.line_segment(
            [point(245.0, y + 2.0), point(248.0, y)],
            Stroke::new(zoom, crate::panel_skin::INK),
        );
    }
}
