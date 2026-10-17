// Signal Atlas Scope: the established engine snapshot, trace aperture and
// voltage calibration stay intact. The rotary travel edits native parameters;
// TIME uses geometric travel over the catalog's literal seconds range.
const ATLAS_SCOPE_X_RANGE_CHOICES: [&str; 9] = [
    "X 20 V",
    "X 10 V",
    "X 5 V",
    "X 2.5 V",
    "X 1.25 V",
    "X 625 mV",
    "X 312.5 mV",
    "X 156.25 mV",
    "X 78.125 mV",
];
const ATLAS_SCOPE_Y_RANGE_CHOICES: [&str; 9] = [
    "Y 20 V",
    "Y 10 V",
    "Y 5 V",
    "Y 2.5 V",
    "Y 1.25 V",
    "Y 625 mV",
    "Y 312.5 mV",
    "Y 156.25 mV",
    "Y 78.125 mV",
];

fn scope_control_rect(index: usize) -> Rect {
    // Leave the real 18-screen-point jack targets clear at 35% zoom. Moving
    // this row and its rotaries together preserves their separate hit areas.
    let (column, y, width) = match index {
        0 => (0, 48.0, 110.0),
        1 => (1, 48.0, 110.0),
        3 => (2, 48.0, 110.0),
        7 => (3, 48.0, 110.0),
        8 => (4, 48.0, 110.0),
        2 => (0, 311.0, 130.0),
        4 => (1, 311.0, 130.0),
        5 => (2, 311.0, 130.0),
        6 => (3, 311.0, 130.0),
        _ => unreachable!("unknown scope control"),
    };
    Rect::from_min_size(
        pos2(
            30.0 + column as f32 * if y < 100.0 { 115.0 } else { 145.0 },
            y,
        ),
        vec2(width, if y < 100.0 { 20.0 } else { 22.0 }),
    )
}

fn scope_knob_center(index: usize) -> Pos2 {
    let column = match index {
        2 => 0,
        4 => 1,
        5 => 2,
        6 => 3,
        _ => unreachable!("scope choice has no rotary"),
    };
    pos2(95.0 + 145.0 * column as f32, 289.0)
}

