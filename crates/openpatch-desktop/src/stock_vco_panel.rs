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
    match parameter {
        "frequency" => "FREQUENCY",
        "pulse_width" => "WIDTH",
        "fm_amount" => "FM DEPTH",
        "pwm_amount" => "PWM DEPTH",
        other => other,
    }
}
fn stock_vco_parameter_value(parameter: &str, value: f32) -> String {
    match parameter {
        "frequency" => format!("{:.3} Hz", stock_vco_frequency_hz(value)),
        "pulse_width" => format!("{:.1}%", value * 100.0),
        "fm_amount" | "pwm_amount" => format!("{:+.1}%", value * 100.0),
        _ => format!("{value:.0}"),
    }
}

// Large tuning/width pair flanks a compact four-wave guide; signed modulation
// lives below. These positions also drive precision and construction anchors.
fn stock_vco_control(parameter: &str) -> (Pos2, f32) {
    match parameter {
        "frequency" | "rate" => (pos2(72.0, 120.0), 35.0),
        "pulse_width" => (pos2(288.0, 120.0), 35.0),
        "fm_amount" => (pos2(66.0, 239.0), 27.0),
        _ => (pos2(294.0, 239.0), 27.0),
    }
}
fn stock_vco_value_center(parameter: &str) -> Pos2 {
    let (center, _) = stock_vco_control(parameter);
    pos2(center.x, if center.y < 200.0 { 183.0 } else { 294.0 })
}
fn stock_vco_mode_rect(parameter: &str) -> Rect {
    Rect::from_min_size(
        pos2(
            144.0,
            if matches!(parameter, "linear_fm" | "offset") {
                204.0
            } else {
                232.0
            },
        ),
        vec2(78.0, 24.0),
    )
}
fn stock_oscillator_control_print(
    painter: &egui::Painter,
    origin: Pos2,
    kind: &str,
    name: &str,
    ratio: bool,
    zoom: f32,
) {
    let (center, _) = stock_vco_control(name);
    painter.text(
        origin + vec2(center.x, if center.y < 200.0 { 165.0 } else { 278.0 }) * zoom,
        Align2::CENTER_CENTER,
        if kind == "stock_lfo" {
            stock_lfo_parameter_label(name, ratio)
        } else {
            stock_vco_parameter_label(name)
        },
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
}
// Nominal periodic shape before minimum-phase reconstruction and the output DC
// blocker. It intentionally does not follow live phase or claim a measured trace.
fn stock_vco_nominal(wave: usize, phase: f32, width: f32) -> f32 {
    match wave {
        0 => (std::f32::consts::TAU * phase).sin(),
        1 => 1.0 - 4.0 * (phase - 0.25).abs().min(1.25 - phase),
        2 => 2.0 * ((phase + 0.5) - (phase + 0.5).floor()) - 1.0,
        _ => {
            if phase < width {
                1.0
            } else {
                -1.0
            }
        }
    }
}
fn paint_stock_oscillator_configuration(
    painter: &egui::Painter,
    node: &Node,
    rect: Rect,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
    clock: Option<f32>,
) {
    let lfo = node.kind == "stock_lfo";
    let offset = lfo && node.params.get("offset").copied().unwrap_or(1.0) >= 0.5;
    let p = |x, y| rect.min + vec2(x, y) * zoom;
    let text = |x, y, value: &str, size, color| {
        painter.text(
            p(x, y),
            Align2::CENTER_CENTER,
            value,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    };
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout { width: 360.0 },
        zoom,
    };
    let start = crate::lighting::next(painter);
    draw.screen(Rect::from_min_max(p(133.0, 77.0), p(227.0, 191.0)));
    let quiet = Color32::from_rgb(153, 163, 140);
    let grid = Color32::from_rgb(43, 57, 43);
    text(
        180.0,
        86.0,
        if lfo {
            if offset {
                "CONFIGURED 0–10 V"
            } else {
                "CONFIGURED ±5 V"
            }
        } else {
            "NOMINAL ±5 V"
        },
        8.0,
        quiet,
    );
    let width = node
        .params
        .get("pulse_width")
        .copied()
        .unwrap_or(0.5)
        .clamp(0.01, 0.99);
    for (wave, label) in ["SIN", "TRI", "SAW", "SQR"].into_iter().enumerate() {
        let y = 106.0 + wave as f32 * 23.0;
        text(144.0, y, label, 7.5, quiet);
        painter.line_segment([p(158.0, y), p(219.0, y)], Stroke::new(0.5 * zoom, grid));
        let mut phases: Vec<f32> = (0..=64).map(|i| i as f32 / 64.0).collect();
        if wave == 3 {
            phases.extend([width - 0.00001, width]);
        }
        if wave == 2 && !offset {
            phases.push(0.5 - 0.00001);
        }
        phases.sort_by(f32::total_cmp);
        let points = phases
            .into_iter()
            .map(|phase| {
                p(
                    158.0 + 61.0 * phase,
                    y - 7.0
                        * if lfo {
                            stock_lfo_guide_sample(node, wave, phase) / 5.0
                                - if offset { 1.0 } else { 0.0 }
                        } else {
                            stock_vco_nominal(wave, phase, width)
                        },
                )
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            Stroke::new(1.2 * zoom, Color32::from_rgb(229, 183, 103)),
        ));
    }
    lighting.preserve(painter, start);
    let ratio = lfo && clock.is_some_and(|c| c != 2.0);
    let midpoint = if lfo {
        if ratio {
            "×1"
        } else {
            "2 Hz"
        }
    } else {
        "C4"
    };
    for (x, label) in [(72.0, midpoint), (288.0, "50%")] {
        text(x, 68.0, label, 7.0, INK);
    }
    let ends = if lfo {
        if ratio {
            ["1/512×", "512×"]
        } else {
            ["1/256 Hz", "1024 Hz"]
        }
    } else {
        ["−76 st", "+76 st"]
    };
    for (x, label) in [
        (26.0, ends[0]),
        (118.0, ends[1]),
        (242.0, "1%"),
        (334.0, "99%"),
    ] {
        text(x, 155.0, label, 7.0, INK);
    }
    for (x, label) in [
        (
            66.0,
            if lfo {
                "FM × A → OCT"
            } else {
                "FM × A → f"
            },
        ),
        (294.0, "PWM/10 × A → PW"),
    ] {
        text(x, 202.0, label, 8.0, INK);
    }
    for center in [66.0, 294.0] {
        text(center - 42.0, 270.0, "−100%", 6.0, MUTED);
        text(center + 42.0, 270.0, "+100%", 6.0, MUTED);
    }
}

impl Rack {
    fn stock_vco_lights(&self, ui: &mut egui::Ui, node: &Node, rect: Rect) {
        let zoom = self.zoom;
        paint_stock_oscillator_configuration(ui.painter(), node, rect, zoom, &self.lighting, None);
        let help_rect = Rect::from_min_max(
            rect.min + vec2(133.0, 77.0) * zoom,
            rect.min + vec2(227.0, 191.0) * zoom,
        );
        ui.interact(help_rect, ui.id().with(("stock-vco-configured", &node.id)), Sense::hover())
            .on_hover_text("Nominal ±5 V shapes over one cycle at zero CV. The engine uses a polynomial sine, minimum-phase step/ramp correction and a 20 Hz output DC blocker; this configured guide is not live output. Width affects Square. EXP target: C4·2^(T/12+V+F·A). LIN target: C4·2^(T/12+V)+C4·F·A. T is tuning in semitones, V is V/OCT, F is FM volts and A is signed FM depth. Runtime phase and mode lamps use engine snapshots only.");
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let painter = ui.painter();
        let packet = self.module_lights.get(&node.id);
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
        for (index, y, color, _label) in [
            (3, 216.0, Color32::from_rgb(87, 183, 128), "LINEAR"),
            (4, 244.0, Color32::from_rgb(220, 167, 77), "SOFT"),
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
        painter.text(
            point(180.0, 300.0),
            Align2::CENTER_CENTER,
            "PHASE",
            crate::panel_skin::print_font(ui.ctx(), 9.0 * zoom),
            MUTED,
        );
        painter.text(
            point(180.0, 267.0),
            Align2::CENTER_CENTER,
            if lights.is_some() {
                "RUNTIME STATE"
            } else {
                "TELEMETRY —"
            },
            crate::panel_skin::print_font(ui.ctx(), 9.0 * zoom),
            MUTED,
        );
    }
}
