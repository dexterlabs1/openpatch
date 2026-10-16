// Shared rack/browser physically modeled resonator panel.
fn resonator_control_rects(index: usize) -> (Rect, Rect) {
    let x = 44.0 + (index % 4) as f32 * 70.0;
    let main = matches!(index, 0 | 2);
    (
        Rect::from_center_size(
            pos2(x, if index < 4 { 139.0 } else { 254.0 }),
            Vec2::splat(if main { 58.0 } else { 52.0 }),
        ),
        Rect::from_center_size(
            pos2(x, if index < 4 { 180.0 } else { 296.0 }),
            vec2(64.0, 18.0),
        ),
    )
}

fn resonator_control_print(index: usize) -> (&'static str, &'static str) {
    match index {
        0 => ("FREQUENCY", "20–4k Hz"),
        1 => ("STIFFNESS", "0–100%"),
        2 => ("DECAY", "0.1–30 s"),
        3 => ("DAMPING", "0–100%"),
        4 => ("STRIKE POS", "1–99%"),
        5 => ("PICKUP POS", "1–99%"),
        6 => ("STRENGTH", "0–2× IMPULSE"),
        _ => ("LEVEL", "0–2× OUTPUT"),
    }
}

fn resonator_parameter_help(name: &str) -> &'static str {
    match name {
        "frequency" => "Undamped natural fundamental at zero pitch CV; damping can lower the ringing frequency. PITCH transposes at 1 V/octave.",
        "stiffness" => "Moves natural modes from harmonic string spacing toward a stiff beam. No material presets.",
        "decay" => "Fundamental RT60: time to lose 60 dB. Higher modes also follow DAMPING.",
        "damping" => "Adds loss to higher modes, shortening their RT60.",
        "strike_position" => "Force position along the string/beam, from one fixed end to the other. Shapes mode excitation.",
        "pickup_position" => "Velocity pickup position along the string/beam. Mode nodes can cancel individual partials.",
        "strike_strength" => "Impulse = 0.01 × strength N·s on each rising STRIKE edge. EXCITE applies continuous audio force.",
        "level" => "Output level after the velocity pickup.",
        _ => "",
    }
}

