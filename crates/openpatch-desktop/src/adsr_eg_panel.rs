// The rack and browser share this panel. Only engine snapshots drive the lamps.
const ADSR_EG_PARAMS: [&str; 8] = [
    "attack",
    "decay",
    "sustain",
    "release",
    "attack_amount",
    "decay_amount",
    "sustain_amount",
    "release_amount",
];
fn adsr_eg_time_edit(edit: &ParameterEdit) -> bool {
    edit.kind == "adsr_eg" && matches!(edit.parameter.as_str(), "attack" | "decay" | "release")
}
fn adsr_eg_knob_rect(index: usize) -> Rect {
    Rect::from_center_size(
        pos2(
            58.0 + (index % 4) as f32 * 80.0,
            if index < 4 { 173.0 } else { 247.0 },
        ),
        Vec2::splat(if index < 4 { 62.0 } else { 46.0 }),
    )
}
fn adsr_eg_value_rect(index: usize) -> Rect {
    Rect::from_center_size(
        pos2(
            58.0 + (index % 4) as f32 * 80.0,
            if index < 4 { 213.0 } else { 286.0 },
        ),
        vec2(70.0, if index < 4 { 16.0 } else { 18.0 }),
    )
}
fn adsr_eg_push_rect() -> Rect {
    Rect::from_center_size(pos2(222.0, 367.0), vec2(66.0, 32.0))
}
fn adsr_eg_parameter_value(index: usize, raw: f32) -> String {
    if index < 4 && index != 2 {
        format!("{:.0} ms", 10000.0_f32.powf(raw))
    } else if index >= 4 {
        format!("{:+.0}%", raw * 100.0)
    } else {
        format!("{:.0}%", raw * 100.0)
    }
}
fn adsr_eg_edit(node: &Node, index: usize, revision: u64) -> ParameterEdit {
    let name = ADSR_EG_PARAMS[index];
    let raw = node
        .params
        .get(name)
        .copied()
        .unwrap_or(if index < 4 { 0.5 } else { 0.0 });
    let time = index < 4 && index != 2;
    ParameterEdit {
        id: node.id.clone(),
        kind: node.kind.clone(),
        parameter: name.into(),
        revision,
        surface: ParameterEditSurface::Window,
        menu_popup: None,
        stale: false,
        text: if time {
            format!("{:.3}", 10000.0_f32.powf(raw))
        } else {
            format!("{:.2}", raw * 100.0)
        },
        min: if time {
            1.0
        } else if index < 4 {
            0.0
        } else {
            -100.0
        },
        max: if time { 10000.0 } else { 100.0 },
        step: None,
        display_scale: if time { 1.0 } else { 100.0 },
        display_unit: if time { "ms" } else { "%" }.into(),
        slew_min: None,
        gain_power: None,
        raw_max: 1.0,
        focus: true,
    }
}

