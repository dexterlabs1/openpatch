// Original shared rack/browser panel. The shape is a printed nominal guide;
// only engine snapshots drive stage and any-gate lamps.
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
fn adsr_eg_slider_rect(index: usize) -> Rect {
    let row = index / 4;
    Rect::from_min_size(
        pos2(23.0 + (index % 4) as f32 * 80.0, 168.0 + row as f32 * 74.0),
        vec2(70.0, 22.0),
    )
}
fn adsr_eg_value_rect(index: usize) -> Rect {
    Rect::from_center_size(
        pos2(
            58.0 + (index % 4) as f32 * 80.0,
            201.0 + (index / 4) as f32 * 73.0,
        ),
        vec2(64.0, 18.0),
    )
}
fn adsr_eg_push_rect() -> Rect {
    Rect::from_center_size(pos2(222.0, 367.0), vec2(66.0, 32.0))
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
        let accent = crate::panel_skin::family_accent("adsr_eg");
        let ctx = ui.ctx().clone();
        let font = |size| crate::panel_skin::print_font(&ctx, size * z);
        let mut action = None;
        let guide = Rect::from_min_max(p(24.0, 77.0), p(336.0, 130.0));
        painter.rect_filled(guide, 3.0 * z, Color32::from_rgb(204, 205, 196));
        painter.text(
            p(32.0, 86.0),
            Align2::LEFT_CENTER,
            "NOMINAL SHAPE",
            font(8.0),
            MUTED,
        );
        let sustain = node
            .params
            .get("sustain")
            .copied()
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        let points = vec![
            p(35.0, 121.0),
            p(78.0, 94.0),
            p(132.0, 121.0 - 24.0 * sustain),
            p(245.0, 121.0 - 24.0 * sustain),
            p(324.0, 121.0),
        ];
        painter.add(egui::Shape::line(points, Stroke::new(1.5 * z, accent)));
        // Stage LEDs are aggregate DSP state across polyphonic lanes; no clock,
        // button state or nominal drawing is used to infer an active stage.
        for (i, label) in ["ATTACK", "DECAY", "SUSTAIN", "RELEASE"].iter().enumerate() {
            let x = 58.0 + i as f32 * 80.0;
            let center = p(x - 23.0, 148.0);
            self.adsr_eg_lamp(&painter, node, i, center, z);
            painter.text(
                p(x + 3.0, 148.0),
                Align2::CENTER_CENTER,
                label,
                font(9.0),
                INK,
            );
        }
        for (i, name) in ADSR_EG_PARAMS.iter().enumerate() {
            let depth = i >= 4;
            if depth {
                painter.text(
                    p(58.0 + (i % 4) as f32 * 80.0, 235.0),
                    Align2::CENTER_CENTER,
                    "CV DEPTH",
                    font(8.0),
                    MUTED,
                );
            }
            let min = if depth { -1.0 } else { 0.0 };
            let mut raw = node
                .params
                .get(*name)
                .copied()
                .unwrap_or(if depth { 0.0 } else { 0.5 })
                .clamp(min, 1.0);
            let slider_rect = screen(adsr_eg_slider_rect(i));
            let response = ui
                .scope(|ui| {
                    crate::panel_skin::control_style(ui, z);
                    ui.spacing_mut().slider_width = slider_rect.width();
                    ui.spacing_mut().interact_size.y = slider_rect.height();
                    ui.put(
                        slider_rect,
                        egui::Slider::new(&mut raw, min..=1.0).show_value(false),
                    )
                })
                .inner;
            response.widget_info(|| {
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
            if response.drag_started()
                || (response.is_pointer_button_down_on()
                    && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary)))
            {
                self.select_primary(node.id.clone());
                response.request_focus();
                if ui.data(|d| d.get_temp::<String>(response.id)).is_none() {
                    self.gesture = self.gesture.wrapping_add(1);
                    ui.data_mut(|d| {
                        d.insert_temp(response.id, format!("rack-adsr-slider-{}", self.gesture))
                    });
                }
            }
            if response.changed() || response.double_clicked() {
                self.select_primary(node.id.clone());
                if response.double_clicked() {
                    raw = if depth { 0.0 } else { 0.5 };
                }
                let mut args =
                    json!({"id":node.id,"parameter":name,"value":raw,"expected_revision":revision});
                if (response.dragged() || response.is_pointer_button_down_on())
                    && !response.double_clicked()
                {
                    if let Some(group) = ui.data(|d| d.get_temp::<String>(response.id)) {
                        args["history_group"] = json!(group);
                    }
                }
                action = Some(RackAction::Command("set_parameter", args));
            }
            if response.drag_stopped()
                || ui.input(|i| i.pointer.button_released(PointerButton::Primary))
            {
                ui.data_mut(|d| d.remove::<String>(response.id));
            }
            if response.secondary_clicked() {
                self.open_precision_editor(
                    adsr_eg_edit(node, i, revision),
                    screen(adsr_eg_value_rect(i)),
                );
            }
            response.on_hover_text(if depth { "Bipolar CV depth: ±100%. Effective stage value adds CV / 10 × depth, then clamps to 0–100%." } else if i == 2 { "Sustain level as a percentage of 10 V. Click the readout for exact entry." } else { "Time constant, 1–10,000 ms. 100 ms at midpoint. Attack time to peak is longer. Click the readout for exact entry." });
            let value_rect = screen(adsr_eg_value_rect(i));
            let label = if !depth && i != 2 {
                format!("{:.0} ms", 10000.0_f32.powf(raw))
            } else if depth {
                format!("{:+.0}%", raw * 100.0)
            } else {
                format!("{:.0}%", raw * 100.0)
            };
            crate::panel_skin::readout(
                &painter,
                value_rect.center(),
                &label,
                value_rect.width(),
                z,
            );
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
            p(180.0, 342.0),
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
        painter.circle_filled(center, 3.6 * zoom, Color32::from_rgb(71, 66, 58));
        self.lighting.circle(
            painter,
            center,
            2.5 * zoom,
            Color32::from_rgb(238, 177, 65).gamma_multiply(0.12 + 0.88 * brightness),
            brightness,
            0.12,
        );
    }
}
