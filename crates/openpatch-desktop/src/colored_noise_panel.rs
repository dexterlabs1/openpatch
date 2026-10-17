// Shared colored-noise controls and configured spectral guides.
fn noise_row_y(index: usize) -> f32 {
    175.0 + index as f32 * 31.0
}
fn noise_control_rects(index: usize) -> (Rect, Rect) {
    if index == 0 {
        (
            Rect::from_center_size(pos2(53.0, 99.0), Vec2::splat(46.0)),
            Rect::from_center_size(pos2(53.0, 131.0), vec2(64.0, 16.0)),
        )
    } else {
        (
            Rect::from_min_size(pos2(111.0, 114.0), vec2(111.0, 18.0)),
            Rect::from_center_size(pos2(166.5, 98.0), vec2(111.0, 18.0)),
        )
    }
}
fn noise_parameter_help(name: &str) -> &'static str {
    match name {
        "seed" => "Integer seed, 0–16,777,215. The same seed and sample rate reproduce the same sequence when the graph restarts. Changing the seed restarts this generator; level changes preserve its sequence.",
        "level" => "Shared output gain from 0× to 2×. The six Gaussian colors have nominal 3.536 V RMS at 1×; black is uniform ±5 V, with 2.887 V RMS.",
        _ => "",
    }
}
fn paint_noise_spectra(
    painter: &egui::Painter,
    origin: Pos2,
    zoom: f32,
    rate: u32,
    lighting: &crate::lighting::RoomLighting,
) {
    let point = |x, y| origin + vec2(x, y) * zoom;
    painter.text(
        point(20.0, 147.0),
        Align2::LEFT_CENTER,
        "CONFIGURED · IDEAL ROLES",
        crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
        MUTED,
    );
    let colors = [
        Color32::from_rgb(212, 217, 199),
        Color32::from_rgb(178, 86, 124),
        Color32::from_rgb(170, 75, 61),
        Color32::from_rgb(123, 89, 171),
        Color32::from_rgb(65, 116, 173),
        Color32::from_rgb(115, 124, 129),
        Color32::from_rgb(174, 185, 165),
    ];
    let descriptions = [
        "0 dB/oct",
        "−3 dB/oct",
        "−6 dB/oct",
        "+6 dB/oct",
        "+3 dB/oct",
        "Inverse A filter",
        "Uniform / flat",
    ];
    let ceiling = (f64::from(rate) * 0.45).clamp(21.0, 20_000.0);
    let start = crate::lighting::next(painter);
    lighting.screen_glow(
        painter,
        Rect::from_min_max(point(119.0, 158.0), point(192.0, 377.0)),
        zoom,
    );
    painter.rect_filled(
        Rect::from_min_max(point(119.0, 158.0), point(192.0, 377.0)),
        3.0 * zoom,
        Color32::from_rgb(23, 34, 28),
    );
    // Curves are configured filter responses, not sampled output activity.
    for (index, port) in NOISE_OUTPUTS.iter().enumerate() {
        let y = noise_row_y(index);
        let color = colors[index];
        let plot = Rect::from_min_max(point(124.0, y - 8.0), point(186.0, y + 8.0));
        painter.rect_filled(
            plot.expand(2.0 * zoom),
            1.0 * zoom,
            Color32::from_rgb(23, 34, 28),
        );
        painter.line_segment(
            [
                pos2(plot.left(), plot.center().y),
                pos2(plot.right(), plot.center().y),
            ],
            Stroke::new(0.5 * zoom, Color32::from_rgb(70, 84, 68)),
        );
        // This is a fixed filter response, normalized at 1 kHz. It never reads
        // runtime sample history, level, seed, or wall-clock animation time.
        let reference =
            openpatch_core::colored_noise_response_db(port, 1000.0, rate).unwrap_or(0.0);
        let points = (0..33)
            .filter_map(|i| {
                let t = i as f64 / 32.0;
                let hz = 20.0 * (ceiling / 20.0).powf(t);
                openpatch_core::colored_noise_response_db(port, hz, rate).map(|db| {
                    pos2(
                        plot.left() + t as f32 * plot.width(),
                        plot.center().y
                            - ((db - reference) / 36.0).clamp(-1.0, 1.0) as f32
                                * plot.height()
                                * 0.5,
                    )
                })
            })
            .collect::<Vec<_>>();
        if points.len() >= 2 {
            painter.add(egui::Shape::line(points, Stroke::new(1.5 * zoom, color)));
        }
    }
    painter.text(
        point(124.0, 373.0),
        Align2::LEFT_CENTER,
        "20",
        crate::panel_skin::print_font(painter.ctx(), 6.0 * zoom),
        Color32::from_rgb(172, 185, 159),
    );
    painter.text(
        point(186.0, 373.0),
        Align2::RIGHT_CENTER,
        format!("{:.1}k Hz", ceiling / 1000.0),
        crate::panel_skin::print_font(painter.ctx(), 6.0 * zoom),
        Color32::from_rgb(172, 185, 159),
    );
    lighting.preserve(painter, start);
    for (index, port) in NOISE_OUTPUTS.iter().enumerate() {
        let y = noise_row_y(index);
        let color = match index {
            0 => Color32::from_rgb(111, 113, 109),
            6 => Color32::from_rgb(54, 61, 57),
            _ => colors[index],
        };
        painter.line_segment(
            [point(18.0, y - 9.0), point(18.0, y + 11.0)],
            Stroke::new(3.0 * zoom, color),
        );
        painter.text(
            point(26.0, y - 5.5),
            Align2::LEFT_CENTER,
            port.to_uppercase(),
            crate::panel_skin::print_font(painter.ctx(), 13.0 * zoom),
            INK,
        );
        painter.text(
            point(26.0, y + 10.0),
            Align2::LEFT_CENTER,
            descriptions[index],
            crate::panel_skin::print_font(painter.ctx(), 9.5 * zoom),
            MUTED,
        );
    }
    painter.text(
        point(120.0, 386.0),
        Align2::CENTER_CENTER,
        format!("STATIC · {} Hz · 1 kHz = 0 dB", rate),
        crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
        MUTED,
    );
}

