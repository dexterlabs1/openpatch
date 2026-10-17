// The printed fan describes topology. Only engine telemetry selects a route;
// de-click gain tails are deliberately not represented as measured activity.
fn switch_steps_rect(steps: u8) -> Rect {
    Rect::from_min_size(
        pos2(24.0 + f32::from(steps - 2) * 44.0, 94.0),
        vec2(38.0, 24.0),
    )
}
fn switch_declick_rect() -> Rect {
    Rect::from_min_size(pos2(24.0, 128.0), vec2(132.0, 24.0))
}
fn switch_route_status(
    state: Option<ModuleTelemetry>,
    bypassed: bool,
    available: bool,
) -> (String, &'static str) {
    let route = state.map_or_else(
        || "ROUTE —".into(),
        |s| format!("ROUTE {} / {}", s.step, s.length),
    );
    let source = if bypassed {
        "BYPASSED · FROZEN"
    } else if state.is_none() {
        "WAITING FOR ENGINE"
    } else if available {
        "ENGINE SELECTED"
    } else {
        "OFFLINE SNAPSHOT"
    };
    (route, source)
}
impl Rack {
    fn switch_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
        interactive: bool,
    ) -> Option<RackAction> {
        let painter = ui.painter().clone();
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let screen = |r: Rect| Rect::from_min_max(point(r.min.x, r.min.y), point(r.max.x, r.max.y));
        let configured_length = node.params.get("steps").copied().unwrap_or(4.0) as u8;
        let state = self.sequence_states.get(&node.id).copied();
        let (route, source) = switch_route_status(state, node.bypassed, self.performance_available);
        let length = state.map_or(configured_length, |s| s.length);
        let selected = state.map(|s| s.step);
        let declick = node.params.get("declick").copied().unwrap_or(0.0) != 0.0;
        let allowed = interactive
            && ui.is_enabled()
            && self.shortcuts_enabled
            && self.editing.is_none()
            && self.annotation_edit.is_none()
            && self.blank_resize.is_none()
            && ui.input(|i| i.focused)
            && !egui::Popup::is_any_open(ui.ctx())
            && !ui.input(|i| i.key_pressed(egui::Key::Escape));
        let mut action = None;
        painter.text(
            point(90.0, 66.0),
            Align2::CENTER_CENTER,
            source,
            crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
            MUTED,
        );
        painter.text(
            point(90.0, 83.0),
            Align2::CENTER_CENTER,
            "POSITIONS",
            crate::panel_skin::print_font(ui.ctx(), 10.0 * zoom),
            INK,
        );
        let mut step_ids = Vec::new();
        for steps in [2_u8, 3, 4] {
            let bounds = screen(switch_steps_rect(steps));
            let response = ui.interact(
                bounds,
                ui.id().with((&node.id, "length", steps)),
                Sense::click(),
            );
            step_ids.push(response.id);
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::RadioButton,
                    allowed,
                    configured_length == steps,
                    format!("{} routing positions: {steps}", node.id),
                )
            });
            paint_switch_key(
                &painter,
                bounds,
                &steps.to_string(),
                configured_length == steps,
                response.hovered(),
                response.has_focus(),
                zoom,
            );
            let keyboard = allowed
                && response.has_focus()
                && ui.input_mut(|i| {
                    let m = i.modifiers;
                    i.consume_key(m, egui::Key::Space) || i.consume_key(m, egui::Key::Enter)
                });
            if allowed && (response.clicked() || keyboard) {
                response.request_focus();
                self.select_primary(node.id.clone());
                if configured_length != steps {
                    action = Some(RackAction::Command(
                        "set_parameter",
                        json!({"id":node.id,"parameter":"steps","value":steps,"expected_revision":revision}),
                    ));
                }
            }
            response.on_hover_text("Saved sequence length. Clock advances through these positions; reset takes priority and selects position 1. A shorter length wraps on the next processed sample.");
        }
        let declick_bounds = screen(switch_declick_rect());
        let response = ui.interact(
            declick_bounds,
            ui.id().with((&node.id, "declick")),
            Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                allowed,
                declick,
                format!("{} De-click", node.id),
            )
        });
        paint_switch_key(
            &painter,
            declick_bounds,
            if declick {
                "De-click · ON"
            } else {
                "De-click · OFF"
            },
            declick,
            response.hovered(),
            response.has_focus(),
            zoom,
        );
        let keyboard = allowed
            && response.has_focus()
            && ui.input_mut(|i| {
                let m = i.modifiers;
                i.consume_key(m, egui::Key::Space) || i.consume_key(m, egui::Key::Enter)
            });
        if allowed && (response.clicked() || keyboard) {
            response.request_focus();
            self.select_primary(node.id.clone());
            action = Some(RackAction::Command(
                "set_parameter",
                json!({"id":node.id,"parameter":"declick","value":if declick {0.0} else {1.0},"expected_revision":revision}),
            ));
        }
        if interactive {
            for (name, bounds, ids) in [
                (
                    "steps",
                    screen(switch_steps_rect(2).union(switch_steps_rect(4))),
                    step_ids,
                ),
                ("declick", declick_bounds, vec![response.id]),
            ] {
                if let Some(cue) = self
                    .construction_animation
                    .parameter(&node.id, &node.kind, name)
                {
                    let text = |v: f32| {
                        if name == "steps" {
                            format!("{v:.0} positions")
                        } else if v == 0.0 {
                            "De-click off".into()
                        } else {
                            "De-click on".into()
                        }
                    };
                    self.register_construction_anchor(
                        ui,
                        node,
                        name,
                        crate::construction_motion::Anchor {
                            rect: bounds,
                            old_text: text(cue.old),
                            new_text: text(cue.new),
                            marker: None,
                        },
                        &ids,
                    );
                }
            }
        }
        response.on_hover_text("Optional linear gain ramps at 400/s: a full-scale transition takes 2.5 ms. Route lamps show the selected position, not the fading gain tails. Disabled de-click freezes gain histories; enabled 4:1 broadcasts mono inputs.");
        paint_atlas_voice_readout(&painter, point(90.0, 166.0), &route, 132.0 * zoom, zoom);
        // Clock/reset jacks stay at y=213; the fan terminates at the existing
        // signal sockets (y=262,298,334,370), with no invented meter values.
        let many_x = if node.kind == "switch_1to4" {
            130.0
        } else {
            50.0
        };
        let main_x = 180.0 - many_x;
        let lamp_x = if node.kind == "switch_1to4" {
            98.0
        } else {
            82.0
        };
        painter.text(
            point(90.0, 238.0),
            Align2::CENTER_CENTER,
            "SELECTED ROUTE",
            crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
            MUTED,
        );
        for step in 1..=4 {
            let y = 262.0 + (step - 1) as f32 * 36.0;
            let active = selected == Some(step);
            let disabled = step > length;
            let ink = if active {
                Color32::from_rgb(171, 115, 45)
            } else if disabled {
                Color32::from_rgb(189, 177, 155)
            } else {
                Color32::from_rgb(144, 137, 117)
            };
            let start = if main_x < many_x {
                main_x + 18.0
            } else {
                main_x - 18.0
            };
            let end = if main_x < many_x {
                many_x - 18.0
            } else {
                many_x + 18.0
            };
            painter.line_segment(
                [point(start, 316.0), point(end, y)],
                Stroke::new(if active { 1.8 * zoom } else { 0.65 * zoom }, ink),
            );
            let at = point(lamp_x, y);
            painter.circle_filled(at, 3.3 * zoom, Color32::from_rgb(91, 80, 60));
            painter.circle_stroke(
                at,
                3.3 * zoom,
                Stroke::new(0.8 * zoom, Color32::from_rgb(185, 172, 140)),
            );
            if active {
                self.lighting.circle(
                    &painter,
                    at,
                    2.3 * zoom,
                    Color32::from_rgb(235, 173, 64),
                    1.0,
                    0.0,
                );
            }
            if disabled {
                painter.line_segment(
                    [at - vec2(2.0, 2.0) * zoom, at + vec2(2.0, 2.0) * zoom],
                    Stroke::new(0.7 * zoom, Color32::from_rgb(160, 146, 123)),
                );
            }
        }
        ui.interact(screen(Rect::from_min_max(pos2(72.0, 248.0), pos2(108.0, 380.0))),
            ui.id().with((&node.id, "route-explanation")), Sense::hover())
            .on_hover_text("Clock advances; reset wins and returns to position 1. Amber identifies the engine-selected route, including a retained offline or bypassed snapshot. Crossed lamps are outside the reported length. De-click may leave fading tails on other routes; these are not measured here.");
        action
    }
}
#[allow(clippy::too_many_arguments)]
fn paint_switch_key(
    painter: &egui::Painter,
    rect: Rect,
    label: &str,
    selected: bool,
    hovered: bool,
    focused: bool,
    zoom: f32,
) {
    painter.rect_filled(
        rect.translate(vec2(0.0, 1.5) * zoom),
        3.0 * zoom,
        Color32::from_rgb(122, 111, 89),
    );
    painter.rect_filled(
        rect,
        3.0 * zoom,
        if selected {
            Color32::from_rgb(215, 177, 108)
        } else if hovered {
            Color32::from_rgb(246, 237, 211)
        } else {
            Color32::from_rgb(231, 221, 193)
        },
    );
    painter.rect_stroke(
        rect,
        3.0 * zoom,
        Stroke::new(
            if focused { 1.4 * zoom } else { 0.7 * zoom },
            Color32::from_rgb(145, 126, 88),
        ),
        egui::StrokeKind::Inside,
    );
    painter.line_segment(
        [
            rect.left_top() + vec2(3.0, 1.0) * zoom,
            rect.right_top() + vec2(-3.0, 1.0) * zoom,
        ],
        Stroke::new(0.6 * zoom, Color32::from_rgb(255, 247, 224)),
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
}
