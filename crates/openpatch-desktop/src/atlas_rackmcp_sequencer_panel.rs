// RackMCP's saved patterns and target lanes have their own panel and identity.
// Authored cells never imply a published live position or gate observation.
fn atlas_rackmcp_control(name: &str) -> (Pos2, f32) {
    let x = match name {
        "bpm" => 60.0,
        "swing" => 150.0,
        "running" => 240.0,
        _ => 330.0,
    };
    (pos2(x, 114.0), 28.0)
}

fn atlas_rackmcp_value_center(name: &str) -> Pos2 {
    pos2(atlas_rackmcp_control(name).0.x, 157.0)
}

fn atlas_rackmcp_port(input: bool, index: usize) -> Pos2 {
    if input {
        pos2(60.0 + index as f32 * 80.0, 373.0)
    } else if index < 16 {
        pos2(
            414.0 + (index % 4) as f32 * 84.0,
            129.0 + (index / 4) as f32 * 74.0,
        )
    } else {
        pos2(230.0 + (index - 16) as f32 * 80.0, 373.0)
    }
}

fn paint_atlas_rackmcp_print(painter: &egui::Painter, rect: Rect, name: &str, zoom: f32) {
    painter.text(
        rect.min + vec2(atlas_rackmcp_control(name).0.x, 77.0) * zoom,
        Align2::CENTER_CENTER,
        match name {
            "bpm" => "TEMPO · BPM",
            "swing" => "SWING · 50–75%",
            "running" => "RUN",
            _ => "RESET",
        },
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
}

fn paint_atlas_rackmcp_configuration(
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
    let bounded = |y, value: String, size, color| {
        let mut job = egui::text::LayoutJob::simple(
            value,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
            338.0 * zoom,
        );
        job.wrap.max_rows = 1;
        let galley = painter.layout_job(job);
        painter.galley(p(190.0, y) - galley.size() * 0.5, galley, color);
    };
    text(
        190.0,
        55.0,
        "FOUR TRACKS · UP TO FOUR VOICES EACH",
        8.5,
        MUTED,
    );
    text(540.0, 55.0, "PITCH = NOTES / 12 V · 0 V = C4", 9.0, INK);
    for track in 0..4 {
        text(
            414.0 + track as f32 * 84.0,
            77.0,
            &format!("TRACK {}", track + 1),
            10.0,
            INK,
        );
    }
    painter.line_segment(
        [p(378.0, 65.0), p(378.0, 389.0)],
        Stroke::new(zoom, Color32::from_rgb(153, 141, 111)),
    );

    let state = node.rackmcp_sequence.as_ref();
    let data = state.map(|state| &state.data);
    let selected = data
        .and_then(|data| {
            data.selected_pattern.as_ref().and_then(|name| {
                data.patterns
                    .iter()
                    .position(|pattern| &pattern.name == name)
            })
        })
        .unwrap_or(0);
    let pattern = data.and_then(|data| data.patterns.get(selected));
    let length = pattern.map_or(16, |pattern| pattern.length).clamp(1, 64);
    let division = pattern.map_or(4, |pattern| pattern.steps_per_beat);
    let start = crate::lighting::next(painter);
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout { width: 720.0 },
        zoom,
    };
    draw.screen(
        Rect::from_min_max(p(20.0, 173.0), p(360.0, 294.0)),
        lighting,
    );
    let amber = Color32::from_rgb(229, 183, 103);
    let hit = Color32::from_rgb(137, 174, 137);
    bounded(
        184.0,
        format!(
            "AUTHORED · {} · {length} STEPS · {division} / BEAT",
            pattern.map_or("A", |pattern| pattern.name.as_str())
        ),
        9.0,
        amber,
    );
    text(46.0, 202.0, "PATTERN", 7.0, amber);
    // Slots indicate saved pattern selection, never the runtime playhead.
    for index in 0..data.map_or(1, |data| data.patterns.len()).min(16) {
        let cell = Rect::from_center_size(
            p(85.0 + index as f32 * 17.2, 202.0),
            vec2(14.0, 12.0) * zoom,
        );
        painter.rect_filled(
            cell,
            zoom,
            if index == selected {
                amber
            } else {
                Color32::from_rgb(44, 53, 45)
            },
        );
        text(
            85.0 + index as f32 * 17.2,
            202.0,
            &format!("{}", index + 1),
            7.0,
            if index == selected { INK } else { amber },
        );
    }
    for track in 0..4 {
        let y = 221.0 + track as f32 * 20.0;
        let authored = pattern.and_then(|pattern| pattern.tracks.get(track));
        text(
            48.0,
            y,
            &format!(
                "T{} · {} VOICES",
                track + 1,
                authored.map_or(1, |track| track.voices)
            ),
            8.0,
            amber,
        );
        let mut steps = [None; 64];
        if let Some(track) = authored {
            for step in &track.steps {
                if let Some(slot) = steps.get_mut(usize::from(step.step)) {
                    *slot = Some(step);
                }
            }
        }
        let width = 267.0 / f32::from(length);
        for (index, step) in steps.iter().enumerate().take(usize::from(length)) {
            let cell = Rect::from_min_max(
                p(81.0 + index as f32 * width, y - 4.0),
                p(81.0 + (index + 1) as f32 * width - 0.7, y + 4.0),
            );
            // Empty notes are source drum hits with held pitch, not rests.
            let enabled = step.is_some_and(|step| step.prob > 0.0);
            let fill = if enabled {
                if step.is_some_and(|step| step.notes.is_empty()) {
                    hit
                } else {
                    amber
                }
            } else {
                Color32::from_rgb(44, 53, 45)
            };
            painter.rect_filled(cell, 0.0, fill);
            if step.is_some() && !enabled {
                painter.rect_stroke(
                    cell,
                    0.0,
                    Stroke::new(0.6 * zoom, Color32::from_rgb(124, 125, 101)),
                    egui::StrokeKind::Inside,
                );
            }
            if enabled && step.is_some_and(|step| step.accent) {
                painter.circle_filled(
                    cell.center() - vec2(0.0, 6.5) * zoom,
                    1.4 * zoom,
                    Color32::from_rgb(250, 234, 190),
                );
            }
        }
    }
    lighting.preserve(painter, start);
    let lanes = data.map_or(&[][..], |data| data.lanes.as_slice());
    let targets = state.map_or(&[][..], |state| state.targets.as_slice());
    let native = lanes
        .iter()
        .filter(|lane| {
            targets.iter().any(|target| {
                target.source_module_id == lane.module_id && target.source_param_id == lane.param_id
            })
        })
        .count();
    bounded(
        305.0,
        format!("AUTOMATION · {} / 8 LANES · {native} NATIVE", lanes.len()),
        8.5,
        INK,
    );
    let sources = if lanes.is_empty() {
        "SOURCE TARGETS · NONE".to_owned()
    } else {
        format!(
            "SOURCE TARGETS · {}",
            lanes
                .iter()
                .map(|lane| format!("{}:{}", lane.module_id, lane.param_id))
                .collect::<Vec<_>>()
                .join(" · ")
        )
    };
    bounded(320.0, sources, 8.0, MUTED);
    text(
        190.0,
        334.0,
        "AMBER NOTES · GREEN HITS · DOT ACCENT",
        8.0,
        MUTED,
    );
    text(
        540.0,
        383.0,
        "GATE / ACCENT 10 V · VELOCITY × 10 V",
        9.0,
        MUTED,
    );
}