impl Rack {
    fn scope_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        rect: Rect,
    ) -> Option<RackAction> {
        let stock = node.kind == "stock_scope";
        if stock {
            if let Some(token) = self.scope_identities.get(&node.id).copied() {
                let view = self
                    .stock_scope_views
                    .entry(node.id.clone())
                    .or_insert_with(|| StockScopeView::new(token));
                if view.token != token {
                    *view = StockScopeView::new(token);
                }
            }
        }
        let zoom = self.zoom;
        let painter = ui.painter().clone();
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let display = Rect::from_min_max(point(30.0, 70.0), point(600.0, 250.0));
        // The measured aperture and trace coordinates are deliberately retained
        // from the established Scope capture evidence. Only its materials change.
        painter.rect_filled(display.expand(3.0 * zoom), 3.0 * zoom, INK);
        if self.scope_states.contains_key(&node.id) {
            self.lighting.screen_glow(&painter, display, zoom);
        }
        painter.rect_stroke(
            display.expand(1.5 * zoom),
            2.0 * zoom,
            Stroke::new(1.2 * zoom, Color32::from_rgb(126, 121, 94)),
            egui::StrokeKind::Inside,
        );
        let screen_start = crate::lighting::next(&painter);
        painter.rect_filled(display, 1.0 * zoom, Color32::from_rgb(15, 24, 18));
        let plot = Rect::from_min_max(point(40.0, 101.0), point(590.0, 213.0));
        for i in 1..10 {
            let x = plot.left() + plot.width() * i as f32 / 10.0;
            painter.line_segment(
                [pos2(x, plot.top()), pos2(x, plot.bottom())],
                Stroke::new(0.5 * zoom, Color32::from_rgb(36, 50, 40)),
            );
        }
        for i in 1..6 {
            let y = plot.top() + plot.height() * i as f32 / 6.0;
            painter.line_segment(
                [pos2(plot.left(), y), pos2(plot.right(), y)],
                Stroke::new(0.5 * zoom, Color32::from_rgb(36, 50, 40)),
            );
        }
        let trace = painter.with_clip_rect(plot.shrink(1.0));
        if let Some(state) = self.scope_states.get(&node.id) {
            let auto_range = if stock {
                self.stock_scope_views
                    .get(&node.id)
                    .is_none_or(|view| view.automatic)
            } else {
                scope_auto_range_enabled(node)
            };
            let range_id = ui.id().with(("scope-auto-range", &node.id));
            let previous = if stock {
                self.stock_scope_views
                    .get(&node.id)
                    .and_then(|view| view.fitted)
            } else {
                ui.data(|data| data.get_temp::<[ScopeDisplayRange; 2]>(range_id))
            };
            let ranges = scope_display_ranges(state, auto_range, previous);
            if stock {
                if let Some(view) = self.stock_scope_views.get_mut(&node.id) {
                    view.fitted = auto_range.then_some(ranges);
                }
            } else {
                ui.data_mut(|data| {
                    if auto_range {
                        data.insert_temp(range_id, ranges);
                    } else {
                        data.remove::<[ScopeDisplayRange; 2]>(range_id);
                    }
                });
            }
            let colors = [
                Color32::from_rgb(86, 215, 222),
                Color32::from_rgb(250, 185, 80),
            ];
            let voltage_y = |v: f32, range: ScopeDisplayRange| {
                (f64::from(plot.center().y)
                    - scope_display_fraction(v, range) * f64::from(plot.height()))
                    as f32
            };
            let zero_stroke = |color: Color32| Stroke::new(0.8 * zoom, color.gamma_multiply(0.55));
            if state.xy {
                let x_zero = (f64::from(plot.center().x)
                    + scope_display_fraction(0.0, ranges[0]) * f64::from(plot.width()))
                    as f32;
                if plot.x_range().contains(x_zero) {
                    trace.line_segment(
                        [pos2(x_zero, plot.top()), pos2(x_zero, plot.bottom())],
                        zero_stroke(colors[0]),
                    );
                    trace.text(
                        pos2(x_zero + 3.0 * zoom, plot.bottom() - 3.0 * zoom),
                        Align2::LEFT_BOTTOM,
                        "X 0 V",
                        crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
                        colors[0],
                    );
                }
            }
            for (range, color, label) in [
                (ranges[0], colors[0], "X 0 V"),
                (ranges[1], colors[1], "Y 0 V"),
            ] {
                if state.xy && label == "X 0 V" {
                    continue;
                }
                let y_zero = voltage_y(0.0, range);
                if plot.y_range().contains(y_zero) {
                    trace.line_segment(
                        [pos2(plot.left(), y_zero), pos2(plot.right(), y_zero)],
                        zero_stroke(color),
                    );
                    trace.text(
                        pos2(
                            plot.right() - 3.0 * zoom,
                            y_zero + if label == "X 0 V" { -2.0 } else { 10.0 } * zoom,
                        ),
                        Align2::RIGHT_BOTTOM,
                        label,
                        crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
                        color,
                    );
                }
            }
            if if stock {
                stock_scope_x_trigger_guide(state)
            } else {
                scope_x_trigger_guide(state)
            } {
                let trigger_y = voltage_y(state.threshold, ranges[0]);
                if plot.y_range().contains(trigger_y) {
                    trace.line_segment(
                        [pos2(plot.left(), trigger_y), pos2(plot.right(), trigger_y)],
                        Stroke::new(0.9 * zoom, SCOPE_TRIGGER_COLOR),
                    );
                    trace.text(
                        pos2(plot.left() + 3.0 * zoom, trigger_y - 2.0 * zoom),
                        Align2::LEFT_BOTTOM,
                        "X TRIGGER",
                        crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
                        SCOPE_TRIGGER_COLOR,
                    );
                } else {
                    let y = trigger_y.clamp(plot.top() + 5.0 * zoom, plot.bottom() - 5.0 * zoom);
                    trace.text(
                        pos2(plot.left() + 3.0 * zoom, y),
                        Align2::LEFT_CENTER,
                        if trigger_y < plot.top() {
                            "↑ X TRIGGER ABOVE RANGE"
                        } else {
                            "↓ X TRIGGER BELOW RANGE"
                        },
                        crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
                        SCOPE_TRIGGER_COLOR,
                    );
                }
            }
            painter.text(
                point(40.0, 89.0),
                Align2::LEFT_TOP,
                if stock {
                    stock_scope_range_label(auto_range, ranges)
                } else {
                    scope_range_label(auto_range, ranges)
                },
                crate::panel_skin::print_font(painter.ctx(), 9.5 * zoom),
                Color32::from_gray(180),
            );
            let measurements = state.measurements();
            for (axis, extrema, y, color) in [
                ("X", measurements.x, 219.0, colors[0]),
                ("Y", measurements.y, 234.0, colors[1]),
            ] {
                painter.text(
                    point(40.0, y),
                    Align2::LEFT_TOP,
                    scope_extrema_label(axis, extrema),
                    crate::panel_skin::print_font(painter.ctx(), 9.5 * zoom),
                    color,
                );
            }
            if state.xy {
                for voice in 0..usize::from(state.x_channels.min(state.y_channels).min(16)) {
                    let mut previous = None;
                    for bin in scope_xy_order(state.buffer_index) {
                        let x = state.x[voice][bin];
                        let y = state.y[voice][bin];
                        if !scope_point_valid(x) || !scope_point_valid(y) {
                            previous = None;
                            continue;
                        }
                        let p = pos2(
                            (f64::from(plot.center().x)
                                + scope_display_fraction(scope_midpoint(x), ranges[0])
                                    * f64::from(plot.width())) as f32,
                            voltage_y(scope_midpoint(y), ranges[1]),
                        );
                        if !scope_position_finite(p) {
                            previous = None;
                            continue;
                        }
                        if let Some(prev) = previous {
                            trace.line_segment(
                                [prev, p],
                                Stroke::new(zoom.max(0.6), scope_voice_color(colors[0], voice)),
                            );
                        }
                        previous = Some(p);
                    }
                }
            } else {
                for (channel, count, range, color) in [
                    (&state.x, state.x_channels, ranges[0], colors[0]),
                    (&state.y, state.y_channels, ranges[1], colors[1]),
                ] {
                    for (voice, samples) in
                        channel.iter().enumerate().take(usize::from(count.min(16)))
                    {
                        let mut previous = None;
                        for (bin, sample) in samples.iter().enumerate() {
                            if !scope_point_valid(*sample) {
                                previous = None;
                                continue;
                            }
                            let x = plot.left() + plot.width() * bin as f32 / 255.0;
                            let lo = pos2(x, voltage_y(sample.min, range));
                            let hi = pos2(x, voltage_y(sample.max, range));
                            if !scope_position_finite(lo) || !scope_position_finite(hi) {
                                previous = None;
                                continue;
                            }
                            let stroke =
                                Stroke::new(zoom.max(0.6), scope_voice_color(color, voice));
                            trace.line_segment([lo, hi], stroke);
                            let mid = pos2(x, ((f64::from(lo.y) + f64::from(hi.y)) * 0.5) as f32);
                            if let Some(prev) = previous {
                                trace.line_segment([prev, mid], stroke);
                            }
                            previous = Some(mid);
                        }
                    }
                }
            }
            painter.text(
                point(40.0, 76.0),
                Align2::LEFT_TOP,
                format!(
                    "X {} ch   Y {} ch   {}  |  {}",
                    state.x_channels,
                    state.y_channels,
                    if state.xy { "XY" } else { "TIME" },
                    if stock {
                        stock_scope_trigger_status(state)
                    } else {
                        scope_trigger_status(state)
                    }
                ),
                crate::panel_skin::print_font(painter.ctx(), 9.5 * zoom),
                Color32::from_gray(216),
            );
        } else {
            painter.text(
                plot.center(),
                Align2::CENTER_CENTER,
                "NO ENGINE CAPTURE · X / Y VOLTS",
                crate::panel_skin::print_font(painter.ctx(), 12.0 * zoom),
                Color32::from_gray(155),
            );
        }
        if self.scope_states.contains_key(&node.id) {
            self.lighting.preserve(&painter, screen_start);
        }
        if stock {
            self.paint_stock_scope_lamps(ui, node, rect);
            return self.stock_scope_controls(ui, node, module, rect);
        }
        let mut action = None;
        for (index, (name, label, suffix)) in SCOPE_CONTROLS.iter().enumerate() {
            let local = scope_control_rect(index);
            let control = Rect::from_min_max(
                point(local.min.x, local.min.y),
                point(local.max.x, local.max.y),
            );
            let Some(spec) = module.and_then(|m| m["params"].get(*name)) else {
                continue;
            };
            let edit = match *name {
                "x_scale" => self.choice_parameter(
                    ui,
                    node,
                    name,
                    spec,
                    control,
                    &ATLAS_SCOPE_X_RANGE_CHOICES,
                ),
                "y_scale" => self.choice_parameter(
                    ui,
                    node,
                    name,
                    spec,
                    control,
                    &ATLAS_SCOPE_Y_RANGE_CHOICES,
                ),
                "xy" => self.choice_parameter(ui, node, name, spec, control, &["TIME", "XY"]),
                "trigger_enabled" => {
                    self.choice_parameter(ui, node, name, spec, control, &["FREE", "TRIG"])
                }
                "auto_range" => {
                    self.choice_parameter(ui, node, name, spec, control, &["MANUAL", "AUTO"])
                }
                _ => {
                    let center = scope_knob_center(index);
                    painter.text(
                        point(center.x, 258.0),
                        Align2::CENTER_CENTER,
                        *label,
                        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
                        INK,
                    );
                    // Keep the original compact editor and its ID. It handles
                    // human interruption before the combined rotary cue below.
                    let field_edit = self.compact_parameter(ui, node, name, spec, control, suffix);
                    let knob_edit = self.scope_rotary_parameter(
                        ui,
                        node,
                        name,
                        spec,
                        rect,
                        (control, index, *suffix),
                    );
                    field_edit.or(knob_edit)
                }
            };
            if let Some(edit) = edit {
                action = Some(scope_parameter_action(node, edit, self.observed_revision));
            }
        }
        action
    }

    fn scope_rotary_parameter(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        name: &str,
        spec: &Value,
        rect: Rect,
        control: (Rect, usize, &str),
    ) -> Option<RackAction> {
        let (field, index, suffix) = control;
        let zoom = self.zoom;
        let point = |p: Pos2| rect.min + p.to_vec2() * zoom;
        let center = point(scope_knob_center(index));
        let knob_rect = Rect::from_center_size(center, vec2(42.0, 42.0) * zoom);
        let knob = ui.interact(
            knob_rect,
            ui.id().with(("scope-knob", &node.id, name)),
            Sense::click_and_drag(),
        );
        let min = spec["min"].as_f64().unwrap_or(0.0) as f32;
        let max = spec["max"].as_f64().unwrap_or(1.0) as f32;
        let default = spec["default"].as_f64().unwrap_or(0.0) as f32;
        let value = node.params.get(name).copied().unwrap_or(default);
        let stock_time = node.kind == "stock_scope" && name == "time";
        let geometric = name == "time" && min > 0.0 && max > min;
        // Stock stores log2 inverse seconds. Rotary travel follows physical
        // screen duration while every emitted value remains the raw control.
        let normalized = if stock_time {
            1.0 - normalize(value, min, max, false)
        } else if geometric {
            normalize_geometric(value, min, max)
        } else {
            normalize(value, min, max, false)
        };
        let enabled = ui.is_enabled()
            && self.shortcuts_enabled
            && self.editing.is_none()
            && self.annotation_edit.is_none()
            && self.blank_resize.is_none()
            && !egui::Popup::is_any_open(ui.ctx());
        knob.widget_info(|| {
            egui::WidgetInfo::slider(enabled, f64::from(value), format!("{} {name}", node.id))
        });
        let display = |raw: f32| {
            if stock_time {
                format!(
                    "{} ms/screen",
                    scope_measurement_number(stock_scope_nominal_ms(raw))
                )
            } else {
                format!("{}{suffix}", scope_measurement_number(f64::from(raw)))
            }
        };
        let input = self.parameter_knob_input(ui, &knob, normalized, enabled, &display(value));
        if enabled
            && ((knob.is_pointer_button_down_on() && ui.input(|i| i.pointer.any_pressed()))
                || knob.drag_started_by(PointerButton::Primary)
                || knob.clicked_by(PointerButton::Primary))
        {
            self.select_primary(node.id.clone());
        }
        let from_normalized = |n: f32| {
            if stock_time {
                denormalize(1.0 - n, min, max, false)
            } else if geometric {
                denormalize_geometric(n, min, max)
            } else {
                denormalize(n, min, max, false)
            }
        };
        let modifiers = ui.input(|i| i.modifiers);
        let fine = if modifiers.command || modifiers.ctrl {
            if modifiers.shift {
                0.0001
            } else {
                0.001
            }
        } else if modifiers.shift {
            0.1
        } else {
            0.01
        };
        let next = if input.reset {
            Some(default)
        } else if let Some(high) = input.boundary {
            Some(if stock_time {
                from_normalized(if high { 1.0 } else { 0.0 })
            } else if high {
                max
            } else {
                min
            })
        } else if input.nudge != 0 {
            Some(from_normalized(normalized + fine * input.nudge as f32))
        } else {
            input.normalized.map(from_normalized)
        };
        let action = next.filter(|next| *next != value).map(|next| {
            let mut args = json!({"id":node.id,"parameter":name,"value":next});
            if let Some(group) = input.history_group {
                args["history_group"] = json!(group);
            }
            RackAction::Command("set_parameter", args)
        });
        if let Some(cue) = self
            .construction_animation
            .parameter(&node.id, &node.kind, name)
        {
            let normalize = |raw| {
                if stock_time {
                    1.0 - normalize(raw, min, max, false)
                } else if geometric {
                    normalize_geometric(raw, min, max)
                } else {
                    normalize(raw, min, max, false)
                }
            };
            self.register_construction_anchor(
                ui,
                node,
                name,
                crate::construction_motion::Anchor {
                    rect: knob_rect.union(field),
                    old_text: display(cue.old),
                    new_text: display(cue.new),
                    marker: Some(crate::construction_motion::Marker::Rotary {
                        center,
                        radius: 22.0 * zoom,
                        old: normalize(cue.old),
                        new: normalize(cue.new),
                    }),
                },
                &[knob.id],
            );
        }
        paint_atlas_voice_rotary(
            ui.painter(),
            center,
            normalized,
            knob.hovered() || knob.dragged() || knob.has_focus(),
            18.0,
            zoom,
        );
        knob.on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text(format!(
                "{}: {}\n{}\nDrag to adjust; Cmd/Ctrl for fine adjustment. Up/Down adjusts, Home/End selects limits, double-click resets. Type an exact value in the field below.",
                name, display(value), spec["description"].as_str().unwrap_or("")
            ));
        action
    }
}
