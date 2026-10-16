fn stock_octave_selector_rect(value: i32) -> Rect {
    let index = value + 4;
    let col = index % 3;
    let row = index / 3;
    Rect::from_min_max(
        pos2(24.0 + col as f32 * 44.0, 118.0 + row as f32 * 46.0),
        pos2(63.0 + col as f32 * 44.0, 158.0 + row as f32 * 46.0),
    )
}

fn stock_octave_value_rect() -> Rect {
    Rect::from_min_max(pos2(19.0, 269.0), pos2(161.0, 291.0))
}

fn stock_octave_authored_label(raw: f32) -> String {
    if raw.is_finite() {
        let magnitude = raw.abs();
        let value = if magnitude >= 100_000.0 || (magnitude != 0.0 && magnitude < 0.001) {
            format!("{raw:+.3e}")
        } else {
            format!("{raw:+.3}")
        };
        format!("{value} oct")
    } else if raw.is_nan() {
        "NaN · invalid".into()
    } else if raw.is_sign_positive() {
        "+∞ · invalid".into()
    } else {
        "−∞ · invalid".into()
    }
}

fn stock_octave_edit(node: &Node, revision: u64) -> ParameterEdit {
    let raw = node.params.get("octaves").copied().unwrap_or(0.0);
    ParameterEdit {
        id: node.id.clone(),
        kind: node.kind.clone(),
        parameter: "octaves".into(),
        revision,
        surface: ParameterEditSurface::Window,
        menu_popup: None,
        stale: false,
        text: format!("{raw}"),
        min: -4.0,
        max: 4.0,
        step: None,
        display_scale: 1.0,
        display_unit: "oct".into(),
        slew_min: None,
        gain_power: None,
        raw_max: 4.0,
        focus: true,
    }
}

fn stock_octave_port_label(port: &str) -> &str {
    match port {
        "pitch" => "PITCH",
        "shift_cv" => "SHIFT CV",
        "out" => "OUT",
        other => other,
    }
}

