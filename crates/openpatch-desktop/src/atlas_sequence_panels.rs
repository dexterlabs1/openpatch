// One shared cell geometry; distinct clock/state semantics stay in each panel.
fn atlas_sequence_kind(kind: &str) -> bool {
    matches!(kind, "seq3" | "stock_seq3")
}
fn atlas_sequence_control(name: &str) -> (Pos2, f32) {
    if let Some((row, column)) = sequence_cell(name) {
        (
            pos2(238.0 + column as f32 * 67.0, 112.0 + row as f32 * 72.0),
            20.0,
        )
    } else {
        (
            stock_seq3_parameter_position(name),
            if matches!(name, "tempo" | "steps") {
                25.0
            } else {
                22.0
            },
        )
    }
}
fn atlas_sequence_value_center(name: &str) -> Pos2 {
    let (p, _) = atlas_sequence_control(name);
    p + vec2(
        0.0,
        if sequence_cell(name).is_some() {
            34.0
        } else if matches!(name, "tempo" | "steps") {
            39.0
        } else {
            36.0
        },
    )
}
fn paint_atlas_sequence_print(
    painter: &egui::Painter,
    rect: Rect,
    kind: &str,
    name: &str,
    zoom: f32,
) {
    if sequence_cell(name).is_some() {
        return;
    }
    let (center, _) = atlas_sequence_control(name);
    let label = stock_seq3_parameter_label(name);
    painter.text(
        rect.min + vec2(center.x, if center.y < 150.0 { 75.0 } else { 176.0 }) * zoom,
        Align2::CENTER_CENTER,
        label,
        crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
        INK,
    );
    if name == "tempo" {
        painter.text(
            rect.min + vec2(center.x, 59.0) * zoom,
            Align2::CENTER_CENTER,
            if kind == "stock_seq3" {
                "60 × 2^TEMPO BPM"
            } else {
                "15–960 BPM"
            },
            crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
            MUTED,
        );
    }
}
fn paint_atlas_sequence_guide(painter: &egui::Painter, rect: Rect, kind: &str, zoom: f32) {
    let p = |x, y| rect.min + vec2(x, y) * zoom;
    painter.text(
        p(487.0, 25.0),
        Align2::CENTER_CENTER,
        "CLOCK → ADVANCE · SELECT STEP → HOLD THREE CV ROWS · −10…+10 V",
        crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
        MUTED,
    );
    // Guide states what is configured; step/running lamps are actual telemetry.
    painter.text(
        p(153.0, 59.0),
        Align2::CENTER_CENTER,
        "1–8 STEPS",
        crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
        MUTED,
    );
    if kind == "seq3" {
        painter.line_segment(
            [p(207.0, 65.0), p(207.0, 388.0)],
            Stroke::new(zoom, Color32::from_rgb(153, 141, 111)),
        );
        for row in 0..3 {
            painter.text(
                p(206.0, 112.0 + row as f32 * 72.0),
                Align2::RIGHT_CENTER,
                format!("CV{}", row + 1),
                crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
                MUTED,
            );
        }
    }
}

impl Rack {
    fn atlas_sequence_toggle_anchor(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        name: &str,
        rect: Rect,
        id: egui::Id,
    ) {
        if let Some(cue) = self
            .construction_animation
            .parameter(&node.id, &node.kind, name)
        {
            self.register_construction_anchor(
                ui,
                node,
                name,
                crate::construction_motion::Anchor {
                    rect,
                    old_text: if cue.old == 0.0 { "Off" } else { "On" }.into(),
                    new_text: if cue.new == 0.0 { "Off" } else { "On" }.into(),
                    marker: None,
                },
                &[id],
            );
        }
    }
}

fn paint_atlas_sequence_readout(
    painter: &egui::Painter,
    center: Pos2,
    text: &str,
    width: f32,
    zoom: f32,
) {
    paint_atlas_voice_readout_size(
        painter,
        center,
        text,
        width,
        zoom,
        if width / zoom > 60.0 { 11.0 } else { 9.0 },
    );
}
