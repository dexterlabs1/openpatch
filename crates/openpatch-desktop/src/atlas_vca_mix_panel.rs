// Four squared-level strips and a linear master retain their actual dB laws.
fn atlas_vca_mix_control(name: &str) -> (Pos2, f32) {
    if name == "mix_level" {
        (pos2(345.0, 126.0), 29.0)
    } else {
        (
            pos2(
                37.5 + name
                    .trim_start_matches("level")
                    .parse::<usize>()
                    .unwrap_or(1)
                    .saturating_sub(1) as f32
                    * 75.0,
                126.0,
            ),
            26.0,
        )
    }
}
fn atlas_vca_mix_value_center(name: &str) -> Pos2 {
    let (center, _) = atlas_vca_mix_control(name);
    pos2(center.x, if name == "mix_level" { 170.0 } else { 167.0 })
}
fn paint_atlas_vca_mix_print(painter: &egui::Painter, rect: Rect, name: &str, zoom: f32) {
    let label = if name == "mix_level" {
        "MASTER".into()
    } else {
        format!("CH {}", name.trim_start_matches("level"))
    };
    painter.text(
        rect.min + vec2(atlas_vca_mix_control(name).0.x, 75.0) * zoom,
        Align2::CENTER_CENTER,
        label,
        crate::panel_skin::print_font(painter.ctx(), 11.0 * zoom),
        INK,
    );
}

impl Rack {
    fn mixing_panel(&self, ui: &mut egui::Ui, node: &Node, rect: Rect) {
        let zoom = self.zoom;
        let painter = ui.painter();
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        for x in [75.0, 150.0, 225.0, 300.0] {
            for (top, bottom) in [
                (84.0, 177.0),
                (208.0, 242.0),
                (278.0, 317.0),
                (343.0, 378.0),
            ] {
                painter.line_segment(
                    [point(x, top), point(x, bottom)],
                    Stroke::new(
                        if x == 300.0 { 1.0 } else { 0.5 } * zoom,
                        Color32::from_gray(178),
                    ),
                );
            }
        }
        let state = self.module_lights.get(&node.id).filter(|s| s.count >= 4);
        for channel in 0..4 {
            let center = point(37.5 + channel as f32 * 75.0, 183.0);
            let value = state
                .map_or(0.0, |s| s.brightness[channel][0])
                .clamp(0.0, 1.0);
            for segment in 0..12 {
                let lit = state.is_some() && value > (segment as f32 / 12.0);
                let color = if segment >= 10 {
                    Color32::from_rgb(223, 151, 58)
                } else {
                    Color32::from_rgb(76, 163, 112)
                };
                self.lighting.rect(
                    painter,
                    Rect::from_min_size(
                        center + vec2(-25.0 + segment as f32 * 4.2, -3.0) * zoom,
                        vec2(3.1, 6.0) * zoom,
                    ),
                    0.5,
                    color.gamma_multiply(if lit { 1.0 } else { 0.16 }),
                    if lit { 1.0 } else { 0.0 },
                    0.16,
                );
            }
            ui.interact(Rect::from_center_size(center,vec2(54.0,13.0)*zoom),ui.id().with((&node.id,"strip-vu",channel)),Sense::hover()).on_hover_text("DSP strip VU: signed channel sum / 5 V, peak smoothing, -24 to 0 dB brightness. Polyphonic lanes can cancel. This is the signal meter; the knob shows configured gain.");
        }
        painter.text(
            point(150.0, 57.0),
            Align2::CENTER_CENTER,
            if state.is_some() {
                "POST-CV VU · −24…0 dB"
            } else {
                "VU UNAVAILABLE"
            },
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            MUTED,
        );
        let channel_exp = node.params.get("channel_exp").copied().unwrap_or(0.0) >= 0.5;
        let mix_exp = node.params.get("mix_exp").copied().unwrap_or(0.0) >= 0.5;
        painter.text(
            point(150.0, 254.0),
            Align2::CENTER_CENTER,
            if channel_exp {
                "CV: [max(0, V/10)]⁴"
            } else {
                "CV: max(0, V/10)"
            },
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            MUTED,
        );
        painter.text(
            point(345.0, 215.0),
            Align2::CENTER_CENTER,
            "SUM",
            crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
            MUTED,
        );
        painter.text(
            point(345.0, 244.0),
            Align2::CENTER_CENTER,
            if mix_exp { "EXP CV" } else { "LINEAR CV" },
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            MUTED,
        );
        for (x, label) in [(150.0, "STRIP = IN × LEVEL² × CV"), (345.0, "LEVEL × CV")] {
            painter.text(
                point(x, 319.0),
                Align2::CENTER_CENTER,
                label,
                crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
                MUTED,
            );
        }
        painter.text(
            point(195.0, 385.0),
            Align2::CENTER_CENTER,
            "STRIP OUTPUTS ALWAYS FEED MIX",
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            MUTED,
        );
    }
}
