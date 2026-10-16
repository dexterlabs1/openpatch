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
    let raw = node.params.get("slew").copied().unwrap_or(STOCK_PROCESS_MIN_SLEW);
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
    pos2(x, if input { 269.0 } else if index < 3 { 323.0 } else { 376.0 })
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
        let raw = node.params.get("slew").copied().unwrap_or(STOCK_PROCESS_MIN_SLEW);
        let bounded = raw.clamp(STOCK_PROCESS_MIN_SLEW, STOCK_PROCESS_MAX_SLEW);
        let normalized = (bounded - STOCK_PROCESS_MIN_SLEW)
            / (STOCK_PROCESS_MAX_SLEW - STOCK_PROCESS_MIN_SLEW);

        painter.text(point(pos2(76.0, 73.0)), Align2::CENTER_CENTER,
            "SLEW TIME", font(12.0), ink);
        painter.text(point(pos2(239.0, 73.0)), Align2::CENTER_CENTER,
            "MANUAL GATE", font(12.0), ink);
        let knob_rect = screen(Rect::from_center_size(STOCK_PROCESS_KNOB, vec2(68.0, 68.0)));
        let knob = ui.interact(knob_rect, ui.id().with((&node.id, "stock-process-slew")),
            Sense::click_and_drag());
        knob.widget_info(|| egui::WidgetInfo::slider(
            control_enabled, f64::from(stock_process_ms_per_volt(raw)),
            format!("{} slew milliseconds per volt", node.id)));
        if knob.drag_started_by(PointerButton::Primary) || knob.clicked_by(PointerButton::Primary) {
            self.select_primary(node.id.clone());
        }
        let input = self.parameter_knob_input(ui, &knob, normalized, control_enabled);
        let span = STOCK_PROCESS_MAX_SLEW - STOCK_PROCESS_MIN_SLEW;
        let proposed = input.reset.then_some(STOCK_PROCESS_MIN_SLEW)
            .or_else(|| input.normalized.map(|value| STOCK_PROCESS_MIN_SLEW + value * span))
            .or_else(|| input.boundary.map(|high| if high { STOCK_PROCESS_MAX_SLEW } else { STOCK_PROCESS_MIN_SLEW }))
            .or_else(|| (input.nudge != 0).then(|| {
                let step = if ui.input(|i| i.modifiers.shift) { span * 0.1 } else { span * 0.01 };
                (bounded + input.nudge as f32 * step).clamp(STOCK_PROCESS_MIN_SLEW, STOCK_PROCESS_MAX_SLEW)
            }));
        let mut action = proposed.filter(|value| *value != raw).map(|value| {
            self.select_primary(node.id.clone());
            let mut args = json!({"id":node.id,"parameter":"slew","value":value,"expected_revision":revision});
            if let Some(group) = input.history_group { args["history_group"] = json!(group); }
            RackAction::Command("set_parameter", args)
        });
        if let Some(cue) = self.construction_animation.parameter(&node.id, &node.kind, "slew") {
            self.register_construction_anchor(ui, node, "slew",
                crate::construction_motion::Anchor {
                    rect: knob.rect,
                    old_text: stock_process_ms_label(cue.old),
                    new_text: stock_process_ms_label(cue.new),
                    marker: Some(crate::construction_motion::Marker::Rotary {
                        center: point(STOCK_PROCESS_KNOB), radius: 31.0 * z,
                        old: ((cue.old - STOCK_PROCESS_MIN_SLEW) / span).clamp(0.0, 1.0),
                        new: ((cue.new - STOCK_PROCESS_MIN_SLEW) / span).clamp(0.0, 1.0),
                    }),
                }, &[knob.id]);
        }
        stock_oscillator_rotary(&painter, point(STOCK_PROCESS_KNOB), normalized,
            knob.hovered() || knob.dragged() || knob.has_focus(), z);
        knob.on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text("Logarithmic slew time. Hard left is instantaneous and ignores SLEW CV; each +1 V of CV doubles the time elsewhere. Drag, use arrow/Home/End keys, or click the value for exact milliseconds per volt.");
        let value_rect = screen(Rect::from_center_size(pos2(76.0, 174.0), vec2(116.0, 25.0)));
        let value = ui.interact(value_rect, ui.id().with((&node.id, "stock-process-slew-value")), Sense::click());
        painter.rect_filled(value_rect, 3.0 * z, Color32::from_rgb(235, 231, 220));
        painter.rect_stroke(value_rect, 3.0 * z, Stroke::new(0.7 * z, muted), egui::StrokeKind::Inside);
        painter.text(value_rect.center(), Align2::CENTER_CENTER, stock_process_ms_label(raw), font(12.0), ink);
        if self.precision_collects(node) {
            self.register_precision_control(stock_process_edit(node, revision), value_rect);
        }
        self.precision_affordance(ui, &value);
        if control_enabled && (value.clicked() || value.secondary_clicked()
            || self.take_precision_request(&node.id, "slew")) {
            consume_parameter_activation(ui.ctx());
            self.open_precision_editor(stock_process_edit(node, revision), value_rect);
        }
        value.on_hover_text("Exact slew time in ms/V. 0 is instantaneous; positive times must exceed 1 ms/V. Saved values beyond knob travel remain unchanged until edited.");
        if raw.is_finite() && !(STOCK_PROCESS_MIN_SLEW..=STOCK_PROCESS_MAX_SLEW).contains(&raw) {
            painter.text(point(pos2(137.0, 174.0)), Align2::CENTER_CENTER, "!", font(12.0),
                Color32::from_rgb(156, 62, 43));
        }

        let gate_rect = screen(Rect::from_center_size(STOCK_PROCESS_GATE, vec2(105.0, 44.0)));
        let gate = ui.interact(gate_rect, ui.id().with((&node.id, "stock-process-gate")), Sense::click_and_drag());
        let lamp = self.module_lights.get(&node.id)
            .filter(|state| state.count == 1 && state.brightness[0][0].is_finite())
            .map(|state| state.brightness[0][0].clamp(0.0, 1.0));
        let enabled = control_enabled && self.performance_available && !node.bypassed;
        let pressed = self.module_controls.get(&node.id).map(|state| state.pressed)
            .unwrap_or(false);
        gate.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled,
            format!("{} momentary gate", node.id)));
        painter.rect_filled(gate_rect, 5.0 * z,
            if pressed { Color32::from_rgb(53, 55, 50) }
            else if enabled && gate.hovered() { Color32::from_rgb(245, 241, 229) }
            else { Color32::from_rgb(218, 213, 202) });
        painter.rect_stroke(gate_rect, 5.0 * z, Stroke::new(1.3 * z, ink), egui::StrokeKind::Inside);
        painter.text(gate_rect.center(), Align2::CENTER_CENTER, "PRESS", font(14.0),
            if pressed { Color32::from_rgb(246, 239, 221) } else { ink });
        let lamp_center = point(pos2(239.0, 174.0));
        painter.circle_filled(lamp_center, 7.0 * z, Color32::from_rgb(48, 49, 45));
        self.lighting.circle(&painter, lamp_center, 2.8 * z,
            Color32::from_rgb(238, 177, 65).gamma_multiply(0.12 + lamp.unwrap_or(0.0) * 0.88),
            lamp.unwrap_or(0.0), 0.12);
        painter.text(point(pos2(239.0, 191.0)), Align2::CENTER_CENTER,
            if lamp.is_some() { "BUTTON STATE" } else { "STATE UNAVAILABLE" }, font(9.0), muted);
        if enabled && allow_press && self.held_manual.is_none()
            && gate.is_pointer_button_down_on()
            && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary)) {
            self.held_manual = Some(HeldManual { id: node.id.clone(), kind: node.kind.clone(),
                release: "gate_release", button_rect: gate_rect });
            self.select_primary(node.id.clone());
            action = Some(RackAction::Command("trigger_module",
                json!({"id":node.id,"action":"gate_press"})));
        }
        gate.on_hover_text(if enabled {
            "Hold to force gate high. Release, focus loss, or bypass ends the press. This lamp is the actual manual button light; gate CV does not light it."
        } else {
            "Momentary gate requires a live audio engine and an active module."
        });

        painter.text(point(pos2(76.0, 192.0)), Align2::CENTER_CENTER,
            "+1 V CV = 2× TIME", font(9.0), muted);
        painter.line_segment([point(pos2(19.0, 200.0)), point(pos2(281.0, 200.0))],
            Stroke::new(0.8 * z, muted));
        for (x, label) in [
            (54.0, "SH2: PREV RAW CAPTURE"),
            (150.0, "TH: HIGH HOLDS / RISE"),
            (249.0, "HT: LOW HOLDS / FALL"),
        ] {
            painter.text(point(pos2(x, 213.0)), Align2::CENTER_CENTER, label, font(9.0), ink);
        }
        painter.text(point(pos2(76.0, 227.0)), Align2::CENTER_CENTER,
            "SLEW: HIGH JUMPS", font(9.0), ink);
        painter.text(point(pos2(218.0, 227.0)), Align2::CENTER_CENTER,
            "GLIDE: HIGH >1ms SLEW / ELSE JUMP", font(9.0), ink);
        for (index, label) in ["SLEW CV", "SIGNAL", "GATE CV"].into_iter().enumerate() {
            painter.text(point(pos2(stock_process_port_position(true, index).x, 244.0)),
                Align2::CENTER_CENTER, label, font(10.5), ink);
        }
        for (index, label) in ["S/H 1", "S/H 2", "TRACK / HOLD", "HOLD / TRACK", "SLEW", "GLIDE"]
            .into_iter().enumerate() {
            let port = stock_process_port_position(false, index);
            painter.text(point(pos2(port.x, port.y - 25.0)), Align2::CENTER_CENTER,
                label, font(10.5), ink);
        }
        action
    }
}
