// Shared original CC/Gate assignment surface. Every instance and browser tile
// uses this geometry and renderer; activity and Learn are never inferred here.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MidiAssignmentPanel {
    Cc,
    Gate,
}
impl MidiAssignmentPanel {
    fn parameters(self) -> &'static [&'static str; 16] {
        match self {
            Self::Cc => &MIDI_CC_PARAMS,
            Self::Gate => &MIDI_GATE_PARAMS,
        }
    }
    fn port_label(self) -> &'static str {
        match self {
            Self::Cc => "OUT",
            Self::Gate => "GATE",
        }
    }
    fn edit_label(self) -> &'static str {
        match self {
            Self::Cc => "Edit CC assignment…",
            Self::Gate => "Edit note assignment…",
        }
    }
    fn learn_label(self) -> &'static str {
        match self {
            Self::Cc => "Learn MIDI CC",
            Self::Gate => "Learn MIDI note",
        }
    }
    fn learn_command(self) -> &'static str {
        match self {
            Self::Cc => "learn_midi_cc",
            Self::Gate => "learn_midi_gate",
        }
    }
    fn cancel_command(self) -> &'static str {
        match self {
            Self::Cc => "cancel_midi_cc_learn",
            Self::Gate => "cancel_midi_gate_learn",
        }
    }
    fn assignment_help(self) -> &'static str {
        match self {
        Self::Cc => "Click to enter CC −1 (unmapped) or 0–127. Tab visits all sixteen assignments. Right-click to Learn. Assigning an occupied CC transfers it atomically to this output.",
        Self::Gate => "Click to enter MIDI note −1 (unmapped) or 0–127; middle C is 60 / C4. Tab visits all sixteen notes. Right-click to Learn. A positive NoteOn learns and starts its gate on that event. Remapping preserves held cell state; use Panic to clear it.",
    }
    }
    fn initial_assignments(self, node: &Node) -> [i16; 16] {
        match self {
            Self::Cc => match openpatch_core::initial_midi_cc_state(node) {
                Ok(openpatch_core::SavedModuleState::MidiCc { assignments, .. }) => assignments,
                _ => std::array::from_fn(|i| (i + 1) as i16),
            },
            Self::Gate => match openpatch_core::initial_midi_gate_state(node) {
                Ok(openpatch_core::SavedModuleState::MidiGateMappings(assignments)) => assignments,
                _ => MIDI_GATE_DEFAULT_NOTES,
            },
        }
    }
}
fn midi_assignment_cell_rect(index: usize) -> Rect {
    Rect::from_min_size(
        pos2(
            20.0 + (index % 4) as f32 * 110.0,
            80.0 + (index / 4) as f32 * 55.0,
        ),
        vec2(107.0, 53.0),
    )
}
fn midi_assignment_value_rect(index: usize) -> Rect {
    Rect::from_min_size(
        midi_assignment_cell_rect(index).min + vec2(12.0, 19.0),
        vec2(62.0, 22.0),
    )
}
fn midi_assignment_output_position(index: usize) -> Pos2 {
    midi_assignment_cell_rect(index).min + vec2(94.0, 30.0)
}
fn midi_assignment_snapshot_valid(state: &Value) -> bool {
    state["assignments"].as_array().is_some_and(|v| {
        v.len() == 16
            && v.iter()
                .all(|n| n.as_i64().is_some_and(|n| (-1..=127).contains(&n)))
    }) && (state["learning"].is_null()
        || (state["learning"]["cell"]
            .as_u64()
            .is_some_and(|n| (1..=16).contains(&n))
            && state["learning"]["serial"].as_u64().is_some()))
}
fn midi_assignment_edit(
    node: &Node,
    index: usize,
    assignment: i16,
    revision: u64,
    family: MidiAssignmentPanel,
) -> ParameterEdit {
    ParameterEdit {
        id: node.id.clone(),
        kind: node.kind.clone(),
        parameter: family.parameters()[index].into(),
        revision,
        surface: ParameterEditSurface::Window,
        menu_popup: None,
        stale: false,
        text: assignment.to_string(),
        min: -1.0,
        max: 127.0,
        step: Some(1.0),
        display_scale: 1.0,
        display_unit: match family {
            MidiAssignmentPanel::Cc => "CC number (−1 = unmapped)",
            MidiAssignmentPanel::Gate => "MIDI note number (60 = C4; −1 = unmapped)",
        }
        .into(),
        slew_min: None,
        gain_power: None,
        raw_max: 127.0,
        focus: true,
    }
}
impl Rack {
    fn midi_assignment_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
        family: MidiAssignmentPanel,
    ) -> Option<RackAction> {
        let z = self.zoom;
        let p = |x, y| rect.min + vec2(x, y) * z;
        let screen = |r: Rect| Rect::from_min_max(p(r.min.x, r.min.y), p(r.max.x, r.max.y));
        let painter = ui.painter().clone();
        let ctx = ui.ctx().clone();
        let font = |size| crate::panel_skin::print_font(&ctx, size * z);
        let accent = crate::panel_skin::family_accent(&node.kind);
        let state = match family {
            MidiAssignmentPanel::Cc => self.midi_cc_states.get(&node.id),
            MidiAssignmentPanel::Gate => self.midi_gate_states.get(&node.id),
        }
        .cloned();
        let learning = state.as_ref().and_then(|s| s["learning"]["cell"].as_u64());
        // Valid saved arrays can contain collisions. Match the DSP's complete
        // last-cell-wins normalization when no live assignment snapshot exists.
        let fallback_assignments = if state.is_none() {
            family.initial_assignments(node)
        } else {
            [0; 16]
        };
        let mut action = None;
        for (index, name) in family.parameters().iter().enumerate() {
            let cell = midi_assignment_cell_rect(index);
            let assignment = state
                .as_ref()
                .and_then(|s| s["assignments"][index].as_i64())
                .map(|n| n as i16)
                .unwrap_or(fallback_assignments[index]);
            let armed = learning == Some((index + 1) as u64);
            painter.rect_filled(screen(cell), 3.0 * z, Color32::from_rgb(211, 212, 203));
            painter.rect_stroke(
                screen(cell),
                3.0 * z,
                Stroke::new(
                    if armed { 1.5 * z } else { 0.6 * z },
                    if armed {
                        accent
                    } else {
                        Color32::from_rgb(175, 179, 169)
                    },
                ),
                egui::StrokeKind::Inside,
            );
            painter.text(
                p(cell.left() + 8.0, cell.top() + 8.0),
                Align2::LEFT_CENTER,
                format!("{:02}", index + 1),
                font(9.0),
                accent,
            );
            painter.text(
                p(cell.left() + 94.0, cell.top() + 8.0),
                Align2::CENTER_CENTER,
                format!("{} {}", family.port_label(), index + 1),
                font(8.0),
                INK,
            );
            let field = screen(midi_assignment_value_rect(index));
            let label = if assignment < 0 {
                "—".into()
            } else {
                match family {
                    MidiAssignmentPanel::Cc => format!("CC {assignment}"),
                    MidiAssignmentPanel::Gate => {
                        format!("{assignment} {}", midi_gate_note_name(assignment))
                    }
                }
            };
            crate::panel_skin::readout(&painter, field.center(), &label, field.width(), z);
            let response = ui.interact(
                field,
                ui.id().with((&node.id, "midi-assignment", index)),
                Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    format!("Edit {} output {} assignment: {label}", node.id, index + 1),
                )
            });
            let edit = midi_assignment_edit(node, index, assignment, revision, family);
            if self.precision_collects(node) {
                self.register_precision_control(edit.clone(), field);
            }
            self.precision_affordance(ui, &response);
            if response.clicked() || self.take_precision_request(&node.id, name) {
                consume_parameter_activation(ui.ctx());
                self.select_primary(node.id.clone());
                self.open_precision_editor(edit.clone(), field);
            }
            response.context_menu(|ui| {
                ui.label(format!("Output {} · {label}", index + 1));
                if ui.button(family.edit_label()).clicked() {
                    self.open_precision_editor(edit.clone(), field);
                    ui.close();
                }
                if ui
                    .add_enabled(!node.bypassed, egui::Button::new(family.learn_label()))
                    .clicked()
                {
                    self.select_primary(node.id.clone());
                    action = Some(RackAction::Command(
                        family.learn_command(),
                        json!({"id":node.id,"cell":index+1,"expected_revision":revision}),
                    ));
                    ui.close();
                }
                if learning.is_some() && ui.button("Cancel MIDI Learn").clicked() {
                    action = Some(RackAction::Command(
                        family.cancel_command(),
                        json!({"id":node.id,"expected_revision":revision}),
                    ));
                    ui.close();
                }
            });
            response.on_hover_text(family.assignment_help());
            if family == MidiAssignmentPanel::Gate {
                self.midi_gate_activity_lamp(
                    &painter,
                    node,
                    index,
                    p(cell.left() + 8.0, cell.top() + 30.0),
                    z,
                );
            }
            if armed {
                painter.text(
                    p(cell.left() + 43.0, cell.top() + 46.0),
                    Align2::CENTER_CENTER,
                    "LRN",
                    font(8.0),
                    accent,
                );
            } else if family == MidiAssignmentPanel::Cc && assignment >= 0 {
                if let Some(value) = state
                    .as_ref()
                    .and_then(|s| s["values"][assignment as usize].as_u64())
                {
                    painter.text(
                        p(cell.left() + 43.0, cell.top() + 46.0),
                        Align2::CENTER_CENTER,
                        format!("VALUE {value}"),
                        font(8.0),
                        MUTED,
                    );
                }
            }
        }
        painter.line_segment(
            [p(20.0, 307.0), p(460.0, 307.0)],
            Stroke::new(0.6 * z, Color32::from_rgb(166, 164, 157)),
        );
        painter.text(
            p(20.0, 326.0),
            Align2::LEFT_CENTER,
            "CHANNEL",
            font(10.0),
            INK,
        );
        if let Some(RackAction::Command(command, mut args)) = self.choice_parameter(
            ui,
            node,
            "channel",
            &json!({"default":0}),
            screen(Rect::from_min_size(pos2(88.0, 315.0), vec2(136.0, 24.0))),
            &[
                "Omni · all channels",
                "Channel 1",
                "Channel 2",
                "Channel 3",
                "Channel 4",
                "Channel 5",
                "Channel 6",
                "Channel 7",
                "Channel 8",
                "Channel 9",
                "Channel 10",
                "Channel 11",
                "Channel 12",
                "Channel 13",
                "Channel 14",
                "Channel 15",
                "Channel 16",
            ],
        ) {
            args["expected_revision"] = json!(revision);
            action = Some(RackAction::Command(command, args));
        }
        painter.text(
            p(460.0, 326.0),
            Align2::RIGHT_CENTER,
            if family == MidiAssignmentPanel::Cc {
                "0–10 V  ·  16 MPE LANES"
            } else {
                "1 ms MIN GATE  ·  16 MPE LANES"
            },
            font(9.0),
            MUTED,
        );
        let options: &[(&str, &str, f32)] = match family {
            MidiAssignmentPanel::Cc => &[
                ("smooth", "Smooth", 1.0),
                ("mpe", "MPE", 0.0),
                ("high_resolution", "14-bit pairs", 0.0),
            ],
            MidiAssignmentPanel::Gate => &[("velocity", "Velocity", 0.0), ("mpe", "MPE", 0.0)],
        };
        for (i, &(name, label, default)) in options.iter().enumerate() {
            let r = screen(Rect::from_min_size(
                pos2(20.0 + i as f32 * 150.0, 346.0),
                vec2(138.0, 23.0),
            ));
            let mut on = node.params.get(name).copied().unwrap_or(default) >= 0.5;
            let response = ui
                .scope(|ui| {
                    crate::panel_skin::control_style(ui, z);
                    ui.style_mut().override_font_id = Some(font(12.0));
                    ui.spacing_mut().icon_width = 12.0 * z;
                    ui.spacing_mut().icon_spacing = 5.0 * z;
                    ui.spacing_mut().interact_size.y = 20.0 * z;
                    ui.put(r, egui::Checkbox::new(&mut on, label))
                })
                .inner;
            if response.changed() {
                action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":name,"value":if on {1.0} else {0.0},"expected_revision":revision}),
                ));
            }
        }
        if family == MidiAssignmentPanel::Gate {
            let response = ui
                .scope(|ui| {
                    crate::panel_skin::control_style(ui, z);
                    ui.style_mut().override_font_id = Some(font(12.0));
                    ui.put(screen(midi_gate_panic_rect()), egui::Button::new("Panic"))
                })
                .inner;
            if response.clicked() {
                action = Some(RackAction::Command(
                    "trigger_module",
                    json!({"id":node.id,"action":"midi_panic","expected_revision":revision}),
                ));
            }
            response.on_hover_text("Clear this module's gates and timers and cancel armed Learn. Available while bypassed. Remapping a held cell preserves its gate until released by its new note or cleared by Panic.");
        }
        painter.text(
            p(240.0, 376.0),
            Align2::CENTER_CENTER,
            "Global MIDI input · choose the device in Settings › MIDI",
            font(10.0),
            MUTED,
        );
        action
    }
}
