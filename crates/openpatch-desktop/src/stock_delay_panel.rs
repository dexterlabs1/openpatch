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
            if index < 4 { 120.0 } else { 220.0 },
        ),
        Vec2::splat(if index < 4 { 64.0 } else { 48.0 }),
    )
}
fn stock_delay_value_rect(index: usize) -> Rect {
    Rect::from_center_size(
        pos2(
            stock_delay_column(index),
            if index < 4 { 164.0 } else { 255.0 },
        ),
        vec2(72.0, 18.0),
    )
}
fn paint_stock_delay_flow(
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
        layout: AtlasVoiceLayout { width: 360.0 },
        zoom,
    };
    let bounds = Rect::from_min_max(point(19.0, 278.0), point(341.0, 313.0));
    let lighting_start = crate::lighting::next(painter);
    draw.screen(bounds);
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
        let direction = (end - from).normalized();
        let side = vec2(-direction.y, direction.x);
        painter.add(egui::Shape::convex_polygon(
            vec![
                end,
                end - direction * 3.0 * zoom + side * 1.8 * zoom,
                end - direction * 3.0 * zoom - side * 1.8 * zoom,
            ],
            amber,
            Stroke::NONE,
        ));
    };
    text(31.0, 292.0, "IN", 7.5);
    painter.circle_stroke(point(58.0, 292.0), 5.0 * zoom, Stroke::new(zoom, ink));
    text(58.0, 292.0, "+", 7.5);
    for (left, right, label) in [
        (73.0, 130.0, "DELAY / SRC"),
        (145.0, 193.0, "TONE"),
        (248.0, 285.0, "MIX"),
    ] {
        painter.rect_stroke(
            Rect::from_min_max(point(left, 286.0), point(right, 298.0)),
            1.5 * zoom,
            Stroke::new(0.8 * zoom, Color32::from_rgb(133, 143, 123)),
            egui::StrokeKind::Inside,
        );
        text((left + right) * 0.5, 292.0, label, 6.5);
    }
    text(222.0, 292.0, "WET", 7.0);
    text(321.0, 292.0, "MIX", 7.0);
    for points in [
        &[(39.0, 292.0), (53.0, 292.0)][..],
        &[(63.0, 292.0), (73.0, 292.0)],
        &[(130.0, 292.0), (145.0, 292.0)],
        &[(193.0, 292.0), (212.0, 292.0)],
        &[(232.0, 292.0), (248.0, 292.0)],
        &[(285.0, 292.0), (314.0, 292.0)],
    ] {
        line(points);
    }
    // Feedback returns the filtered wet sample to the input sum. Dry input
    // also reaches MIX; the independent WET output remains available.
    line(&[(178.0, 298.0), (178.0, 306.0), (58.0, 306.0), (58.0, 297.0)]);
    painter.rect_filled(
        Rect::from_center_size(point(120.0, 306.0), vec2(53.0, 8.0) * zoom),
        0,
        Color32::from_rgb(15, 24, 18),
    );
    text(120.0, 306.0, "FEEDBACK", 6.0);
    line(&[(31.0, 298.0), (31.0, 309.0), (248.0, 309.0), (248.0, 298.0)]);
    painter.rect_filled(
        Rect::from_center_size(point(214.0, 306.0), vec2(51.0, 8.0) * zoom),
        0,
        Color32::from_rgb(15, 24, 18),
    );
    text(214.0, 306.0, "DRY + WET", 6.0);
    text(320.0, 306.0, "CLK → TIME", 6.0);
    lighting.preserve(painter, lighting_start);
    ui.interact(
        bounds,
        ui.id().with(("stock-delay-flow", &node.id)),
        Sense::hover(),
    )
    .on_hover_text("Configured signal-flow guide, not a live waveform or delay measurement. Audio inputs sum to mono; feedback uses the preceding filtered wet sample. Tone filters the wet path. MIX blends dry and wet, while WET remains an independent output. CLK reports the engine light state.");
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
            painter.text(p(x, 67.0), Align2::CENTER_CENTER, label, font(9.0), INK);
            painter.text(
                p(x, 188.0),
                Align2::CENTER_CENTER,
                "CV DEPTH",
                font(7.5),
                MUTED,
            );
            painter.text(
                p(x, 177.0),
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
                p(x, 268.0),
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
            let input = self.parameter_knob_input(
                ui,
                &knob,
                normalized,
                input_enabled,
                &stock_delay_readout(index, raw),
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
            paint_atlas_voice_rotary(
                &painter,
                knob_rect.center(),
                normalized,
                knob.hovered() || knob.dragged() || knob.has_focus(),
                if index < 4 { 29.0 } else { 21.0 },
                z,
            );
            let label = stock_delay_readout(index, raw);
            paint_atlas_voice_readout(&painter, value_rect.center(), &label, value_rect.width(), z);
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
        paint_stock_delay_flow(ui, &painter, node, rect, z, &self.lighting);
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