impl Rack {
    fn noise_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        rect: Rect,
        rate: u32,
        revision: u64,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let painter = ui.painter().clone();
        let mut action = None;
        for (index, (name, spec)) in ordered_parameters(node, module).into_iter().enumerate() {
            let (knob_local, value_local) = noise_control_rects(index);
            let screen = |local: Rect| {
                Rect::from_min_max(
                    point(local.min.x, local.min.y),
                    point(local.max.x, local.max.y),
                )
            };
            let knob_rect = screen(knob_local);
            let value_rect = screen(value_local);
            let min = spec["min"].as_f64().unwrap_or(0.0) as f32;
            let mut max = spec["max"].as_f64().unwrap_or(1.0) as f32;
            if let Some(ratio) = spec["sample_rate_max_ratio"].as_f64() {
                max = max.min(rate as f32 * ratio as f32);
            }
            let default = (spec["default"].as_f64().unwrap_or(0.0) as f32).clamp(min, max);
            let value = node
                .params
                .get(name)
                .copied()
                .unwrap_or(default)
                .clamp(min, max);
            let step = spec["step"].as_f64().filter(|s| *s > 0.0).map(|s| s as f32);
            let scale = 1.0;
            let display = if name == "level" {
                format!("{value:.2}×")
            } else {
                parameter_value(name, value, spec)
            };
            let make_edit = |surface| ParameterEdit {
                id: node.id.clone(),
                kind: node.kind.clone(),
                parameter: name.into(),
                revision,
                surface,
                menu_popup: None,
                stale: false,
                text: (value * scale).to_string(),
                min: min * scale,
                max: max * scale,
                step: step.map(|s| s * scale),
                display_scale: scale,
                display_unit: spec["unit"].as_str().unwrap_or("").into(),
                slew_min: None,
                gain_power: None,
                raw_max: max,
                focus: true,
            };
            if self.precision_collects(node) {
                self.register_precision_control(
                    make_edit(ParameterEditSurface::Window),
                    value_rect,
                );
            }
            let is_level = name == "level";
            painter.text(
                point(if is_level { 53.0 } else { 166.5 }, 69.0),
                Align2::CENTER_CENTER,
                if is_level {
                    "LEVEL · 0–2×"
                } else {
                    "SEED · 24-BIT"
                },
                crate::panel_skin::print_font(ui.ctx(), 9.0 * zoom),
                INK,
            );
            if !is_level {
                painter.text(
                    point(166.5, 81.0),
                    Align2::CENTER_CENTER,
                    "0–16,777,215",
                    crate::panel_skin::print_font(ui.ctx(), 7.0 * zoom),
                    MUTED,
                );
            }
            let knob = ui.interact(
                knob_rect,
                ui.id().with(("noise-control", &node.id, name)),
                if is_level {
                    Sense::click_and_drag()
                } else {
                    Sense::click()
                },
            );
            knob.widget_info(|| {
                egui::WidgetInfo::slider(
                    ui.is_enabled(),
                    value as f64,
                    format!("{} {}", node.id, name),
                )
            });
            if is_level {
                let normalized = normalize(value, min, max, false);
                let input_enabled = ui.is_enabled()
                    && self.shortcuts_enabled
                    && self.editing.is_none()
                    && self.annotation_edit.is_none()
                    && !egui::Popup::is_any_open(ui.ctx());
                let input =
                    self.parameter_knob_input(ui, &knob, normalized, input_enabled, &display);
                if input_enabled
                    && (knob.is_pointer_button_down_on()
                        || knob.clicked_by(PointerButton::Primary)
                        || knob.drag_started_by(PointerButton::Primary))
                {
                    self.select_primary(node.id.clone());
                }
                let keyboard_step = ui.input(|i| {
                    if i.modifiers.command || i.modifiers.ctrl {
                        if i.modifiers.shift {
                            0.0001
                        } else {
                            0.001
                        }
                    } else if i.modifiers.shift {
                        0.1
                    } else {
                        0.01
                    }
                });
                let next = if input.reset {
                    Some(default)
                } else if let Some(high) = input.boundary {
                    Some(if high { max } else { min })
                } else if input.nudge != 0 {
                    Some(denormalize(
                        (normalized + input.nudge as f32 * keyboard_step).clamp(0.0, 1.0),
                        min,
                        max,
                        false,
                    ))
                } else {
                    input.normalized.map(|n| denormalize(n, min, max, false))
                };
                if let Some(next) = next
                    .map(|n| quantize(n, min, max, step))
                    .filter(|n| *n != value)
                {
                    let mut args = json!({"id":node.id,"parameter":name,"value":next,"expected_revision":revision});
                    if let Some(group) = input.history_group {
                        args["history_group"] = json!(group);
                    }
                    action = Some(RackAction::Command("set_parameter", args));
                }
                paint_atlas_voice_rotary(
                    &painter,
                    knob_rect.center(),
                    normalized,
                    knob.hovered() || knob.dragged() || knob.has_focus(),
                    23.0,
                    zoom,
                );
            } else {
                painter.text(
                    knob_rect.center(),
                    Align2::CENTER_CENTER,
                    "EDIT INTEGER",
                    crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
                    MUTED,
                );
            }
            if self
                .requested_parameter_menu
                .as_ref()
                .is_some_and(|(id, parameter)| id == &node.id && parameter == name)
            {
                egui::Popup::open_id(ui.ctx(), knob.id.with("popup"));
                self.requested_parameter_menu = None;
            }
            let mut popup = egui::Popup::context_menu(&knob)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
            if !knob.secondary_clicked()
                && egui::Popup::position_of_id(ui.ctx(), knob.id.with("popup")).is_none()
            {
                popup = popup.anchor(egui::PopupAnchor::Position(knob.rect.right_bottom()));
            }
            popup.show(|ui| {
                if !self.editing.as_ref().is_some_and(|edit| {
                    edit.surface == ParameterEditSurface::KnobMenu
                        && edit.id == node.id
                        && edit.parameter == name
                }) {
                    self.editing = Some(make_edit(ParameterEditSurface::KnobMenu));
                }
                self.editing.as_mut().unwrap().menu_popup = Some(knob.id.with("popup"));
                ui.set_min_width(244.0);
                ui.strong(parameter_label(name));
                ui.label(format!("Current: {display}"));
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
            if name == "seed" && knob.clicked() {
                self.select_primary(node.id.clone());
                consume_parameter_activation(ui.ctx());
                self.open_precision_editor(make_edit(ParameterEditSurface::Window), value_rect);
            }
            let motion_knob_id = knob.id;
            let instructions = if is_level {
                "Drag vertically; Ctrl/Cmd for fine adjustment. Arrows adjust; Home/End set limits; double-click resets. Click the value or right-click to type."
            } else {
                "Click to enter an exact integer; right-click for the value menu."
            };
            knob.on_hover_text(format!(
                "{display}\n{instructions}\n{}",
                noise_parameter_help(name)
            ));
            paint_atlas_voice_readout(
                &painter,
                value_rect.center(),
                &display,
                value_rect.width(),
                zoom,
            );
            let readout = ui.interact(
                value_rect,
                ui.id().with(("noise-value", &node.id, name)),
                Sense::click(),
            );
            readout.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    format!("Edit {} {} value, {}", node.id, name, display),
                )
            });
            self.precision_affordance(ui, &readout);
            if readout.clicked() {
                consume_parameter_activation(ui.ctx());
                self.open_precision_editor(make_edit(ParameterEditSurface::Window), value_rect);
            }
            readout
                .clone()
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if let Some(cue) = self
                .construction_animation
                .parameter(&node.id, &node.kind, name)
            {
                self.register_construction_anchor(
                    ui,
                    node,
                    name,
                    crate::construction_motion::Anchor {
                        rect: knob_rect.union(value_rect),
                        old_text: if name == "seed" {
                            format!("{:.0}", cue.old)
                        } else {
                            format!("{:.2}×", cue.old)
                        },
                        new_text: display.clone(),
                        marker: (name == "level").then_some(
                            crate::construction_motion::Marker::Rotary {
                                center: knob_rect.center(),
                                radius: 23.0 * zoom,
                                old: normalize(cue.old, min, max, false),
                                new: normalize(cue.new, min, max, false),
                            },
                        ),
                    },
                    &[motion_knob_id, readout.id],
                );
            }
            readout.on_hover_text("Click to type a value or expression");
        }
        paint_noise_spectra(&painter, rect.min, zoom, rate, &self.lighting);
        let spectra = Rect::from_min_max(point(18.0, 146.0), point(194.0, 389.0));
        ui.interact(spectra, ui.id().with(("noise-spectra", &node.id)), Sense::hover())
            .on_hover_text(format!("Configured static filter responses at {} Hz, relative to 1 kHz; vertical range ±36 dB and logarithmic frequency 20 Hz to min(20 kHz, 0.45 × sample rate). No live signal is displayed.\nPrinted slopes are ideal power-density roles; actual digital shapes bend near their limits. Slopes are qualified from 100 Hz to min(10 kHz, sample rate/8). Gray approximates inverse A weighting, with soft 60 Hz / high-frequency rolloffs; qualified 80 Hz to min(20 kHz, 0.4 × sample rate). Black is uniform ±5 V at 1×. Gaussian peaks are unbounded; all outputs are mono.", rate));
        action
    }
}
