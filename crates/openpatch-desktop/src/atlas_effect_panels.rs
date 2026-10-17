// Configured signal paths for the original studio effects, not sampled traces.
fn atlas_studio_effect(kind: &str) -> bool {
    matches!(kind, "stereo_delay" | "stereo_reverb")
}
fn atlas_studio_effect_control(kind: &str, name: &str) -> (Pos2, f32) {
    if kind == "stereo_delay" {
        let (column, row) = match name {
            "time_left" => (0, 0),
            "time_right" => (1, 0),
            "feedback" => (2, 0),
            "crossfeed" => (3, 0),
            "mod_rate" => (4, 0),
            "low_cut" => (0, 1),
            "high_cut" => (1, 1),
            "dry" => (2, 1),
            "wet" => (3, 1),
            _ => (4, 1),
        };
        (
            pos2(
                42.0 + 84.0 * column as f32,
                if row == 0 { 111.0 } else { 253.0 },
            ),
            if row == 0 { 25.0 } else { 23.0 },
        )
    } else {
        let (column, row) = match name {
            "predelay" => (0, 0),
            "size" => (1, 0),
            "decay" => (2, 0),
            "damping" => (0, 1),
            "dry" => (1, 1),
            _ => (2, 1),
        };
        (
            pos2(
                60.0 + 120.0 * column as f32,
                if row == 0 { 112.0 } else { 263.0 },
            ),
            if row == 0 { 29.0 } else { 25.0 },
        )
    }
}
fn atlas_studio_effect_value_center(kind: &str, name: &str) -> Pos2 {
    let (center, _) = atlas_studio_effect_control(kind, name);
    pos2(
        center.x,
        if center.y < 200.0 {
            164.0
        } else if kind == "stereo_delay" {
            295.0
        } else {
            306.0
        },
    )
}
fn atlas_space_rt60(node: &Node) -> f64 {
    let decay = atlas_voice_parameter(node, "decay", 0.65, 0.0, 1.0);
    0.3 + 8.0 * decay * decay
}
fn paint_atlas_studio_effect_configuration(
    ui: &mut egui::Ui,
    rect: Rect,
    node: &Node,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    let painter = ui.painter();
    let width = rect.width() / zoom;
    let p = |x, y| rect.min + vec2(x, y) * zoom;
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout { width },
        zoom,
    };
    let band = Rect::from_min_max(p(12.0, 178.0), p(width - 12.0, 205.0));
    let lighting_start = crate::lighting::next(painter);
    draw.screen(band);
    let ink = Color32::from_rgb(195, 200, 180);
    let rule = Stroke::new(0.8 * zoom, ink);
    let text = |x, y, label: &str, size, color| {
        painter.text(
            p(x, y),
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    };
    let block = |x, w, label| {
        painter.rect_stroke(
            Rect::from_center_size(p(x, 190.0), vec2(w, 16.0) * zoom),
            2.0 * zoom,
            rule,
            egui::StrokeKind::Inside,
        );
        text(x, 190.0, label, 8.0, ink);
    };
    if node.kind == "stereo_delay" {
        painter.circle_stroke(p(27.0, 190.0), 5.0 * zoom, rule);
        text(27.0, 190.0, "+", 8.0, ink);
        for (x, w, label) in [
            (74.0, 58.0, "DELAY"),
            (144.0, 32.0, "HP"),
            (202.0, 32.0, "LP"),
            (272.0, 56.0, "CROSS"),
            (350.0, 44.0, "× FB"),
        ] {
            block(x, w, label);
        }
        for (a, b) in [
            (32.0, 45.0),
            (103.0, 128.0),
            (160.0, 186.0),
            (218.0, 244.0),
            (300.0, 328.0),
        ] {
            painter.arrow(p(a, 190.0), vec2(b - a, 0.0) * zoom, rule);
        }
        painter.add(egui::Shape::line(
            vec![
                p(372.0, 190.0),
                p(394.0, 190.0),
                p(394.0, 202.0),
                p(27.0, 202.0),
                p(27.0, 195.0),
            ],
            rule,
        ));
    } else {
        for (x, w, label) in [
            (53.0, 54.0, "PRE"),
            (154.0, 94.0, "4-LINE FDN"),
            (277.0, 112.0, "WET + DRY"),
        ] {
            block(x, w, label);
        }
        for (a, b) in [(80.0, 107.0), (201.0, 221.0)] {
            painter.arrow(p(a, 190.0), vec2(b - a, 0.0) * zoom, rule);
        }
    }
    lighting.preserve(painter, lighting_start);
    if node.kind == "stereo_reverb" {
        text(
            266.0,
            24.0,
            &format!("RT60 {:.2} s", atlas_space_rt60(node)),
            11.0,
            INK,
        );
    }
    ui.interact(band,ui.id().with(("atlas-effect-guide",&node.id)),Sense::hover()).on_hover_text(
        if node.kind == "stereo_delay" {
            "Configured feedback path, not live audio. Delayed samples feed HP then LP; CROSS blends each filtered channel with the opposite channel before feedback gain and the input sum. WET taps the delay before feedback filtering. Output = DRY × input + WET × delayed. Right input normals to left. Modulation uses opposite sine phases, at most ±0.5% of each delay time. No tempo input."
        } else {
            "Configured four-line feedback delay network, not a live impulse response. Predelay feeds four delays; damped taps pass through a Hadamard feedback matrix and per-line decay gains. RT60 = 0.3 + 8 × DECAY² seconds nominally; damping shortens the high-frequency tail. SIZE scales line times by 0.5 + 1.5 × SIZE. Dry bypasses predelay/network; output has independent DRY and WET gains. Right input normals to left."
        });
}
fn paint_atlas_studio_effect_control_print(
    painter: &egui::Painter,
    rect: Rect,
    kind: &str,
    name: &str,
    zoom: f32,
) {
    let (center, _) = atlas_studio_effect_control(kind, name);
    let y = if center.y < 200.0 { 65.0 } else { 213.0 };
    painter.text(
        rect.min + vec2(center.x, y) * zoom,
        Align2::CENTER_CENTER,
        parameter_label(name),
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
}
