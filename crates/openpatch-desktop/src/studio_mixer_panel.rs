fn studio_mixer_parameter_edit(
    node: &Node,
    name: &str,
    spec: &Value,
    revision: u64,
) -> ParameterEdit {
    let value = node
        .params
        .get(name)
        .copied()
        .unwrap_or_else(|| spec["default"].as_f64().unwrap_or(0.0) as f32);
    let min = spec["min"].as_f64().unwrap_or(0.0) as f32;
    let max = spec["max"].as_f64().unwrap_or(1.0) as f32;
    ParameterEdit {
        id: node.id.clone(),
        kind: node.kind.clone(),
        parameter: name.to_owned(),
        revision,
        surface: ParameterEditSurface::Window,
        menu_popup: None,
        stale: false,
        text: value.to_string(),
        min,
        max,
        step: spec["step"].as_f64().map(|step| step as f32),
        display_scale: 1.0,
        display_unit: spec["unit"].as_str().unwrap_or("").to_owned(),
        slew_min: None,
        gain_power: None,
        raw_max: max,
        focus: true,
    }
}

fn studio_mixer_channels(node: &Node) -> usize {
    node.params
        .get("channels")
        .copied()
        .unwrap_or(8.0)
        .clamp(1.0, 16.0) as usize
}
fn studio_mixer_input_visible(node: &Node, port: &str) -> bool {
    node.kind != "studio_mixer"
        || port
            .strip_prefix("in")
            .and_then(|s| s.parse::<usize>().ok())
            .is_none_or(|i| i <= studio_mixer_channels(node))
}
fn studio_mixer_port_label(port: &str) -> &str {
    match port {
        "send_a_left" => "A L",
        "send_a_right" => "A R",
        "send_b_left" => "B L",
        "send_b_right" => "B R",
        "cue_left" => "C L",
        "cue_right" => "C R",
        "return_a" => "RET A",
        "return_b" => "RET B",
        "send_a_cv" => "A CV",
        "send_b_cv" => "B CV",
        "left" => "L",
        "right" => "R",
        _ => "IN",
    }
}
fn studio_mixer_port(input: bool, index: usize, width: f32) -> Pos2 {
    let channels = ((width - 228.0) / 72.0).round().clamp(1.0, 16.0) as usize;
    let base = channels as f32 * 72.0;
    if input {
        if index < 16 {
            return pos2(36.0 + index as f32 * 72.0, 382.0);
        }
        return pos2(
            base + if index.is_multiple_of(2) {
                106.0
            } else {
                176.0
            },
            if index < 18 { 344.0 } else { 310.0 },
        );
    }
    let (x, y) = match index {
        0 => (106., 382.),
        1 => (176., 382.),
        2 => (25., 272.),
        3 => (60., 272.),
        4 => (25., 311.),
        5 => (60., 311.),
        6 => (25., 354.),
        _ => (60., 354.),
    };
    pos2(base + x, y)
}

fn studio_mixer_strip_caption(
    painter: &egui::Painter,
    index: usize,
    source: &str,
    font: FontId,
    max_width: f32,
) -> String {
    let measure = |label: &str| {
        painter
            .layout_no_wrap(label.to_owned(), font.clone(), INK)
            .size()
            .x
    };
    let prefix = format!("{index} · ");
    let full = format!("{prefix}{source}");
    if measure(&full) <= max_width {
        return full;
    }
    let mut short = prefix.clone();
    for character in source.chars() {
        let candidate = format!("{short}{character}…");
        if measure(&candidate) > max_width {
            break;
        }
        short.push(character);
    }
    short.push('…');
    if measure(&short) <= max_width {
        short
    } else if measure(&index.to_string()) <= max_width {
        index.to_string()
    } else {
        String::new()
    }
}

