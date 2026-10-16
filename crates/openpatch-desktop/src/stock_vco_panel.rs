// Frequency is saved as semitones around the source contract's C4 reference.
const STOCK_VCO_C4_HZ: f32 = 261.6256;
fn stock_vco_frequency_hz(raw: f32) -> f32 {
    STOCK_VCO_C4_HZ * (raw / 12.0).exp2()
}
fn stock_vco_frequency_raw(hz: f32) -> f32 {
    // The editor validates inclusive Hz bounds first. Clamp inverse roundoff
    // at the two endpoints back onto the saved semitone range.
    (12.0 * (hz / STOCK_VCO_C4_HZ).log2()).clamp(-76.0, 76.0)
}
fn stock_vco_parameter_label(parameter: &str) -> &str {
    match parameter { "frequency" => "FREQUENCY", "pulse_width" => "WIDTH", "fm_amount" => "FM DEPTH", "pwm_amount" => "PWM DEPTH", other => other }
}
fn stock_vco_parameter_value(parameter: &str, value: f32) -> String {
    match parameter {
        "frequency" => format!("{:.3} Hz", stock_vco_frequency_hz(value)),
        "pulse_width" => format!("{:.1}%", value * 100.0),
        "fm_amount" | "pwm_amount" => format!("{:+.1}%", value * 100.0),
        _ => format!("{value:.0}"),
    }
}
impl Rack {
    fn stock_vco_lights(&self, ui: &mut egui::Ui, node: &Node, rect: Rect) {
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let painter = ui.painter();
        let packet = self.module_lights.get(&node.id);
        let lights = packet.filter(|state| state.count == 5 && (0..5).all(|i| state.brightness[i][0].is_finite()));
        painter.text(point(180.0, 79.0), Align2::CENTER_CENTER, "PHASE",
            crate::panel_skin::print_font(ui.ctx(), 10.0 * zoom), MUTED);
        let center = point(180.0, 108.0);
        painter.circle_filled(center, 11.0 * zoom, Color32::from_rgb(59, 63, 61));
        let components: [f32; 3] = std::array::from_fn(|i| lights.map_or(0.0, |state| state.brightness[i][0].clamp(0.0, 1.0)));
        let strength = components.iter().copied().fold(0.0_f32, f32::max);
        self.lighting.circle(painter, center, 8.5 * zoom,
            if lights.is_some() { Color32::from_rgb((components[0] * 255.0) as u8, (components[1] * 255.0) as u8, (components[2] * 255.0) as u8) }
            else { Color32::from_rgb(99, 105, 99) }, strength, 0.0);
        for (index, x, color) in [(3, 157.0, Color32::from_rgb(87, 183, 128)), (4, 337.0, Color32::from_rgb(220, 167, 77))] {
            let center = point(x, 293.0);
            let value = lights.map_or(0.0, |state| state.brightness[index][0].clamp(0.0, 1.0));
            painter.circle_filled(center, 7.0 * zoom, Color32::from_rgb(59, 63, 61));
            self.lighting.circle(painter, center, 5.0 * zoom,
                if lights.is_some() { color.gamma_multiply(0.12 + value * 0.88) } else { Color32::from_rgb(99, 105, 99) }, value, 0.12);
        }
        painter.text(point(180.0, 183.0), Align2::CENTER_CENTER,
            if lights.is_some() { "PHASE / MODES" } else { "TELEMETRY —" },
            crate::panel_skin::print_font(ui.ctx(), 10.0 * zoom), MUTED);
    }
}
