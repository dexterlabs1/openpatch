// Fundamental Process exposes one durable logarithmic slew control and one
// runtime momentary gate. All other markings describe actual output roles.
const STOCK_PROCESS_MIN_SLEW: f32 = -9.965_784;
const STOCK_PROCESS_MAX_SLEW: f32 = std::f32::consts::LOG2_10;
const STOCK_PROCESS_KNOB: Pos2 = pos2(76.0, 126.0);
const STOCK_PROCESS_GATE: Pos2 = pos2(239.0, 126.0);

fn stock_process_ms_per_volt(raw: f32) -> f32 {
    if raw <= STOCK_PROCESS_MIN_SLEW {
        0.0
    } else {
        1000.0 * raw.exp2()
    }
}

fn stock_process_ms_label(raw: f32) -> String {
    let ms = stock_process_ms_per_volt(raw);
    if ms == 0.0 {
        "0 ms/V".into()
    } else if ms < 10.0 {
        format!("{ms:.2} ms/V")
    } else if ms < 1000.0 {
        format!("{ms:.1} ms/V")
    } else {
        format!("{ms:.0} ms/V")
    }
}

fn stock_process_edit(node: &Node, revision: u64) -> ParameterEdit {
    let raw = node
        .params
        .get("slew")
        .copied()
        .unwrap_or(STOCK_PROCESS_MIN_SLEW);
    ParameterEdit {
        id: node.id.clone(),
        kind: node.kind.clone(),
        parameter: "slew".into(),
        revision,
        surface: ParameterEditSurface::Window,
        menu_popup: None,
        stale: false,
        text: stock_process_ms_per_volt(raw).to_string(),
        min: 0.0,
        max: 10_000.0,
        step: None,
        display_scale: 1.0,
        display_unit: "ms/V".into(),
        slew_min: Some(STOCK_PROCESS_MIN_SLEW),
        gain_power: None,
        raw_max: STOCK_PROCESS_MAX_SLEW,
        focus: true,
    }
}

fn stock_process_port_label(port: &str) -> &str {
    match port {
        "slew_cv" => "SLEW CV",
        "signal" => "SIGNAL",
        "gate" => "GATE CV",
        "sh1" => "S/H 1",
        "sh2" => "S/H 2",
        "track_hold" => "TRACK / HOLD",
        "hold_track" => "HOLD / TRACK",
        "slew" => "SLEW",
        "glide" => "GLIDE",
        other => other,
    }
}

fn stock_process_port_position(input: bool, index: usize) -> Pos2 {
    let x = [53.0, 150.0, 247.0][index % 3];
    pos2(
        x,
        if input {
            269.0
        } else if index < 3 {
            323.0
        } else {
            376.0
        },
    )
}

