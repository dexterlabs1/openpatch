include!("stock_vca_mix_topology.rs");

// Signal Atlas faceplate. The annotations below follow the pinned native
// VCMixer contract: each strip is input × level² × channel-CV law; the four
// individual strip outputs are summed, then multiplied by the linear master
// level and optional master-CV law. No strip output is normalized or inferred.
const STOCK_VCA_MIX_CHANNEL_PARAMS: [&str; 4] = ["level1", "level2", "level3", "level4"];
const STOCK_VCA_MIX_ALL_PARAMS: [&str; 7] = [
    "level1", "level2", "level3", "level4", "mix_level", "channel_exp", "mix_exp",
];
const STOCK_VCA_MIX_KNOB_RADIUS: f32 = 17.0;
const STOCK_VCA_MIX_KNOB_Y: f32 = 138.0;
const STOCK_VCA_MIX_VALUE_Y: f32 = 178.0;
const STOCK_VCA_MIX_METER_Y: f32 = 212.0;

fn stock_vca_mix_column(index: usize) -> f32 {
    35.0 + index as f32 * 50.0
}

fn stock_vca_mix_knob_center(index: usize) -> Pos2 {
    pos2(stock_vca_mix_column(index), STOCK_VCA_MIX_KNOB_Y)
}

fn stock_vca_mix_knob_rect(index: usize) -> Rect {
    Rect::from_center_size(
        stock_vca_mix_knob_center(index),
        vec2(44.0, 44.0),
    )
}

fn stock_vca_mix_value_rect(index: usize) -> Rect {
    Rect::from_center_size(
        pos2(stock_vca_mix_column(index), STOCK_VCA_MIX_VALUE_Y),
        vec2(48.0, 17.0),
    )
}

fn stock_vca_mix_edit(node: &Node, parameter: &str, revision: u64) -> ParameterEdit {
    let raw = node.params.get(parameter).copied().unwrap_or_else(|| {
        if parameter == "mix_level" {
            1.0
        } else if matches!(parameter, "channel_exp" | "mix_exp") {
            0.0
        } else {
            1.0
        }
    });
    let flag = matches!(parameter, "channel_exp" | "mix_exp");
    ParameterEdit {
        id: node.id.clone(),
        kind: node.kind.clone(),
        parameter: parameter.into(),
        revision,
        surface: ParameterEditSurface::Window,
        menu_popup: None,
        stale: false,
        text: raw.to_string(),
        min: 0.0,
        max: if flag {
            1.0
        } else if parameter == "mix_level" {
            2.0
        } else {
            std::f32::consts::SQRT_2
        },
        step: flag.then_some(1.0),
        display_scale: 1.0,
        display_unit: if parameter == "mix_level" { "x" } else { "" }.into(),
        slew_min: None,
        gain_power: None,
        raw_max: if flag {
            1.0
        } else if parameter == "mix_level" {
            2.0
        } else {
            std::f32::consts::SQRT_2
        },
        focus: true,
    }
}

fn stock_vca_mix_raw_label(value: f32) -> String {
    if !value.is_finite() {
        if value.is_nan() {
            "NaN".into()
        } else if value.is_sign_positive() {
            "+∞".into()
        } else {
            "−∞".into()
        }
    } else if value.abs() >= 100_000.0 || (value != 0.0 && value.abs() < 0.001) {
        format!("{value:.2e}")
    } else {
        format!("{value:.3}")
    }
}

fn stock_vca_mix_port_label(port: &str) -> &str {
    match port {
        "mix_cv" => "MIX CV",
        "in1" => "IN 1",
        "in2" => "IN 2",
        "in3" => "IN 3",
        "in4" => "IN 4",
        "cv1" => "CV 1",
        "cv2" => "CV 2",
        "cv3" => "CV 3",
        "cv4" => "CV 4",
        "mix" => "MIX",
        "out1" => "OUT 1",
        "out2" => "OUT 2",
        "out3" => "OUT 3",
        "out4" => "OUT 4",
        other => other,
    }
}

