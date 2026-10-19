// Shared Studio rotary input. Pointer state is local UI state, never a graph mutation.
#[derive(Default)]
struct FaderInput {
    normalized: Option<f32>,
    nudge: i32,
    boundary: Option<bool>,
    reset: bool,
    history_group: Option<String>,
}
impl Rack {
    fn parameter_rotary_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        track: Rect,
        normalized: f32,
        enabled: bool,
    ) -> FaderInput {
        self.parameter_control_input(ui, response, track, normalized, enabled, false)
    }
    fn parameter_strip_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        track: Rect,
        normalized: f32,
        enabled: bool,
    ) -> FaderInput {
        self.parameter_control_input(ui, response, track, normalized, enabled, true)
    }
    #[allow(clippy::too_many_arguments)]
    fn parameter_control_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        track: Rect,
        normalized: f32,
        enabled: bool,
        strip: bool,
    ) -> FaderInput {
        let mut result = FaderInput::default();
        let id = response.id;
        let relative_id = id.with("relative-after-fine");
        let interrupted =
            !enabled || !ui.input(|i| i.focused) || ui.input(|i| i.key_pressed(egui::Key::Escape));
        if interrupted {
            ui.data_mut(|d| {
                d.remove::<f32>(id);
                d.remove::<String>(id);
                d.remove::<bool>(relative_id);
            });
            return result;
        }
        let completed_motion = crate::panel_skin::completed_pointer_drag(ui, response);
        let pressed = response.is_pointer_button_down_on()
            && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary));
        if pressed || response.drag_started_by(PointerButton::Primary) || completed_motion.is_some() {
            response.request_focus();
            if ui.data(|d| d.get_temp::<String>(id)).is_none() {
                self.gesture = self.gesture.wrapping_add(1);
                ui.data_mut(|d| {
                    d.insert_temp(id, format!("rack-fader-{}", self.gesture));
                    d.insert_temp(id, normalized);
                    d.insert_temp(relative_id, false);
                });
            }
        }
        let modifiers = ui.input(|i| i.modifiers);
        if response.dragged_by(PointerButton::Primary)
            || response.drag_stopped_by(PointerButton::Primary)
            || completed_motion.is_some()
        {
            if let Some(_pointer) = response
                .interact_pointer_pos()
                .or_else(|| ui.input(|i| i.pointer.interact_pos()))
            {
                let fine = modifiers.command || modifiers.ctrl;
                let next = {
                    let prior = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(normalized);
                    let speed = if fine {
                        if modifiers.shift {
                            0.01
                        } else {
                            0.1
                        }
                    } else {
                        1.0
                    };
                    // Follow screen points: native integrations may provide a zero raw
                    // mouse delta alongside real PointerMoved events.
                    let motion = completed_motion.unwrap_or_else(|| response.drag_delta());
                    if strip {
                        prior - motion.y / track.height().max(self.zoom) * speed
                    } else {
                        prior + (motion.x * 0.35 - motion.y) / (160.0 * self.zoom) * speed
                    }
                }
                .clamp(0.0, 1.0);
                ui.data_mut(|d| {
                    d.insert_temp(id, next);
                    d.insert_temp(relative_id, true);
                });
                result.normalized = Some(next);
                result.history_group = ui.data(|d| d.get_temp::<String>(id));
            }
        }
        result.reset = response.double_clicked_by(PointerButton::Primary);
        if response.clicked_by(PointerButton::Primary) {
            // Coalesce the first click's position change with a double-click
            // reset, so one undo restores the value before either click.
            let last = id.with("last-click-history");
            if result.reset {
                result.history_group = ui
                    .data(|d| d.get_temp::<String>(last))
                    .or(result.history_group);
            }
            if let Some(group) = &result.history_group {
                ui.data_mut(|d| d.insert_temp(last, group.clone()));
            }
        }
        if response.has_focus() {
            ui.ctx().memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        vertical_arrows: true,
                        ..Default::default()
                    },
                )
            });
            result.nudge = ui.input_mut(|i| {
                i32::from(i.consume_key(modifiers, egui::Key::ArrowUp))
                    - i32::from(i.consume_key(modifiers, egui::Key::ArrowDown))
            });
            result.boundary = ui.input_mut(|i| {
                if i.consume_key(modifiers, egui::Key::Home) {
                    Some(false)
                } else if i.consume_key(modifiers, egui::Key::End) {
                    Some(true)
                } else {
                    None
                }
            });
        }
        if response.drag_stopped_by(PointerButton::Primary)
            || ui.input(|i| i.pointer.button_released(PointerButton::Primary))
        {
            ui.data_mut(|d| {
                d.remove::<f32>(id);
                d.remove::<String>(id);
                d.remove::<bool>(relative_id);
            });
        }
        result
    }
}