fn paint_stock_process_guide(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    node: &Node,
    rect: Rect,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    let origin = rect.min;
    let point = |x: f32, y: f32| origin + vec2(x, y) * zoom;
    let bounds = Rect::from_min_max(point(15.0, 202.0), point(285.0, 238.0));
    let screen = AtlasVoicePaint {
        painter,
        origin,
        layout: AtlasVoiceLayout { width: 420.0 },
        zoom,
    };
    let start = crate::lighting::next(painter);
    screen.screen(bounds, lighting);
    let ink = Color32::from_rgb(195, 200, 180);
    let amber = Color32::from_rgb(229, 183, 103);
    let quiet = Color32::from_rgb(133, 143, 123);
    let text = |x: f32, y: f32, label: &str, size: f32, color: Color32| {
        painter.text(
            point(x, y),
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    };
    let line = |coords: &[(f32, f32)], color: Color32, width: f32| {
        painter.add(egui::Shape::line(
            coords.iter().map(|&(x, y)| point(x, y)).collect(),
            Stroke::new(width * zoom, color),
        ));
    };
    // The guide shows configured signal paths only; it does not sample audio.
    text(57.0, 207.0, "S/H EDGE", 8.0, ink);
    text(150.0, 207.0, "TRACK / HOLD", 8.0, ink);
    text(243.0, 207.0, "SLEW / GLIDE", 8.0, ink);
    painter.line_segment(
        [point(104.0, 211.0), point(104.0, 234.0)],
        Stroke::new(0.5 * zoom, quiet),
    );
    painter.line_segment(
        [point(196.0, 211.0), point(196.0, 234.0)],
        Stroke::new(0.5 * zoom, quiet),
    );

    // A rising edge stores the current signal and shifts the old capture.
    line(
        &[
            (22.0, 229.0),
            (33.0, 229.0),
            (33.0, 222.0),
            (44.0, 222.0),
            (44.0, 229.0),
            (57.0, 229.0),
        ],
        quiet,
        0.8,
    );
    line(
        &[
            (57.0, 229.0),
            (57.0, 221.0),
            (78.0, 221.0),
            (78.0, 216.0),
            (93.0, 216.0),
        ],
        amber,
        1.5,
    );
    text(58.0, 234.0, "NOW → 1   /   PREV → 2", 6.5, ink);

    // Opposed switches convey the reciprocal track/hold outputs.
    text(127.0, 216.0, "GATE", 6.0, quiet);
    painter.circle_filled(point(148.0, 216.0), 2.0 * zoom, amber);
    line(
        &[(148.0, 216.0), (162.0, 222.0), (182.0, 222.0)],
        amber,
        1.0,
    );
    line(
        &[(148.0, 216.0), (162.0, 216.0), (182.0, 216.0)],
        quiet,
        0.8,
    );
    text(164.0, 229.0, "HIGH: TH HOLDS  /  HT TRACKS", 5.8, ink);
    text(164.0, 234.5, "LOW: TH TRACKS  /  HT HOLDS", 5.8, ink);

    // SlewFilter clips change to a fixed increment: linear ramp, not RC decay.
    // The guide uses normalized time and zero CV, with the current bypass detent.
    let raw = node
        .params
        .get("slew")
        .copied()
        .unwrap_or(STOCK_PROCESS_MIN_SLEW);
    let limited = raw.is_finite() && raw > STOCK_PROCESS_MIN_SLEW;
    let step = [
        (209.0, 231.0),
        (218.0, 231.0),
        (218.0, 218.0),
        (239.0, 218.0),
    ];
    line(&step, quiet, 0.8);
    if limited {
        line(
            &[
                (209.0, 231.0),
                (218.0, 231.0),
                (236.0, 218.0),
                (239.0, 218.0),
            ],
            amber,
            1.3,
        );
    } else {
        line(&step, amber, 1.3);
    }
    text(
        259.0,
        226.0,
        if limited { "LIMITED" } else { "INSTANT" },
        6.0,
        ink,
    );
    lighting.preserve(painter, start);
    ui.interact(bounds, ui.id().with(("stock-process-configured-guide",&node.id)), Sense::hover())
        .on_hover_text("Signal behavior guide, not live samples. The step illustration uses normalized time and zero CV; the hard-left setting is instantaneous, otherwise a linear ramp illustrates the limiting phase of SLEW/GLIDE. A rising GATE CV edge at ≥2 V enters gate state: S/H 1 captures SIGNAL and S/H 2 receives the prior S/H 1 capture. Gate CV ≤0.1 V exits gate state; values between thresholds retain state. TRACK / HOLD outputs the rise-captured value while high, then follows SIGNAL while low. HOLD / TRACK follows SIGNAL while high, then holds the value at the falling edge while low. SLEW follows SIGNAL directly while high and limits changes while low. GLIDE follows directly until gate state has lasted more than 1 ms, then limits changes; it follows directly again when gate state ends. PRESS also forces gate state. SLEW CV adds to log2 seconds per volt, so +1 V doubles the time. Hard-left 0 ms/V is instantaneous and ignores CV; positive times must exceed 1 ms/V.");
}

impl Rack {
    fn stock_process_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
        allow_press: bool,
        interactive: bool,
    ) -> Option<RackAction> {
        let z = self.zoom;
        let point = |p: Pos2| rect.min + p.to_vec2() * z;
        let screen = |r: Rect| Rect::from_min_max(point(r.min), point(r.max));
        let painter = ui.painter().clone();
        let ctx = ui.ctx().clone();
        let font = |size| crate::panel_skin::print_font(&ctx, size * z);
        let ink = crate::panel_skin::INK;
        let muted = Color32::from_rgb(92, 89, 80);
        let control_enabled = interactive
            && self.shortcuts_enabled
            && !self.annotation_input_guard
            && self.annotation_edit.is_none()
            && self.editing.is_none()
            && ui.is_enabled()
            && ui.input(|input| input.focused)
            && !egui::Popup::is_any_open(ui.ctx());
        let raw = node
            .params
            .get("slew")
            .copied()
            .unwrap_or(STOCK_PROCESS_MIN_SLEW);
        let bounded = raw.clamp(STOCK_PROCESS_MIN_SLEW, STOCK_PROCESS_MAX_SLEW);
        let normalized =
            (bounded - STOCK_PROCESS_MIN_SLEW) / (STOCK_PROCESS_MAX_SLEW - STOCK_PROCESS_MIN_SLEW);

        painter.text(
            point(pos2(76.0, 73.0)),
            Align2::CENTER_CENTER,
            "SLEW TIME",
            font(12.0),
            ink,
        );
        painter.text(
            point(pos2(239.0, 73.0)),
            Align2::CENTER_CENTER,
            "MANUAL GATE",
            font(12.0),
            ink,
        );
        let knob_rect = screen(Rect::from_center_size(STOCK_PROCESS_KNOB, vec2(68.0, 68.0)));
        let knob = ui.interact(
            knob_rect,
            ui.id().with((&node.id, "stock-process-slew")),
            Sense::click_and_drag(),
        );
        knob.widget_info(|| {
            egui::WidgetInfo::slider(
                control_enabled,
                f64::from(stock_process_ms_per_volt(raw)),
                format!("{} slew milliseconds per volt", node.id),
            )
        });
        if knob.drag_started_by(PointerButton::Primary) || knob.clicked_by(PointerButton::Primary) {
            self.select_primary(node.id.clone());
        }
        let input = self.parameter_knob_input(
            ui,
            &knob,
            normalized,
            control_enabled,
            &stock_process_ms_label(raw),
        );
        let span = STOCK_PROCESS_MAX_SLEW - STOCK_PROCESS_MIN_SLEW;
        let proposed = input
            .reset
            .then_some(STOCK_PROCESS_MIN_SLEW)
            .or_else(|| {
                input
                    .normalized
                    .map(|value| STOCK_PROCESS_MIN_SLEW + value * span)
            })
            .or_else(|| {
                input.boundary.map(|high| {
                    if high {
                        STOCK_PROCESS_MAX_SLEW
                    } else {
                        STOCK_PROCESS_MIN_SLEW
                    }
                })
            })
            .or_else(|| {
                (input.nudge != 0).then(|| {
                    let step = if ui.input(|i| i.modifiers.shift) {
                        span * 0.1
                    } else {
                        span * 0.01
                    };
                    (bounded + input.nudge as f32 * step)
                        .clamp(STOCK_PROCESS_MIN_SLEW, STOCK_PROCESS_MAX_SLEW)
                })
            });
        let mut action = proposed.filter(|value| *value != raw).map(|value| {
            self.select_primary(node.id.clone());
            let mut args =
                json!({"id":node.id,"parameter":"slew","value":value,"expected_revision":revision});
            if let Some(group) = input.history_group {
                args["history_group"] = json!(group);
            }
            RackAction::Command("set_parameter", args)
        });
        if let Some(cue) = self
            .construction_animation
            .parameter(&node.id, &node.kind, "slew")
        {
            self.register_construction_anchor(
                ui,
                node,
                "slew",
                crate::construction_motion::Anchor {
                    rect: knob.rect,
                    old_text: stock_process_ms_label(cue.old),
                    new_text: stock_process_ms_label(cue.new),
                    marker: Some(crate::construction_motion::Marker::Rotary {
                        center: point(STOCK_PROCESS_KNOB),
                        radius: 31.0 * z,
                        old: ((cue.old - STOCK_PROCESS_MIN_SLEW) / span).clamp(0.0, 1.0),
                        new: ((cue.new - STOCK_PROCESS_MIN_SLEW) / span).clamp(0.0, 1.0),
                    }),
                },
                &[knob.id],
            );
        }
        paint_atlas_voice_rotary(
            &painter,
            point(STOCK_PROCESS_KNOB),
            normalized,
            knob.hovered() || knob.dragged() || knob.has_focus(),
            31.0,
            z,
        );
        knob.on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text("Logarithmic slew time. Hard left is instantaneous and ignores SLEW CV; each +1 V of CV doubles the time elsewhere. Drag, use arrow/Home/End keys, or click the value for exact milliseconds per volt.");
        let value_rect = screen(Rect::from_center_size(pos2(76.0, 174.0), vec2(116.0, 25.0)));
        let value = ui.interact(
            value_rect,
            ui.id().with((&node.id, "stock-process-slew-value")),
            Sense::click(),
        );
        paint_atlas_voice_readout(
            &painter,
            value_rect.center(),
            &stock_process_ms_label(raw),
            value_rect.width(),
            z,
        );
        if self.precision_collects(node) {
            self.register_precision_control(stock_process_edit(node, revision), value_rect);
        }
        self.precision_affordance(ui, &value);
        if control_enabled
            && (value.clicked()
                || value.secondary_clicked()
                || self.take_precision_request(&node.id, "slew"))
        {
            consume_parameter_activation(ui.ctx());
            self.open_precision_editor(stock_process_edit(node, revision), value_rect);
        }
        value.on_hover_text("Exact slew time in ms/V. 0 is instantaneous; positive times must exceed 1 ms/V. Saved values beyond knob travel remain unchanged until edited.");
        if raw.is_finite() && !(STOCK_PROCESS_MIN_SLEW..=STOCK_PROCESS_MAX_SLEW).contains(&raw) {
            painter.text(
                point(pos2(137.0, 174.0)),
                Align2::CENTER_CENTER,
                "!",
                font(12.0),
                Color32::from_rgb(156, 62, 43),
            );
        }

        let gate_rect = screen(Rect::from_center_size(
            STOCK_PROCESS_GATE,
            vec2(105.0, 44.0),
        ));
        let gate = ui.interact(
            gate_rect,
            ui.id().with((&node.id, "stock-process-gate")),
            Sense::click_and_drag(),
        );
        let lamp = self
            .module_lights
            .get(&node.id)
            .filter(|state| state.count == 1 && state.brightness[0][0].is_finite())
            .map(|state| state.brightness[0][0].clamp(0.0, 1.0));
        let enabled = control_enabled && self.performance_available && !node.bypassed;
        let pressed = self
            .module_controls
            .get(&node.id)
            .map(|state| state.pressed)
            .unwrap_or(false);
        gate.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                enabled,
                format!("{} momentary gate", node.id),
            )
        });
        painter.rect_filled(
            gate_rect,
            5.0 * z,
            if pressed {
                Color32::from_rgb(53, 55, 50)
            } else if enabled && gate.hovered() {
                Color32::from_rgb(245, 241, 229)
            } else {
                Color32::from_rgb(218, 213, 202)
            },
        );
        painter.rect_stroke(
            gate_rect,
            5.0 * z,
            Stroke::new(1.3 * z, ink),
            egui::StrokeKind::Inside,
        );
        painter.text(
            gate_rect.center(),
            Align2::CENTER_CENTER,
            "PRESS",
            font(14.0),
            if pressed {
                Color32::from_rgb(246, 239, 221)
            } else {
                ink
            },
        );
        let lamp_center = point(pos2(239.0, 174.0));
        painter.circle_filled(lamp_center, 7.0 * z, Color32::from_rgb(48, 49, 45));
        self.lighting.circle(
            &painter,
            lamp_center,
            2.8 * z,
            Color32::from_rgb(238, 177, 65).gamma_multiply(0.12 + lamp.unwrap_or(0.0) * 0.88),
            lamp.unwrap_or(0.0),
            0.12,
        );
        painter.text(
            point(pos2(239.0, 191.0)),
            Align2::CENTER_CENTER,
            if lamp.is_some() {
                "BUTTON STATE"
            } else {
                "STATE UNAVAILABLE"
            },
            font(9.0),
            muted,
        );
        if enabled
            && allow_press
            && self.held_manual.is_none()
            && gate.is_pointer_button_down_on()
            && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
        {
            self.held_manual = Some(HeldManual {
                id: node.id.clone(),
                kind: node.kind.clone(),
                release: "gate_release",
                button_rect: gate_rect,
            });
            self.select_primary(node.id.clone());
            action = Some(RackAction::Command(
                "trigger_module",
                json!({"id":node.id,"action":"gate_press"}),
            ));
        }
        gate.on_hover_text(if enabled {
            "Hold to force gate high. Release, focus loss, or bypass ends the press. This lamp is the actual manual button light; gate CV does not light it."
        } else {
            "Momentary gate requires a live audio engine and an active module."
        });

        paint_stock_process_guide(ui, &painter, node, rect, z, &self.lighting);
        for (index, label) in ["SLEW CV", "SIGNAL", "GATE CV"].into_iter().enumerate() {
            painter.text(
                point(pos2(stock_process_port_position(true, index).x, 244.0)),
                Align2::CENTER_CENTER,
                label,
                font(10.5),
                ink,
            );
        }
        for (index, label) in [
            "S/H 1",
            "S/H 2",
            "TRACK / HOLD",
            "HOLD / TRACK",
            "SLEW",
            "GLIDE",
        ]
        .into_iter()
        .enumerate()
        {
            let port = stock_process_port_position(false, index);
            painter.text(
                point(pos2(port.x, port.y - 25.0)),
                Align2::CENTER_CENTER,
                label,
                font(10.5),
                ink,
            );
        }
        action
    }
}
