// Camera assistance belongs to the held cable gesture, never its graph edit.
const CABLE_AUTOPAN_EDGE: f32 = 48.0;
const CABLE_AUTOPAN_SPEED: f32 = 600.0;

fn cable_autopan_velocity(pointer: Pos2, visible: Rect) -> Vec2 {
    if !pointer.is_finite()
        || !visible.min.is_finite()
        || !visible.max.is_finite()
        || !visible.is_positive()
        || !visible.contains(pointer)
    {
        return Vec2::ZERO;
    }
    let axis = |position: f32, lo: f32, hi: f32| {
        let edge = CABLE_AUTOPAN_EDGE.min((hi - lo) * 0.5);
        if edge <= 0.0 {
            return 0.0;
        }
        let ease = |distance: f32| {
            let t = (1.0 - distance / edge).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        // Half-viewport bounds keep opposing zones from overlapping, even in
        // a very small visible rack. Camera translation is inverse to travel.
        if position < lo + edge {
            ease(position - lo)
        } else if position > hi - edge {
            -ease(hi - position)
        } else {
            0.0
        }
    };
    let direction = vec2(
        axis(pointer.x, visible.left(), visible.right()),
        axis(pointer.y, visible.top(), visible.bottom()),
    );
    let length = direction.length();
    if length > 1.0 {
        direction / length * CABLE_AUTOPAN_SPEED
    } else {
        direction * CABLE_AUTOPAN_SPEED
    }
}

impl Rack {
    /// Call before painting modules and computing jack hit regions. Release
    /// frames retain the geometry the user last saw, so the eventual cable
    /// command keeps the ordinary target/revision/history path.
    fn cable_edge_autopan(&mut self, ui: &egui::Ui, canvas: Rect, enabled: bool) -> bool {
        if !enabled
            || !ui.is_enabled()
            || !self.shortcuts_enabled
            || !self.dragging_cable
            || self
                .pending
                .as_ref()
                .is_none_or(|p| p.revision != self.observed_revision)
            || self.editing.is_some()
            || self.precision_next.is_some()
            || self.annotation_edit.is_some()
            || self.annotation_input_guard
            || self.group_drag.is_some()
            || self.blank_resize.is_some()
            || egui::Popup::is_any_open(ui.ctx())
            || ui.ctx().memory(|memory| memory.top_modal_layer().is_some())
        {
            return false;
        }
        let pointer_and_dt = ui.input(|input| {
            if !input.focused
                || !input.pointer.primary_down()
                || input.pointer.button_released(PointerButton::Primary)
                || input.key_pressed(egui::Key::Escape)
                || input.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::PointerGone | egui::Event::WindowFocused(false)
                    )
                })
                || !input.stable_dt.is_finite()
            {
                return None;
            }
            input
                .pointer
                .hover_pos()
                .map(|pointer| (pointer, input.stable_dt.clamp(0.0, 0.05)))
        });
        let Some((pointer, dt)) = pointer_and_dt else {
            return false;
        };
        let visible = canvas.intersect(ui.clip_rect());
        if !visible.contains(pointer)
            || ui.ctx().layer_id_at(pointer) != Some(ui.layer_id())
            || !self.pan.is_finite()
        {
            return false;
        }
        let velocity = cable_autopan_velocity(pointer, visible);
        let pan = bounded_pan(self.pan + velocity * dt);
        if pan == self.pan {
            return false;
        }
        self.pan = pan;
        // Rack::show observes the pan delta and persists it as ordinary view
        // state; no module layout, graph, revision or history is changed here.
        ui.ctx().request_repaint();
        true
    }
}