impl Rack {
    #[allow(clippy::too_many_arguments)]
    fn console_parameter(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: &Value,
        name: &str,
        center: Pos2,
        radius: f32,
        fader: bool,
        revision: u64,
        interactive: bool,
        paint: bool,
    ) -> Option<RackAction> {
        let spec = &module["params"][name];
        let min = spec["min"].as_f64().unwrap_or(0.) as f32;
        let max = spec["max"].as_f64().unwrap_or(1.) as f32;
        let default = spec["default"].as_f64().unwrap_or(0.) as f32;
        let value = node.params.get(name).copied().unwrap_or(default);
        let norm = normalize(value, min, max, name == "frequency");
        let z = self.zoom;
        let track = if fader {
            crate::panel_skin::parameter_fader_track(center, radius, z)
        } else {
            Rect::from_center_size(center, vec2(radius * 2., radius * 2.) * z)
        };
        let hit = if fader {
            track.expand2(vec2(16., 6.) * z)
        } else {
            track
        };
        let response = ui.interact(
            hit,
            ui.id().with(("console-parameter", &node.id, name)),
            if interactive {
                Sense::click_and_drag()
            } else {
                Sense::hover()
            },
        );
        let popup_open = egui::Popup::is_any_open(ui.ctx());
        let direct_interactive = interactive && !popup_open;
        let own_menu_open = egui::Popup::is_id_open(ui.ctx(), response.id.with("popup"));
        response.widget_info(|| {
            egui::WidgetInfo::slider(
                direct_interactive,
                value as f64,
                format!("{} {}", node.id, name),
            )
        });
        let active = response.hovered() || response.dragged() || response.has_focus();
        if paint {
            if fader {
                crate::panel_skin::parameter_fader(
                    ui.painter(),
                    track,
                    norm,
                    active,
                    Color32::from_rgb(93, 163, 143),
                    z,
                );
            } else {
                crate::panel_skin::parameter_rotary(
                    ui.painter(),
                    center,
                    radius,
                    norm,
                    active,
                    Color32::from_rgb(104, 166, 155),
                    z,
                    &self.lighting,
                );
            }
        }
        // Accepted MCP values are authoritative immediately. The construction
        // cursor follows this control's actual travel without replaying edits.
        if paint {
            if let Some(cue) = self
                .construction_animation
                .parameter(&node.id, &node.kind, name)
            {
                let old = normalize(cue.old, min, max, name == "frequency");
                let new = normalize(cue.new, min, max, name == "frequency");
                let marker = if fader {
                    crate::construction_motion::Marker::VerticalSlider {
                        rect: track,
                        old,
                        new,
                    }
                } else {
                    crate::construction_motion::Marker::Rotary {
                        center,
                        radius: radius * z,
                        old,
                        new,
                    }
                };
                self.register_construction_anchor(
                    ui,
                    node,
                    name,
                    crate::construction_motion::Anchor {
                        rect: hit,
                        old_text: parameter_value(name, cue.old, spec),
                        new_text: parameter_value(name, cue.new, spec),
                        marker: Some(marker),
                    },
                    &[response.id],
                );
            }
        }
        let mut action = None;
        if direct_interactive {
            let input = if fader {
                self.parameter_fader_input(ui, &response, track, norm, direct_interactive)
            } else {
                self.parameter_knob_input(
                    ui,
                    &response,
                    norm,
                    direct_interactive,
                    &parameter_value(name, value, spec),
                )
            };
            let next = if input.reset {
                Some(default)
            } else if let Some(high) = input.boundary {
                Some(if high { max } else { min })
            } else if input.nudge != 0 {
                Some((value + input.nudge as f32 * (max - min) * 0.01).clamp(min, max))
            } else {
                input
                    .normalized
                    .map(|n| denormalize(n, min, max, name == "frequency"))
            };
            if let Some(mut next) = next {
                if spec["step"] == 1 {
                    next = next.round();
                }
                if next != value {
                    let mut args = json!({"id":node.id,"parameter":name,"value":next,"expected_revision":revision});
                    if let Some(group) = input.history_group {
                        args["history_group"] = json!(group);
                    }
                    action = Some(RackAction::Command("set_parameter", args));
                }
            }
        } else {
            // Clear a focused control's gesture state while an editor, palette,
            // annotation, or popup owns keyboard input.
            if fader {
                self.parameter_fader_input(ui, &response, track, norm, false);
            } else {
                self.parameter_knob_input(
                    ui,
                    &response,
                    norm,
                    false,
                    &parameter_value(name, value, spec),
                );
            }
        }
        if self.precision_collects(node) {
            self.register_precision_control(
                studio_mixer_parameter_edit(node, name, spec, revision),
                hit,
            );
        }
        self.precision_affordance(ui, &response);
        if direct_interactive && self.take_precision_request(&node.id, name) {
            consume_parameter_activation(ui.ctx());
            self.open_precision_editor(
                studio_mixer_parameter_edit(node, name, spec, revision),
                hit,
            );
        }
        if interactive && (!popup_open || own_menu_open) {
            response.context_menu(|ui| {
                if ui.button(format!("Edit {name}…")).clicked() {
                    self.open_precision_editor(
                        studio_mixer_parameter_edit(node, name, spec, revision),
                        hit,
                    );
                    ui.close();
                }
            });
        }
        response.on_hover_text(format!(
            "{}: {}\nDrag to change; double-click resets; right-click for exact entry.",
            name,
            parameter_value(name, value, spec)
        ));
        action
    }
    #[allow(clippy::too_many_arguments)]
    fn studio_mixer_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        patch: &Patch,
        rect: Rect,
        revision: u64,
        interactive: bool,
    ) -> Option<RackAction> {
        let module = module?;
        let interactive = interactive
            && self.shortcuts_enabled
            && !self.annotation_input_guard
            && self.annotation_edit.is_none()
            && self.editing.is_none()
            && ui.is_enabled()
            && ui.input(|input| input.focused);
        let direct_interactive = interactive && !egui::Popup::is_any_open(ui.ctx());
        let n = studio_mixer_channels(node);
        let z = self.zoom;
        let p = |x, y| rect.min + vec2(x, y) * z;
        let painter = ui.painter().clone();
        let context = ui.ctx().clone();
        let font = |s| crate::panel_skin::print_font(&context, s * z);
        let mut action = None;
        for i in 0..n {
            let x = 36. + i as f32 * 72.;
            painter.line_segment(
                [p(x + 36., 73.), p(x + 36., 393.)],
                Stroke::new(0.5 * z, Color32::from_gray(143)),
            );
            let source = patch
                .cables
                .iter()
                .find(|c| c.to == node.id && c.input == format!("in{}", i + 1))
                .map(|c| c.from.replace('_', " "))
                .unwrap_or_else(|| "unpatched".into());
            let header = Rect::from_center_size(p(x, 83.), vec2(68., 23.) * z);
            let caption = studio_mixer_strip_caption(
                &painter,
                i + 1,
                &source,
                font(8.),
                header.width() - 4.0 * z,
            );
            painter.with_clip_rect(header.shrink(2.0 * z)).text(
                header.center(),
                Align2::CENTER_CENTER,
                caption,
                font(8.),
                INK,
            );
            let menu = ui.interact(
                header,
                ui.id().with((&node.id, "strip-settings", i)),
                Sense::click(),
            );
            let own_menu_open = egui::Popup::is_id_open(ui.ctx(), menu.id.with("popup"));
            let menu = menu.on_hover_text(format!("Channel {} / {}\nRight-click for trim, EQ, high-pass and pre/post send settings. Inactive channels keep their settings and wiring.", i + 1, source));
            if interactive && (direct_interactive || own_menu_open) {
                menu.context_menu(|ui| {
                    ui.label(format!("Channel {} / {}", i + 1, source));
                    for prefix in ["trim", "low_db", "high_db", "hpf", "pre_a", "pre_b"] {
                        let key = format!("{prefix}{}", i + 1);
                        let spec = &module["params"][&key];
                        let mut value = node.params.get(&key).copied().unwrap_or(
                            spec["default"].as_f64().unwrap() as f32,
                        );
                        ui.horizontal(|ui| {
                            ui.label(prefix);
                            let speed = if spec["step"] == 1 || prefix == "hpf" {
                                    1.0
                            } else {
                                0.01
                            };
                            let changed = ui.add(
                                egui::DragValue::new(&mut value)
                                    .range(spec["min"].as_f64().unwrap() as f32
                                        ..=spec["max"].as_f64().unwrap() as f32)
                                    .speed(speed),
                            ).changed();
                            if changed {
                                if spec["step"] == 1 {
                                    value = value.round();
                                }
                                action = Some(RackAction::Command(
                                    "set_parameter",
                                    json!({"id":node.id,"parameter":key,"value":value,"expected_revision":revision}),
                                ));
                            }
                        });
                    }
                });
            }
            for (prefix, label, y) in [
                ("send_a", "SEND A", 117.),
                ("send_b", "SEND B", 160.),
                ("pan", "PAN", 203.),
            ] {
                painter.text(p(x, y - 16.), Align2::CENTER_CENTER, label, font(7.), MUTED);
                if let Some(a) = self.console_parameter(
                    ui,
                    node,
                    module,
                    &format!("{prefix}{}", i + 1),
                    p(x, y),
                    11.,
                    false,
                    revision,
                    interactive,
                    true,
                ) {
                    action = Some(a);
                }
            }
            let level = format!("level{}", i + 1);
            if let Some(a) = self.console_parameter(
                ui,
                node,
                module,
                &level,
                p(x - 6., 268.),
                48.,
                true,
                revision,
                interactive,
                true,
            ) {
                action = Some(a);
            }
            let lights = self.module_lights.get(&node.id);
            let brightness = lights.map_or([0.; 2], |l| l.brightness[i]);
            for (ch, brightness) in brightness.iter().enumerate() {
                for segment in 0..14 {
                    let lit = *brightness > (segment as f32 / 14.);
                    let c = if segment >= 12 {
                        Color32::from_rgb(232, 152, 55)
                    } else {
                        Color32::from_rgb(80, 186, 136)
                    };
                    self.lighting.rect(
                        &painter,
                        Rect::from_min_size(
                            p(x + 12. + ch as f32 * 4., 308. - segment as f32 * 5.5),
                            vec2(2.8, 4.) * z,
                        ),
                        0.,
                        c.gamma_multiply(if lit { 1. } else { 0.12 }),
                        if lit { 1. } else { 0. },
                        0.12,
                    );
                }
            }
            for (prefix, label, y) in [("mute", "MUTE", 329.), ("solo", "SOLO", 351.)] {
                let key = format!("{prefix}{}", i + 1);
                let selected = node.params.get(&key).copied().unwrap_or(0.) > 0.5;
                let r = crate::panel_skin::toggle_button(
                    ui,
                    Rect::from_center_size(p(x, y), vec2(51., 18.) * z),
                    label,
                    selected,
                    z,
                );
                if let Some(cue) = self
                    .construction_animation
                    .parameter(&node.id, &node.kind, &key)
                {
                    self.register_construction_anchor(
                        ui,
                        node,
                        &key,
                        crate::construction_motion::Anchor {
                            rect: r.rect,
                            old_text: if cue.old > 0.5 { "ON" } else { "OFF" }.into(),
                            new_text: if cue.new > 0.5 { "ON" } else { "OFF" }.into(),
                            marker: None,
                        },
                        &[r.id],
                    );
                }
                if direct_interactive && r.clicked() {
                    action = Some(RackAction::Command(
                        "set_parameter",
                        json!({"id":node.id,"parameter":key,"value":if selected{0}else{1},"expected_revision":revision}),
                    ));
                }
            }
        }
        let base = n as f32 * 72.;
        for (dx, text, next) in [
            (24., "−", n.saturating_sub(1).max(1)),
            (64., "+", (n + 1).min(16)),
        ] {
            let r = crate::panel_skin::toggle_button(
                ui,
                Rect::from_center_size(p(base + dx, 83.), vec2(32., 21.) * z),
                text,
                false,
                z,
            );
            if direct_interactive && r.clicked() && next != n {
                action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":"channels","value":next,"expected_revision":revision}),
                ));
            }
            r.on_hover_text("1–16 channels. Inactive strips and their jacks are hidden and silent. Existing cables stay in the patch and reappear when expanded.");
        }
        let hidden_cables = patch
            .cables
            .iter()
            .filter(|cable| {
                cable.to == node.id
                    && cable
                        .input
                        .strip_prefix("in")
                        .and_then(|index| index.parse::<usize>().ok())
                        .is_some_and(|index| index > n && index <= 16)
            })
            .count();
        if hidden_cables != 0 {
            painter.text(
                p(base + 152., 99.),
                Align2::CENTER_CENTER,
                format!("{hidden_cables} HIDDEN CABLES"),
                font(7.),
                MUTED,
            );
        }
        painter.text(
            p(base + 152., 83.),
            Align2::CENTER_CENTER,
            format!("{n} CHANNELS"),
            font(10.),
            INK,
        );
        for (name, label, x) in [
            ("master", "MASTER", 36.),
            ("return_a", "RET A", 106.),
            ("return_b", "RET B", 176.),
        ] {
            painter.text(
                p(base + x, 111.),
                Align2::CENTER_CENTER,
                label,
                font(9.),
                INK,
            );
            if let Some(a) = self.console_parameter(
                ui,
                node,
                module,
                name,
                p(base + x, 179.),
                54.,
                true,
                revision,
                interactive,
                true,
            ) {
                action = Some(a);
            }
            let v = node
                .params
                .get(name)
                .copied()
                .unwrap_or(module["params"][name]["default"].as_f64().unwrap() as f32);
            painter.text(
                p(base + x, 244.),
                Align2::CENTER_CENTER,
                if v <= 0. {
                    "−∞ dB".into()
                } else {
                    format!("{:.1} dB", 20. * v.log10())
                },
                font(9.),
                MUTED,
            );
        }
        for (name, label, x) in [
            ("send_a_master", "A MASTER", 106.),
            ("send_b_master", "B MASTER", 176.),
        ] {
            painter.text(
                p(base + x, 258.),
                Align2::CENTER_CENTER,
                label,
                font(7.),
                MUTED,
            );
            if let Some(a) = self.console_parameter(
                ui,
                node,
                module,
                name,
                p(base + x, 276.),
                11.,
                false,
                revision,
                interactive,
                true,
            ) {
                action = Some(a);
            }
        }
        action
    }
}