// Configured continuous-time approximation of the source law. This is a guide,
// not a sampled engine trace or a live envelope display.
fn paint_adsr_eg_guide(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    node: &Node,
    rect: Rect,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    let p = |x: f32, y: f32| rect.min + vec2(x, y) * zoom;
    let start = crate::lighting::next(painter);
    let layout = AtlasVoiceLayout {
        width: rect.width() / zoom,
    };
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout,
        zoom,
    };
    let sx = 420.0 / layout.width;
    let sy = 720.0 / PANEL_HEIGHT;
    draw.screen(
        draw.rect(32.0 * sx, 77.0 * sy, 296.0 * sx, 49.5 * sy),
        lighting,
    );
    let screen_rect = Rect::from_min_max(p(32.0, 77.0), p(328.0, 126.5));
    ui.interact(
        screen_rect,
        ui.id().with(("adsr-eg-guide", &node.id)),
        Sense::hover(),
    )
    .on_hover_text("Configured continuous-time approximation of the Stock ADSR exponential law. The envelope is not sampled from the engine; the DSP uses a per-sample forward-Euler update. Time is normalized in this guide.");
    painter.text(
        p(180.0, 84.0),
        Align2::CENTER_CENTER,
        "CONFIGURED SHAPE · TIME NORMALIZED",
        crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
        Color32::from_rgb(195, 200, 180),
    );
    let x0 = 40.0;
    let top = 101.0;
    let bottom = 122.0;
    let y = |level: f32| bottom - level.clamp(0.0, 1.2) / 1.2 * (bottom - top);
    for level in [0.0, 1.0, 1.2] {
        painter.line_segment(
            [p(x0, y(level)), p(145.0, y(level))],
            Stroke::new(
                0.45 * zoom,
                Color32::from_rgb(if level == 1.2 { 108 } else { 72 }, 83, 67),
            ),
        );
    }
    let sustain = node
        .params
        .get("sustain")
        .copied()
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    painter.text(
        p(44.0, 94.0),
        Align2::LEFT_CENTER,
        "12 V TARGET",
        crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
        Color32::from_rgb(169, 177, 153),
    );
    let mut curve = Vec::with_capacity(37);
    // Attack reaches the 1.0 transition at ln(6) time constants; decay and
    // release approach their targets. The display is continuous time, unlike
    // the engine's per-sample forward-Euler update.
    for i in 0..=8 {
        let t = i as f32 / 8.0;
        let level = 1.2 * (1.0 - (-1.7917595 * t).exp());
        curve.push(p(x0 + 25.0 * t, y(level)));
    }
    for i in 1..=8 {
        let t = i as f32 / 8.0;
        let level = sustain + (1.0 - sustain) * (-4.60517 * t).exp();
        curve.push(p(x0 + 25.0 + 23.0 * t, y(level)));
    }
    let sustain_y = y(sustain);
    curve.push(p(x0 + 48.0, sustain_y));
    curve.push(p(x0 + 68.0, sustain_y));
    for i in 1..=8 {
        let t = i as f32 / 8.0;
        curve.push(p(x0 + 68.0 + 37.0 * t, y(sustain * (-4.60517 * t).exp())));
    }
    painter.add(egui::Shape::line(
        curve,
        Stroke::new(1.5 * zoom, Color32::from_rgb(226, 181, 96)),
    ));
    // Four CV inputs share the same visible processing chain before the
    // corresponding stage coefficient/level is clamped to its legal range.
    let y0 = 109.0;
    let boxes = [
        (153.0, 191.0, "A/D/S/R CV"),
        (197.0, 250.0, "CV/10 × DEPTH"),
        (256.0, 292.0, "+ BASE"),
        (298.0, 325.0, "0–1"),
    ];
    for (left, right, label) in boxes {
        let box_rect = Rect::from_min_max(p(left, 100.0), p(right, 118.0));
        painter.rect_filled(box_rect, 1.2 * zoom, Color32::from_rgb(48, 57, 44));
        painter.rect_stroke(
            box_rect,
            1.2 * zoom,
            Stroke::new(0.6 * zoom, Color32::from_rgb(133, 143, 123)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            p((left + right) * 0.5, y0),
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
            Color32::from_rgb(195, 200, 180),
        );
    }
    for x in [194.0, 253.0, 295.0] {
        painter.line_segment(
            [p(x - 2.0, y0), p(x + 2.0, y0)],
            Stroke::new(0.8 * zoom, Color32::from_rgb(226, 181, 96)),
        );
        painter.add(egui::Shape::convex_polygon(
            vec![p(x + 2.5, y0), p(x - 1.5, y0 - 2.0), p(x - 1.5, y0 + 2.0)],
            Color32::from_rgb(226, 181, 96),
            Stroke::NONE,
        ));
    }
    lighting.preserve(painter, start);
}

