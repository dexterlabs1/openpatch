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
const STOCK_VCF_KNOB_BODY: f32 = 20.0;
// The final row clears the unchanged input-jack caption at reduced zoom.
const STOCK_VCF_LAST_ROW_LIFT: f32 = 2.0;
fn stock_vcf_row_shift(index: usize) -> f32 {
    if index % 3 == 2 { -STOCK_VCF_LAST_ROW_LIFT } else { 0.0 }
}
fn stock_vcf_knob_rect(index: usize) -> Rect {
    Rect::from_center_size(
        pos2(if index >= 3 { 190.0 } else { 70.0 }, 103.0 + (index % 3) as f32 * 82.0 + stock_vcf_row_shift(index)),
        Vec2::splat(46.0),
    )
}
fn stock_vcf_value_rect(index: usize) -> Rect {
    Rect::from_center_size(
        pos2(if index >= 3 { 190.0 } else { 70.0 }, 137.0 + (index % 3) as f32 * 82.0 + stock_vcf_row_shift(index)),
        vec2(100.0, 18.0),
    )
}
fn stock_vcf_readout(index: usize, raw: f32) -> String {
    if index == 0 {
        format!("{:.0} Hz", stock_vcf_frequency_hz(raw))
    } else if index == 2 {
        format!("{:.0}%", (raw + 1.0) * 100.0)
    } else if index >= 3 {
        format!("{:+.0}%", raw * 100.0)
    } else {
        format!("{:.0}%", raw * 100.0)
    }
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
        for (row, (main, depth)) in [
            ("CUTOFF", "FREQ CV DEPTH"),
            ("RESONANCE", "RES CV DEPTH"),
            ("DRIVE", "DRIVE CV DEPTH"),
        ]
        .iter()
        .enumerate()
        {
            let y = 73.0 + row as f32 * 82.0 + stock_vcf_row_shift(row);
            painter.text(p(70.0, y), Align2::CENTER_CENTER, main, font(9.0), INK);
            painter.text(p(190.0, y), Align2::CENTER_CENTER, depth, font(8.0), MUTED);
        }
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
            let raw = node
                .params
                .get(*name)
                .copied()
                .unwrap_or(stock_vcf_default(i))
                .clamp(min, max);
            let normalized = ((raw - min) / (max - min)).clamp(0.0, 1.0);
            let knob_rect = screen(stock_vcf_knob_rect(i));
            // Global node+parameter identity survives graph reorder and culling.
            let knob = ui.interact(
                knob_rect,
                egui::Id::new(("stock-vcf-knob", &node.id, name)),
                Sense::click_and_drag(),
            );
            knob.widget_info(|| {
                egui::WidgetInfo::slider(
                    ui.is_enabled(),
                    f64::from(raw),
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
                    if i.modifiers.shift { 0.0001 } else { 0.001 }
                } else if i.modifiers.shift {
                    0.1
                } else {
                    0.01
                }
            });
            let next = if input.reset {
                Some(stock_vcf_default(i))
            } else if let Some(high) = input.boundary {
                Some(if high { max } else { min })
            } else if input.nudge != 0 {
                Some(min + (normalized + input.nudge as f32 * keyboard_step).clamp(0.0, 1.0) * (max - min))
            } else {
                input.normalized.map(|n| min + n * (max - min))
            };
            if let Some(value) = next.filter(|value| *value != raw) {
                let mut args = json!({"id":node.id,"parameter":name,"value":value,"expected_revision":revision});
                if let Some(group) = input.history_group {
                    args["history_group"] = json!(group);
                }
                action = Some(RackAction::Command("set_parameter", args));
            }
            if input_enabled && knob.secondary_clicked() {
                self.open_precision_editor(
                    stock_vcf_edit(node, i, revision),
                    screen(stock_vcf_value_rect(i)),
                );
            }
            knob.clone().on_hover_cursor(egui::CursorIcon::ResizeVertical).on_hover_text(if i == 0 { "Cutoff 8 Hz–22 kHz. The rotary follows octaves; click the readout for exact Hz or note entry. Drag vertically; Ctrl/Cmd for fine adjustment. Home/End select limits." } else if i == 2 { "Input drive 0–200%. 100% is nominal. Double-click resets to 100%. Drag vertically; Ctrl/Cmd for fine adjustment." } else if i >= 3 { "Bipolar CV depth −100% to +100%. Negative depth inverts modulation. Drag vertically; Ctrl/Cmd for fine adjustment." } else { "Resonance 0–100%. Drag vertically; Ctrl/Cmd for fine adjustment." });
            crate::panel_skin::signal_atlas_rotary(
                &painter,
                knob_rect.center(),
                normalized,
                knob.hovered() || knob.dragged() || knob.has_focus(),
                STOCK_VCF_KNOB_BODY,
                z,
            );
            let value_rect = screen(stock_vcf_value_rect(i));
            let label = stock_vcf_readout(i, raw);
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
            if let Some(cue) = self.construction_animation.parameter(&node.id, &node.kind, name).cloned() {
                let normalized = |value: f32| ((value - min) / (max - min)).clamp(0.0, 1.0);
                self.register_construction_anchor(
                    ui,
                    node,
                    name,
                    crate::construction_motion::Anchor {
                        rect: knob_rect.union(value_rect),
                        old_text: stock_vcf_readout(i, cue.old),
                        new_text: stock_vcf_readout(i, cue.new),
                        marker: Some(crate::construction_motion::Marker::Rotary {
                            center: knob_rect.center(),
                            radius: 23.0 * z,
                            old: normalized(cue.old),
                            new: normalized(cue.new),
                        }),
                    },
                    &[knob.id, value.id],
                );
            }
            value.on_hover_text("Click for exact values or expressions. Tab applies and moves through all six controls.");
        }
        action
    }
}
