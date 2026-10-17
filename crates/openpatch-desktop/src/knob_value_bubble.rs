// A rack-owned, paint-only overlay. It never creates a popup or captures input.
struct KnobValueBubble {
    id: egui::Id,
    text: String,
    anchor: Rect,
    appeared: f64,
    last_active: f64,
    seen_frame: u64,
}
impl Rack {
    fn observe_knob_value_bubble(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        input: &FaderInput,
        enabled: bool,
        text: &str,
    ) {
        if !enabled {
            if self
                .knob_value_bubble
                .as_ref()
                .is_some_and(|b| b.id == response.id)
            {
                self.knob_value_bubble = None;
            }
            return;
        }
        let now = ui.input(|i| i.time);
        let active = response.dragged_by(PointerButton::Primary)
            || input.normalized.is_some()
            || input.nudge != 0
            || input.boundary.is_some()
            || input.reset;
        if active
            && self
                .knob_value_bubble
                .as_ref()
                .is_none_or(|b| b.id != response.id)
        {
            self.knob_value_bubble = Some(KnobValueBubble {
                id: response.id,
                text: text.into(),
                anchor: response.rect,
                appeared: now,
                last_active: now,
                seen_frame: ui.ctx().cumulative_frame_nr(),
            });
        }
        if let Some(bubble) = self
            .knob_value_bubble
            .as_mut()
            .filter(|b| b.id == response.id)
        {
            bubble.text = text.into();
            bubble.anchor = response.rect;
            bubble.seen_frame = ui.ctx().cumulative_frame_nr();
            if active {
                bubble.last_active = now;
            }
        }
    }
    fn paint_knob_value_bubble(&mut self, ui: &egui::Ui, canvas: Rect) {
        let Some(bubble) = self.knob_value_bubble.as_ref() else {
            return;
        };
        let now = ui.input(|i| i.time);
        let age = (now - bubble.last_active).max(0.0);
        if age >= 0.72
            || bubble.seen_frame != ui.ctx().cumulative_frame_nr()
            || self.editing.is_some()
            || self.annotation_edit.is_some()
            || !self.shortcuts_enabled
            || !ui.input(|i| i.focused)
            || ui.input(|i| i.key_pressed(egui::Key::Escape))
            || egui::Popup::is_any_open(ui.ctx())
        {
            self.knob_value_bubble = None;
            return;
        }
        let ease = |v: f32| {
            let t = v.clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let enter = ease(((now - bubble.appeared) / 0.09) as f32);
        let fade = 1.0 - ease(((age - 0.5) / 0.22) as f32);
        let alpha = enter * fade;
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            ui.id().with("knob-value-bubble"),
        ));
        let color = Color32::from_rgb(249, 240, 213).gamma_multiply(alpha);
        // Screen-point type remains readable even when the rack is zoomed out.
        let font = crate::panel_skin::print_font(ui.ctx(), (16.0 * self.zoom).max(22.0));
        let galley = painter.layout_no_wrap(bubble.text.clone(), font, color);
        let size = galley.size() + vec2(24.0, 14.0);
        let mut body = Rect::from_center_size(
            pos2(
                bubble.anchor.center().x,
                bubble.anchor.top() - 10.0 - size.y * 0.5 + (1.0 - enter) * 3.0,
            ),
            size,
        );
        let bounds = canvas.intersect(ui.ctx().content_rect()).shrink(5.0);
        if bounds.is_positive() {
            body = body.translate(vec2(
                if body.left() < bounds.left() {
                    bounds.left() - body.left()
                } else if body.right() > bounds.right() {
                    bounds.right() - body.right()
                } else {
                    0.0
                },
                if body.top() < bounds.top() {
                    bounds.top() - body.top()
                } else if body.bottom() > bounds.bottom() {
                    bounds.bottom() - body.bottom()
                } else {
                    0.0
                },
            ));
        }
        for (spread, opacity) in [(5.0, 12), (3.0, 18), (1.0, 25)] {
            painter.rect_filled(
                body.translate(vec2(0.0, 3.0)).expand(spread),
                10.0,
                Color32::from_black_alpha(opacity).gamma_multiply(alpha),
            );
        }
        let fill = Color32::from_rgb(22, 31, 25).gamma_multiply(alpha);
        painter.rect_filled(body, 8.0, fill);
        painter.rect_stroke(
            body,
            8.0,
            Stroke::new(
                1.0_f32,
                Color32::from_rgb(169, 167, 128).gamma_multiply(alpha),
            ),
            egui::StrokeKind::Inside,
        );
        // Keep the tail aimed at the knob when clamping near the canvas edge.
        let tail_x = bubble
            .anchor
            .center()
            .x
            .clamp(body.left() + 10.0, body.right() - 10.0);
        if body.bottom() + 6.0 <= bubble.anchor.top() {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(tail_x - 5.0, body.bottom() - 0.5),
                    pos2(tail_x + 5.0, body.bottom() - 0.5),
                    pos2(tail_x, body.bottom() + 6.0),
                ],
                fill,
                Stroke::NONE,
            ));
        }
        painter.galley(body.center() - galley.size() * 0.5, galley, color);
        if enter < 1.0 || age > 0.5 {
            ui.ctx().request_repaint();
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64((0.5 - age).max(0.001)));
        }
    }
}