impl Rack {
    fn adsr_eg_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
        allow_press: bool,
    ) -> Option<RackAction> {
        let z = self.zoom;
        let p = |x, y| rect.min + vec2(x, y) * z;
        let screen = |r: Rect| Rect::from_min_max(p(r.min.x, r.min.y), p(r.max.x, r.max.y));
        let painter = ui.painter().clone();
        let ctx = ui.ctx().clone();
        let font = |size| crate::panel_skin::print_font(&ctx, size * z);
        let mut action = None;
        // Stage LEDs are aggregate DSP state across polyphonic lanes; no clock,
        // button state or knob position is used to infer an active stage.
        for (i, label) in ["ATTACK", "DECAY", "SUSTAIN", "RELEASE"].iter().enumerate() {
            let x = 58.0 + i as f32 * 80.0;
            let center = p(x - 23.0, 64.0);
            self.adsr_eg_lamp(&painter, node, i, center, z);
            painter.text(
                p(x + 3.0, 64.0),
                Align2::CENTER_CENTER,
                label,
                font(9.0),
                INK,
            );
        }
        paint_adsr_eg_guide(ui, &painter, node, rect, z, &self.lighting);
        for (i, name) in ADSR_EG_PARAMS.iter().enumerate() {
            let depth = i >= 4;
            let center_x = 58.0 + (i % 4) as f32 * 80.0;
            painter.text(
                p(center_x, if depth { 273.0 } else { 132.5 }),
                Align2::CENTER_CENTER,
                if depth {
                    "CV DEPTH ±100%"
                } else if i == 2 {
                    "0  50  100%"
                } else {
                    "1  100  10k ms"
                },
                font(if depth { 7.0 } else { 6.5 }),
                MUTED,
            );
            let min = if depth { -1.0 } else { 0.0 };
            let raw = node
                .params
                .get(*name)
                .copied()
                .unwrap_or(if depth { 0.0 } else { 0.5 })
                .clamp(min, 1.0);
            let normalized = if depth { (raw + 1.0) * 0.5 } else { raw };
            let knob_rect = screen(adsr_eg_knob_rect(i));
            let knob = ui.interact(
                knob_rect,
                ui.id().with(("adsr-eg-knob", &node.id, name)),
                Sense::click_and_drag(),
            );
            knob.widget_info(|| {
                egui::WidgetInfo::slider(
                    ui.is_enabled(),
                    f64::from(raw),
                    if depth {
                        format!("{} CV depth", parameter_label(ADSR_EG_PARAMS[i - 4]))
                    } else {
                        format!("{} envelope control", parameter_label(name))
                    },
                )
            });
            let input_enabled = ui.is_enabled()
                && self.shortcuts_enabled
                && self.editing.is_none()
                && self.annotation_edit.is_none()
                && !egui::Popup::is_any_open(ui.ctx());
            let input = self.parameter_knob_input(
                ui,
                &knob,
                normalized,
                input_enabled,
                &adsr_eg_parameter_value(i, raw),
            );
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
                Some(if depth { 0.0 } else { 0.5 })
            } else if let Some(high) = input.boundary {
                Some(if high { 1.0 } else { min })
            } else if input.nudge != 0 {
                let n = (normalized + input.nudge as f32 * keyboard_step).clamp(0.0, 1.0);
                Some(if depth { n * 2.0 - 1.0 } else { n })
            } else {
                input
                    .normalized
                    .map(|n| if depth { n * 2.0 - 1.0 } else { n })
            };
            if let Some(next) = next.filter(|next| *next != raw) {
                let mut args = json!({"id":node.id,"parameter":name,"value":next,"expected_revision":revision});
                if let Some(group) = input.history_group {
                    args["history_group"] = json!(group);
                }
                action = Some(RackAction::Command("set_parameter", args));
            }
            if input_enabled && knob.secondary_clicked() {
                self.open_precision_editor(
                    adsr_eg_edit(node, i, revision),
                    screen(adsr_eg_value_rect(i)),
                );
            }
            knob.clone().on_hover_cursor(egui::CursorIcon::ResizeVertical).on_hover_text(if depth { "Bipolar CV depth: ±100%. Effective stage value adds CV / 10 × depth, then clamps to 0–100%. Drag vertically; Ctrl/Cmd for fine adjustment. Double-click resets to 0%." } else if i == 2 { "Sustain level as a percentage of 10 V. Drag vertically; Ctrl/Cmd for fine adjustment. Double-click resets to 50%." } else { "Time constant, 1–10,000 ms. 100 ms at midpoint. Attack time to peak is longer. Drag vertically; Ctrl/Cmd for fine adjustment. Double-click resets to 100 ms." });
            paint_atlas_voice_rotary(
                &painter,
                knob_rect.center(),
                normalized,
                knob.hovered() || knob.dragged() || knob.has_focus(),
                if depth { 20.0 } else { 28.0 },
                z,
            );
            let value_rect = screen(adsr_eg_value_rect(i));
            let label = adsr_eg_parameter_value(i, raw);
            paint_atlas_voice_readout(&painter, value_rect.center(), &label, value_rect.width(), z);
            let value = ui.interact(
                value_rect,
                ui.id().with((&node.id, "adsr-eg-value", i)),
                Sense::click(),
            );
            value.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    format!("Edit {} {}: {label}", node.id, name),
                )
            });
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
                        old_text: adsr_eg_parameter_value(i, cue.old),
                        new_text: label.clone(),
                        marker: Some(crate::construction_motion::Marker::Rotary {
                            center: knob_rect.center(),
                            radius: (if depth { 23.0 } else { 31.0 }) * z,
                            old: (cue.old.clamp(min, 1.0) - min) / (1.0 - min),
                            new: normalized,
                        }),
                    },
                    &[knob.id, value.id],
                );
            }
            if self.precision_collects(node) {
                self.register_precision_control(adsr_eg_edit(node, i, revision), value_rect);
            }
            self.precision_affordance(ui, &value);
            if value.clicked()
                || value.secondary_clicked()
                || self.take_precision_request(&node.id, name)
            {
                consume_parameter_activation(ui.ctx());
                self.open_precision_editor(adsr_eg_edit(node, i, revision), value_rect);
            }
            value.on_hover_text("Click for precise human-unit entry and expressions. Tab moves through all eight controls.");
        }
        painter.line_segment(
            [p(22.0, 329.0), p(338.0, 329.0)],
            Stroke::new(0.6 * z, Color32::from_rgb(166, 164, 157)),
        );
        painter.text(
            p(180.0, 337.0),
            Align2::CENTER_CENTER,
            "GATE ≥ 1 V  ·  ENV 0–10 V NOMINAL",
            font(8.0),
            MUTED,
        );
        let enabled = ui.is_enabled()
            && self.shortcuts_enabled
            && self.performance_available
            && !node.bypassed
            && ui.input(|i| i.focused);
        let button_rect = screen(adsr_eg_push_rect());
        let response = ui.interact(
            button_rect,
            ui.id().with((&node.id, "adsr-eg-push")),
            Sense::click_and_drag(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                enabled,
                format!("ADSR EG {} held PUSH", node.id),
            )
        });
        let physical = self
            .module_controls
            .get(&node.id)
            .is_some_and(|s| s.pressed_mask & 1 != 0);
        painter.rect_filled(
            button_rect.translate(vec2(0.8, 1.5) * z),
            3.0 * z,
            Color32::from_black_alpha(70),
        );
        painter.rect_filled(
            button_rect,
            3.0 * z,
            if physical {
                Color32::from_rgb(226, 197, 143)
            } else {
                Color32::from_rgb(190, 186, 175)
            },
        );
        painter.rect_stroke(
            button_rect,
            3.0 * z,
            Stroke::new(z, MUTED),
            egui::StrokeKind::Inside,
        );
        painter.text(
            button_rect.center(),
            Align2::CENTER_CENTER,
            "PUSH",
            font(11.0),
            if enabled { INK } else { MUTED },
        );
        self.adsr_eg_lamp(&painter, node, 4, p(267.0, 367.0), z);
        if enabled
            && allow_press
            && self.held_manual.is_none()
            && self.editing.is_none()
            && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
            && (response.clicked_by(PointerButton::Primary) || response.is_pointer_button_down_on())
        {
            self.select_primary(node.id.clone());
            if response.clicked_by(PointerButton::Primary) {
                // The app schedules a guarded release on a later UI frame.
                action = Some(RackAction::ManualClick {
                    id: node.id.clone(),
                    press: "push_press",
                    release: "push_release",
                });
            } else {
                self.held_manual = Some(HeldManual {
                    id: node.id.clone(),
                    kind: node.kind.clone(),
                    release: "push_release",
                    button_rect,
                });
                action = Some(RackAction::Command(
                    "trigger_module",
                    json!({"id":node.id,"action":"push_press"}),
                ));
            }
        }
        response.on_hover_text("Hold to gate every processed lane without changing polyphony. The adjacent lamp shows any engine gate, including external GATE. Moving outside, focus loss or bypass releases PUSH.");
        action
    }
    fn adsr_eg_lamp(
        &self,
        painter: &egui::Painter,
        node: &Node,
        index: usize,
        center: Pos2,
        zoom: f32,
    ) {
        let brightness = self
            .module_lights
            .get(&node.id)
            .filter(|s| index < usize::from(s.count))
            .map_or(0.0, |s| s.brightness[index][0])
            .clamp(0.0, 1.0);
        painter.circle_filled(center, 5.5 * zoom, Color32::from_rgb(71, 66, 58));
        self.lighting.circle(
            painter,
            center,
            2.4 * zoom,
            Color32::from_rgb(238, 177, 65).gamma_multiply(0.12 + 0.88 * brightness),
            brightness,
            0.12,
        );
    }
}
