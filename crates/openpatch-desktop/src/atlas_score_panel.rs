// Typed authored data only: this overview does not invent a live playhead.
fn atlas_score_control(name: &str) -> (Pos2, f32) {
    (
        pos2(
            match name {
                "tempo" => 65.0,
                "swing" => 185.0,
                _ => 300.0,
            },
            120.0,
        ),
        29.0,
    )
}
fn atlas_score_value_center(name: &str) -> Pos2 {
    pos2(atlas_score_control(name).0.x, 164.0)
}
fn paint_atlas_score_print(painter: &egui::Painter, rect: Rect, name: &str, zoom: f32) {
    painter.text(
        rect.min + vec2(atlas_score_control(name).0.x, 77.0) * zoom,
        Align2::CENTER_CENTER,
        match name {
            "tempo" => "TEMPO",
            "swing" => "SWING",
            _ => "INITIAL RUN",
        },
        crate::panel_skin::print_font(painter.ctx(), 11.0 * zoom),
        INK,
    );
}
fn paint_atlas_score_configuration(
    painter: &egui::Painter,
    node: &Node,
    rect: Rect,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
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
    text(
        454.0,
        84.0,
        "FOUR TRACKS · UP TO FOUR VOICES EACH",
        9.0,
        INK,
    );
    text(454.0, 104.0, "PITCH = SEMITONES FROM C4 / 12", 9.0, INK);
    text(454.0, 124.0, "GATES 10 V · VELOCITY × 10 V", 9.0, INK);
    text(454.0, 144.0, "EXTERNAL CLOCK: ONE STEP / EDGE", 8.5, MUTED);
    let length = node.score.as_ref().map_or(64, |s| s.length).clamp(1, 64);
    let division = node
        .score
        .as_ref()
        .map_or(4, |s| s.steps_per_beat)
        .clamp(1, 8);
    let lanes = node.score.as_ref().map_or(0, |s| s.lanes.len());
    let start = crate::lighting::next(painter);
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout { width: 600.0 },
        zoom,
    };
    draw.screen(
        Rect::from_min_max(p(20.0, 179.0), p(270.0, 258.0)),
        lighting,
    );
    let amber = Color32::from_rgb(229, 183, 103);
    text(
        145.0,
        186.0,
        &format!("AUTHORED · {length} STEPS · {division} / BEAT"),
        9.0,
        amber,
    );
    for track in 0..4 {
        let y = 200.0 + track as f32 * 16.0;
        text(35.0, y, &format!("T{}", track + 1), 9.0, amber);
        // A bounded lookup keeps a full 64-step score linear in authored size.
        let mut steps = [None; 64];
        if let Some(t) = node.score.as_ref().and_then(|s| s.tracks.get(track)) {
            for step in &t.steps {
                if let Some(slot) = steps.get_mut(usize::from(step.step)) {
                    *slot = Some(step);
                }
            }
        }
        let width = 210.0 / f32::from(length);
        for (index, step) in steps.iter().enumerate().take(usize::from(length)) {
            let cell = Rect::from_min_max(
                p(52.0 + index as f32 * width, y - 4.0),
                p(52.0 + (index + 1) as f32 * width - 0.8, y + 4.0),
            );
            let playable = step.is_some_and(|s| !s.notes.is_empty() && s.probability > 0.0);
            painter.rect_filled(
                cell,
                0.0,
                if playable {
                    amber
                } else {
                    Color32::from_rgb(44, 53, 45)
                },
            );
            if step.is_some() && !playable {
                painter.rect_stroke(
                    cell,
                    0.0,
                    Stroke::new(0.6 * zoom, Color32::from_rgb(124, 125, 101)),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }
    lighting.preserve(painter, start);
    text(
        145.0,
        316.0,
        &format!("AUTOMATION · {lanes} / 16 LANES"),
        9.0,
        INK,
    );
    text(
        145.0,
        332.0,
        "NOTES / CURVES EDITABLE THROUGH MCP",
        8.5,
        MUTED,
    );
    for track in 0..4 {
        text(
            310.0 + track as f32 * 78.0,
            182.0,
            &format!("TRACK {}", track + 1),
            10.0,
            INK,
        );
    }
}
