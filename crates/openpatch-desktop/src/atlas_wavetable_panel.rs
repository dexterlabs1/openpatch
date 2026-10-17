// The actual selected source-table geometry, shared by rack and browser. This
// is not an oscilloscope: only the table-position marker may follow telemetry.
fn atlas_oscillator(kind: &str) -> bool {
    is_wavetable(kind) || matches!(kind, "multi_vco" | "multi_lfo")
}
fn wavetable_parameter_radius(index: usize) -> f32 {
    if index < 3 {
        25.0
    } else {
        22.0
    }
}
fn wavetable_value_center(index: usize) -> Pos2 {
    let center = wavetable_parameter_position(index);
    pos2(center.x, if index < 3 { 234.0 } else { 324.0 })
}
fn paint_atlas_wavetable_print(
    painter: &egui::Painter,
    rect: Rect,
    index: usize,
    name: &str,
    clock_ratio: bool,
    zoom: f32,
) {
    let center = wavetable_parameter_position(index);
    painter.text(
        rect.min + vec2(center.x, if index < 3 { 154.0 } else { 253.0 }) * zoom,
        Align2::CENTER_CENTER,
        if name == "rate" && clock_ratio {
            "CLOCK RATIO"
        } else {
            wavetable_parameter_label(name)
        },
        crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
        INK,
    );
}
fn paint_wavetable_display(
    painter: &egui::Painter,
    node: &Node,
    patch: &Patch,
    rect: Rect,
    zoom: f32,
    lights: Option<&ModuleLightState>,
    lighting: &crate::lighting::RoomLighting,
) {
    let point = |x, y| rect.min + vec2(x, y) * zoom;
    let start = crate::lighting::next(painter);
    let outer_clip = painter.clip_rect();
    let screen = Rect::from_min_max(point(18.0, 66.0), point(342.0, 143.0));
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout { width: 360.0 },
        zoom,
    };
    draw.screen(screen, lighting);
    let painter = painter.with_clip_rect(outer_clip.intersect(screen));
    let quiet = Color32::from_rgb(143, 155, 133);
    let amber = Color32::from_rgb(232, 184, 100);
    let position = wavetable_display_position(node, lights);
    let (name, points) = match &node.content {
        Some(openpatch_core::ModuleContent::Wavetable {
            name, wave_points, ..
        }) => (name.as_str(), *wave_points),
        _ => ("Basic", 1024),
    };
    let meta = painter.with_clip_rect(
        Rect::from_min_max(point(25.0, 68.0), point(215.0, 84.0)).intersect(screen),
    );
    meta.text(
        point(25.0, 76.0),
        Align2::LEFT_CENTER,
        format!("TABLE · {name} · {points} pt"),
        crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
        quiet,
    );
    let plot = Rect::from_min_max(point(25.0, 89.0), point(335.0, 120.0));
    for y in [plot.top(), plot.center().y, plot.bottom()] {
        painter.line_segment(
            [pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(0.5 * zoom, Color32::from_rgb(47, 61, 44)),
        );
    }
    for fraction in [0.25, 0.5, 0.75] {
        let x = egui::lerp(plot.x_range(), fraction);
        painter.line_segment(
            [pos2(x, plot.top()), pos2(x, plot.bottom())],
            Stroke::new(0.5 * zoom, Color32::from_rgb(47, 61, 44)),
        );
    }
    if let Some(samples) = wavetable_display_samples(node, patch, position) {
        let peak = samples
            .iter()
            .map(|v| f64::from(*v).abs())
            .fold(0.0_f64, f64::max);
        let scale = if peak > 0.0 { peak } else { 1.0 };
        let line = samples
            .iter()
            .enumerate()
            .map(|(i, sample)| {
                pos2(
                    plot.left() + i as f32 / 160.0 * plot.width(),
                    plot.center().y - (f64::from(*sample) / scale) as f32 * plot.height() * 0.46,
                )
            })
            .collect();
        painter.add(egui::Shape::line(line, Stroke::new(1.2 * zoom, amber)));
        painter.text(
            point(335.0, 76.0),
            Align2::RIGHT_CENTER,
            format!("AUTO ±{}", atlas_signal_number(peak)),
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            quiet,
        );
    } else {
        painter.text(
            plot.center(),
            Align2::CENTER_CENTER,
            "Table unavailable",
            crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
            quiet,
        );
    }
    let observed = lights
        .and_then(|s| s.waveform_position)
        .is_some_and(f32::is_finite);
    painter.text(
        point(25.0, 126.0),
        Align2::LEFT_CENTER,
        if observed { "LIVE POS" } else { "SET POS" },
        crate::panel_skin::print_font(painter.ctx(), 6.5 * zoom),
        quiet,
    );
    let track = Rect::from_min_max(point(69.0, 125.0), point(335.0, 127.0));
    painter.rect_filled(track, 0.0, Color32::from_rgb(63, 75, 58));
    painter.circle_filled(
        pos2(egui::lerp(track.x_range(), position), track.center().y),
        2.0 * zoom,
        amber,
    );
    let painter = painter.with_clip_rect(rect.intersect(outer_clip));
    let labels = if node.kind == "wavetable_vco" {
        ["−", "+", "POLY", "SOFT", "LIN"]
    } else {
        ["A", "B", "POLY", "OFFSET", "INV"]
    };
    // The lens floor still follows room illumination; absent telemetry cannot
    // create signal glow even inside the emissive table display.
    for (index, label) in labels.iter().enumerate() {
        let center = point(54.0 + index as f32 * 63.0, 136.0);
        let strength = lights
            .filter(|s| index < usize::from(s.count))
            .map_or(0.0, |s| s.brightness[index][0])
            .clamp(0.0, 1.0);
        lighting.circle(
            &painter,
            center,
            2.5 * zoom,
            Color32::from_rgb(207, 176, 83).gamma_multiply(0.12 + strength * 0.88),
            strength,
            0.12,
        );
        painter.text(
            center + vec2(6.0, 0.0) * zoom,
            Align2::LEFT_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), 6.5 * zoom),
            quiet,
        );
    }
    lighting.preserve(&painter, start);
}
