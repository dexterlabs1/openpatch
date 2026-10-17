// Functional, configured signal paths for the stochastic family. These diagrams
// never invent running random samples; measured lamps and held values are separate.
fn atlas_stochastic_kind(kind: &str) -> bool {
    matches!(
        kind,
        "white_noise" | "stock_noise" | "colored_noise" | "random" | "random_values"
    )
}
fn atlas_stochastic_rotary(kind: &str, name: &str) -> bool {
    (kind == "white_noise" && name == "level")
        || (kind == "random" && !matches!(name, "seed" | "unipolar"))
}
fn random_parameter_radius(index: usize) -> f32 {
    if index == 8 {
        10.0
    } else if index < 4 {
        23.0
    } else {
        21.0
    }
}
fn random_value_center(index: usize) -> Pos2 {
    if index == 8 {
        pos2(280.0, 298.0)
    } else {
        let center = random_parameter_position(index);
        pos2(center.x, if index < 4 { 166.0 } else { 273.0 })
    }
}
fn white_noise_value_center(name: &str) -> Pos2 {
    pos2(90.0, if name == "level" { 185.0 } else { 292.0 })
}
fn paint_stochastic_control_print(
    painter: &egui::Painter,
    rect: Rect,
    kind: &str,
    index: usize,
    name: &str,
    zoom: f32,
) {
    let (center, y, label) = if kind == "random" {
        if index == 8 {
            return;
        }
        (
            random_parameter_position(index),
            if index < 4 { 91.0 } else { 202.0 },
            random_parameter_label(name),
        )
    } else {
        (
            pos2(90.0, 0.0),
            if name == "level" { 78.0 } else { 201.0 },
            if name == "level" {
                "RMS LEVEL"
            } else {
                "SEED · 24-BIT"
            },
        )
    };
    painter.text(
        rect.min + vec2(center.x, y) * zoom,
        Align2::CENTER_CENTER,
        label,
        crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
        INK,
    );
    if kind == "white_noise" && name == "level" {
        painter.text(
            rect.min + vec2(90.0, 91.0) * zoom,
            Align2::CENTER_CENTER,
            "0–10 V RMS",
            crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
            MUTED,
        );
    }
}
fn paint_stochastic_configuration(
    painter: &egui::Painter,
    rect: Rect,
    kind: &str,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout {
            width: rect.width() / zoom,
        },
        zoom,
    };
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
    if kind == "random" {
        text(42.0, 298.0, "RANGE", 9.0, INK);
    }
    let start = crate::lighting::next(painter);
    let amber = Color32::from_rgb(229, 183, 103);
    let ink = Color32::from_rgb(195, 200, 180);
    if kind == "white_noise" {
        draw.screen(
            Rect::from_min_max(p(18.0, 313.0), p(162.0, 350.0)),
            lighting,
        );
        text(90.0, 325.0, "N(0,1) × LEVEL → OUT", 9.0, amber);
        text(90.0, 340.0, "RMS VOLTS · NO PEAK LIMIT", 7.0, ink);
    } else {
        draw.screen(
            Rect::from_min_max(p(17.0, 180.0), p(343.0, 193.0)),
            lighting,
        );
        text(180.0, 186.5, "CLOCK → PROB → TARGET → CURVES", 8.0, amber);
    }
    lighting.preserve(painter, start);
}

fn random_seed_badge_rect() -> Rect {
    Rect::from_min_max(pos2(228.0, 15.0), pos2(344.0, 34.0))
}
impl Rack {
    fn random_seed_badge(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
        interactive: bool,
    ) {
        let local = random_seed_badge_rect();
        let bounds = Rect::from_min_max(
            rect.min + local.min.to_vec2() * self.zoom,
            rect.min + local.max.to_vec2() * self.zoom,
        );
        let badge_id = ui.id().with(("random-seed", &node.id));
        let edit = random_seed_edit(node, revision);
        if self.precision_collects(node) {
            self.register_precision_control(edit.clone(), bounds);
        }
        if let Some(cue) = self
            .construction_animation
            .parameter(&node.id, &node.kind, "seed")
        {
            self.register_construction_anchor(
                ui,
                node,
                "seed",
                crate::construction_motion::Anchor {
                    rect: bounds,
                    old_text: format!("SEED {:.0}", cue.old),
                    new_text: format!("SEED {}", edit.text),
                    marker: None,
                },
                &[badge_id],
            );
        }
        paint_atlas_voice_readout(
            ui.painter(),
            bounds.center(),
            &format!("SEED {}", edit.text),
            bounds.width(),
            self.zoom,
        );
        if interactive {
            let response = ui.interact(bounds, badge_id, Sense::click());
            if response.clicked() {
                consume_parameter_activation(ui.ctx());
                self.open_precision_editor(edit, bounds);
                self.select_primary(node.id.clone());
            }
            response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Edit integer seed, 1–16,777,215. Changing the seed restarts this native random generator; other graph edits preserve its state.");
        }
    }
}
