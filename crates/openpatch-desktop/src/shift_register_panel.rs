impl Rack {
    fn shift_register_panel(
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
        // Printed clock/shift schematic; these stages are not activity lamps.
        let print = Stroke::new(0.7 * zoom, Color32::from_rgb(151, 141, 113));
        painter.text(
            point(20.0, 64.0),
            Align2::LEFT_CENTER,
            "CLOCK",
            crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
            MUTED,
        );
        painter.line_segment([point(58.0, 64.0), point(320.0, 64.0)], print);
        for index in 0..8 {
            let x = 83.0 + index as f32 * 31.0;
            let stage = screen(Rect::from_center_size(pos2(x, 64.0), vec2(21.0, 13.0)));
            painter.rect_filled(stage, zoom, Color32::from_rgb(223, 214, 189));
            painter.rect_stroke(stage, zoom, print, egui::StrokeKind::Inside);
            painter.text(
                stage.center(),
                Align2::CENTER_CENTER,
                format!("{}", index + 1),
                crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
                INK,
            );
            if index < 7 {
                painter.line_segment([point(x + 15.0, 61.5), point(x + 18.0, 64.0)], print);
                painter.line_segment([point(x + 15.0, 66.5), point(x + 18.0, 64.0)], print);
            }
        }
        painter.text(
            point(370.0, 64.0),
            Align2::RIGHT_CENTER,
            "OLD → NEXT",
            crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
            MUTED,
        );
        let random = node.params.get("random").copied().unwrap_or(0.0) != 0.0;
        let random_rect = screen(shift_register_random_rect());
        let response = ui.interact(
            random_rect,
            ui.id().with((&node.id, "shift-register-random")),
            Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                ui.is_enabled(),
                random,
                "Random voltage normal",
            )
        });
        painter.rect_filled(
            random_rect.translate(vec2(0.0, 1.5) * zoom),
            3.0 * zoom,
            Color32::from_black_alpha(65),
        );
        painter.rect_filled(
            random_rect,
            3.0 * zoom,
            if random {
                Color32::from_rgb(218, 176, 103)
            } else {
                Color32::from_rgb(186, 188, 179)
            },
        );
        painter.rect_stroke(
            random_rect,
            3.0 * zoom,
            Stroke::new(zoom, MUTED),
            egui::StrokeKind::Inside,
        );
        painter.text(
            random_rect.center(),
            Align2::CENTER_CENTER,
            if random { "RND ON" } else { "RND OFF" },
            crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
            INK,
        );
        if let Some(cue) = self
            .construction_animation
            .parameter(&node.id, &node.kind, "random")
        {
            let label = |value: f32| if value == 0.0 { "RND OFF" } else { "RND ON" }.to_owned();
            self.register_construction_anchor(
                ui,
                node,
                "random",
                crate::construction_motion::Anchor {
                    rect: random_rect,
                    old_text: label(cue.old),
                    new_text: label(cue.new),
                    marker: None,
                },
                &[response.id],
            );
        }
        if response.clicked() {
            consume_parameter_activation(ui.ctx());
            action = Some(RackAction::Command(
                "set_parameter",
                json!({"id":node.id,"parameter":"random","value":if random {0} else {1},"expected_revision":revision}),
            ));
        }
        response.on_hover_text("When IN 1 is unpatched, capture a random voltage instead of 0 V. With RND on, each row-1 capture advances the generator, even with IN 1 patched. RND changes future captures only.");
        painter.text(
            point(104.0, 88.0),
            Align2::LEFT_CENTER,
            "RANGE",
            crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
            INK,
        );
        if let Some(spec) = module.and_then(|module| module["params"].get("range")) {
            if let Some(RackAction::Command(tool, mut args)) = self.choice_parameter(
                ui,
                node,
                "range",
                spec,
                screen(shift_register_range_rect()),
                &RANDOM_VALUES_RANGES,
            ) {
                args["expected_revision"] = json!(revision);
                action = Some(RackAction::Command(tool, args));
            }
        }
        painter.text(
            point(20.0, 112.0),
            Align2::LEFT_CENTER,
            "SEED",
            crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
            INK,
        );
        if let Some(spec) = module.and_then(|module| module["params"].get("seed")) {
            let value = node
                .params
                .get("seed")
                .copied()
                .unwrap_or(spec["default"].as_f64().unwrap_or(1.0) as f32);
            let seed_rect = screen(shift_register_seed_rect());
            paint_atlas_voice_readout(
                &painter,
                seed_rect.center(),
                &format!("{value:.0}"),
                seed_rect.width(),
                zoom,
            );
            let response = ui.interact(
                seed_rect,
                ui.id().with((&node.id, "shift-register-seed")),
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
            response.on_hover_text("Click to enter an exact 24-bit seed or expression. Changing the seed resets the random generator while preserving held voltages. Range and RND changes affect future captures only.");
        }
        let enabled = ui.is_enabled()
            && self.shortcuts_enabled
            && self.performance_available
            && !node.bypassed
            && ui.input(|i| i.focused);
        for (index, label, press, release) in [
            (0, "PUSH", "push_press", "push_release"),
            (1, "CLEAR", "reset_press", "reset_release"),
        ] {
            let button_rect = screen(shift_register_button_rect(index));
            let response = ui.interact(
                button_rect,
                ui.id().with((&node.id, "shift-register-button", index)),
                Sense::click_and_drag(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    enabled,
                    format!("Shift Register {} {label} performance button", node.id),
                )
            });
            let pressed = self
                .module_controls
                .get(&node.id)
                .is_some_and(|state| state.pressed_mask & (1 << index) != 0);
            painter.rect_filled(
                button_rect.translate(vec2(1.0, 2.0) * zoom),
                3.0 * zoom,
                Color32::from_black_alpha(70),
            );
            painter.rect_filled(
                button_rect,
                3.0 * zoom,
                if pressed {
                    Color32::from_rgb(226, 197, 143)
                } else if enabled && response.hovered() {
                    Color32::from_rgb(246, 237, 210)
                } else {
                    Color32::from_rgb(190, 186, 175)
                },
            );
            painter.rect_stroke(
                button_rect,
                3.0 * zoom,
                Stroke::new(zoom, MUTED),
                egui::StrokeKind::Inside,
            );
            painter.text(
                button_rect.center(),
                Align2::CENTER_CENTER,
                label,
                crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
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
                        json!({"id":node.id,"action":press}),
                    ));
                }
            }
            response.on_hover_text(if !enabled { "Performance buttons require a live audio engine and an active module." } else if index == 0 { "Capture row 1 once; its trigger propagates through unpatched TRIG jacks. Holding does not repeat. Each row samples the previous held row before the capture." } else { "Clear all eight held voltages once. Holding does not continually clear. CLEAR wins over simultaneous captures; random draws still advance." });
        }
        for (x, label) in [
            (80.0, "IN"),
            (146.0, "TRIG"),
            (263.0, "HELD VOLTAGE"),
            (360.0, "OUT"),
        ] {
            painter.text(
                point(x, 133.0),
                Align2::CENTER_CENTER,
                label,
                crate::panel_skin::print_font(ui.ctx(), 9.0 * zoom),
                MUTED,
            );
        }
        // Dashed printed traces describe normalled routes, not live cables.
        // Signal rails link the previous OUT to this IN; TRIG inherits above.
        let guide = Stroke::new(0.65 * zoom, Color32::from_rgb(151, 151, 141));
        for index in 1..8 {
            let y = shift_register_row_y(index);
            let previous = shift_register_row_y(index - 1);
            for [from, to] in [
                [point(360.0, previous + 10.0), point(360.0, previous + 15.0)],
                [point(360.0, previous + 15.0), point(43.0, previous + 15.0)],
                [point(43.0, previous + 15.0), point(43.0, y)],
                [point(43.0, y), point(69.0, y)],
                [point(146.0, previous + 11.0), point(146.0, y - 11.0)],
            ] {
                painter.add(egui::Shape::dashed_line(
                    &[from, to],
                    guide,
                    2.5 * zoom,
                    2.0 * zoom,
                ));
            }
            painter.line_segment([point(65.0, y - 2.5), point(69.0, y)], guide);
            painter.line_segment([point(65.0, y + 2.5), point(69.0, y)], guide);
        }
        painter.text(
            point(45.0, shift_register_row_y(0)),
            Align2::CENTER_CENTER,
            "0 / RND",
            crate::panel_skin::print_font(ui.ctx(), 7.0 * zoom),
            MUTED,
        );
        painter.add(egui::Shape::dashed_line(
            &[
                point(61.0, shift_register_row_y(0)),
                point(69.0, shift_register_row_y(0)),
            ],
            guide,
            2.5 * zoom,
            2.0 * zoom,
        ));
        let state = self.shift_register_states.get(&node.id);
        for index in 0..8 {
            let y = shift_register_row_y(index);
            painter.text(
                point(20.0, y),
                Align2::CENTER_CENTER,
                format!("{}", index + 1),
                crate::panel_skin::print_font(ui.ctx(), 10.0 * zoom),
                INK,
            );
            let readout = screen(shift_register_readout_rect(index));
            let display = state.map_or_else(
                || "UNAVAILABLE".to_owned(),
                |state| format!("{:+.3} V", state["values"][index].as_f64().unwrap()),
            );
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
                FontId::monospace(11.0 * zoom),
                if state.is_some() {
                    Color32::from_rgb(202, 216, 176)
                } else {
                    Color32::from_rgb(126, 139, 118)
                },
            );
            if state.is_some() {
                self.lighting.preserve(&painter, screen_start);
            }
            // The solid capture and output ticks distinguish the clock from the held signal.
            painter.line_segment([point(163.0, y), point(180.0, y)], print);
            painter.line_segment([point(177.0, y - 2.5), point(180.0, y)], print);
            painter.line_segment([point(177.0, y + 2.5), point(180.0, y)], print);
            painter.line_segment([point(343.0, y), point(346.0, y)], print);
            let detail = state.map_or_else(
                || "Held voltage unavailable: no engine snapshot has been published.".to_owned(),
                |state| {
                    format!(
                        "Actual held snapshot: {:+} V",
                        state["values"][index].as_f64().unwrap()
                    )
                },
            );
            ui.interact(readout, ui.id().with((&node.id, "shift-register-readout", index)), Sense::hover()).on_hover_text(format!("Row {} · mono, input channel 1 only.\n{detail}\n{}\nPUSH clocks row 1. Unpatched TRIG rows 2–8 inherit the preceding row's capture event; patched TRIG breaks inheritance. Dashed lines show unpatched normals.", index + 1, if index == 0 { "Unpatched IN captures 0 V, or random voltage with RND enabled.".to_owned() } else { format!("Unpatched IN captures the OLD held OUT {} value.", index) }));
        }
        painter.text(
            point(195.0, 389.0),
            Align2::CENTER_CENTER,
            if state.is_none() {
                "HELD VALUES UNAVAILABLE · DASHED = NORMALS"
            } else if self.performance_available {
                "LIVE MONO VALUES · DASHED = NORMALS"
            } else {
                "LAST ENGINE STATE · DASHED = NORMALS"
            },
            crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
            MUTED,
        );
        action
    }
}
