// Shared fader input. Pointer state is local UI state, never a graph mutation.
#[derive(Default)]
struct FaderInput {
    normalized: Option<f32>,
    nudge: i32,
    boundary: Option<bool>,
    reset: bool,
    history_group: Option<String>,
}

#[derive(Clone, Copy)]
enum ParameterPointer {
    Fader(Rect),
    Knob { travel: f32 },
}

impl Rack {
    pub(crate) fn parameter_keyboard_active(&self, ctx: &egui::Context) -> bool {
        self.parameter_focus
            .is_some_and(|id| ctx.memory(|memory| memory.focused()) == Some(id))
    }

    pub(crate) fn prepare_parameter_history(&mut self, ctx: &egui::Context) {
        // A release in this frame must not replay the pre-undo cached value.
        self.parameter_history_frame = Some(ctx.cumulative_frame_nr());
        self.knob_value_bubble = None;
    }

    fn parameter_fader_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        track: Rect,
        normalized: f32,
        enabled: bool,
    ) -> FaderInput {
        self.parameter_pointer_input(
            ui,
            response,
            ParameterPointer::Fader(track),
            normalized,
            enabled,
        )
    }

    fn parameter_knob_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        normalized: f32,
        enabled: bool,
        display_value: &str,
    ) -> FaderInput {
        let input = self.parameter_pointer_input(
            ui,
            response,
            ParameterPointer::Knob {
                travel: 150.0 * self.zoom,
            },
            normalized,
            enabled,
        );
        self.observe_knob_value_bubble(ui, response, &input, enabled, display_value);
        input
    }

    fn parameter_pointer_input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        pointer_mode: ParameterPointer,
        normalized: f32,
        enabled: bool,
    ) -> FaderInput {
        let mut result = FaderInput::default();
        let rotary = matches!(pointer_mode, ParameterPointer::Knob { .. });
        let travel = match pointer_mode {
            ParameterPointer::Fader(track) => track.height(),
            ParameterPointer::Knob { travel } => travel,
        };
        let id = response.id;
        let relative_id = id.with("relative-after-fine");
        if enabled && response.has_focus() {
            self.parameter_focus = Some(id);
        } else if self.parameter_focus == Some(id) {
            self.parameter_focus = None;
        }
        let interrupted = !enabled
            || !ui.input(|i| i.focused)
            || ui.input(|i| i.key_pressed(egui::Key::Escape))
            || self.parameter_history_frame == Some(ui.ctx().cumulative_frame_nr());
        if interrupted {
            ui.data_mut(|d| {
                d.remove::<f32>(id);
                d.remove::<String>(id);
                d.remove::<bool>(relative_id);
            });
            return result;
        }
        let pressed = response.is_pointer_button_down_on()
            && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary));
        if pressed || response.drag_started_by(PointerButton::Primary) {
            response.request_focus();
            self.parameter_focus = Some(id);
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
            || (!rotary && response.clicked_by(PointerButton::Primary))
        {
            if let Some(pointer) = response
                .interact_pointer_pos()
                .or_else(|| ui.input(|i| i.pointer.interact_pos()))
            {
                let fine = (modifiers.command || modifiers.ctrl)
                    && !response.clicked_by(PointerButton::Primary);
                // Once fine adjustment introduces an offset from the pointer,
                // retain it until release. Returning to full speed must not
                // snap the value back to the absolute cursor position.
                let relative =
                    rotary || fine || ui.data(|d| d.get_temp::<bool>(relative_id).unwrap_or(false));
                let next = if relative {
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
                    prior - response.drag_motion().y / travel * speed
                } else {
                    let ParameterPointer::Fader(track) = pointer_mode else {
                        unreachable!()
                    };
                    (track.bottom() - pointer.y) / track.height()
                }
                .clamp(0.0, 1.0);
                ui.data_mut(|d| {
                    d.insert_temp(id, next);
                    d.insert_temp(relative_id, relative);
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
