// Original shared rack/browser faceplate. CLK uses a coherent DSP snapshot.
const STOCK_DELAY_PARAMS: [&str; 8] = [
    "time",
    "feedback",
    "tone",
    "mix",
    "time_cv_amount",
    "feedback_cv_amount",
    "tone_cv_amount",
    "mix_cv_amount",
];
fn stock_delay_default(index: usize) -> f32 {
    if index == 0 {
        500.0_f32.log10() / 4.0
    } else if index < 4 {
        0.5
    } else {
        0.0
    }
}
fn stock_delay_time_edit(edit: &ParameterEdit) -> bool {
    edit.kind == "stock_delay" && edit.parameter == "time"
}
fn stock_delay_column(index: usize) -> f32 {
    46.0 + (index % 4) as f32 * 86.0
}
fn stock_delay_slider_rect(index: usize) -> Rect {
    Rect::from_center_size(
        pos2(
            stock_delay_column(index),
            if index < 4 { 177.0 } else { 286.0 },
        ),
        if index < 4 {
            vec2(22.0, 126.0)
        } else {
            vec2(60.0, 18.0)
        },
    )
}
fn stock_delay_value_rect(index: usize) -> Rect {
    Rect::from_center_size(
        pos2(
            stock_delay_column(index),
            if index < 4 { 255.0 } else { 308.0 },
        ),
        vec2(72.0, 18.0),
    )
}
fn stock_delay_readout(index: usize, raw: f32) -> String {
    if index == 0 {
        let millis = 10000.0_f32.powf(raw);
        if millis >= 1000.0 {
            format!("{:.2} s", millis / 1000.0)
        } else {
            format!("{millis:.1} ms")
        }
    } else if index == 2 {
        format!("{:+.0}%", (raw * 2.0 - 1.0) * 100.0)
    } else if index >= 4 {
        format!("{:+.0}%", raw * 100.0)
    } else {
        format!("{:.0}%", raw * 100.0)
    }
}
fn stock_delay_edit(node: &Node, index: usize, revision: u64) -> ParameterEdit {
    let raw = node
        .params
        .get(STOCK_DELAY_PARAMS[index])
        .copied()
        .unwrap_or(stock_delay_default(index));
    let value = if index == 0 {
        10000.0_f32.powf(raw)
    } else if index == 2 {
        (raw * 2.0 - 1.0) * 100.0
    } else {
        raw * 100.0
    };
    ParameterEdit {
        id: node.id.clone(),
        kind: node.kind.clone(),
        parameter: STOCK_DELAY_PARAMS[index].into(),
        revision,
        surface: ParameterEditSurface::Window,
        menu_popup: None,
        stale: false,
        text: format!("{value:.3}"),
        min: if index == 0 {
            1.0
        } else if index == 2 || index >= 4 {
            -100.0
        } else {
            0.0
        },
        max: if index == 0 { 10000.0 } else { 100.0 },
        step: None,
        display_scale: if index == 0 { 1.0 } else { 100.0 },
        display_unit: if index == 0 { "ms" } else { "%" }.into(),
        slew_min: None,
        gain_power: None,
        raw_max: 1.0,
        focus: true,
    }
}
fn stock_delay_port_label(port: &str) -> &str {
    match port {
        "time_cv" => "TIME CV",
        "feedback_cv" => "FDBK CV",
        "tone_cv" => "TONE CV",
        "mix_cv" => "MIX CV",
        "signal" => "IN",
        "clock" => "CLK",
        "mix" => "MIX",
        "wet" => "WET",
        other => other,
    }
}
impl Rack {
    fn stock_delay_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
    ) -> Option<RackAction> {
        let z = self.zoom;
        let p = |x, y| rect.min + vec2(x, y) * z;
        let screen = |r: Rect| Rect::from_min_max(p(r.min.x, r.min.y), p(r.max.x, r.max.y));
        let painter = ui.painter().clone();
        let font = |size| crate::panel_skin::print_font(ui.ctx(), size * z);
        let mut action = None;
        for (index, label) in ["TIME", "FEEDBACK", "TONE", "MIX"].iter().enumerate() {
            let x = stock_delay_column(index);
            painter.text(p(x, 85.0), Align2::CENTER_CENTER, label, font(9.0), INK);
            painter.text(
                p(x, 101.0),
                Align2::CENTER_CENTER,
                if index == 0 {
                    "1 ms – 10 s"
                } else if index == 2 {
                    "DARK / BRIGHT"
                } else if index == 3 {
                    "DRY / WET"
                } else {
                    "REPEATS"
                },
                font(7.5),
                MUTED,
            );
            painter.text(
                p(x, 270.0),
                Align2::CENTER_CENTER,
                "CV DEPTH",
                font(7.5),
                MUTED,
            );
            if index < 3 {
                painter.line_segment(
                    [p(x + 43.0, 79.0), p(x + 43.0, 323.0)],
                    Stroke::new(0.5 * z, Color32::from_rgb(175, 173, 165)),
                );
            }
        }
        for (index, name) in STOCK_DELAY_PARAMS.iter().enumerate() {
            let min = if index >= 4 { -1.0 } else { 0.0 };
            let mut raw = node
                .params
                .get(*name)
                .copied()
                .unwrap_or(stock_delay_default(index))
                .clamp(min, 1.0);
            let slider_rect = screen(stock_delay_slider_rect(index));
            // Node/parameter identity survives graph edits and viewport culling.
            let slider_id = ui.id().with(("stock-delay-slider", &node.id, name));
            let response = ui
                .scope_builder(egui::UiBuilder::new().id(slider_id), |ui| {
                    crate::panel_skin::control_style(ui, z);
                    ui.spacing_mut().slider_width = if index < 4 {
                        slider_rect.height()
                    } else {
                        slider_rect.width()
                    };
                    ui.spacing_mut().interact_size = vec2(18.0, 18.0) * z;
                    let slider = egui::Slider::new(&mut raw, min..=1.0).show_value(false);
                    ui.put(
                        slider_rect,
                        if index < 4 { slider.vertical() } else { slider },
                    )
                })
                .inner;
            response.widget_info(|| {
                egui::WidgetInfo::slider(
                    ui.is_enabled(),
                    f64::from(raw),
                    format!("{} {}", node.id, name),
                )
            });
            if response.drag_started()
                || (response.is_pointer_button_down_on()
                    && ui.input(|input| input.pointer.button_pressed(PointerButton::Primary)))
            {
                self.select_primary(node.id.clone());
                response.request_focus();
                if ui
                    .data(|data| data.get_temp::<String>(response.id))
                    .is_none()
                {
                    self.gesture = self.gesture.wrapping_add(1);
                    ui.data_mut(|data| {
                        data.insert_temp(
                            response.id,
                            format!("rack-stock-delay-slider-{}", self.gesture),
                        )
                    });
                }
            }
            if response.changed() || response.double_clicked() {
                self.select_primary(node.id.clone());
                if response.double_clicked() {
                    raw = stock_delay_default(index);
                }
                let mut args =
                    json!({"id":node.id,"parameter":name,"value":raw,"expected_revision":revision});
                if (response.dragged() || response.is_pointer_button_down_on())
                    && !response.double_clicked()
                {
                    if let Some(group) = ui.data(|data| data.get_temp::<String>(response.id)) {
                        args["history_group"] = json!(group);
                    }
                }
                action = Some(RackAction::Command("set_parameter", args));
            }
            if response.drag_stopped()
                || ui.input(|input| input.pointer.button_released(PointerButton::Primary))
            {
                ui.data_mut(|data| data.remove::<String>(response.id));
            }
            let value_rect = screen(stock_delay_value_rect(index));
            if response.secondary_clicked() {
                self.open_precision_editor(stock_delay_edit(node, index, revision), value_rect);
            }
            let slider_response_id = response.id;
            response.on_hover_text(match index {
                0 => "Delay time from 1 ms to 10 s. Click the readout for milliseconds, seconds or expressions. Clock input sets the time from incoming edges.",
                1 => "Feedback 0–100%. Higher feedback produces more repeats.",
                2 => "Tone −100% to +100%, dark to bright. Center is neutral.",
                3 => "Mix 0–100%, dry to wet. WET output is independent of this control.",
                _ => "Signed CV depth −100% to +100%. Negative depth inverts modulation.",
            });
            let label = stock_delay_readout(index, raw);
            crate::panel_skin::readout(
                &painter,
                value_rect.center(),
                &label,
                value_rect.width(),
                z,
            );
            let value = ui.interact(
                value_rect,
                ui.id().with((&node.id, "stock-delay-value", index)),
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
                self.register_precision_control(
                    stock_delay_edit(node, index, revision),
                    value_rect,
                );
            }
            self.precision_affordance(ui, &value);
            if value.clicked()
                || value.secondary_clicked()
                || self.take_precision_request(&node.id, name)
            {
                consume_parameter_activation(ui.ctx());
                self.open_precision_editor(stock_delay_edit(node, index, revision), value_rect);
            }
            if let Some(cue) = self
                .construction_animation
                .parameter(&node.id, &node.kind, name)
                .cloned()
            {
                let normalized = |value: f32| ((value - min) / (1.0 - min)).clamp(0.0, 1.0);
                let marker = if index < 4 {
                    crate::construction_motion::Marker::VerticalSlider {
                        rect: slider_rect,
                        old: normalized(cue.old),
                        new: normalized(cue.new),
                    }
                } else {
                    crate::construction_motion::Marker::Slider {
                        rect: slider_rect,
                        old: normalized(cue.old),
                        new: normalized(cue.new),
                    }
                };
                self.register_construction_anchor(
                    ui,
                    node,
                    name,
                    crate::construction_motion::Anchor {
                        rect: slider_rect.union(value_rect),
                        old_text: stock_delay_readout(index, cue.old),
                        new_text: stock_delay_readout(index, cue.new),
                        marker: Some(marker),
                    },
                    &[slider_response_id, value.id],
                );
            }
            value.on_hover_text("Click for exact values or expressions. Tab applies and moves through all eight controls.");
        }
        let brightness = self
            .module_lights
            .get(&node.id)
            .filter(|state| state.count == 1)
            .map_or(0.0, |state| state.brightness[0][0]);
        let brightness = if brightness.is_finite() {
            brightness.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let center = p(154.0, 366.0);
        painter.circle_filled(center, 3.6 * z, Color32::from_rgb(71, 66, 58));
        self.lighting.circle(
            &painter,
            center,
            2.5 * z,
            Color32::from_rgb(238, 177, 65).gamma_multiply(0.12 + 0.88 * brightness),
            brightness,
            0.12,
        );
        action
    }
}