fn stock_vca_mix_paint_knob(
    painter: &egui::Painter,
    center: Pos2,
    normalized: f32,
    active: bool,
    zoom: f32,
) {
    let value = if normalized.is_finite() {
        normalized.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let black = Color32::from_rgb(22, 23, 22);
    let rim = Color32::from_rgb(102, 102, 96);
    let face = Color32::from_rgb(39, 40, 38);
    let radius = STOCK_VCA_MIX_KNOB_RADIUS * zoom;

    // The five ticks describe the bounded UI travel; the pointer represents
    // that proposed UI value, while the readout retains the stored raw value.
    let start = 3.0 * std::f32::consts::PI / 4.0;
    let sweep = 3.0 * std::f32::consts::PI / 2.0;
    for tick in 0..=4 {
        let angle = start + sweep * tick as f32 / 4.0;
        let (sin, cos) = angle.sin_cos();
        let inner = radius + 1.6 * zoom;
        let tick_length = if tick == 0 || tick == 4 { 3.0 } else { 1.8 };
        let outer = inner + tick_length * zoom;
        painter.line_segment(
            [center + vec2(cos, sin) * inner, center + vec2(cos, sin) * outer],
            Stroke::new(0.9 * zoom, black),
        );
    }
    painter.circle_filled(center + vec2(0.8, 1.1) * zoom, radius + 1.0 * zoom, Color32::from_black_alpha(55));
    painter.circle_filled(center, radius, black);
    painter.circle_stroke(center, radius, Stroke::new(0.8 * zoom, rim));
    painter.circle_filled(center, radius - 2.0 * zoom, face);
    painter.circle_stroke(
        center,
        radius - 2.0 * zoom,
        Stroke::new(0.6 * zoom, Color32::from_rgb(12, 13, 12)),
    );
    let angle = start + sweep * value;
    let (sin, cos) = angle.sin_cos();
    painter.line_segment(
        [center, center + vec2(cos, sin) * (radius - 5.0 * zoom)],
        Stroke::new(
            (if active { 2.0 } else { 1.7 }) * zoom,
            Color32::from_rgb(245, 241, 228),
        ),
    );
    painter.circle_filled(center, 1.8 * zoom, Color32::from_rgb(196, 191, 178));
}

impl Rack {
    fn stock_vca_mix_mode_control(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        revision: u64,
        parameter: &'static str,
        rect: Rect,
        interactive: bool,
    ) -> Option<RackAction> {
        let label = if parameter == "channel_exp" { "CHANNEL CV" } else { "MIX CV" };
        let enabled = node.params.get(parameter).copied().unwrap_or(0.0) >= 0.5;
        let text = format!("{label}  {}", if enabled { "EXP" } else { "LIN" });
        let response = crate::panel_skin::toggle_button(ui, rect, &text, enabled, self.zoom);
        let id = response.id;
        if interactive {
            self.register_construction_anchor(
                ui,
                node,
                parameter,
                crate::construction_motion::Anchor {
                    rect: response.rect,
                    old_text: format!("{label} {}", if enabled { "LIN" } else { "EXP" }),
                    new_text: format!("{label} {}", if enabled { "EXP" } else { "LIN" }),
                    marker: None,
                },
                &[id],
            );
        }
        if self.precision_collects(node) {
            self.register_precision_control(stock_vca_mix_edit(node, parameter, revision), response.rect);
        }
        let response = response.on_hover_text(if parameter == "channel_exp" {
            "Native chExp: each connected channel CV is divided by 10, negative values are floored at zero, then the CV is applied linearly or raised to the fourth power. Values above 10 V are not capped. This law multiplies the strip signal after level². Right-click for exact entry."
        } else {
            "Native mixExp: the summed strip outputs are multiplied by the linear mix_level, then connected master CV is divided by 10, negative values are floored at zero, and the CV is applied linearly or raised to the fourth power. Values above 10 V are not capped. Right-click for exact entry."
        });
        if !interactive {
            return None;
        }
        if response.secondary_clicked() {
            self.open_precision_editor(stock_vca_mix_edit(node, parameter, revision), response.rect);
        }
        if self.take_precision_request(&node.id, parameter) {
            self.open_precision_editor(stock_vca_mix_edit(node, parameter, revision), response.rect);
        }
        response.clicked().then(|| {
            self.select_primary(node.id.clone());
            RackAction::Command(
                "set_parameter",
                json!({"id":node.id,"parameter":parameter,"value":if enabled {0.0} else {1.0},"expected_revision":revision}),
            )
        })
    }

    fn stock_vca_mix_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
        interactive: bool,
    ) -> Option<RackAction> {
        let z = self.zoom;
        let point = |x: f32, y: f32| rect.min + vec2(x, y) * z;
        let screen = |r: Rect| Rect::from_min_max(point(r.min.x, r.min.y), point(r.max.x, r.max.y));
        let painter = ui.painter().clone();
        let ctx = ui.ctx().clone();
        let font = |size| crate::panel_skin::print_font(&ctx, size * z);
        let interactive = interactive
            && self.shortcuts_enabled
            && !self.annotation_input_guard
            && self.annotation_edit.is_none()
            && self.editing.is_none()
            && ui.input(|input| input.focused)
            && !egui::Popup::is_any_open(ui.ctx());
        let muted = Color32::from_rgb(93, 91, 82);
        let black = crate::panel_skin::INK;
        let mut action = self.stock_vca_mix_mode_control(
            ui,
            node,
            revision,
            "channel_exp",
            screen(Rect::from_min_max(pos2(10.0, 62.0), pos2(148.0, 84.0))),
            interactive,
        );
        let master_action = self.stock_vca_mix_mode_control(
            ui,
            node,
            revision,
            "mix_exp",
            screen(Rect::from_min_max(pos2(154.0, 62.0), pos2(260.0, 84.0))),
            interactive,
        );
        action = action.or(master_action);

        painter.text(
            point(110.0, 92.0),
            Align2::CENTER_CENTER,
            if node.params.get("channel_exp").copied().unwrap_or(0.0) == 1.0 {
                "c = max(CV/10, 0)^4 · 1 unpatched"
            } else {
                "c = max(CV/10, 0) · 1 unpatched"
            },
            font(8.0),
            muted,
        );

        for index in 0..5 {
            let column = stock_vca_mix_column(index);
            painter.text(
                point(column, 104.0),
                Align2::CENTER_CENTER,
                if index == 4 {
                    "MASTER"
                } else {
                    match index {
                        0 => "CH 1",
                        1 => "CH 2",
                        2 => "CH 3",
                        _ => "CH 4",
                    }
                },
                font(10.0),
                black,
            );
        }

        for (index, parameter) in STOCK_VCA_MIX_CHANNEL_PARAMS.into_iter()
            .chain(std::iter::once("mix_level")).enumerate() {
            let max = if index == 4 { 2.0 } else { std::f32::consts::SQRT_2 };
            let raw = node.params.get(parameter).copied().unwrap_or(1.0);
            let normalized = if raw.is_finite() {
                (raw.clamp(0.0, max) / max).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let center = point(stock_vca_mix_column(index), STOCK_VCA_MIX_KNOB_Y);
            let knob_rect = screen(stock_vca_mix_knob_rect(index));
            let id = egui::Id::new(("stock-vca-mix-knob", &node.id, parameter));
            let knob = ui.interact(knob_rect, id, Sense::click_and_drag());
            knob.widget_info(|| {
                egui::WidgetInfo::slider(interactive, f64::from(raw), format!("{} {parameter}", node.id))
            });
            if knob.drag_started_by(PointerButton::Primary) || knob.clicked_by(PointerButton::Primary) {
                self.select_primary(node.id.clone());
            }
            let input = self.parameter_knob_input(ui, &knob, normalized, interactive);
            let mut new_value = input
                .reset
                .then_some(1.0_f32.min(max))
                .or_else(|| input.normalized.map(|n| n.clamp(0.0, 1.0) * max))
                .or_else(|| input.boundary.map(|high| if high { max } else { 0.0 }))
                .or_else(|| {
                    (input.nudge != 0).then_some(input.nudge).map(|nudge| {
                        let step = if ui.input(|i| i.modifiers.shift) { max * 0.1 } else { max * 0.01 };
                        (normalized * max + nudge as f32 * step).clamp(0.0, max)
                    })
                });
            if let Some(value) = new_value.as_mut() {
                if *value == 0.0 {
                    *value = 0.0;
                }
                if *value != raw && action.is_none() {
                    self.select_primary(node.id.clone());
                    let mut args = json!({"id":node.id,"parameter":parameter,"value":*value,"expected_revision":revision});
                    if let Some(group) = input.history_group {
                        args["history_group"] = json!(group);
                    }
                    action = Some(RackAction::Command("set_parameter", args));
                }
            }
            if let Some(cue) = self.construction_animation.parameter(&node.id, &node.kind, parameter) {
                self.register_construction_anchor(
                    ui,
                    node,
                    parameter,
                    crate::construction_motion::Anchor {
                        rect: knob.rect,
                        old_text: stock_vca_mix_raw_label(cue.old),
                        new_text: stock_vca_mix_raw_label(cue.new),
                        marker: None,
                    },
                    &[knob.id],
                );
            }
            stock_vca_mix_paint_knob(
                &painter,
                center,
                normalized,
                knob.hovered() || knob.dragged() || knob.has_focus(),
                z,
            );
            let raw_outside_ui = raw.is_finite() && !(0.0..=max).contains(&raw);
            if raw_outside_ui {
                let marker = Rect::from_center_size(point(stock_vca_mix_column(index) + 18.0, 116.0), vec2(13.0, 11.0) * z);
                painter.rect_filled(marker, 2.0 * z, Color32::from_rgb(156, 62, 43));
                painter.text(marker.center(), Align2::CENTER_CENTER, "!", font(8.0), Color32::WHITE);
                ui.interact(marker, ui.id().with((&node.id, parameter, "raw-outside-ui")), Sense::hover())
                    .on_hover_text(format!("Saved {parameter} value {raw:?} is outside knob travel 0–{max:.7}. The knob points to the nearest UI edge; the saved value stays unchanged until you edit it."));
            }
            let description = if index == 4 {
                format!(
                    "Exact loaded raw mix_level: {raw:?}. UI range 0–2. Native master formula: (out1 + out2 + out3 + out4) × mix_level, then optional master CV law. mix_level is linear; it is not squared. Drag, arrows, Home/End, double-click reset, or use the value for bounded exact entry."
                )
            } else {
                format!(
                    "Exact loaded raw {parameter}: {raw:?}. UI range 0–{max:.7}. Native strip formula: in{} × level{}² × channel-CV factor → out{}. Each strip output remains independent; the four outputs are summed only for the master mix. Drag, arrows, Home/End, double-click reset, or use the value for bounded exact entry.",
                    index + 1,
                    index + 1,
                    index + 1
                )
            };
            knob.on_hover_cursor(egui::CursorIcon::ResizeVertical)
                .on_hover_text(description);

            // The strip knob is a raw `levelN` setting that DSP squares; the
            // master `mix_level` remains a linear setting.
            let label = if index == 4 { "MIX LEVEL" } else { "LEVEL" };
            painter.text(point(stock_vca_mix_column(index), 164.0), Align2::CENTER_CENTER, label, font(9.0), muted);
            let value_rect = screen(stock_vca_mix_value_rect(index));
            let value_response = ui.interact(
                value_rect,
                egui::Id::new(("stock-vca-mix-value", &node.id, parameter)),
                Sense::click(),
            );
            painter.text(
                value_rect.center(),
                Align2::CENTER_CENTER,
                if index == 4 {
                    format!("{}×", stock_vca_mix_raw_label(raw))
                } else {
                    stock_vca_mix_raw_label(raw)
                },
                font(9.5),
                black,
            );
            let value_response = value_response.on_hover_text(format!(
                "Exact loaded raw {parameter}: {raw:?}. The visible UI range is 0–{max:.7}; edits are bounded and never rewrite a loaded value during rendering. Click or right-click for exact entry."
            ));
            if self.precision_collects(node) {
                self.register_precision_control(stock_vca_mix_edit(node, parameter, revision), value_rect);
            }
            self.precision_affordance(ui, &value_response);
            if interactive
                && (value_response.clicked()
                    || value_response.secondary_clicked()
                    || self.take_precision_request(&node.id, parameter))
            {
                consume_parameter_activation(ui.ctx());
                self.open_precision_editor(stock_vca_mix_edit(node, parameter, revision), value_rect);
            }

            if index < 4 {
                painter.text(
                    point(stock_vca_mix_column(index), 195.0),
                    Align2::CENTER_CENTER,
                    "IN × L² × c",
                    font(8.0),
                    muted,
                );
                ui.interact(
                    screen(Rect::from_center_size(
                        pos2(stock_vca_mix_column(index), 195.0),
                        vec2(48.0, 13.0),
                    )),
                    ui.id().with((&node.id, "strip-law", index)),
                    Sense::hover(),
                )
                .on_hover_text(format!(
                    "Strip {}: input × the raw LEVEL setting squared × channel-CV factor → independent OUT {}. A connected channel CV is divided by 10, floored at zero, and optionally raised to the fourth power; values above 10 V are not capped.",
                    index + 1,
                    index + 1
                ));
                let brightness = self
                    .module_lights
                    .get(&node.id)
                    .filter(|state| state.count >= 4)
                    .map(|state| state.brightness[index][0].clamp(0.0, 1.0));
                for segment in 0..8 {
                    let lit = brightness.is_some_and(|value| value > segment as f32 / 8.0);
                    let color = if segment >= 6 {
                        Color32::from_rgb(190, 113, 42)
                    } else {
                        Color32::from_rgb(49, 126, 79)
                    };
                    let led = screen(Rect::from_min_size(
                        pos2(stock_vca_mix_column(index) - 18.0 + segment as f32 * 4.8, STOCK_VCA_MIX_METER_Y - 3.0),
                        vec2(3.4, 6.0),
                    ));
                    self.lighting.rect(
                        &painter,
                        led,
                        1.0 * z,
                        if lit { color } else { Color32::from_rgb(93, 93, 83) },
                        if lit { brightness.unwrap_or(0.0) } else { 0.0 },
                        0.15,
                    );
                    ui.interact(
                        led.expand(1.0 * z),
                        ui.id().with((&node.id, "actual-vu-led", index, segment)),
                        Sense::hover(),
                    )
                    .on_hover_text(format!(
                        "Strip {} actual native VU snapshot: {}. It follows the signed channel-output sum divided by 5 V before absolute value; polyphonic lanes can cancel. No engine snapshot means dark/offline.",
                        index + 1,
                        brightness.map_or_else(|| "offline".to_owned(), |value| format!("{value:.6}"))
                    ));
                }
            }
        }

        painter.text(
            point(235.0, 207.0),
            Align2::CENTER_CENTER,
            if node.bypassed {
                "VU HELD"
            } else if self.module_lights.get(&node.id).is_some_and(|state| state.count >= 4) {
                "VU LIVE"
            } else {
                "VU OFFLINE"
            },
            font(7.0),
            muted,
        );
        stock_vca_mix_topology(
            &painter,
            screen(Rect::from_min_size(pos2(0.0, 214.0), vec2(270.0, 54.0))),
            z,
            node.params.get("mix_exp").copied().unwrap_or(0.0) == 1.0,
        );
        action
    }
}
