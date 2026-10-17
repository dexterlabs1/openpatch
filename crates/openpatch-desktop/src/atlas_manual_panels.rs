fn atlas_manual_kind(kind: &str) -> bool {
    matches!(kind, "logic" | "gates" | "pulses" | "push")
}

fn paint_atlas_logic_guides(painter: &egui::Painter, rect: Rect, zoom: f32) {
    for (x, y, label, size) in [
        (120.0, 73.0, "HIGH ≥1 V · OUTPUTS 0 / 10 V", 9.0),
        (60.0, 158.0, "A · INPUT ONLY", 8.5),
        (60.0, 175.0, "B · INPUT OR BUTTON", 8.5),
        (120.0, 224.0, "NAND / NOR / XNOR = INVERTED", 8.5),
    ] {
        painter.text(
            rect.min + vec2(x, y) * zoom,
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            INK,
        );
    }
}

// Manual controls retain runtime observation and existing press/release ownership.
impl Rack {
    fn pulses_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        allow_press: bool,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let painter = ui.painter().clone();
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let enabled = ui.is_enabled()
            && self.shortcuts_enabled
            && self.performance_available
            && !node.bypassed
            && ui.input(|i| i.focused);
        for (x, label) in [(92.0, "FIRE"), (155.0, "TRIG"), (205.0, "GATE")] {
            painter.text(
                point(x, 71.0),
                Align2::CENTER_CENTER,
                label,
                crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
                INK,
            );
        }
        for (x, label) in [(155.0, "1 ms"), (205.0, "HELD")] {
            painter.text(
                point(x, 82.0),
                Align2::CENTER_CENTER,
                label,
                crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
                MUTED,
            );
        }
        let pressed_mask = self
            .module_controls
            .get(&node.id)
            .map_or(0, |controls| controls.pressed_mask);
        let mut action = None;
        for row in 0..10 {
            let y = 100.0 + row as f32 * 29.0;
            let active = pressed_mask & (1_u16 << row) != 0;
            let button_rect = Rect::from_center_size(point(92.0, y), vec2(62.0, 21.0) * zoom);
            let response = ui.interact(
                button_rect,
                ui.id().with((&node.id, "pulse-button", row)),
                Sense::click_and_drag(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    enabled,
                    format!(
                        "PULSES {} row {} momentary trigger and gate",
                        node.id,
                        row + 1
                    ),
                )
            });
            painter.text(
                point(35.0, y),
                Align2::CENTER_CENTER,
                format!("{:02}", row + 1),
                crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
                MUTED,
            );
            paint_atlas_manual_button(
                &painter,
                button_rect,
                zoom,
                active,
                enabled && response.hovered(),
                enabled,
                ("PUSH", 9.5),
            );
            let brightness = self
                .module_lights
                .get(&node.id)
                .filter(|lights| row < usize::from(lights.count))
                .map_or(0.0, |lights| lights.brightness[row][0])
                .clamp(0.0, 1.0);
            painter.circle_filled(point(132.0, y), 4.0 * zoom, Color32::from_rgb(71, 66, 58));
            self.lighting.circle(
                &painter,
                point(132.0, y),
                2.8 * zoom,
                Color32::from_rgb(238, 177, 65).gamma_multiply(0.12 + 0.88 * brightness),
                brightness,
                0.12,
            );
            if row < 9 {
                painter.line_segment(
                    [point(24.0, y + 14.0), point(216.0, y + 14.0)],
                    Stroke::new(0.55 * zoom, Color32::from_white_alpha(28)),
                );
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
                        press: PULSE_PRESS[row],
                        release: PULSE_RELEASE[row],
                    });
                } else if response.is_pointer_button_down_on() {
                    self.held_manual = Some(HeldManual {
                        id: node.id.clone(),
                        kind: node.kind.clone(),
                        release: PULSE_RELEASE[row],
                        button_rect,
                    });
                    self.select_primary(node.id.clone());
                    action = Some(RackAction::Command(
                        "trigger_module",
                        json!({"id":node.id,"action":PULSE_PRESS[row]}),
                    ));
                }
            }
            response.on_hover_text(if enabled {
                format!(
                    "Row {}: a rising press emits TRIG {} at 10 V for 1 ms; GATE {} is 10 V while held. Controls are runtime-only. Moving outside or releasing the button ends the gate.",
                    row + 1,
                    row + 1,
                    row + 1
                )
            } else {
                "Momentary controls require a live audio engine and an active module.".into()
            });
        }
        painter.text(
            point(120.0, 384.0),
            Align2::CENTER_CENTER,
            if node.bypassed {
                "BYPASSED"
            } else if !self.module_controls.contains_key(&node.id) {
                if self.performance_available {
                    "WAITING FOR ENGINE"
                } else {
                    "AUDIO OFFLINE"
                }
            } else if !self.performance_available {
                "LAST ENGINE STATE"
            } else if self
                .module_lights
                .get(&node.id)
                .is_none_or(|s| s.count < 10)
            {
                "LAMPS UNAVAILABLE"
            } else {
                "10 V · RUNTIME CONTROLS"
            },
            crate::panel_skin::print_font(painter.ctx(), 8.5 * zoom),
            MUTED,
        );
        action
    }

    fn push_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        allow_press: bool,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let painter = ui.painter().clone();
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let enabled = ui.is_enabled()
            && self.shortcuts_enabled
            && self.performance_available
            && !node.bypassed
            && ui.input(|i| i.focused);
        for (y, label) in [
            (77.0, "REMOTE · CV INPUTS"),
            (159.0, "(PUSH OR CV) XOR HOLD"),
        ] {
            painter.text(
                point(90.0, y),
                Align2::CENTER_CENTER,
                label,
                crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
                MUTED,
            );
        }
        let pressed_mask = self
            .module_controls
            .get(&node.id)
            .map_or(0, |controls| controls.pressed_mask);
        let lights = self.module_lights.get(&node.id).copied();
        let mut action = None;
        for (index, x, label, led_label, led_color) in [
            (0, 49.0, "PUSH", "GATE", Color32::from_rgb(238, 177, 65)),
            (1, 131.0, "HOLD", "LATCH", Color32::from_rgb(75, 165, 178)),
        ] {
            let active = pressed_mask & (1_u16 << index) != 0;
            let button_rect = Rect::from_center_size(point(x, 209.0), vec2(72.0, 48.0) * zoom);
            let response = ui.interact(
                button_rect,
                ui.id().with((&node.id, "push-button", index)),
                Sense::click_and_drag(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    enabled,
                    format!("PUSH {} {} performance button", node.id, label),
                )
            });
            paint_atlas_manual_button(
                &painter,
                button_rect,
                zoom,
                active,
                enabled && response.hovered(),
                enabled,
                (label, 13.0),
            );
            painter.text(
                point(x, 245.0),
                Align2::CENTER_CENTER,
                if index == 0 {
                    "MOMENTARY"
                } else {
                    "TOGGLE LATCH"
                },
                crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
                INK,
            );
            let brightness = lights
                .filter(|state| index < usize::from(state.count))
                .map_or(0.0, |state| state.brightness[index][0])
                .clamp(0.0, 1.0);
            painter.circle_filled(point(x, 265.0), 6.0 * zoom, Color32::from_rgb(71, 66, 58));
            self.lighting.circle(
                &painter,
                point(x, 265.0),
                4.4 * zoom,
                led_color.gamma_multiply(0.12 + 0.88 * brightness),
                brightness,
                0.12,
            );
            painter.text(
                point(x, 283.0),
                Align2::CENTER_CENTER,
                led_label,
                crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
                MUTED,
            );
            if enabled
                && allow_press
                && self.held_manual.is_none()
                && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
            {
                if response.clicked_by(PointerButton::Primary) {
                    self.select_primary(node.id.clone());
                    action = Some(RackAction::ManualClick {
                        id: node.id.clone(),
                        press: PUSH_PRESS[index],
                        release: PUSH_RELEASE[index],
                    });
                } else if response.is_pointer_button_down_on() {
                    self.held_manual = Some(HeldManual {
                        id: node.id.clone(),
                        kind: node.kind.clone(),
                        release: PUSH_RELEASE[index],
                        button_rect,
                    });
                    self.select_primary(node.id.clone());
                    action = Some(RackAction::Command(
                        "trigger_module",
                        json!({"id":node.id,"action":PUSH_PRESS[index]}),
                    ));
                }
            }
            response.on_hover_text(if enabled {
                if index == 0 {
                    "Momentary PUSH: gate = (PUSH button OR PUSH CV high) XOR HOLD. Each final gate rise emits a 1 ms, 10 V trigger. The Gate lamp follows the audio engine. Moving outside or releasing ends the press."
                } else {
                    "Press HOLD to toggle the persistent latch; HOLD CV rising edges also toggle it. The Latch lamp shows the actual engine latch, independent of the physical button. Saved holding seeds the runtime latch; live capture records it. Moving outside or releasing ends only the physical press."
                }
            } else {
                "Performance controls require a live audio engine and an active module. Lamps show the last reported engine state."
            });
        }
        painter.text(
            point(90.0, 302.0),
            Align2::CENTER_CENTER,
            if node.bypassed {
                "BYPASSED"
            } else if self.module_controls.contains_key(&node.id) && !self.performance_available {
                "LAST ENGINE STATE"
            } else if !self.performance_available {
                "AUDIO OFFLINE"
            } else if !self.module_controls.contains_key(&node.id) {
                "WAITING FOR ENGINE"
            } else if lights.is_none_or(|s| s.count < 2) {
                "LAMPS UNAVAILABLE"
            } else {
                "ENGINE · GATE / LATCH"
            },
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            MUTED,
        );
        painter.text(
            point(90.0, 319.0),
            Align2::CENTER_CENTER,
            "GATE RISE → 1 ms / 10 V TRIG",
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            INK,
        );
        painter.text(
            point(90.0, 384.0),
            Align2::CENTER_CENTER,
            "HOLD: PERSISTENT LATCH",
            crate::panel_skin::print_font(painter.ctx(), 8.5 * zoom),
            MUTED,
        );
        action
    }
}