impl Rack {
    fn stock_octave_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
        interactive: bool,
    ) -> Option<RackAction> {
        let z = self.zoom;
        let p = |x: f32, y: f32| rect.min + vec2(x, y) * z;
        let screen = |r: Rect| Rect::from_min_max(p(r.min.x, r.min.y), p(r.max.x, r.max.y));
        let painter = ui.painter().clone();
        let font = |size| crate::panel_skin::print_font(ui.ctx(), size * z);
        let raw = node.params.get("octaves").copied().unwrap_or(0.0);
        let selected = (raw.is_finite() && (-4.0..=4.0).contains(&raw) && raw.fract() == 0.0)
            .then_some(raw as i32);
        let mut action = None;
        let construction_cue = interactive
            .then(|| self.construction_animation.parameter(&node.id, &node.kind, "octaves"))
            .flatten()
            .map(|cue| (cue.old, cue.new));
        let keyboard_enabled = interactive
            && self.shortcuts_enabled
            && !self.annotation_input_guard
            && self.annotation_edit.is_none()
            && self.editing.is_none()
            && ui.input(|input| input.focused)
            && !egui::Popup::is_any_open(ui.ctx());

        painter.text(p(90.0, 88.0), Align2::CENTER_CENTER, "SHIFT (OCTAVES)", font(11.0), INK);
        for value in -4..=4 {
            let button_rect = screen(stock_octave_selector_rect(value));
            let id = egui::Id::new(("stock-octave", &node.id, value));
            let response = ui.interact(button_rect, id, Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    true,
                    selected == Some(value),
                    format!("{} authored octave shift {value:+}", node.id),
                )
            });
            let focused_value = response.has_focus();
            if focused_value && keyboard_enabled {
                ui.ctx().memory_mut(|memory| {
                    memory.set_focus_lock_filter(
                        response.id,
                        egui::EventFilter {
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            ..Default::default()
                        },
                    );
                });
                let delta = ui.input(|input| {
                    if input.key_pressed(egui::Key::ArrowRight)
                        || input.key_pressed(egui::Key::ArrowUp)
                    {
                        1
                    } else if input.key_pressed(egui::Key::ArrowLeft)
                        || input.key_pressed(egui::Key::ArrowDown)
                    {
                        -1
                    } else {
                        0
                    }
                });
                if delta != 0 {
                    // Clamp before integer addition: finite saved project values may be far
                    // outside the button range, and their rounded cast can saturate at i32 extrema.
                    let committed = if raw.is_finite() {
                        (raw.round() as i32).clamp(-4, 4)
                    } else {
                        0
                    };
                    let next = (committed + delta).clamp(-4, 4);
                    if next as f32 != raw {
                        action = Some(RackAction::Command(
                            "set_parameter",
                            json!({"id":node.id,"parameter":"octaves","value":next,"expected_revision":revision}),
                        ));
                    }
                }
            }
            let is_selected = selected == Some(value);
            let selection_start = is_selected.then(|| crate::lighting::next(&painter));
            let fill = if is_selected {
                Color32::from_rgb(40, 82, 63)
            } else if focused_value {
                Color32::from_rgb(226, 223, 210)
            } else {
                Color32::from_rgb(202, 199, 188)
            };
            painter.rect_filled(button_rect, 3.0 * z, fill);
            painter.rect_stroke(
                button_rect,
                3.0 * z,
                Stroke::new(
                    if is_selected { 1.5 } else { 0.8 } * z,
                    if is_selected {
                        Color32::from_rgb(114, 213, 157)
                    } else {
                        INK
                    },
                ),
                egui::StrokeKind::Inside,
            );
            if is_selected {
                // A short bar marks the authored detent; it is UI state, not an engine lamp.
                self.lighting.rect(
                    &painter,
                    Rect::from_min_max(
                        button_rect.min + vec2(4.0 * z, 9.0 * z),
                        button_rect.min + vec2(6.0 * z, 31.0 * z),
                    ),
                    1.0 * z,
                    Color32::from_rgb(114, 213, 157),
                    0.58,
                    0.0,
                );
            }
            painter.text(
                button_rect.center(),
                Align2::CENTER_CENTER,
                format!("{value:+}"),
                font(17.0),
                if is_selected {
                    Color32::from_rgb(177, 239, 203)
                } else {
                    INK
                },
            );
            if let Some(start) = selection_start {
                // The authored display remains readable in darkness; its button still has a
                // dark background and a small selected-state bar, not an engine light.
                self.lighting.preserve(&painter, start);
            }
            if interactive && response.clicked() {
                response.request_focus();
                self.select_primary(node.id.clone());
                action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":"octaves","value":value,"expected_revision":revision}),
                ));
            }
            response.on_hover_text(format!("Set authored pitch shift to {value:+} octave{}", if value.abs() == 1 { "" } else { "s" }));
        }
        if let Some((old, new)) = construction_cue {
            let selector_group = screen(stock_octave_selector_rect(-4))
                .union(screen(stock_octave_selector_rect(4)));
            let ids = (-4..=4)
                .map(|value| egui::Id::new(("stock-octave", &node.id, value)))
                .collect::<Vec<_>>();
            self.register_construction_anchor(
                ui,
                node,
                "octaves",
                crate::construction_motion::Anchor {
                    rect: selector_group,
                    old_text: stock_octave_authored_label(old),
                    new_text: stock_octave_authored_label(new),
                    marker: None,
                },
                &ids,
            );
        }

        let value_rect = screen(stock_octave_value_rect());
        let value_response = ui.interact(
            value_rect,
            egui::Id::new(("value", &node.id, "octaves")),
            Sense::click(),
        );
        painter.text(p(90.0, 263.0), Align2::CENTER_CENTER, "SHIFT VALUE", font(8.5), MUTED);
        let authored_text = stock_octave_authored_label(raw);
        painter.text(
            value_rect.center(),
            Align2::CENTER_CENTER,
            authored_text,
            font(16.0),
            if selected.is_some() { INK } else { Color32::from_rgb(164, 72, 46) },
        );
        if selected.is_none() {
            let status = if raw.is_finite() && (-4.0..=4.0).contains(&raw) {
                "CUSTOM VALUE"
            } else {
                "OUTSIDE ±4 · SELECT A SHIFT"
            };
            painter.text(p(90.0, 296.0), Align2::CENTER_CENTER, status, font(7.0), Color32::from_rgb(145, 59, 38));
        } else {
            painter.text(p(90.0, 296.0), Align2::CENTER_CENTER, "CLICK VALUE TO TYPE", font(7.0), MUTED);
        }
        if self.precision_collects(node) {
            self.register_precision_control(stock_octave_edit(node, revision), value_rect);
        }
        if interactive && (value_response.clicked() || self.take_precision_request(&node.id, "octaves")) {
            consume_parameter_activation(ui.ctx());
            self.open_precision_editor(stock_octave_edit(node, revision), value_rect);
        }
        if interactive && value_response.secondary_clicked() {
            self.open_precision_editor(stock_octave_edit(node, revision), value_rect);
        }
        value_response.on_hover_text(format!("Saved raw octaves parameter: {raw:?}. Selectors edit integer octaves; exact entry accepts finite values from −4 through +4."));

        painter.line_segment([p(17.0, 303.0), p(163.0, 303.0)], Stroke::new(0.8 * z, Color32::from_gray(161)));
        painter.text(p(90.0, 314.0), Align2::CENTER_CENTER, "EFFECTIVE SHIFT", font(9.0), INK);
        match self.module_lights.get(&node.id).and_then(|state| state.octave_shift) {
            Some(offset) => {
                let actual_start = crate::lighting::next(&painter);
                painter.text(p(90.0, 332.0), Align2::CENTER_CENTER, format!("{offset:+} oct"), font(18.0), Color32::from_rgb(71, 136, 97));
                // Actual channel telemetry stays legible in the dark room; unavailable state does not glow.
                self.lighting.preserve(&painter, actual_start);
                painter.text(p(90.0, 347.0), Align2::CENTER_CENTER, "CH 1 · HOLDS IN BYPASS", font(6.5), MUTED);
            }
            None => {
                painter.text(p(90.0, 332.0), Align2::CENTER_CENTER, "—", font(18.0), MUTED);
                painter.text(p(90.0, 347.0), Align2::CENTER_CENTER, "TELEMETRY UNAVAILABLE", font(6.5), MUTED);
            }
        }
        action
    }
}
