// Stock Noise's printed signal paths are source-backed process roles. They do
// not represent live samples, output activity, or a transfer-function plot.
const STOCK_NOISE_OUTPUTS: [&str; 7] = [
    "white", "pink", "red", "violet", "blue", "gray", "black",
];
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
    Color32::from_rgb(82, 81, 72),
    Color32::from_rgb(174, 78, 116),
    Color32::from_rgb(176, 59, 51),
    Color32::from_rgb(112, 78, 151),
    Color32::from_rgb(53, 103, 159),
    Color32::from_rgb(105, 111, 112),
    Color32::from_rgb(32, 34, 35),
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

fn paint_stock_noise_face(painter: &egui::Painter, rect: Rect, zoom: f32) {
    let point = |x: f32, y: f32| rect.min + vec2(x, y) * zoom;
    // The shared rack renderer supplies the face, title and actual jacks. These
    // compact process marks end at the corresponding independently generated
    // output; no drawn link joins one output to another.
    for index in 0..STOCK_NOISE_OUTPUTS.len() {
        let y = stock_noise_row_y(index);
        painter.text(
            point(13.0, y),
            Align2::LEFT_CENTER,
            STOCK_NOISE_PROCESSES[index],
            crate::panel_skin::print_font(painter.ctx(), 10.5 * zoom),
            crate::panel_skin::INK,
        );
        painter.text(
            point(224.0, y),
            Align2::RIGHT_CENTER,
            stock_noise_port_label(STOCK_NOISE_OUTPUTS[index]),
            crate::panel_skin::print_font(painter.ctx(), 10.5 * zoom),
            STOCK_NOISE_TERMINAL_COLORS[index],
        );
        painter.line_segment(
            [point(229.0, y), point(249.0, y)],
            Stroke::new(1.0 * zoom, STOCK_NOISE_TERMINAL_COLORS[index]),
        );
    }
}