fn paint_resonator_configuration(
    painter: &egui::Painter,
    node: &Node,
    origin: Pos2,
    zoom: f32,
    rate: u32,
) {
    let point = |x, y| origin + vec2(x, y) * zoom;
    let print = |x, y, text: String, size, color| {
        painter.text(
            point(x, y),
            Align2::LEFT_CENTER,
            text,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    };
    let copper = Color32::from_rgb(151, 93, 57);
    let teal = Color32::from_rgb(68, 113, 121);
    print(311.0, 83.0, "NATURAL MODES".into(), 11.0, INK);
    print(
        311.0,
        99.0,
        format!("CONFIG · {} kHz · ZERO CV", rate as f32 / 1000.0),
        8.0,
        MUTED,
    );
    let Ok(modes) = openpatch_core::modal_resonator_modes(node, rate) else {
        print(311.0, 164.0, "Configuration unavailable".into(), 9.0, MUTED);
        return;
    };
    let plot = Rect::from_min_max(point(313.0, 118.0), point(456.0, 210.0));
    painter.rect_filled(
        plot.expand(7.0 * zoom),
        2.0 * zoom,
        Color32::from_rgb(215, 207, 188),
    );
    let ceiling = f64::from(rate) * 0.45;
    let x = |frequency: f64| {
        plot.left()
            + ((frequency.max(20.0) / 20.0).ln() / (ceiling / 20.0).ln()).clamp(0.0, 1.0) as f32
                * plot.width()
    };
    for (hz, label) in [
        (20.0, "20"),
        (200.0, "200"),
        (2000.0, "2k"),
        (20000.0, "20k"),
    ] {
        if hz > ceiling {
            continue;
        }
        let axis_x = x(hz);
        painter.line_segment(
            [pos2(axis_x, plot.top()), pos2(axis_x, plot.bottom())],
            Stroke::new(0.5 * zoom, Color32::from_rgb(182, 183, 170)),
        );
        painter.text(
            pos2(axis_x, plot.bottom() + 11.0 * zoom),
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            MUTED,
        );
    }
    let baseline = plot.center().y;
    painter.line_segment(
        [pos2(plot.left(), baseline), pos2(plot.right(), baseline)],
        Stroke::new(0.8 * zoom, Color32::from_rgb(128, 132, 120)),
    );
    for mode in &modes {
        if !mode.active {
            continue;
        }
        let foot = pos2(x(mode.frequency_hz), baseline);
        let tip = foot - vec2(0.0, mode.coupling as f32 * plot.height() * 0.45);
        let color = if mode.coupling >= 0.0 { copper } else { teal };
        painter.line_segment([foot, tip], Stroke::new(1.6 * zoom, color));
        painter.circle_filled(tip, 1.8 * zoom, color);
    }
    print(
        311.0,
        232.0,
        format!(
            "{} / {} modes below {:.1} kHz",
            modes.iter().filter(|mode| mode.active).count(),
            modes.len(),
            ceiling / 1000.0
        ),
        8.0,
        MUTED,
    );
    print(311.0, 245.0, "± FORCE × PICKUP · LOG Hz".into(), 8.0, MUTED);
    print(
        311.0,
        261.0,
        format!(
            "RT60  {:.2} → {:.2} s",
            modes[0].decay_seconds,
            modes[modes.len() - 1].decay_seconds
        ),
        9.0,
        INK,
    );
    // A static decay profile indexed by mode, independent of audio energy.
    for (index, mode) in modes.iter().enumerate() {
        let left = 313.0 + index as f32 * 9.0;
        let height = (mode.decay_seconds / modes[0].decay_seconds).clamp(0.0, 1.0) as f32 * 17.0;
        painter.rect_filled(
            Rect::from_min_max(point(left, 288.0 - height), point(left + 5.0, 288.0)),
            0.5 * zoom,
            if mode.active {
                copper
            } else {
                Color32::from_rgb(178, 178, 162)
            },
        );
    }
    print(
        311.0,
        300.0,
        format!("MODE 1 → {} · DECAY PROFILE", modes.len()),
        8.0,
        MUTED,
    );
}

impl Rack {
    /// Computer-native parameter controls beside an explicitly configured
    /// modal response. The core helper supplies the same modal coefficients
    /// used by DSP; no telemetry or simulated signal is drawn here.
    #[allow(clippy::too_many_arguments)]
    fn resonator_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        rect: Rect,
        rate: u32,
        parameter_rate: u32,
        revision: u64,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let painter = ui.painter().clone();
        let mut action = None;
        painter.line_segment(
            [point(296.0, 79.0), point(296.0, 316.0)],
            Stroke::new(0.6 * zoom, Color32::from_rgb(166, 164, 157)),
        );
        for (index, (name, spec)) in ordered_parameters(node, module).into_iter().enumerate() {
            let (knob_local, value_local) = resonator_control_rects(index);
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
                max = max.min(parameter_rate as f32 * ratio as f32);
            }
            let default = (spec["default"].as_f64().unwrap_or(0.0) as f32).clamp(min, max);
            let value = node
                .params
                .get(name)
                .copied()
                .unwrap_or(default)
                .clamp(min, max);
            let step = spec["step"].as_f64().filter(|s| *s > 0.0).map(|s| s as f32);
            let percent = matches!(
                name,
                "stiffness" | "damping" | "strike_position" | "pickup_position"
            );
            let scale = if percent { 100.0 } else { 1.0 };
            let display = if percent {
                format!("{:.1}%", value * scale)
            } else if matches!(name, "strike_strength" | "level") {
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
                display_unit: if percent {
                    "%"
                } else {
                    spec["unit"].as_str().unwrap_or("")
                }
                .into(),
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
            let (printed_label, printed_range) = resonator_control_print(index);
            let frequency_range = if name == "frequency" {
                Some(format!("{min:.0}–{max:.0} Hz"))
            } else {
                None
            };
            let printed_range = frequency_range.as_deref().unwrap_or(printed_range);
            let x = knob_local.center().x;
            painter.text(
                point(x, if index < 4 { 86.0 } else { 202.0 }),
                Align2::CENTER_CENTER,
                printed_label,
                crate::panel_skin::print_font(ui.ctx(), 9.0 * zoom),
                INK,
            );
            painter.text(
                point(x, if index < 4 { 102.0 } else { 218.0 }),
                Align2::CENTER_CENTER,
                printed_range,
                crate::panel_skin::print_font(ui.ctx(), 7.5 * zoom),
                MUTED,
            );
            let logarithmic = matches!(name, "frequency" | "decay");
            let normalized = if logarithmic {
                normalize_geometric(value, min, max)
            } else {
                normalize(value, min, max, false)
            };
            let knob = ui.interact(
                knob_rect,
                ui.id().with(("resonator-knob", &node.id, name)),
                Sense::click_and_drag(),
            );
            knob.widget_info(|| {
                egui::WidgetInfo::slider(
                    ui.is_enabled(),
                    value as f64,
                    format!("{} {}", node.id, name),
                )
            });
            let input_enabled = ui.is_enabled()
                && self.shortcuts_enabled
                && self.editing.is_none()
                && self.annotation_edit.is_none()
                && !egui::Popup::is_any_open(ui.ctx());
            let input = self.parameter_knob_input(ui, &knob, normalized, input_enabled);
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
            let from_normalized = |n: f32| {
                if logarithmic {
                    denormalize_geometric(n, min, max)
                } else {
                    denormalize(n, min, max, false)
                }
            };
            let next = if input.reset {
                Some(default)
            } else if let Some(high) = input.boundary {
                Some(if high { max } else { min })
            } else if input.nudge != 0 {
                Some(from_normalized(
                    (normalized + input.nudge as f32 * keyboard_step).clamp(0.0, 1.0),
                ))
            } else {
                input.normalized.map(from_normalized)
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
            crate::panel_skin::signal_atlas_rotary(
                &painter,
                knob_rect.center(),
                normalized,
                knob.hovered() || knob.dragged() || knob.has_focus(),
                if matches!(index, 0 | 2) { 25.0 } else { 22.0 },
                zoom,
            );
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
            knob.clone().on_hover_cursor(egui::CursorIcon::ResizeVertical).on_hover_text(format!(
                "{}\nDrag vertically; Ctrl/Cmd for fine adjustment. Up/Down adjust, Home/End choose limits. Double-click resets; click the value or right-click to type.\n{}",
                display,
                resonator_parameter_help(name)
            ));
            crate::panel_skin::readout(
                &painter,
                value_rect.center(),
                &display,
                value_rect.width(),
                zoom,
            );
            let readout = ui.interact(
                value_rect,
                ui.id().with(("resonator-value", &node.id, name)),
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
                        old_text: if percent {
                            format!("{:.1}%", cue.old * scale)
                        } else if matches!(name, "strike_strength" | "level") {
                            format!("{:.2}×", cue.old)
                        } else {
                            parameter_value(name, cue.old, spec)
                        },
                        new_text: display.clone(),
                        marker: step.is_none().then_some(
                            crate::construction_motion::Marker::Rotary {
                                center: knob_rect.center(),
                                radius: if matches!(index, 0 | 2) { 27.0 } else { 24.0 } * zoom,
                                old: if logarithmic {
                                    normalize_geometric(cue.old, min, max)
                                } else {
                                    normalize(cue.old, min, max, false)
                                },
                                new: if logarithmic {
                                    normalize_geometric(cue.new, min, max)
                                } else {
                                    normalize(cue.new, min, max, false)
                                },
                            },
                        ),
                    },
                    &[knob.id, readout.id],
                );
            }
            readout.on_hover_text("Click to type a value or expression");
        }
        paint_resonator_configuration(&painter, node, rect.min, zoom, rate);
        painter.text(
            point(312.0, 315.0),
            Align2::LEFT_CENTER,
            "STRIKE / PICKUP",
            crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
            MUTED,
        );
        painter.line_segment(
            [point(320.0, 328.0), point(450.0, 328.0)],
            Stroke::new(1.0 * zoom, MUTED),
        );
        let strike = node.params.get("strike_position").copied().unwrap_or(0.23);
        let pickup = node.params.get("pickup_position").copied().unwrap_or(0.61);
        painter.rect_filled(
            Rect::from_center_size(point(320.0 + strike * 130.0, 328.0), vec2(6.0, 6.0) * zoom),
            0.0,
            Color32::from_rgb(151, 93, 57),
        );
        painter.circle_filled(
            point(320.0 + pickup * 130.0, 328.0),
            3.0 * zoom,
            Color32::from_rgb(68, 113, 121),
        );
        painter.line_segment(
            [point(20.0, 325.0), point(460.0, 325.0)],
            Stroke::new(0.6 * zoom, Color32::from_rgb(166, 164, 157)),
        );
        painter.text(
            point(155.0, 387.0),
            Align2::CENTER_CENTER,
            "PITCH · GATE STRIKE · AUDIO FORCE",
            crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
            MUTED,
        );
        painter.text(
            point(410.0, 387.0),
            Align2::CENTER_CENTER,
            "VELOCITY PICKUP",
            crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
            MUTED,
        );
        action
    }
}
