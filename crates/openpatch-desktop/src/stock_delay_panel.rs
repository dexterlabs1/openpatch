// Shared rack/browser faceplate. CLK uses a coherent DSP snapshot.
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
fn stock_delay_knob_rect(index: usize) -> Rect {
    Rect::from_center_size(
        pos2(
            stock_delay_column(index),
            if index < 4 { 148.0 } else { 257.0 },
        ),
        Vec2::splat(if index < 4 { 62.0 } else { 46.0 }),
    )
}
fn stock_delay_value_rect(index: usize) -> Rect {
    Rect::from_center_size(
        pos2(
            stock_delay_column(index),
            if index < 4 { 210.0 } else { 302.0 },
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
                p(x, 228.0),
                Align2::CENTER_CENTER,
                "CV DEPTH",
                font(7.5),
                MUTED,
            );
            painter.text(
                p(x, 188.0),
                Align2::CENTER_CENTER,
                match index {
                    0 => "1  100  10k ms",
                    2 => "−100  0  +100%",
                    _ => "0  50  100%",
                },
                font(7.0),
                MUTED,
            );
            painter.text(
                p(x, 284.0),
                Align2::CENTER_CENTER,
                "−100  0  +100%",
                font(7.0),
                MUTED,
            );
        }
        for (index, name) in STOCK_DELAY_PARAMS.iter().enumerate() {
            let min = if index >= 4 { -1.0 } else { 0.0 };
            let raw = node
                .params
                .get(*name)
                .copied()
                .unwrap_or(stock_delay_default(index))
                .clamp(min, 1.0);
            let normalized = if index >= 4 { (raw + 1.0) * 0.5 } else { raw };
            let knob_rect = screen(stock_delay_knob_rect(index));
            // Node/parameter identity survives graph edits and viewport culling.
            let knob = ui.interact(
                knob_rect,
                ui.id().with(("stock-delay-knob", &node.id, name)),
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
                Some(stock_delay_default(index))
            } else if let Some(high) = input.boundary {
                Some(if high { 1.0 } else { min })
            } else if input.nudge != 0 {
                let n = (normalized + input.nudge as f32 * keyboard_step).clamp(0.0, 1.0);
                Some(if index >= 4 { n * 2.0 - 1.0 } else { n })
            } else {
                input
                    .normalized
                    .map(|n| if index >= 4 { n * 2.0 - 1.0 } else { n })
            };
            if let Some(next) = next.filter(|next| *next != raw) {
                let mut args = json!({"id":node.id,"parameter":name,"value":next,"expected_revision":revision});
                if let Some(group) = input.history_group {
                    args["history_group"] = json!(group);
                }
                action = Some(RackAction::Command("set_parameter", args));
            }
            let value_rect = screen(stock_delay_value_rect(index));
            if input_enabled && knob.secondary_clicked() {
                self.open_precision_editor(stock_delay_edit(node, index, revision), value_rect);
            }
            knob.clone().on_hover_cursor(egui::CursorIcon::ResizeVertical).on_hover_text(match index {
                0 => "Delay time from 1 ms to 10 s. Click the readout for milliseconds, seconds or expressions. Clock input sets the time from incoming edges.",
                1 => "Feedback 0–100%. Higher feedback produces more repeats.",
                2 => "Tone −100% to +100%, dark to bright. Center is neutral.",
                3 => "Mix 0–100%, dry to wet. WET output is independent of this control.",
                _ => "Signed CV depth −100% to +100%. Negative depth inverts modulation.",
            });
            crate::panel_skin::signal_atlas_rotary(
                &painter,
                knob_rect.center(),
                normalized,
                knob.hovered() || knob.dragged() || knob.has_focus(),
                if index < 4 { 28.0 } else { 20.0 },
                z,
            );
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
                let marker = crate::construction_motion::Marker::Rotary {
                    center: knob_rect.center(),
                    radius: if index < 4 { 30.0 } else { 22.0 } * z,
                    old: normalized(cue.old),
                    new: normalized(cue.new),
                };
                self.register_construction_anchor(
                    ui,
                    node,
                    name,
                    crate::construction_motion::Anchor {
                        rect: knob_rect.union(value_rect),
                        old_text: stock_delay_readout(index, cue.old),
                        new_text: stock_delay_readout(index, cue.new),
                        marker: Some(marker),
                    },
                    &[knob.id, value.id],
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
