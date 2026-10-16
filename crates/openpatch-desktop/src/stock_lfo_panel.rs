fn stock_lfo_clock(state: Option<&ModuleLightState>) -> Option<f32> {
    state
        .and_then(|state| state.clock_frequency)
        .filter(|clock| clock.is_finite() && *clock > 0.0)
}
fn value_or_default(node: &Node, parameter: &str, spec: &Value) -> f32 {
    node.params
        .get(parameter)
        .copied()
        .unwrap_or(spec["default"].as_f64().unwrap_or(0.0) as f32)
}
fn stock_lfo_parameter_label(parameter: &str, ratio: bool) -> &str {
    match parameter {
        "rate" if ratio => "CLOCK RATIO",
        "rate" => "RATE",
        "pulse_width" => "WIDTH",
        "fm_amount" => "FM DEPTH",
        "pwm_amount" => "PWM DEPTH",
        other => other,
    }
}
fn stock_lfo_parameter_value(parameter: &str, value: f32, clock: Option<f32>) -> String {
    match parameter {
        "rate" => match clock {
            Some(2.0) => wavetable_rate_value(value, false),
            Some(_) => {
                let ratio = value.exp2() * 0.5;
                if ratio < 1.0 {
                    format!("{ratio:.8}x")
                } else {
                    format!("{ratio:.4}x")
                }
            }
            None => format!(
                "{} / {:.4}x",
                wavetable_rate_value(value, false),
                value.exp2() * 0.5
            ),
        },
        "pulse_width" => format!("{:.1}%", value * 100.0),
        "fm_amount" | "pwm_amount" => format!("{:+.1}%", value * 100.0),
        _ => format!("{value:.0}"),
    }
}
impl Rack {
    fn stock_lfo_lights(&self, ui: &mut egui::Ui, node: &Node, rect: Rect) {
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let painter = ui.painter();
        let packet = self.module_lights.get(&node.id);
        let lights = packet
            .filter(|state| state.count == 5 && (0..5).all(|i| state.brightness[i][0].is_finite()));
        let center = point(180.0, 279.0);
        painter.circle_filled(center, 11.0 * zoom, Color32::from_rgb(59, 63, 61));
        let components: [f32; 3] = std::array::from_fn(|i| {
            lights.map_or(0.0, |state| state.brightness[i][0].clamp(0.0, 1.0))
        });
        let strength = components.iter().copied().fold(0.0_f32, f32::max);
        // Lighting expands the requested radius by 2.1; keep the lens inside its 11-point bezel.
        self.lighting.circle(
            painter,
            center,
            (8.5 / 2.1) * zoom,
            if lights.is_some() {
                Color32::from_rgb(
                    (components[0] * 255.0) as u8,
                    (components[1] * 255.0) as u8,
                    (components[2] * 255.0) as u8,
                )
            } else {
                Color32::from_rgb(99, 105, 99)
            },
            strength,
            0.0,
        );
        for (index, x, color, caption) in [
            (4, 38.0, Color32::from_rgb(87, 183, 128), "OFFSET"),
            (3, 322.0, Color32::from_rgb(220, 167, 77), "INVERT"),
        ] {
            let center = point(x, 279.0);
            let value = lights.map_or(0.0, |state| state.brightness[index][0].clamp(0.0, 1.0));
            painter.circle_filled(center, 7.0 * zoom, Color32::from_rgb(59, 63, 61));
            self.lighting.circle(
                painter,
                center,
                (5.0 / 2.1) * zoom,
                if lights.is_some() {
                    color.gamma_multiply(0.12 + value * 0.88)
                } else {
                    Color32::from_rgb(99, 105, 99)
                },
                value,
                0.12,
            );
            painter.text(
                point(x, 296.0),
                Align2::CENTER_CENTER,
                caption,
                crate::panel_skin::print_font(ui.ctx(), 9.0 * zoom),
                MUTED,
            );
        }
        let clock = stock_lfo_clock(packet);
        let label = clock.map_or_else(
            || "CLOCK —".to_owned(),
            |value| {
                if !(0.001..100_000.0).contains(&value) {
                    format!("CLOCK {value:.2e} Hz")
                } else {
                    format!("CLOCK {value:.3} Hz")
                }
            },
        );
        let text_start = crate::lighting::next(painter);
        painter.text(
            point(180.0, 261.0),
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(ui.ctx(), 9.0 * zoom),
            if clock.is_some() {
                Color32::from_rgb(155, 220, 197)
            } else {
                MUTED
            },
        );
        if clock.is_some() {
            self.lighting.preserve(painter, text_start);
        }
        painter.text(
            point(180.0, 296.0),
            Align2::CENTER_CENTER,
            "PHASE",
            crate::panel_skin::print_font(ui.ctx(), 9.0 * zoom),
            MUTED,
        );
    }
}
