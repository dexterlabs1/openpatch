// Stock VCA-2 shares one face, native level gestures and sockets in the rack
// and browser: two identical amplifier channels stacked top and bottom. The
// source has no lights or meters, so the face shows none.
const STOCK_VCA2_SECTION: f32 = 172.0;
fn stock_vca2_section(name: &str) -> f32 {
    if name.ends_with('2') {
        STOCK_VCA2_SECTION
    } else {
        0.0
    }
}
fn stock_vca2_control(name: &str) -> (Pos2, f32) {
    (pos2(75.0, 84.0 + stock_vca2_section(name)), 18.0)
}
fn stock_vca2_value_center(name: &str) -> Pos2 {
    pos2(75.0, 115.0 + stock_vca2_section(name))
}
fn stock_vca2_value_width(_name: &str) -> f32 {
    64.0
}
fn stock_vca2_value(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}
fn stock_vca2_port(input: bool, index: usize) -> Pos2 {
    // Inputs are EXP, LIN, IN per channel; outputs OUT 1, OUT 2.
    let (section, x, row) = if input {
        let (x, row) = match index % 3 {
            0 => (40.0, 0.0),
            1 => (110.0, 0.0),
            _ => (40.0, 1.0),
        };
        (index / 3, x, row)
    } else {
        (index, 110.0, 1.0)
    };
    pos2(x, 152.0 + row * 44.0 + section as f32 * STOCK_VCA2_SECTION)
}
fn stock_vca2_port_label(port: &str) -> &str {
    match port {
        "exp1" | "exp2" => "EXP",
        "lin1" | "lin2" => "LIN",
        "in1" | "in2" => "IN",
        "out1" | "out2" => "OUT",
        other => port_label(other),
    }
}
fn paint_stock_vca2_control_print(painter: &egui::Painter, rect: Rect, name: &str, zoom: f32) {
    let (center, radius) = stock_vca2_control(name);
    painter.text(
        rect.min + vec2(center.x, center.y - radius - 14.0) * zoom,
        Align2::CENTER_CENTER,
        if name.ends_with('2') {
            "LEVEL 2"
        } else {
            "LEVEL 1"
        },
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
}
fn paint_stock_vca2_configuration(ui: &mut egui::Ui, rect: Rect, node: &Node, zoom: f32) {
    // A channel number beside each level knob.
    for (section, label) in [(0.0, "1"), (STOCK_VCA2_SECTION, "2")] {
        let center = rect.min + vec2(20.0, 84.0 + section) * zoom;
        ui.painter().text(
            center,
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(ui.ctx(), 16.0 * zoom),
            INK,
        );
        // The channel number explains the module on hover.
        let area = Rect::from_center_size(center, Vec2::splat(18.0 * zoom));
        ui.interact(area, ui.id().with(("stock-vca2-channel", label, &node.id)), Sense::hover())
            .on_hover_text("Channel {label} of two amplifiers. The channel follows its IN jack, with as many voices as the cable carries. LEVEL sets the gain. LIN scales it in proportion to 0–10 V; EXP follows a steep curve that stays quiet until high voltages. A one-voice CV controls every voice; a polyphonic CV controls each voice separately, and voices it does not carry are silent.".replace("{label}", label));
    }
}
