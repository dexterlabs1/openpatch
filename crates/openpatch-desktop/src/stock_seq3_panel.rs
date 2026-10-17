// Stock SEQ3 keeps source voltage/clock semantics separate from the legacy SEQ3.
const STOCK_SEQ3_GATE_PRESS: [&str; 8] = [
    "seq_gate1_press",
    "seq_gate2_press",
    "seq_gate3_press",
    "seq_gate4_press",
    "seq_gate5_press",
    "seq_gate6_press",
    "seq_gate7_press",
    "seq_gate8_press",
];
const STOCK_SEQ3_GATE_RELEASE: [&str; 8] = [
    "seq_gate1_release",
    "seq_gate2_release",
    "seq_gate3_release",
    "seq_gate4_release",
    "seq_gate5_release",
    "seq_gate6_release",
    "seq_gate7_release",
    "seq_gate8_release",
];
fn stock_seq3_scalar(name: &str) -> bool {
    sequence_cell(name).is_some()
        || matches!(name, "tempo" | "steps" | "tempo_amount" | "steps_amount")
}
fn stock_seq3_parameter_position(name: &str) -> Pos2 {
    match name {
        "tempo" => pos2(62.0, 118.0),
        "steps" => pos2(153.0, 118.0),
        "tempo_amount" => pos2(62.0, 214.0),
        _ => pos2(153.0, 214.0),
    }
}
fn stock_seq3_parameter_label(name: &str) -> &str {
    match name {
        "tempo" => "TEMPO",
        "steps" => "STEPS",
        "tempo_amount" => "TEMPO CV",
        "steps_amount" => "STEPS CV",
        other => other,
    }
}
fn stock_seq3_tempo_value(raw: f32) -> String {
    format!("{:.2} BPM", 60.0 * raw.exp2())
}
impl Rack {
    #[allow(clippy::too_many_arguments)]
    fn stock_seq3_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
        allow_press: bool,
        interactive: bool,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let painter = ui.painter().clone();
        let packet = self.module_lights.get(&node.id).copied().filter(|s| {
            s.count == 19 && s.brightness[..19].iter().flatten().all(|v| v.is_finite())
        });
        let controls = self.module_controls.get(&node.id).copied();
        let actual = self.stock_seq3_states.get(&node.id).cloned();
        let enabled = interactive
            && ui.is_enabled()
            && self.performance_available
            && !node.bypassed
            && self.editing.is_none()
            && self.annotation_edit.is_none()
            && self.blank_resize.is_none()
            && self.shortcuts_enabled
            && ui.input(|i| i.focused);
        let mut action = None;
        painter.line_segment(
            [point(207.0, 64.0), point(207.0, 388.0)],
            Stroke::new(0.8 * zoom, Color32::from_rgb(172, 163, 143)),
        );
        painter.line_segment(
            [point(208.0, 64.0), point(208.0, 388.0)],
            Stroke::new(0.6 * zoom, Color32::from_rgb(245, 241, 227)),
        );
        for row in 0..3 {
            painter.text(
                point(206.0, 112.0 + row as f32 * 72.0),
                Align2::RIGHT_CENTER,
                format!("CV{}", row + 1),
                crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
                MUTED,
            );
        }
        // These are measured source lamps. No current-step or saved-param value
        // substitutes for a missing packet (including the library preview).
        for column in 0..8 {
            let x = 238.0 + column as f32 * 67.0;
            for (component, dx, color) in [
                (0, -8.0, Color32::from_rgb(242, 204, 70)),
                (1, 8.0, Color32::from_rgb(239, 72, 61)),
            ] {
                let strength = packet.map_or(0.0, |s| {
                    s.brightness[11 + column][component].clamp(0.0, 1.0)
                });
                self.lighting.circle(
                    &painter,
                    point(x + dx, 54.0),
                    3.2 * zoom,
                    if packet.is_some() {
                        color.gamma_multiply(0.12 + strength * 0.88)
                    } else {
                        Color32::from_rgb(99, 105, 99)
                    },
                    strength,
                    0.12,
                );
            }
            ui.interact(Rect::from_center_size(point(x,54.0),vec2(31.0,15.0)*zoom),ui.id().with((&node.id,"seq-step-status",column)),Sense::hover())
                .on_hover_text("Observed step status: yellow is selected step; red is outside the engine's current length. No packet means unavailable; knob voltages never substitute for these lights.");
        }
        for index in 0..11 {
            let (label, parameter, press, release, button_rect) = if index < 3 {
                let (label, parameter, press, release) = [
                    ("Clock", "", "clock_press", "clock_release"),
                    ("Run", "running", "run_press", "run_release"),
                    ("Reset", "", "reset_press", "reset_release"),
                ][index];
                (
                    label.to_owned(),
                    parameter.to_owned(),
                    press,
                    release,
                    Rect::from_min_size(
                        point(20.0 + index as f32 * 60.0, 278.0),
                        vec2(55.0, 25.0) * zoom,
                    ),
                )
            } else {
                let column = index - 3;
                (
                    (column + 1).to_string(),
                    format!("gate{}", column + 1),
                    STOCK_SEQ3_GATE_PRESS[column],
                    STOCK_SEQ3_GATE_RELEASE[column],
                    Rect::from_center_size(
                        point(238.0 + column as f32 * 67.0, 73.0),
                        vec2(48.0, 18.0) * zoom,
                    ),
                )
            };
            let response = ui.interact(
                button_rect,
                ui.id().with((&node.id, "stock-seq3-held", index)),
                Sense::click_and_drag(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    enabled,
                    format!("{} {label} momentary", node.id),
                )
            });
            let held = controls.is_some_and(|s| s.pressed_mask & (1 << index) != 0);
            painter.rect_filled(
                button_rect.translate(vec2(0.0, 1.0) * zoom),
                2.0 * zoom,
                Color32::from_rgb(150, 141, 122),
            );
            painter.rect_filled(
                button_rect,
                2.0 * zoom,
                if held {
                    Color32::from_rgb(218, 190, 138)
                } else if enabled && response.hovered() {
                    Color32::from_rgb(246, 241, 227)
                } else {
                    Color32::from_rgb(233, 226, 207)
                },
            );
            painter.rect_stroke(
                button_rect,
                2.0 * zoom,
                Stroke::new(0.7 * zoom, Color32::from_rgb(142, 131, 108)),
                egui::StrokeKind::Inside,
            );
            painter.line_segment(
                [
                    button_rect.left_top() + vec2(2.0, 1.5) * zoom,
                    button_rect.right_top() + vec2(-2.0, 1.5) * zoom,
                ],
                Stroke::new(
                    0.6 * zoom,
                    if held {
                        Color32::from_rgb(185, 158, 109)
                    } else {
                        Color32::from_rgb(255, 251, 235)
                    },
                ),
            );
            painter.text(
                button_rect.center() - vec2(6.0, 0.0) * zoom,
                Align2::CENTER_CENTER,
                &label,
                crate::panel_skin::print_font(ui.ctx(), 10.0 * zoom),
                if enabled { INK } else { MUTED },
            );
            let strength = packet.map_or(0.0, |s| s.brightness[index][0].clamp(0.0, 1.0));
            self.lighting.circle(
                &painter,
                button_rect.right_center() - vec2(8.0, 0.0) * zoom,
                3.0 * zoom,
                if packet.is_some() {
                    Color32::from_rgb(86, 201, 119).gamma_multiply(0.12 + strength * 0.88)
                } else {
                    Color32::from_rgb(99, 105, 99)
                },
                strength,
                0.12,
            );
            if interactive && !parameter.is_empty() {
                if let Some(cue) = self
                    .construction_animation
                    .parameter(&node.id, &node.kind, &parameter)
                {
                    let text = |v| {
                        if v >= 0.5 {
                            "On".to_owned()
                        } else {
                            "Off".to_owned()
                        }
                    };
                    self.register_construction_anchor(
                        ui,
                        node,
                        &parameter,
                        crate::construction_motion::Anchor {
                            rect: button_rect,
                            old_text: text(cue.old),
                            new_text: text(cue.new),
                            marker: None,
                        },
                        &[response.id],
                    );
                }
            }
            if enabled
                && allow_press
                && self.held_manual.is_none()
                && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
            {
                if response.clicked_by(PointerButton::Primary) {
                    self.select_primary(node.id.clone());
                    action = Some(RackAction::ManualClick {
                        id: node.id.clone(),
                        press,
                        release,
                    });
                } else if response.is_pointer_button_down_on() {
                    self.held_manual = Some(HeldManual {
                        id: node.id.clone(),
                        kind: node.kind.clone(),
                        release,
                        button_rect,
                    });
                    self.select_primary(node.id.clone());
                    action = Some(RackAction::Command(
                        "trigger_module",
                        json!({"id":node.id,"action":press,"expected_revision":revision}),
                    ));
                }
            }
            response.on_hover_text(if enabled { "Hold to press the source button; release anywhere to release. Lamps follow the audio engine." } else { "Momentary control requires a live audio engine and an active module. Lamps are unavailable in previews." });
        }
        for (input, ports) in [
            (true, &["tempo", "clock", "run", "reset", "steps"][..]),
            (
                false,
                &[
                    "cv1", "cv2", "cv3", "trigger", "step1", "step2", "step3", "step4", "step5",
                    "step6", "step7", "step8", "steps", "clock", "run", "reset",
                ][..],
            ),
        ] {
            for (index, port) in ports.iter().enumerate() {
                let position = port_position(
                    "stock_seq3",
                    input,
                    index,
                    ports.len(),
                    panel_width("stock_seq3"),
                );
                let lower = !input && (4..12).contains(&index);
                let center = point(position.x, position.y - if lower { 20.5 } else { 25.0 });
                let label = port_label(port).to_uppercase();
                let color = if input {
                    INK
                } else {
                    Color32::from_rgb(245, 242, 233)
                };
                let galley = painter.layout_no_wrap(
                    label,
                    crate::panel_skin::print_font(ui.ctx(), if lower { 9.5 } else { 10.5 } * zoom),
                    color,
                );
                if !input {
                    painter.rect_filled(
                        Rect::from_center_size(center, galley.size() + vec2(6.0, 1.0) * zoom),
                        1.5 * zoom,
                        INK,
                    );
                }
                painter.galley(center - galley.size() * 0.5, galley, color);
            }
        }
        let display = actual.as_ref().and_then(|s| s["passthrough"].as_bool());
        let old = display
            .unwrap_or_else(|| node.params.get("passthrough").copied().unwrap_or(0.0) >= 0.5);
        let mut value = old;
        let control = Rect::from_min_size(point(20.0, 311.0), vec2(175.0, 24.0) * zoom);
        let response = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(control)
                    .id_salt((&node.id, "stock-seq3-mode")),
                |ui| {
                    *ui.visuals_mut() = egui::Visuals::light();
                    crate::panel_skin::control_style(ui, zoom);
                    ui.style_mut().override_font_id =
                        Some(crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom));
                    let spacing = ui.spacing_mut();
                    spacing.button_padding = vec2(6.0, 3.0) * zoom;
                    spacing.icon_width = 14.0 * zoom;
                    spacing.icon_width_inner = 8.0 * zoom;
                    spacing.icon_spacing = 4.0 * zoom;
                    spacing.interact_size = vec2(40.0, 18.0) * zoom;
                    spacing.item_spacing = vec2(8.0, 3.0) * zoom;
                    egui::ComboBox::from_id_salt("passthrough")
                        .width(control.width())
                        .selected_text(if old {
                            "Gate passthrough"
                        } else {
                            "Trigger pulses"
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut value, false, "Trigger pulses");
                            ui.selectable_value(&mut value, true, "Gate passthrough");
                        })
                        .response
                },
            )
            .inner;
        let opening_revision_id = response.id.with("opening-revision");
        if interactive && response.clicked() {
            ui.ctx()
                .data_mut(|data| data.insert_temp(opening_revision_id, revision));
        }
        if interactive {
            if let Some(cue) =
                self.construction_animation
                    .parameter(&node.id, &node.kind, "passthrough")
            {
                let label = |v| {
                    if v >= 0.5 {
                        "Gate passthrough".to_owned()
                    } else {
                        "Trigger pulses".to_owned()
                    }
                };
                self.register_construction_anchor(
                    ui,
                    node,
                    "passthrough",
                    crate::construction_motion::Anchor {
                        rect: response.rect,
                        old_text: label(cue.old),
                        new_text: label(cue.new),
                        marker: None,
                    },
                    &[response.id],
                );
            }
            if value != old {
                let opening_revision = ui
                    .ctx()
                    .data(|data| data.get_temp::<u64>(opening_revision_id))
                    .unwrap_or(revision);
                self.select_primary(node.id.clone());
                action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":"passthrough","value":if value {1.0} else {0.0},"expected_revision":opening_revision}),
                ));
            }
        }
        for (index, (label, direction)) in [("Rotate ←", "left"), ("Rotate →", "right")]
            .iter()
            .enumerate()
        {
            if panel_button(
                ui,
                &painter,
                Rect::from_min_size(
                    point(20.0 + index as f32 * 91.0, 334.0),
                    vec2(84.0, 19.0) * zoom,
                ),
                ui.id().with((&node.id, "stock-seq3-rotate", direction)),
                label,
                false,
                // This configured operation shares its normal metal in both
                // surfaces. Preview's disabled Ui blocks interaction; the
                // render-mode guard also prevents emitting a command.
                true,
                zoom,
            ) && interactive
            {
                action = Some(RackAction::Command(
                    "rotate_sequence",
                    json!({"id":node.id,"direction":direction,"expected_revision":revision}),
                ));
            }
        }
        painter.text(
            point(107.0, 266.0),
            Align2::CENTER_CENTER,
            if node.bypassed {
                "BYPASSED / FROZEN"
            } else if packet.is_none() {
                "TELEMETRY —"
            } else {
                "ENGINE SNAPSHOT"
            },
            crate::panel_skin::print_font(ui.ctx(), 9.0 * zoom),
            MUTED,
        );
        action
    }
}
