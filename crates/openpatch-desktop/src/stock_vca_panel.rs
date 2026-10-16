// Shared rack/browser faceplate. Only a coherent DSP snapshot illuminates gains.
const STOCK_VCA_PARAMS: [&str; 2] = ["level", "response"];
const STOCK_VCA_KNOB_BODY: f32 = 27.0;
fn stock_vca_knob_rect() -> Rect {
    Rect::from_center_size(pos2(40.0, 202.0), Vec2::splat(60.0))
}
fn stock_vca_value_rect() -> Rect {
    Rect::from_center_size(pos2(40.0, 295.0), vec2(54.0, 22.0))
}
fn stock_vca_mode_rect() -> Rect {
    Rect::from_min_max(pos2(77.0, 284.0), pos2(160.0, 306.0))
}
fn stock_vca_gain_rect(channel: usize) -> Rect {
    Rect::from_min_max(
        pos2(77.0 + channel as f32 * 5.0, 117.0),
        pos2(80.5 + channel as f32 * 5.0, 273.0),
    )
}
fn stock_vca_edit(node: &Node, index: usize, revision: u64) -> ParameterEdit {
    let level = index == 0;
    let raw = node
        .params
        .get(STOCK_VCA_PARAMS[index])
        .copied()
        .unwrap_or(1.0);
    ParameterEdit {
        id: node.id.clone(),
        kind: node.kind.clone(),
        parameter: STOCK_VCA_PARAMS[index].into(),
        revision,
        surface: ParameterEditSurface::Window,
        menu_popup: None,
        stale: false,
        text: if level {
            format!("{:.2}", raw * 100.0)
        } else {
            format!("{raw:.0}")
        },
        min: 0.0,
        max: if level { 100.0 } else { 1.0 },
        step: if level { None } else { Some(1.0) },
        display_scale: if level { 100.0 } else { 1.0 },
        display_unit: if level { "%" } else { "" }.into(),
        slew_min: None,
        gain_power: None,
        raw_max: 1.0,
        focus: true,
    }
}
fn stock_vca_port_label(port: &str) -> &str {
    match port {
        "cv" => "CV",
        "signal" => "IN",
        "out" => "OUT",
        other => other,
    }
}
impl Rack {
    fn stock_vca_panel(
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
        let font = |size| crate::panel_skin::print_font(ui.ctx(), size * z);
        let accent = Color32::from_rgb(86, 181, 137);
        let level = node
            .params
            .get("level")
            .copied()
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        let response_mode = node.params.get("response").copied().unwrap_or(1.0);
        let mut action = None;
        painter.text(
            p(40.0, 158.0),
            Align2::CENTER_CENTER,
            "LEVEL",
            font(10.0),
            INK,
        );
        painter.text(p(40.0, 248.0), Align2::CENTER_CENTER, "0–100%", font(8.0), MUTED);
        painter.text(
            p(118.0, 91.0),
            Align2::CENTER_CENTER,
            "GAIN / VOICE",
            font(9.0),
            INK,
        );
        painter.text(
            p(118.0, 107.0),
            Align2::CENTER_CENTER,
            "0 — 100%",
            font(8.0),
            MUTED,
        );

        // Global node+parameter identity survives graph reorder and culling.
        let knob_rect = screen(stock_vca_knob_rect());
        let knob = ui.interact(
            knob_rect,
            egui::Id::new(("stock-vca-knob", &node.id, "level")),
            Sense::click_and_drag(),
        );
        knob.widget_info(|| {
            egui::WidgetInfo::slider(
                ui.is_enabled(),
                f64::from(level),
                format!("{} level", node.id),
            )
        });
        let input_enabled = ui.is_enabled()
            && self.shortcuts_enabled
            && self.editing.is_none()
            && self.annotation_edit.is_none()
            && !egui::Popup::is_any_open(ui.ctx());
        let input = self.parameter_knob_input(ui, &knob, level, input_enabled);
        if input_enabled
            && (knob.is_pointer_button_down_on()
                || knob.clicked_by(PointerButton::Primary)
                || knob.drag_started_by(PointerButton::Primary))
        {
            self.select_primary(node.id.clone());
        }
        let keyboard_step = ui.input(|i| {
            if i.modifiers.command || i.modifiers.ctrl {
                if i.modifiers.shift { 0.0001 } else { 0.001 }
            } else if i.modifiers.shift {
                0.1
            } else {
                0.01
            }
        });
        let next = if input.reset {
            Some(1.0)
        } else if let Some(high) = input.boundary {
            Some(if high { 1.0 } else { 0.0 })
        } else if input.nudge != 0 {
            Some((level + input.nudge as f32 * keyboard_step).clamp(0.0, 1.0))
        } else {
            input.normalized
        };
        if let Some(value) = next.filter(|value| *value != level) {
            let mut args = json!({"id":node.id,"parameter":"level","value":value,"expected_revision":revision});
            if let Some(group) = input.history_group {
                args["history_group"] = json!(group);
            }
            action = Some(RackAction::Command("set_parameter", args));
        }
        if input_enabled && knob.secondary_clicked() {
            self.open_precision_editor(
                stock_vca_edit(node, 0, revision),
                screen(stock_vca_value_rect()),
            );
        }
        knob.clone().on_hover_cursor(egui::CursorIcon::ResizeVertical).on_hover_text("Saved level 0–100%. Drag vertically; Ctrl/Cmd for fine adjustment. Arrow keys adjust, Home/End choose limits. Double-click resets to 100%. Right-click or click the readout for exact entry. The adjacent columns show actual gain, not audio amplitude.");
        crate::panel_skin::signal_atlas_rotary(
            &painter,
            knob_rect.center(),
            level,
            knob.hovered() || knob.dragged() || knob.has_focus(),
            STOCK_VCA_KNOB_BODY,
            z,
        );

        let state = self
            .module_lights
            .get(&node.id)
            .filter(|s| s.count == 16 && s.input_channels.is_some_and(|c| (1..=16).contains(&c)));
        for channel in 0..16 {
            let lane = screen(stock_vca_gain_rect(channel));
            let active = state.is_some_and(|s| channel < usize::from(s.input_channels.unwrap()));
            painter.rect_filled(lane, 1.0 * z, Color32::from_rgb(46, 57, 53));
            if active {
                painter.rect_stroke(
                    lane,
                    1.0 * z,
                    Stroke::new(0.5 * z, Color32::from_rgb(122, 140, 125)),
                    egui::StrokeKind::Inside,
                );
                let gain = state.unwrap().brightness[channel][0];
                let gain = if gain.is_finite() {
                    gain.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                if gain > 0.0 {
                    let fill = Rect::from_min_max(
                        pos2(lane.left(), lane.bottom() - lane.height() * gain),
                        lane.max,
                    );
                    self.lighting
                        .rect(&painter, fill, 0.7 * z, accent, gain, 0.0);
                }
            }
        }
        if state.is_none() {
            // Outline is explicitly a configuration reference, never a live meter.
            let y = 273.0 - level * 156.0;
            painter.rect_stroke(
                Rect::from_min_max(p(75.0, y - 2.0), p(158.0, y + 2.0)),
                0.0,
                Stroke::new(0.8 * z, MUTED),
                egui::StrokeKind::Inside,
            );
        }
        let status = match (state, node.bypassed) {
            (None, true) => "BYPASS · — OFFLINE".into(),
            (None, false) => "— TELEMETRY OFFLINE".into(),
            (Some(_), true) => "BYPASS · GAIN FROZEN".into(),
            (Some(state), false) => format!("{} ACTIVE VOICES", state.input_channels.unwrap()),
        };
        painter.text(
            p(90.0, 323.0),
            Align2::CENTER_CENTER,
            status,
            font(8.0),
            MUTED,
        );
        ui.interact(screen(Rect::from_min_max(pos2(72.0, 111.0), pos2(162.0, 276.0))), egui::Id::new(("stock-vca", &node.id, "gain")), Sense::hover())
            .on_hover_text("Each column shows actual effective gain, not audio amplitude. Silent audio can have full gain. CV is clamped to 0–10 V; EXP raises normalized CV to the fourth power. Bypass passes raw signal and freezes this meter. Offline: no live gain; the outline marks configured level only.");

        let value_rect = screen(stock_vca_value_rect());
        let label = format!("{:.0}%", level * 100.0);
        crate::panel_skin::readout(&painter, value_rect.center(), &label, value_rect.width(), z);
        let value = ui.interact(
            value_rect,
            egui::Id::new(("stock-vca", &node.id, "level-value")),
            Sense::click(),
        );
        value.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                format!("Edit {} level: {label}", node.id),
            )
        });
        if self.precision_collects(node) {
            self.register_precision_control(stock_vca_edit(node, 0, revision), value_rect);
        }
        self.precision_affordance(ui, &value);
        if value.clicked()
            || value.secondary_clicked()
            || self.take_precision_request(&node.id, "level")
        {
            consume_parameter_activation(ui.ctx());
            self.open_precision_editor(stock_vca_edit(node, 0, revision), value_rect);
        }
        if let Some(cue) = self.construction_animation.parameter(&node.id, &node.kind, "level").cloned() {
            self.register_construction_anchor(
                ui,
                node,
                "level",
                crate::construction_motion::Anchor {
                    rect: knob_rect.union(value_rect),
                    old_text: format!("{:.0}%", cue.old * 100.0),
                    new_text: format!("{:.0}%", cue.new * 100.0),
                    marker: Some(crate::construction_motion::Marker::Rotary {
                        center: knob_rect.center(),
                        radius: 30.0 * z,
                        old: cue.old.clamp(0.0, 1.0),
                        new: cue.new.clamp(0.0, 1.0),
                    }),
                },
                &[knob.id, value.id],
            );
        }
        value.on_hover_text(
            "Click for exact level in percent or expressions. Tab applies and moves to response.",
        );
        let mode_rect = screen(stock_vca_mode_rect());
        for (index, text) in ["EXP", "LIN"].iter().enumerate() {
            let half = Rect::from_min_max(
                pos2(
                    mode_rect.left() + index as f32 * mode_rect.width() / 2.0,
                    mode_rect.top(),
                ),
                pos2(
                    mode_rect.left() + (index + 1) as f32 * mode_rect.width() / 2.0,
                    mode_rect.bottom(),
                ),
            );
            let selected = response_mode == index as f32;
            painter.rect_filled(
                half,
                2.0 * z,
                if selected {
                    Color32::from_rgb(75, 110, 91)
                } else {
                    Color32::from_rgb(212, 212, 201)
                },
            );
            painter.text(
                half.center(),
                Align2::CENTER_CENTER,
                text,
                font(9.0),
                if selected {
                    Color32::from_rgb(243, 246, 232)
                } else {
                    INK
                },
            );
            let mode = ui.interact(
                half,
                egui::Id::new(("stock-vca", &node.id, "response", index)),
                Sense::click(),
            );
            mode.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    ui.is_enabled(),
                    selected,
                    format!("{} response {text}", node.id),
                )
            });
            if mode.clicked() {
                self.select_primary(node.id.clone());
                action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":"response","value":index,"expected_revision":revision}),
                ));
            }
            if mode.secondary_clicked() {
                self.open_precision_editor(stock_vca_edit(node, 1, revision), mode_rect);
            }
            mode.on_hover_text("CV response: LIN is proportional; EXP uses the fourth power. Right-click for numeric entry (0 = EXP, 1 = LIN).");
        }
        if self.precision_collects(node) {
            self.register_precision_control(stock_vca_edit(node, 1, revision), mode_rect);
        }
        if self.take_precision_request(&node.id, "response") {
            self.open_precision_editor(stock_vca_edit(node, 1, revision), mode_rect);
        }
        action
    }
}
