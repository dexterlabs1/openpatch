// Phase-domain configuration guides for the four-output functional oscillators.
// The VCO view shows its actual source curves before correction tails and the
// square's high-pass, not an invented audio trace or a circuit response.
fn atlas_multi_control(name: &str) -> (Pos2, f32) {
    match name {
        "frequency" => (pos2(75.0, 125.0), 28.0),
        "pulse_width" => (pos2(225.0, 125.0), 28.0),
        "fm_amount" => (pos2(75.0, 225.0), 23.0),
        _ => (pos2(225.0, 225.0), 23.0),
    }
}

fn atlas_multi_value_center(name: &str) -> Pos2 {
    let (center, _) = atlas_multi_control(name);
    pos2(
        center.x,
        if matches!(name, "frequency" | "pulse_width") {
            174.0
        } else {
            269.0
        },
    )
}

fn paint_atlas_multi_print(
    painter: &egui::Painter,
    rect: Rect,
    kind: &str,
    name: &str,
    zoom: f32,
    clock_ratio: bool,
) {
    let (center, _) = atlas_multi_control(name);
    let (label, range) = match name {
        "frequency" if kind == "multi_lfo" && clock_ratio => ("CLOCK RATIO", "1/512 – 512×"),
        "frequency" if kind == "multi_lfo" => ("FREQUENCY", "1/256 – 1024 Hz"),
        "frequency" => ("FREQUENCY", "11.6 Hz – 5.92 kHz"),
        "pulse_width" => ("PULSE WIDTH", "1 – 99%"),
        "fm_amount" => ("FM ±100%", ""),
        _ => ("PWM ±100%", ""),
    };
    let top = matches!(name, "frequency" | "pulse_width");
    painter.text(
        rect.min + vec2(center.x, if top { 66.0 } else { 188.0 }) * zoom,
        Align2::CENTER_CENTER,
        label,
        crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
        INK,
    );
    if top {
        painter.text(
            rect.min + vec2(center.x, 80.0) * zoom,
            Align2::CENTER_CENTER,
            range,
            crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
            MUTED,
        );
    }
}

// Source laws deliberately mirror multi_vco::waves and MultiLfo::process.
// VCO values are dimensionless nominal core values. LFO values are volts.
fn atlas_multi_nominal_sample(node: &Node, output: usize, phase: f64) -> f64 {
    let p = phase.rem_euclid(1.0);
    let width = atlas_voice_parameter(node, "pulse_width", 0.5, 0.01, 0.99);
    let square = if p < width { 1.0 } else { -1.0 };
    if node.kind == "multi_vco" {
        let curve = |x: f64| (5.0 * x * x - 13.0 * x + 3.0) / (2.0 * x + 3.0);
        match output {
            0 if p < 0.5 => 1.0 - 16.0 * (p - 0.25).powi(2),
            0 => -1.0 + 16.0 * (p - 0.75).powi(2),
            1 => {
                let x = (p + 0.25).fract();
                curve((2.0 * x).fract()) * if x < 0.5 { -1.0 } else { 1.0 }
            }
            2 => -curve((p + 0.5).fract()),
            _ => square,
        }
    } else {
        let value = match output {
            0 => (std::f64::consts::TAU * p).sin(),
            1 => 1.0 - 4.0 * (p - 0.5).abs(),
            2 => 2.0 * p - 1.0,
            _ => square,
        };
        let value = if atlas_voice_parameter(node, "invert", 0.0, 0.0, 1.0) >= 0.5 {
            -value
        } else {
            value
        };
        5.0 * if atlas_voice_parameter(node, "unipolar", 0.0, 0.0, 1.0) >= 0.5 {
            value + 1.0
        } else {
            value
        }
    }
}

