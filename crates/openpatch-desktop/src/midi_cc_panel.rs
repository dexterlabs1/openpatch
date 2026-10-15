// Original shared sixteen-cell MIDI controller panel. Learn state and controller
// readouts come only from coherent host snapshots, never local click guesses.
const MIDI_CC_PARAMS: [&str; 16] = [
    "cc1", "cc2", "cc3", "cc4", "cc5", "cc6", "cc7", "cc8", "cc9", "cc10", "cc11", "cc12", "cc13",
    "cc14", "cc15", "cc16",
];
fn midi_cc_cell_rect(index: usize) -> Rect {
    Rect::from_min_size(
        pos2(
            20.0 + (index % 4) as f32 * 110.0,
            80.0 + (index / 4) as f32 * 55.0,
        ),
        vec2(107.0, 53.0),
    )
}
fn midi_cc_value_rect(index: usize) -> Rect {
    let cell = midi_cc_cell_rect(index);
    Rect::from_min_size(cell.min + vec2(12.0, 19.0), vec2(62.0, 22.0))
}
fn midi_cc_output_position(index: usize) -> Pos2 {
    midi_cc_cell_rect(index).min + vec2(94.0, 30.0)
}
fn midi_cc_snapshot_valid(state: &Value) -> bool {
    state["assignments"].as_array().is_some_and(|v| {
        v.len() == 16
            && v.iter()
                .all(|n| n.as_i64().is_some_and(|n| (-1..=127).contains(&n)))
    }) && state["values"]
        .as_array()
        .is_some_and(|v| v.len() == 128 && v.iter().all(|n| n.as_u64().is_some_and(|n| n <= 127)))
        && (state["learning"].is_null()
            || (state["learning"]["cell"]
                .as_u64()
                .is_some_and(|n| (1..=16).contains(&n))
                && state["learning"]["serial"].as_u64().is_some()))
}
fn midi_cc_edit(node: &Node, index: usize, assignment: i16, revision: u64) -> ParameterEdit {
    ParameterEdit {
        id: node.id.clone(),
        kind: node.kind.clone(),
        parameter: MIDI_CC_PARAMS[index].into(),
        revision,
        surface: ParameterEditSurface::Window,
        menu_popup: None,
        stale: false,
        text: assignment.to_string(),
        min: -1.0,
        max: 127.0,
        step: Some(1.0),
        display_scale: 1.0,
        display_unit: "CC number (−1 = unmapped)".into(),
        slew_min: None,
        gain_power: None,
        raw_max: 127.0,
        focus: true,
    }
}
impl Rack {
    fn midi_cc_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
    ) -> Option<RackAction> {
        let z = self.zoom;
        let p = |x, y| rect.min + vec2(x, y) * z;
        let screen = |r: Rect| Rect::from_min_max(p(r.min.x, r.min.y), p(r.max.x, r.max.y));
        let painter = ui.painter().clone();
        let ctx = ui.ctx().clone();
        let font = |size| crate::panel_skin::print_font(&ctx, size * z);
        let accent = crate::panel_skin::family_accent("midi_cc");
        let state = self.midi_cc_states.get(&node.id).cloned();
        let learning = state.as_ref().and_then(|s| s["learning"]["cell"].as_u64());
        // Valid saved arrays can contain collisions. Match the DSP's complete
        // last-cell-wins normalization when no live assignment snapshot exists.
        let fallback_assignments = if state.is_none() {
            match openpatch_core::initial_midi_cc_state(node) {
                Ok(openpatch_core::SavedModuleState::MidiCc { assignments, .. }) => assignments,
                _ => std::array::from_fn(|i| (i + 1) as i16),
            }
        } else {
            [0; 16]
        };
        let mut action = None;
        for (index, name) in MIDI_CC_PARAMS.iter().enumerate() {
            let cell = midi_cc_cell_rect(index);
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
                format!("OUT {}", index + 1),
                font(8.0),
                INK,
            );
            let field = screen(midi_cc_value_rect(index));
            let label = if assignment < 0 {
                "—".into()
            } else {
                format!("CC {assignment}")
            };
            crate::panel_skin::readout(&painter, field.center(), &label, field.width(), z);
            let response = ui.interact(
                field,
                ui.id().with((&node.id, "midi-cc-assignment", index)),
                Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    format!("Edit {} output {} assignment: {label}", node.id, index + 1),
                )
            });
            let edit = midi_cc_edit(node, index, assignment, revision);
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
                if ui.button("Edit CC assignment…").clicked() {
                    self.open_precision_editor(edit.clone(), field);
                    ui.close();
                }
                if ui
                    .add_enabled(!node.bypassed, egui::Button::new("Learn MIDI CC"))
                    .clicked()
                {
                    self.select_primary(node.id.clone());
                    action = Some(RackAction::Command(
                        "learn_midi_cc",
                        json!({"id":node.id,"cell":index+1,"expected_revision":revision}),
                    ));
                    ui.close();
                }
                if learning.is_some() && ui.button("Cancel MIDI Learn").clicked() {
                    action = Some(RackAction::Command(
                        "cancel_midi_cc_learn",
                        json!({"id":node.id,"expected_revision":revision}),
                    ));
                    ui.close();
                }
            });
            response.on_hover_text("Click to enter CC −1 (unmapped) or 0–127. Tab visits all sixteen assignments. Right-click to Learn. Assigning an occupied CC transfers it atomically to this output.");
            if armed {
                painter.text(
                    p(cell.left() + 43.0, cell.top() + 46.0),
                    Align2::CENTER_CENTER,
                    "LRN",
                    font(8.0),
                    accent,
                );
            } else if assignment >= 0 {
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
            "0–10 V  ·  16 MPE LANES",
            font(9.0),
            MUTED,
        );
        for (i, (name, label, default)) in [
            ("smooth", "Smooth", 1.0),
            ("mpe", "MPE", 0.0),
            ("high_resolution", "14-bit pairs", 0.0),
        ]
        .into_iter()
        .enumerate()
        {
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
        painter.text(
            p(240.0, 376.0),
            Align2::CENTER_CENTER,
            "Global MIDI input · select endpoint in MIDI settings",
            font(10.0),
            MUTED,
        );
        action
    }
}
