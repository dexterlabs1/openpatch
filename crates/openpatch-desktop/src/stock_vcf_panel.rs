// One panel implementation is used by the rack and module-browser preview.
// This filter has no lights or inferred response display.
const STOCK_VCF_PARAMS: [&str; 6] = [
    "frequency",
    "resonance",
    "drive",
    "cutoff_cv_amount",
    "resonance_cv_amount",
    "drive_cv_amount",
];
const STOCK_VCF_C4: f32 = 261.625_6;
fn stock_vcf_frequency_raw(hz: f32) -> f32 {
    ((hz / STOCK_VCF_C4).log2() + 5.0) / 10.0
}
fn stock_vcf_frequency_hz(raw: f32) -> f32 {
    STOCK_VCF_C4 * 2.0_f32.powf(raw * 10.0 - 5.0)
}
fn stock_vcf_slider_rect(index: usize) -> Rect {
    let depth = index >= 3;
    Rect::from_center_size(
        pos2(
            if depth { 123.0 } else { 95.0 },
            105.0 + (index % 3) as f32 * 80.0 + if depth { 31.0 } else { 0.0 },
        ),
        vec2(if depth { 82.0 } else { 138.0 }, 20.0),
    )
}
fn stock_vcf_value_rect(index: usize) -> Rect {
    Rect::from_center_size(
        pos2(
            213.0,
            105.0 + (index % 3) as f32 * 80.0 + if index >= 3 { 31.0 } else { 0.0 },
        ),
        vec2(68.0, 18.0),
    )
}
fn stock_vcf_default(index: usize) -> f32 {
    if index == 0 {
        0.5
    } else {
        0.0
    }
}
fn stock_vcf_edit(node: &Node, index: usize, revision: u64) -> ParameterEdit {
    let name = STOCK_VCF_PARAMS[index];
    let raw = node
        .params
        .get(name)
        .copied()
        .unwrap_or(stock_vcf_default(index));
    let value = if index == 0 {
        stock_vcf_frequency_hz(raw).clamp(8.0, 22000.0)
    } else if index == 2 {
        (raw + 1.0) * 100.0
    } else {
        raw * 100.0
    };
    ParameterEdit {
        id: node.id.clone(),
        kind: node.kind.clone(),
        parameter: name.into(),
        revision,
        surface: ParameterEditSurface::Window,
        menu_popup: None,
        stale: false,
        text: if index == 0 {
            format!("{value:.3}")
        } else {
            format!("{value:.2}")
        },
        min: if index == 0 {
            8.0
        } else if index >= 3 {
            -100.0
        } else {
            0.0
        },
        max: if index == 0 {
            22000.0
        } else if index == 2 {
            200.0
        } else {
            100.0
        },
        step: None,
        display_scale: if index == 0 { 1.0 } else { 100.0 },
        display_unit: if index == 0 { "Hz" } else { "%" }.into(),
        slew_min: None,
        gain_power: None,
        raw_max: 1.0,
        focus: true,
    }
}
fn stock_vcf_port_label(port: &str) -> &str {
    match port {
        "cutoff_cv" => "FREQ CV",
        "resonance_cv" => "RES CV",
        "drive_cv" => "DRIVE CV",
        "signal" => "IN",
        "lowpass" => "LP",
        "highpass" => "HP",
        other => other,
    }
}
impl Rack {
    fn stock_vcf_panel(
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
        for (row, label) in ["CUTOFF", "RESONANCE", "DRIVE"].iter().enumerate() {
            painter.text(
                p(26.0, 83.0 + row as f32 * 80.0),
                Align2::LEFT_CENTER,
                label,
                font(10.0),
                INK,
            );
            painter.text(
                p(26.0, 136.0 + row as f32 * 80.0),
                Align2::LEFT_CENTER,
                "CV DEPTH",
                font(8.0),
                MUTED,
            );
            if row < 2 {
                painter.line_segment(
                    [
                        p(25.0, 151.0 + row as f32 * 80.0),
                        p(245.0, 151.0 + row as f32 * 80.0),
                    ],
                    Stroke::new(0.5 * z, Color32::from_rgb(175, 173, 165)),
                );
            }
        }
        painter.text(
            p(245.0, 83.0),
            Align2::RIGHT_CENTER,
            "8 Hz – 22 kHz",
            font(8.0),
            MUTED,
        );
        for (i, name) in STOCK_VCF_PARAMS.iter().enumerate() {
            let min = if i == 0 {
                stock_vcf_frequency_raw(8.0)
            } else if i >= 2 {
                -1.0
            } else {
                0.0
            };
            let max = if i == 0 {
                stock_vcf_frequency_raw(22000.0)
            } else {
                1.0
            };
            let mut raw = node
                .params
                .get(*name)
                .copied()
                .unwrap_or(stock_vcf_default(i))
                .clamp(min, max);
            let slider_rect = screen(stock_vcf_slider_rect(i));
            // A graph edit or viewport cull can change the number and order
            // of painted widgets while this slider is held. A global scope
            // keeps its active ID and gesture history attached to this control.
            let slider_id = ui.id().with(("stock-vcf-slider", &node.id, name));
            let response = ui
                .scope_builder(egui::UiBuilder::new().id(slider_id), |ui| {
                    crate::panel_skin::control_style(ui, z);
                    ui.spacing_mut().slider_width = slider_rect.width();
                    ui.spacing_mut().interact_size.y = slider_rect.height();
                    ui.put(
                        slider_rect,
                        egui::Slider::new(&mut raw, min..=max).show_value(false),
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
                    && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary)))
            {
                self.select_primary(node.id.clone());
                response.request_focus();
                if ui.data(|d| d.get_temp::<String>(response.id)).is_none() {
                    self.gesture = self.gesture.wrapping_add(1);
                    ui.data_mut(|d| {
                        d.insert_temp(
                            response.id,
                            format!("rack-stock-vcf-slider-{}", self.gesture),
                        )
                    });
                }
            }
            if response.changed() || response.double_clicked() {
                self.select_primary(node.id.clone());
                if response.double_clicked() {
                    raw = stock_vcf_default(i);
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
                    stock_vcf_edit(node, i, revision),
                    screen(stock_vcf_value_rect(i)),
                );
            }
            response.on_hover_text(if i == 0 { "Cutoff 8 Hz–22 kHz. The slider follows octaves; click the readout for exact Hz or note entry." } else if i == 2 { "Input drive 0–200%. 100% is the nominal drive. Double-click resets to 100%." } else if i >= 3 { "Bipolar CV depth −100% to +100%. Negative depth inverts modulation." } else { "Resonance 0–100%." });
            let value_rect = screen(stock_vcf_value_rect(i));
            let label = if i == 0 {
                format!("{:.0} Hz", stock_vcf_frequency_hz(raw))
            } else if i == 2 {
                format!("{:.0}%", (raw + 1.0) * 100.0)
            } else if i >= 3 {
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
                ui.id().with((&node.id, "stock-vcf-value", i)),
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
                self.register_precision_control(stock_vcf_edit(node, i, revision), value_rect);
            }
            self.precision_affordance(ui, &value);
            if value.clicked()
                || value.secondary_clicked()
                || self.take_precision_request(&node.id, name)
            {
                consume_parameter_activation(ui.ctx());
                self.open_precision_editor(stock_vcf_edit(node, i, revision), value_rect);
            }
            value.on_hover_text("Click for exact values or expressions. Tab applies and moves through all six controls.");
        }
        action
    }
}
