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
// One saved zero-CV cycle. Same unfiltered waveform law and nearest-even
// boundaries as the production LFO; independent of live phase/clock/lamp state.
fn stock_lfo_guide_sample(node: &Node, wave: usize, phase: f32) -> f32 {
    let offset = node.params.get("offset").copied().unwrap_or(1.0) >= 0.5;
    let invert = node.params.get("invert").copied().unwrap_or(0.0) >= 0.5;
    let width = node
        .params
        .get("pulse_width")
        .copied()
        .unwrap_or(0.5)
        .clamp(0.01, 0.99);
    let mut value = match wave {
        0 => (std::f32::consts::TAU * (phase - if offset { 0.25 } else { 0.0 })).sin(),
        1 => {
            let p = phase + if offset { 0.0 } else { 0.25 };
            4.0 * (p - p.round_ties_even()).abs() - 1.0
        }
        2 => {
            let p = phase - if offset { 0.5 } else { 0.0 };
            2.0 * (p - p.round_ties_even())
        }
        _ => {
            if phase < width {
                1.0
            } else {
                -1.0
            }
        }
    };
    if invert {
        value = -value;
    }
    if offset {
        value += 1.0;
    }
    5.0 * value
}
impl Rack {
    fn stock_lfo_lights(&self, ui: &mut egui::Ui, node: &Node, rect: Rect) {
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let painter = ui.painter();
        let packet = self.module_lights.get(&node.id);
        paint_stock_oscillator_configuration(
            painter,
            node,
            rect,
            zoom,
            &self.lighting,
            stock_lfo_clock(packet),
        );
        ui.interact(Rect::from_min_max(point(133.0,77.0),point(227.0,191.0)),ui.id().with(("stock-lfo-configured",&node.id)),Sense::hover())
            .on_hover_text("Configured unfiltered LFO shapes over one cycle at zero FM/PWM. Offset selects ±5 V or 0–10 V; Invert flips the shape within that range. This guide does not follow live phase or report measured output. Rate target = observed clock Hz / 2 × 2^(RATE + FM × DEPTH); without a clock the engine uses 2 Hz. PWM / 10 × depth adds to width; RESET returns phase to zero. Only engine snapshots drive the phase/mode lamps and observed clock readout.");
        let lights = packet
            .filter(|state| state.count == 5 && (0..5).all(|i| state.brightness[i][0].is_finite()));
        let center = point(180.0, 285.0);
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
        for (index, y, color) in [
            (4, 216.0, Color32::from_rgb(87, 183, 128)),
            (3, 244.0, Color32::from_rgb(220, 167, 77)),
        ] {
            let center = point(132.0, y);
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
            point(180.0, 267.0),
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
            point(180.0, 300.0),
            Align2::CENTER_CENTER,
            "PHASE",
            crate::panel_skin::print_font(ui.ctx(), 9.0 * zoom),
            MUTED,
        );
    }
}
