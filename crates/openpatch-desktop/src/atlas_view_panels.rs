// Stereo voltage conversion and observed polyphonic polarity are distinct.
fn atlas_view_kind(kind: &str) -> bool {
    matches!(
        kind,
        "audio_stereo" | "poly_viz" | "scope" | "notes" | "blank"
    )
}
fn atlas_viz_lamp_position(channel: usize, polarity: usize) -> Pos2 {
    pos2(
        32.0 + (channel / 8) as f32 * 80.0 + if polarity == 0 { 21.0 } else { 35.0 },
        120.0 + (channel % 8) as f32 * 24.0,
    )
}
fn atlas_audio_control(name: &str) -> (Pos2, f32) {
    if name == "gain" {
        (pos2(90.0, 120.0), 31.0)
    } else {
        (pos2(90.0, 274.0), 25.0)
    }
}
fn atlas_audio_value_center(name: &str) -> Pos2 {
    pos2(90.0, if name == "gain" { 166.0 } else { 300.0 })
}
fn atlas_audio_port_label(port: &str) -> &str {
    match port {
        "left" => "L",
        "right" => "R",
        _ => "STEREO",
    }
}
fn paint_atlas_audio_print(painter: &egui::Painter, rect: Rect, name: &str, zoom: f32) {
    painter.text(
        rect.min + vec2(90.0, if name == "gain" { 68.0 } else { 248.0 }) * zoom,
        Align2::CENTER_CENTER,
        if name == "gain" {
            "GAIN"
        } else {
            "10 Hz DC BLOCK"
        },
        crate::panel_skin::print_font(painter.ctx(), 11.0 * zoom),
        INK,
    );
}
fn paint_atlas_audio_configuration(painter: &egui::Painter, rect: Rect, zoom: f32) {
    for (y, label) in [
        (190.0, "PRE-FILTER: Σ × GAIN / 10"),
        (211.0, "OUT CH 1 = L · CH 2 = R"),
        (231.0, "R NO CHANNELS → L"),
    ] {
        painter.text(
            rect.min + vec2(90.0, y) * zoom,
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), 8.5 * zoom),
            INK,
        );
    }
}

impl Rack {
    fn viz_panel(&self, ui: &mut egui::Ui, node: &Node, rect: Rect) {
        let zoom = self.zoom;
        let painter = ui.painter();
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let state = self
            .module_lights
            .get(&node.id)
            .filter(|s| s.count == 16 && s.input_channels.is_some_and(|c| c <= 16));
        painter.text(
            point(90.0, 91.0),
            Align2::CENTER_CENTER,
            state.map_or_else(
                || "TELEMETRY UNAVAILABLE".to_owned(),
                |s| format!("{} / 16 CHANNELS", s.input_channels.unwrap()),
            ),
            crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
            MUTED,
        );
        for base in [32.0, 112.0] {
            for (x, label) in [(base + 21.0, "+"), (base + 35.0, "−")] {
                painter.text(
                    point(x, 105.0),
                    Align2::CENTER_CENTER,
                    label,
                    crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
                    MUTED,
                );
            }
        }
        for channel in 0..16 {
            let x = if channel < 8 { 32.0 } else { 112.0 };
            let y = 120.0 + (channel % 8) as f32 * 24.0;
            let active = state.is_some_and(|s| channel < s.input_channels.unwrap() as usize);
            painter.text(
                point(x, y),
                Align2::CENTER_CENTER,
                format!("{:02}", channel + 1),
                crate::panel_skin::print_font(painter.ctx(), 14.0 * zoom),
                if active { INK } else { Color32::from_gray(155) },
            );
            for (polarity, color) in [
                Color32::from_rgb(65, 174, 102),
                Color32::from_rgb(216, 75, 65),
            ]
            .into_iter()
            .enumerate()
            {
                let local = atlas_viz_lamp_position(channel, polarity);
                let center = point(local.x, local.y);
                // Retired voices can retain a fading light after their number dims.
                let value = state.map_or(0.0, |s| s.brightness[channel][polarity].clamp(0.0, 1.0));
                painter.circle_filled(center, 7.0 * zoom, Color32::from_rgb(71, 66, 58));
                self.lighting.circle(
                    painter,
                    center,
                    3.0 * zoom,
                    if state.is_none() {
                        Color32::from_gray(100)
                    } else {
                        color.gamma_multiply(0.15 + value * 0.85)
                    },
                    value,
                    0.15,
                );
            }
        }
        painter.text(
            point(90.0, 313.0),
            Align2::CENTER_CENTER,
            "+ GREEN   − RED",
            crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
            MUTED,
        );
        painter.text(
            point(90.0, 333.0),
            Align2::CENTER_CENTER,
            "OBSERVE · NO OUTPUT",
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            MUTED,
        );
        ui.interact(Rect::from_min_max(point(20.0, 106.0), point(158.0, 302.0)),
            ui.id().with((&node.id, "viz-meter")), Sense::hover()).on_hover_text(
                "Sixteen input voices. Bright numbers identify active channels, including silent voices. Green shows positive voltage; red shows negative voltage. Gray indicators mean telemetry is unavailable.");
    }
}
