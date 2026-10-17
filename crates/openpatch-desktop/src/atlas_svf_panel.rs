// Configured TPT core response. The upstream nonlinear drive/antialias filters
// and their latency are deliberately excluded; this is not a live spectrum.
fn atlas_svf_control(name: &str) -> (Pos2, f32) {
    match name {
        "cutoff" => (pos2(67.5, 139.0), 32.0),
        "resonance" => (pos2(45.0, 255.0), 21.0),
        "drive" => (pos2(135.0, 255.0), 21.0),
        _ => (pos2(225.0, 255.0), 21.0),
    }
}
fn atlas_svf_value_center(name: &str) -> Pos2 {
    let (center, _) = atlas_svf_control(name);
    pos2(center.x, if name == "cutoff" { 195.0 } else { 292.0 })
}
fn atlas_svf_value(name: &str, value: f32, spec: &Value) -> String {
    match name {
        "resonance" | "drive" => format!("{:.0}%", value * 100.0),
        "cutoff_cv_amount" => format!("{value:+.2} oct/V"),
        _ => parameter_value(name, value, spec),
    }
}
fn atlas_svf_core_magnitudes(node: &Node, rate: u32, frequency: f64) -> [f64; 3] {
    let rate = f64::from(rate.max(1));
    let cutoff = atlas_voice_parameter(node, "cutoff", 1000.0, 5.0, 20000.0)
        .clamp(1.0_f64.min(rate * 0.45), rate * 0.45);
    let resonance = atlas_voice_parameter(node, "resonance", 0.2, 0.0, 1.0);
    let damping = 2.0 - 1.95 * resonance;
    let relative = (std::f64::consts::PI * frequency.clamp(0.0, rate * 0.45) / rate).tan()
        / (std::f64::consts::PI * cutoff / rate).tan();
    let denominator = (1.0 - relative * relative).hypot(damping * relative);
    [
        1.0 / denominator,
        relative / denominator,
        relative * relative / denominator,
    ]
}
fn paint_atlas_svf_print(painter: &egui::Painter, rect: Rect, name: &str, zoom: f32) {
    let (c, _) = atlas_svf_control(name);
    let (label, range) = match name {
        "cutoff" => ("CUTOFF", "5 Hz – 20 kHz"),
        "resonance" => ("RESONANCE", "0 – 100%"),
        "drive" => ("DRIVE", "0 – 100%"),
        _ => ("CV DEPTH", "−4 – +4 oct/V"),
    };
    for (y, text, size, color) in [
        (
            if name == "cutoff" { 75.0 } else { 209.0 },
            label,
            10.0,
            INK,
        ),
        (
            if name == "cutoff" { 89.0 } else { 221.0 },
            range,
            7.0,
            MUTED,
        ),
    ] {
        painter.text(
            rect.min + vec2(c.x, y) * zoom,
            Align2::CENTER_CENTER,
            text,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    }
}
fn paint_atlas_svf_configuration(
    ui: &mut egui::Ui,
    rect: Rect,
    node: &Node,
    zoom: f32,
    rate: u32,
    lighting: &crate::lighting::RoomLighting,
) {
    let painter = ui.painter();
    let p = |x, y| rect.min + vec2(x, y) * zoom;
    let start = crate::lighting::next(painter);
    let screen = Rect::from_min_max(p(131.0, 65.0), p(256.0, 198.0));
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout {
            width: rect.width() / zoom,
        },
        zoom,
    };
    draw.screen(screen, lighting);
    let muted = Color32::from_rgb(143, 155, 133);
    let colors = [
        Color32::from_rgb(232, 184, 100),
        Color32::from_rgb(111, 192, 169),
        Color32::from_rgb(149, 169, 221),
    ];
    let text = |x, y, text: &str, size, color| {
        painter.text(
            p(x, y),
            Align2::CENTER_CENTER,
            text,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    };
    text(193.5, 76.0, "TPT CORE · dB", 8.0, muted);
    text(193.5, 88.0, "CONFIG · ZERO CV", 7.0, muted);
    let plot = Rect::from_min_max(p(148.0, 101.0), p(246.0, 156.0));
    let upper = (f64::from(rate) * 0.45).clamp(21.0, 20000.0);
    let y =
        |db: f64| plot.bottom() - ((db.clamp(-48.0, 28.0) + 48.0) / 76.0) as f32 * plot.height();
    for db in [-48.0, 0.0, 28.0] {
        painter.line_segment(
            [pos2(plot.left(), y(db)), pos2(plot.right(), y(db))],
            Stroke::new(0.5 * zoom, Color32::from_rgb(56, 69, 53)),
        );
        painter.text(
            pos2(plot.left() - 3.0 * zoom, y(db)),
            Align2::RIGHT_CENTER,
            format!("{db:.0}"),
            crate::panel_skin::print_font(painter.ctx(), 5.8 * zoom),
            muted,
        );
    }
    for (band, color) in colors.into_iter().enumerate() {
        let points = (0..=80)
            .map(|i| {
                let t = f64::from(i) / 80.0;
                let frequency = 20.0 * (upper / 20.0).powf(t);
                let magnitude = atlas_svf_core_magnitudes(node, rate, frequency)[band];
                pos2(
                    plot.left() + t as f32 * plot.width(),
                    y(20.0 * magnitude.max(1e-12).log10()),
                )
            })
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(1.0 * zoom, color)));
    }
    text(153.0, 166.0, "20", 6.5, muted);
    text(
        234.0,
        166.0,
        &format!("{} Hz", atlas_short_hz(upper)),
        6.5,
        muted,
    );
    for (i, label) in ["LOW", "BAND", "HIGH"].into_iter().enumerate() {
        text(153.0 + 40.0 * i as f32, 179.0, label, 7.0, colors[i]);
    }
    let resonance = atlas_voice_parameter(node, "resonance", 0.2, 0.0, 1.0);
    text(
        193.5,
        190.0,
        &format!("Q {:.2} · LOG Hz", 1.0 / (2.0 - 1.95 * resonance)),
        7.0,
        muted,
    );
    lighting.preserve(painter, start);
    // This path describes the existing processor, including its fixed latency.
    painter.text(
        p(135.0, 58.0),
        Align2::CENTER_CENTER,
        "16× DRIVE → TPT → LOW / BAND / HIGH",
        crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
        MUTED,
    );
    ui.interact(screen,ui.id().with(("atlas-svf-config",&node.id)),Sense::hover()).on_hover_text(
        "Configured small-signal TPT core response at zero cutoff CV. The 16× nonlinear drive, antialias filters and fixed 76-sample input/CV delay are excluded from this plot; it is not a live spectrum or the complete nonlinear response. Resonance sets damping k = 2 − 1.95R and Q = 1/k. Cutoff CV is exponential: cutoff × 2^(CV × depth), clamped to 1 Hz…0.45 × sample rate. SIGNAL determines voice width; mono CV broadcasts.");
}
