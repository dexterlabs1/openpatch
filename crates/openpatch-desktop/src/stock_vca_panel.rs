// Shared rack/browser faceplate. Only a coherent DSP snapshot illuminates gains.
const STOCK_VCA_PARAMS: [&str; 2] = ["level", "response"];
fn stock_vca_slider_rect() -> Rect {
    Rect::from_min_max(pos2(25.0, 109.0), pos2(55.0, 273.0))
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
        let mut level = node
            .params
            .get("level")
            .copied()
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        let response_mode = node.params.get("response").copied().unwrap_or(1.0);
        let mut action = None;
        painter.text(
            p(40.0, 91.0),
            Align2::CENTER_CENTER,
            "LEVEL",
            font(10.0),
            INK,
        );
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

        // Use global node+parameter identity: UI auto IDs change after graph edits.
        let id = egui::Id::new(("stock-vca", &node.id, "level"));
        let slider_rect = screen(stock_vca_slider_rect());
        let slider = ui.interact(slider_rect, id, Sense::click_and_drag());
        slider.widget_info(|| {
            egui::WidgetInfo::slider(
                ui.is_enabled(),
                f64::from(level),
                format!("{} level", node.id),
            )
        });
        if slider.drag_started()
            || (slider.is_pointer_button_down_on()
                && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary)))
        {
            self.select_primary(node.id.clone());
            slider.request_focus();
            if ui.data(|d| d.get_temp::<String>(id)).is_none() {
                self.gesture = self.gesture.wrapping_add(1);
                ui.data_mut(|d| {
                    d.insert_temp(id, format!("rack-stock-vca-slider-{}", self.gesture))
                });
            }
        }
        let old_level = level;
        if (slider.dragged() || slider.is_pointer_button_down_on()) && ui.is_enabled() {
            if let Some(pointer) = slider.interact_pointer_pos() {
                level = ((slider_rect.bottom() - pointer.y) / slider_rect.height()).clamp(0.0, 1.0);
            }
        }
        if slider.double_clicked() {
            level = 1.0;
        }
        if slider.has_focus() {
            let step = ui.input(|i| if i.modifiers.shift { 0.1 } else { 0.01 });
            if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                level = (level + step).min(1.0);
            }
            if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                level = (level - step).max(0.0);
            }
        }
        if level != old_level || slider.double_clicked() {
            self.select_primary(node.id.clone());
            let mut args = json!({"id":node.id,"parameter":"level","value":level,"expected_revision":revision});
            if !slider.double_clicked() && (slider.dragged() || slider.is_pointer_button_down_on())
            {
                if let Some(group) = ui.data(|d| d.get_temp::<String>(id)) {
                    args["history_group"] = json!(group);
                }
            }
            action = Some(RackAction::Command("set_parameter", args));
        }
        if slider.drag_stopped() || ui.input(|i| i.pointer.button_released(PointerButton::Primary))
        {
            ui.data_mut(|d| d.remove::<String>(id));
        }
        if slider.secondary_clicked() {
            self.open_precision_editor(
                stock_vca_edit(node, 0, revision),
                screen(stock_vca_value_rect()),
            );
        }
        slider.on_hover_text("Level 0–100%. Drag vertically; arrows adjust 1%, Shift adjusts 10%. Double-click resets to 100%. Right-click or click the readout for exact entry.");
        let track = Rect::from_min_max(p(36.0, 109.0), p(44.0, 273.0));
        painter.rect_filled(track, 4.0 * z, Color32::from_rgb(52, 61, 58));
        for tick in 0..=4 {
            let y = 273.0 - tick as f32 * 41.0;
            painter.line_segment([p(19.0, y), p(25.0, y)], Stroke::new(0.8 * z, MUTED));
        }
        // The configured fader is reflective; it never claims to be live gain.
        let thumb = Rect::from_center_size(p(40.0, 273.0 - level * 164.0), vec2(24.0, 9.0) * z);
        painter.rect_filled(thumb, 2.0 * z, Color32::from_rgb(222, 226, 212));
        painter.rect_stroke(
            thumb,
            2.0 * z,
            Stroke::new(z, Color32::from_rgb(76, 105, 90)),
            egui::StrokeKind::Inside,
        );
        painter.line_segment(
            [
                thumb.left_center() + vec2(4.0 * z, 0.0),
                thumb.right_center() - vec2(4.0 * z, 0.0),
            ],
            Stroke::new(z, INK),
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
        let status = if node.bypassed {
            "BYPASS · GAIN FROZEN".into()
        } else {
            state.map_or_else(
                || "— TELEMETRY OFFLINE".into(),
                |s| format!("{} ACTIVE VOICES", s.input_channels.unwrap()),
            )
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