// Mechanical cap state comes only from observed physical-button masks; lamps
// remain separate audio-engine observations, painted by actual RoomLighting.
fn paint_atlas_manual_button(
    painter: &egui::Painter,
    rect: Rect,
    zoom: f32,
    active: bool,
    hovered: bool,
    enabled: bool,
    (label, font_size): (&str, f32),
) {
    painter.rect_filled(
        rect.translate(vec2(0.9, 1.8) * zoom),
        4.0 * zoom,
        Color32::from_black_alpha(85),
    );
    painter.rect_filled(rect, 4.0 * zoom, Color32::from_rgb(87, 83, 74));
    let cap = rect.shrink(1.6 * zoom);
    painter.rect_filled(
        cap,
        3.0 * zoom,
        if active {
            Color32::from_rgb(204, 161, 91)
        } else if hovered {
            Color32::from_rgb(244, 233, 207)
        } else {
            Color32::from_rgb(213, 207, 188)
        },
    );
    painter.line_segment(
        [
            cap.left_top() + vec2(3.0, 1.0) * zoom,
            cap.right_top() + vec2(-3.0, 1.0) * zoom,
        ],
        Stroke::new(
            zoom,
            if active {
                Color32::from_rgb(153, 115, 62)
            } else {
                Color32::from_rgb(250, 245, 226)
            },
        ),
    );
    painter.line_segment(
        [
            cap.left_bottom() + vec2(3.0, -1.0) * zoom,
            cap.right_bottom() + vec2(-3.0, -1.0) * zoom,
        ],
        Stroke::new(zoom, Color32::from_rgb(151, 142, 122)),
    );
    if label.is_empty() {
        return;
    }
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        crate::panel_skin::print_font(painter.ctx(), font_size * zoom),
        if enabled { INK } else { MUTED },
    );
}