fn paint_atlas_multi_configuration(
    ui: &mut egui::Ui,
    rect: Rect,
    node: &Node,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    let painter = ui.painter();
    let p = |x, y| rect.min + vec2(x, y) * zoom;
    let screen = Rect::from_min_max(p(121.0, 65.0), p(179.0, 262.0));
    let start = crate::lighting::next(painter);
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout {
            width: rect.width() / zoom,
        },
        zoom,
    };
    draw.screen(screen);
    let quiet = Color32::from_rgb(153, 165, 142);
    let amber = Color32::from_rgb(232, 184, 100);
    let grid = Color32::from_rgb(51, 67, 50);
    let text = |y, label: &str, size, color| {
        painter.text(
            p(150.0, y),
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    };
    let vco = node.kind == "multi_vco";
    let unipolar = !vco && atlas_voice_parameter(node, "unipolar", 0.0, 0.0, 1.0) == 1.0;
    text(
        73.0,
        if vco { "NOMINAL CORE" } else { "CONFIGURED" },
        6.8,
        quiet,
    );
    text(82.0, "ZERO CV", 6.0, quiet);
    text(
        91.0,
        if vco {
            "−1 … +1"
        } else if unipolar {
            "0 … +10 V"
        } else {
            "−5 … +5 V"
        },
        6.3,
        quiet,
    );
    for (output, label) in ["SINE", "TRIANGLE", "SAW", "PULSE"].into_iter().enumerate() {
        let y = 101.0 + output as f32 * 39.0;
        text(y, label, 6.5, quiet);
        let plot = Rect::from_min_max(p(127.0, y + 7.0), p(173.0, y + 27.0));
        let (low, high) = if vco {
            (-1.0, 1.0)
        } else if unipolar {
            (0.0, 10.0)
        } else {
            (-5.0, 5.0)
        };
        let point = |phase: f64| {
            let value = atlas_multi_nominal_sample(node, output, phase);
            pos2(
                plot.left() + phase as f32 * plot.width(),
                plot.bottom() - ((value - low) / (high - low)) as f32 * plot.height(),
            )
        };
        for x in [plot.left(), plot.center().x, plot.right()] {
            painter.line_segment(
                [pos2(x, plot.top()), pos2(x, plot.bottom())],
                Stroke::new(0.5 * zoom, grid),
            );
        }
        let zero_y = plot.bottom() - ((-low) / (high - low)) as f32 * plot.height();
        painter.line_segment(
            [pos2(plot.left(), zero_y), pos2(plot.right(), zero_y)],
            Stroke::new(0.6 * zoom, grid),
        );
        // Split every discontinuity so no diagonal implies a source-law ramp.
        let seam = if output == 3 {
            Some(atlas_voice_parameter(node, "pulse_width", 0.5, 0.01, 0.99))
        } else if vco && output == 2 {
            Some(0.5)
        } else {
            None
        };
        let mut boundaries = vec![0.0];
        if let Some(seam) = seam {
            boundaries.push(seam);
        }
        boundaries.push(1.0);
        for span in boundaries.windows(2) {
            let points = (0..=48)
                .map(|i| {
                    let phase = span[0] + (span[1] - span[0]) * f64::from(i) / 48.0;
                    point(if i == 48 { phase - 1e-9 } else { phase })
                })
                .collect();
            painter.add(egui::Shape::line(points, Stroke::new(1.1 * zoom, amber)));
        }
        if let Some(seam) = seam {
            painter.line_segment(
                [point(seam - 1e-9), point(seam)],
                Stroke::new(1.1 * zoom, amber),
            );
        }
    }
    text(254.0, "0 → 1 PHASE", 6.3, quiet);
    lighting.preserve(painter, start);
    ui.interact(
        screen,
        ui.id().with(("atlas-multi-guide", &node.id)),
        Sense::hover(),
    )
    .on_hover_text(if vco {
        "Nominal forward phase cycle at zero pitch, FM, PWM and sync input. Curves use the actual parabolic sine and rational triangle/saw source laws; pulse uses saved width. The vertical range is dimensionless −1 to +1 before the engine's ×5 V scaling, discontinuity correction and square's 20 Hz high-pass / ×0.95 stage. These are configured core shapes, not live audio, measured output bounds, a circuit model or a bandlimited claim. Exponential FM is 1 V/oct at full depth; linear FM adds frequency × depth × FM voltage. PWM adds depth × PWM/10 V, clamped to 1–99%. Hard sync resets phase; soft sync reverses direction."
    } else {
        "Configured unfiltered phase cycle at zero FM, PWM and reset input. Sine = sin(2πp), triangle = 1−4|p−0.5|, saw = 2p−1; pulse is high while p < saved width. Invert applies before mapping to ±5 V or 0–10 V. This phase axis is not a live waveform or clock acquisition indicator. FM is exponential, 1 V/oct at full depth; PWM adds depth × PWM/10 V, clamped to 1–99%. Reset emits phase zero immediately. Clock acquires after two rising edges, sets frequency × frequency-knob/2, and never resets phase. Original functional DSP; no audio antialiasing or circuit claim."
    });
}

impl Rack {
    fn register_multi_mode_anchor(
        &mut self,
        ui: &egui::Ui,
        node: &Node,
        name: &str,
        rect: Rect,
        response: &egui::Response,
    ) {
        if let Some(cue) = self
            .construction_animation
            .parameter(&node.id, &node.kind, name)
        {
            let label = |value: f32| match (name, value >= 0.5) {
                ("linear_fm", true) => "Linear FM",
                ("linear_fm", false) => "Exp FM",
                ("soft_sync", true) => "Soft sync",
                ("soft_sync", false) => "Hard sync",
                ("unipolar", true) => "0–10 V",
                ("unipolar", false) => "±5 V",
                ("invert", true) => "Invert",
                _ => "Normal",
            };
            self.register_construction_anchor(
                ui,
                node,
                name,
                crate::construction_motion::Anchor {
                    rect,
                    old_text: label(cue.old).into(),
                    new_text: label(cue.new).into(),
                    marker: None,
                },
                &[response.id],
            );
        }
    }
}
