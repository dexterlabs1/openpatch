impl Rack {
    fn random_values_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        rect: Rect,
        revision: u64,
        allow_press: bool,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let screen = |local: Rect| {
            Rect::from_min_max(
                point(local.min.x, local.min.y),
                point(local.max.x, local.max.y),
            )
        };
        let painter = ui.painter().clone();
        let mut action = None;
        // Fixed routing legend: the display below is held engine telemetry.
        painter.text(
            point(20.0, 65.0),
            Align2::LEFT_CENTER,
            "DRAW / HOLD",
            crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
            MUTED,
        );
        painter.line_segment(
            [point(106.0, 65.0), point(282.0, 65.0)],
            Stroke::new(0.7 * zoom, Color32::from_rgb(150, 141, 112)),
        );
        for (name, y) in [("RANGE", 88.0), ("SEED", 114.0)] {
            painter.text(
                point(20.0, y),
                Align2::LEFT_CENTER,
                name,
                crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
                INK,
            );
        }
        if let Some(spec) = module.and_then(|module| module["params"].get("range")) {
            if let Some(RackAction::Command(tool, mut args)) = self.choice_parameter(
                ui,
                node,
                "range",
                spec,
                screen(random_values_range_rect()),
                &RANDOM_VALUES_RANGES,
            ) {
                args["expected_revision"] = json!(revision);
                action = Some(RackAction::Command(tool, args));
            }
        }
        if let Some(spec) = module.and_then(|module| module["params"].get("seed")) {
            let value = node
                .params
                .get("seed")
                .copied()
                .unwrap_or(spec["default"].as_f64().unwrap_or(1.0) as f32);
            let seed_rect = screen(random_values_seed_rect());
            paint_atlas_voice_readout(
                &painter,
                seed_rect.center(),
                &format!("{value:.0}"),
                seed_rect.width(),
                zoom,
            );
            let response = ui.interact(
                seed_rect,
                ui.id().with((&node.id, "random-values-seed")),
                Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    format!("Edit {} seed value, {value:.0}", node.id),
                )
            });
            let make_edit = |surface| ParameterEdit {
                id: node.id.clone(),
                kind: node.kind.clone(),
                parameter: "seed".into(),
                revision,
                surface,
                menu_popup: None,
                stale: false,
                text: format!("{value:.0}"),
                min: 0.0,
                max: 16_777_215.0,
                step: Some(1.0),
                display_scale: 1.0,
                display_unit: String::new(),
                slew_min: None,
                gain_power: None,
                raw_max: 16_777_215.0,
                focus: true,
            };
            if self.precision_collects(node) {
                self.register_precision_control(make_edit(ParameterEditSurface::Window), seed_rect);
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
                        rect: seed_rect,
                        old_text: format!("{:.0}", cue.old),
                        new_text: format!("{:.0}", cue.new),
                        marker: None,
                    },
                    &[response.id],
                );
            }
            self.precision_affordance(ui, &response);
            if response.clicked() {
                consume_parameter_activation(ui.ctx());
                self.open_precision_editor(make_edit(ParameterEditSurface::Window), seed_rect);
            }
            if self
                .requested_parameter_menu
                .as_ref()
                .is_some_and(|(id, name)| id == &node.id && name == "seed")
            {
                egui::Popup::open_id(ui.ctx(), response.id.with("popup"));
                self.requested_parameter_menu = None;
            }
            let mut popup = egui::Popup::context_menu(&response)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
            if !response.secondary_clicked()
                && egui::Popup::position_of_id(ui.ctx(), response.id.with("popup")).is_none()
            {
                popup = popup.anchor(egui::PopupAnchor::Position(response.rect.right_bottom()));
            }
            popup.show(|ui| {
                if !self.editing.as_ref().is_some_and(|edit| {
                    edit.surface == ParameterEditSurface::KnobMenu
                        && edit.id == node.id
                        && edit.parameter == "seed"
                }) {
                    self.editing = Some(make_edit(ParameterEditSurface::KnobMenu));
                }
                self.editing.as_mut().unwrap().menu_popup = Some(response.id.with("popup"));
                ui.set_min_width(244.0);
                ui.strong("SEED · 24 BIT");
                match parameter_editor_contents(ui, self.editing.as_mut().unwrap()) {
                    ParameterEditOutcome::Keep => {}
                    ParameterEditOutcome::Apply(raw)
                    | ParameterEditOutcome::ApplyAndNavigate { raw, .. } => {
                        action = Some(parameter_edit_action(&self.editing.take().unwrap(), raw));
                        ui.close();
                    }
                    ParameterEditOutcome::Cancel => {
                        self.editing = None;
                        ui.close();
                    }
                }
            });
            response.on_hover_text("Click to enter an exact 24-bit seed or expression. Changing the seed rerolls every held value immediately. Range changes affect future draws only.");
        }
        let enabled = ui.is_enabled()
            && self.shortcuts_enabled
            && self.performance_available
            && !node.bypassed
            && ui.input(|i| i.focused);
        let pressed = self
            .module_controls
            .get(&node.id)
            .is_some_and(|state| state.pressed_mask & 1 != 0);
        let button_rect = screen(random_values_push_rect());
        let response = ui.interact(
            button_rect,
            ui.id().with((&node.id, "random-values-push")),
            Sense::click_and_drag(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                enabled,
                format!("Random Values {} PUSH performance button", node.id),
            )
        });
        painter.rect_filled(
            button_rect.translate(vec2(1.0, 2.0) * zoom),
            3.0 * zoom,
            Color32::from_black_alpha(70),
        );
        painter.rect_filled(
            button_rect,
            3.0 * zoom,
            if pressed {
                Color32::from_rgb(207, 169, 103)
            } else if enabled && response.hovered() {
                Color32::from_rgb(250, 243, 220)
            } else {
                Color32::from_rgb(225, 216, 188)
            },
        );
        painter.rect_stroke(
            button_rect,
            3.0 * zoom,
            Stroke::new(zoom, Color32::from_rgb(115, 104, 78)),
            egui::StrokeKind::Inside,
        );
        painter.line_segment(
            [
                button_rect.left_top() + vec2(3.0, 2.0) * zoom,
                button_rect.right_top() + vec2(-3.0, 2.0) * zoom,
            ],
            Stroke::new(zoom, Color32::from_rgb(255, 250, 229)),
        );
        painter.text(
            button_rect.center(),
            Align2::CENTER_CENTER,
            "PUSH",
            crate::panel_skin::print_font(ui.ctx(), 12.0 * zoom),
            if enabled { INK } else { MUTED },
        );
        if enabled
            && allow_press
            && self.held_manual.is_none()
            && self.editing.is_none()
            && self.annotation_edit.is_none()
            && self.blank_resize.is_none()
            && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
        {
            if response.clicked_by(PointerButton::Primary) {
                self.select_primary(node.id.clone());
                action = Some(RackAction::ManualClick {
                    id: node.id.clone(),
                    press: "push_press",
                    release: "push_release",
                });
            } else if response.is_pointer_button_down_on() {
                self.held_manual = Some(HeldManual {
                    id: node.id.clone(),
                    kind: node.kind.clone(),
                    release: "push_release",
                    button_rect,
                });
                self.select_primary(node.id.clone());
                action = Some(RackAction::Command(
                    "trigger_module",
                    json!({"id":node.id,"action":"push_press"}),
                ));
            }
        }
        response.on_hover_text(if enabled {
            "Draw all seven outputs once. Holding does not redraw. TRIG draws a fresh value for each channel on its rising edge. Release or move outside to end the press."
        } else { "PUSH requires a live audio engine and an active module." });
        painter.text(
            point(228.0, 151.0),
            Align2::CENTER_CENTER,
            "7 × 16",
            crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
            MUTED,
        );
        let state = self.random_values_states.get(&node.id);
        let channels = state.and_then(|s| s["active_channels"].as_u64());
        for index in 0..RANDOM_VALUES_OUTPUTS.len() {
            let y = random_values_row_y(index);
            painter.line_segment(
                [point(18.0, y - 15.0), point(282.0, y - 15.0)],
                Stroke::new(0.5 * zoom, Color32::from_rgb(169, 157, 126)),
            );
            painter.text(
                point(20.0, y),
                Align2::LEFT_CENTER,
                format!("OUT {}", index + 1),
                crate::panel_skin::print_font(ui.ctx(), 10.0 * zoom),
                INK,
            );
            let readout = screen(random_values_readout_rect(index));
            let display = random_values_voltage_text(state, index);
            // A bounded screen, with illumination only for a real snapshot.
            painter.rect_filled(
                readout.expand(2.0 * zoom),
                2.5 * zoom,
                Color32::from_rgb(126, 121, 94),
            );
            if state.is_some() {
                self.lighting.screen_glow(&painter, readout, zoom);
            }
            let screen_start = crate::lighting::next(&painter);
            painter.rect_filled(readout, 1.5 * zoom, Color32::from_rgb(15, 24, 18));
            painter.rect_stroke(
                readout,
                1.5 * zoom,
                Stroke::new(zoom, Color32::from_rgb(5, 12, 8)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                readout.center(),
                Align2::CENTER_CENTER,
                &display,
                FontId::monospace(10.0 * zoom),
                if state.is_some() {
                    Color32::from_rgb(202, 216, 176)
                } else {
                    Color32::from_rgb(126, 139, 118)
                },
            );
            if state.is_some() {
                self.lighting.preserve(&painter, screen_start);
            }
            let detail = state.map_or_else(
                || "Held values unavailable. No engine snapshot has been published.".to_owned(),
                |s| {
                    let mut text = format!(
                        "OUT {} · {} active channel{}\nActual held engine voltages",
                        index + 1,
                        channels.unwrap(),
                        if channels == Some(1) { "" } else { "s" }
                    );
                    for channel in 0..16 {
                        text.push_str(&format!(
                            "\nCH {:02}: {:+.5} V{}",
                            channel + 1,
                            s["values"][index][channel].as_f64().unwrap(),
                            if channel < channels.unwrap() as usize {
                                ""
                            } else {
                                " · held, inactive"
                            }
                        ));
                    }
                    text
                },
            );
            ui.interact(
                readout,
                ui.id().with((&node.id, "random-values-readout", index)),
                Sense::hover(),
            )
            .on_hover_text(detail);
        }
        painter.text(
            point(150.0, 382.0),
            Align2::CENTER_CENTER,
            state.map_or_else(
                || "HELD VALUES UNAVAILABLE".to_owned(),
                |_| {
                    format!(
                        "{} · {} CHANNEL{}",
                        if self.performance_available {
                            "LIVE"
                        } else {
                            "LAST ENGINE STATE"
                        },
                        channels.unwrap(),
                        if channels == Some(1) { "" } else { "S" }
                    )
                },
            ),
            crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
            MUTED,
        );
        action
    }
}
