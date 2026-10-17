// One panel implementation is used by the rack and module-browser preview.
// The center schematic describes the saved CV paths and two filter outputs;
// it is not a live response display.
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
const STOCK_VCF_KNOB_BODY: f32 = 25.0;
const STOCK_VCF_DEPTH_BODY: f32 = 16.0;
fn stock_vcf_knob_rect(index: usize) -> Rect {
    Rect::from_center_size(
        pos2(
            45.0 + (index % 3) as f32 * 90.0,
            if index < 3 { 114.0 } else { 220.0 },
        ),
        Vec2::splat(if index < 3 { 62.0 } else { 44.0 }),
    )
}
fn stock_vcf_value_rect(index: usize) -> Rect {
    Rect::from_center_size(
        pos2(
            45.0 + (index % 3) as f32 * 90.0,
            if index < 3 { 159.0 } else { 253.0 },
        ),
        vec2(78.0, 16.0),
    )
}
fn stock_vcf_control_center(index: usize) -> Pos2 {
    pos2(
        45.0 + (index % 3) as f32 * 90.0,
        if index < 3 { 114.0 } else { 220.0 },
    )
}
fn stock_vcf_control_radius(index: usize) -> f32 {
    if index < 3 {
        STOCK_VCF_KNOB_BODY
    } else {
        STOCK_VCF_DEPTH_BODY
    }
}
fn stock_vcf_label_y(index: usize) -> f32 {
    if index < 3 {
        72.0
    } else {
        183.0
    }
}
fn paint_stock_vcf_routing(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    node: &Node,
    rect: Rect,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    let point = |x: f32, y: f32| rect.min + vec2(x, y) * zoom;
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout { width: 270.0 },
        zoom,
    };
    let bounds = Rect::from_min_max(point(22.0, 268.0), point(248.0, 307.0));
    let start = crate::lighting::next(painter);
    draw.screen(bounds, lighting);
    let ink = Color32::from_rgb(195, 200, 180);
    let amber = Color32::from_rgb(229, 183, 103);
    let text = |x, y, label: &str, size| {
        painter.text(
            point(x, y),
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            ink,
        );
    };
    let line = |points: &[(f32, f32)]| {
        painter.add(egui::Shape::line(
            points.iter().map(|&(x, y)| point(x, y)).collect(),
            Stroke::new(zoom, amber),
        ));
        let n = points.len();
        let from = point(points[n - 2].0, points[n - 2].1);
        let end = point(points[n - 1].0, points[n - 1].1);
        let d = (end - from).normalized();
        let side = vec2(-d.y, d.x);
        painter.add(egui::Shape::convex_polygon(
            vec![
                end,
                end - d * 3.0 * zoom + side * 1.8 * zoom,
                end - d * 3.0 * zoom - side * 1.8 * zoom,
            ],
            amber,
            Stroke::NONE,
        ));
    };
    text(135.0, 274.0, "SIGNAL FLOW", 8.0);
    text(37.0, 288.0, "IN", 8.0);
    for (left, right, label) in [(53.0, 85.0, "DRIVE"), (114.0, 186.0, "4P LADDER")] {
        painter.rect_stroke(
            Rect::from_min_max(point(left, 281.0), point(right, 295.0)),
            zoom,
            Stroke::new(0.8 * zoom, Color32::from_rgb(133, 143, 123)),
            egui::StrokeKind::Inside,
        );
        text((left + right) * 0.5, 288.0, label, 8.0);
    }
    painter.circle_stroke(point(99.0, 288.0), 6.0 * zoom, Stroke::new(zoom, ink));
    text(99.0, 288.0, "+", 9.0);
    for points in [
        &[(44.0, 288.0), (53.0, 288.0)][..],
        &[(85.0, 288.0), (93.0, 288.0)],
        &[(105.0, 288.0), (114.0, 288.0)],
        &[(186.0, 288.0), (209.0, 288.0)],
    ] {
        line(points);
    }
    text(230.0, 288.0, "LP / HP", 7.5);
    // Negative resonance feeds the previous fourth-stage state back before
    // saturation. The ladder block includes the weighted highpass tap mix.
    line(&[(179.0, 295.0), (179.0, 302.0), (99.0, 302.0), (99.0, 294.0)]);
    painter.rect_filled(
        Rect::from_center_size(point(140.0, 302.0), vec2(52.0, 8.0) * zoom),
        0,
        Color32::from_rgb(15, 24, 18),
    );
    text(140.0, 302.0, "−RESONANCE", 6.5);
    lighting.preserve(painter, start);
    ui.interact(bounds,ui.id().with(("stock-vcf-routing",&node.id)),Sense::hover())
        .on_hover_text("Signal-flow guide, not a live response plot. Frequency CV × signed depth adds to octave pitch (1 V/oct at +100%). Resonance and drive use base + CV / 10 × depth, then clamp. Drive precedes the four-pole ladder. Resonance subtracts the previous fourth-stage state before saturation. Lowpass uses the final stage; highpass combines the input and all four stages before output saturation and decimation.");
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
        for (index, label) in [
            "CUTOFF",
            "RESONANCE",
            "DRIVE",
            "FREQ CV DEPTH",
            "RES CV DEPTH",
            "DRIVE CV DEPTH",
        ]
        .iter()
        .enumerate()
        {
            painter.text(
                p(stock_vcf_control_center(index).x, stock_vcf_label_y(index)),
                Align2::CENTER_CENTER,
                *label,
                font(if index < 3 { 9.0 } else { 7.5 }),
                if index < 3 { INK } else { MUTED },
            );
        }
        paint_stock_vcf_routing(ui, &painter, node, rect, z, &self.lighting);
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
            // Surface + node + parameter survives graph reorder and separates previews.
            let knob = ui.interact(
                knob_rect,
                ui.id().with(("stock-vcf-knob", &node.id, name)),
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
            let input = self.parameter_knob_input(
                ui,
                &knob,
                normalized,
                input_enabled,
                &stock_vcf_readout(i, raw),
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
                Some(stock_vcf_default(i))
            } else if let Some(high) = input.boundary {
                Some(if high { max } else { min })
            } else if input.nudge != 0 {
                Some(
                    min + (normalized + input.nudge as f32 * keyboard_step).clamp(0.0, 1.0)
                        * (max - min),
                )
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
            knob.clone().on_hover_cursor(egui::CursorIcon::ResizeVertical).on_hover_text(if i == 0 { "Cutoff 8 Hz–22 kHz. The rotary follows octaves; click the readout for exact Hz or note entry. Drag vertically; Ctrl/Cmd for fine adjustment. Home/End select limits." } else if i == 2 { "Input drive 0–200%. 100% is nominal. Double-click resets to 100%. Drag vertically; Ctrl/Cmd for fine adjustment." } else if i == 3 { "Frequency CV adds volts × signed depth to octave pitch; +100% gives 1 V/oct, and negative depth inverts modulation. Drag vertically; Ctrl/Cmd for fine adjustment." } else if i >= 4 { "Resonance/drive CV adds CV / 10 × signed depth to the base control, then clamps. Negative depth inverts modulation. Drag vertically; Ctrl/Cmd for fine adjustment." } else { "Resonance 0–100%. Drag vertically; Ctrl/Cmd for fine adjustment." });
            paint_atlas_voice_rotary(
                &painter,
                knob_rect.center(),
                normalized,
                knob.hovered() || knob.dragged() || knob.has_focus(),
                stock_vcf_control_radius(i),
                z,
            );
            let value_rect = screen(stock_vcf_value_rect(i));
            let label = stock_vcf_readout(i, raw);
            paint_atlas_voice_readout(&painter, value_rect.center(), &label, value_rect.width(), z);
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
            if let Some(cue) = self
                .construction_animation
                .parameter(&node.id, &node.kind, name)
                .cloned()
            {
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
                            radius: (stock_vcf_control_radius(i) + 3.0) * z,
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
