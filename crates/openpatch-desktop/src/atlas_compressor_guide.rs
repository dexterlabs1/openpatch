// Configured compressor transfer guide for the shared rack and browser face.
// It draws the static detector target from StudioFx::Compressor::target; it is
// not a trace of the live detector, attack/release follower, or audio output.
fn atlas_compressor_guide_target_db(
    input_db: f64,
    threshold_db: f64,
    ratio: f64,
    knee_db: f64,
) -> f64 {
    let above = input_db - threshold_db;
    let reduction = if knee_db > 0.0 && above.abs() < knee_db * 0.5 {
        (1.0 - 1.0 / ratio) * (above + knee_db * 0.5).powi(2) / (2.0 * knee_db)
    } else {
        (1.0 - 1.0 / ratio) * above.max(0.0)
    };
    input_db - reduction
}

fn paint_atlas_compressor_guide(
    ui: &mut egui::Ui,
    rect: Rect,
    node: &Node,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    if node.kind != "sidechain_compressor" || zoom <= 0.0 {
        return;
    }

    let painter = ui.painter();
    let p = |x: f32, y: f32| rect.min + vec2(x, y) * zoom;
    // The module is 420 × 396 design points. Keep the guide clear of the
    // numeric wells at y=250 and the input captions at y=321.
    let bounds = Rect::from_min_max(p(112.0, 264.0), p(402.0, 313.0));
    let lighting_start = crate::lighting::next(painter);
    let width = rect.width() / zoom;
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout { width },
        zoom,
    };
    draw.screen(bounds, lighting);

    let threshold = atlas_voice_parameter(node, "threshold_db", -10.5, -60.0, 12.0);
    let ratio = atlas_voice_parameter(node, "ratio", 4.0, 1.0, 20.0);
    let knee = atlas_voice_parameter(node, "knee_db", 6.0, 0.0, 24.0);
    let makeup = atlas_voice_parameter(node, "makeup_db", 0.0, -24.0, 24.0);

    const INPUT_MIN: f64 = -60.0;
    const INPUT_MAX: f64 = 12.0;
    let mut output_min = f64::INFINITY;
    let mut output_max = f64::NEG_INFINITY;
    for index in 0..=96 {
        let input_db = INPUT_MIN + (INPUT_MAX - INPUT_MIN) * index as f64 / 96.0;
        let output_db = atlas_compressor_guide_target_db(input_db, threshold, ratio, knee) + makeup;
        output_min = output_min.min(output_db);
        output_max = output_max.max(output_db);
    }
    // Round the fitted output limits to useful 6 dB markings with a little
    // headroom so both ends of the configured curve remain visible.
    let pad = ((output_max - output_min) * 0.06).max(1.0);
    let output_min = ((output_min - pad) / 6.0).floor() * 6.0;
    let output_max = ((output_max + pad) / 6.0).ceil() * 6.0;
    let output_span = (output_max - output_min).max(6.0);

    let left = 132.0;
    let right = 398.0;
    let top = 277.0;
    let bottom = 302.0;
    let to_plot = |input_db: f64, output_db: f64| {
        let x = left + ((input_db - INPUT_MIN) / (INPUT_MAX - INPUT_MIN)) as f32 * (right - left);
        let y = bottom - ((output_db - output_min) / output_span) as f32 * (bottom - top);
        p(x, y)
    };

    let grid = Color32::from_rgb(76, 91, 77);
    let curve = Color32::from_rgb(228, 185, 103);
    let rule = Stroke::new((0.55 * zoom).max(0.45), grid);
    painter.line_segment([p(left, top), p(left, bottom)], rule);
    painter.line_segment([p(left, bottom), p(right, bottom)], rule);
    let zero_x = left + ((0.0 - INPUT_MIN) / (INPUT_MAX - INPUT_MIN)) as f32 * (right - left);
    painter.line_segment(
        [p(zero_x, top), p(zero_x, bottom)],
        Stroke::new((0.45 * zoom).max(0.4), grid),
    );
    if output_min < 0.0 && output_max > 0.0 {
        let zero_y = bottom - ((0.0 - output_min) / output_span) as f32 * (bottom - top);
        painter.line_segment(
            [p(left, zero_y), p(right, zero_y)],
            Stroke::new((0.45 * zoom).max(0.4), grid),
        );
    }

    let points: Vec<_> = (0..=96)
        .map(|index| {
            let input_db = INPUT_MIN + (INPUT_MAX - INPUT_MIN) * index as f64 / 96.0;
            let output_db =
                atlas_compressor_guide_target_db(input_db, threshold, ratio, knee) + makeup;
            to_plot(input_db, output_db)
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        Stroke::new((1.5 * zoom).max(0.75), curve),
    ));

    let label_color = Color32::from_rgb(195, 200, 180);
    let label_size = 7.0 * zoom;
    let label = |at: Pos2, text: String, align: Align2| {
        painter.text(
            at,
            align,
            text,
            crate::panel_skin::print_font(painter.ctx(), label_size),
            label_color,
        );
    };
    // Printed output limits and input endpoints make both axis scales explicit.
    label(
        p(128.0, top),
        if output_max == 0.0 {
            "0".into()
        } else {
            format!("{output_max:+.0}")
        },
        Align2::RIGHT_CENTER,
    );
    label(
        p(128.0, bottom),
        format!("{output_min:+.0}"),
        Align2::RIGHT_CENTER,
    );
    label(p(left, 307.0), "−60".into(), Align2::LEFT_CENTER);
    label(p(right, 307.0), "+12".into(), Align2::RIGHT_CENTER);
    painter.text(
        p(395.0, 266.0),
        Align2::RIGHT_TOP,
        "IN → OUT dB",
        crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
        label_color,
    );

    lighting.preserve(painter, lighting_start);
    ui.interact(
        bounds,
        ui.id().with(("atlas-compressor-guide", &node.id)),
        Sense::hover(),
    )
    .on_hover_text(
        "Configured static transfer target before detector, attack, and release smoothing. Horizontal axis spans −60 to +12 dB; vertical limits auto-fit and are printed. Input dB is relative to the compressor's 5 V reference. The target uses the configured threshold, ratio, and quadratic soft knee; output dB includes makeup gain. In the processor, stereo-linked RMS power is smoothed with DETECTOR, and an external SIDECHAIN input replaces the max stereo detector when connected. ATTACK and RELEASE smooth gain toward this target. Audio output is input × gain × makeup; the GAIN output is 10 V × applied gain factor before makeup.",
    );
}
