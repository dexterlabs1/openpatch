impl Rack {
    fn paint_stock_scope_lamps(&self, ui: &mut egui::Ui, node: &Node, rect: Rect) {
        let zoom = self.zoom;
        let painter = ui.painter();
        let actual = self
            .module_lights
            .get(&node.id)
            .filter(|lights| lights.count >= 2);
        for (index, label, y, base) in [
            (0, "XY", 225.0, Color32::from_rgb(242, 199, 59)),
            (1, "TRIG", 240.0, Color32::from_rgb(62, 155, 238)),
        ] {
            let center = rect.min + vec2(554.0, y) * zoom;
            painter.circle_filled(center, 5.0 * zoom, Color32::from_rgb(45, 49, 48));
            if let Some(brightness) = actual
                .map(|lights| lights.brightness[index][0])
                .filter(|brightness| brightness.is_finite())
            {
                let brightness = brightness.clamp(0.0, 1.0);
                self.lighting.circle(
                    painter,
                    center,
                    2.3 * zoom,
                    base.gamma_multiply(0.12 + brightness * 0.88),
                    brightness,
                    0.1,
                );
                painter.circle_stroke(center, 5.0 * zoom, Stroke::new(0.6 * zoom, base));
            } else {
                painter.circle_stroke(
                    center,
                    5.0 * zoom,
                    Stroke::new(0.6 * zoom, Color32::from_gray(118)),
                );
            }
            painter.text(
                center + vec2(9.0, 0.0) * zoom,
                Align2::LEFT_CENTER,
                label,
                FontId::monospace(8.0 * zoom),
                Color32::from_gray(184),
            );
        }
    }

    fn stock_scope_controls(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        rect: Rect,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let painter = ui.painter().clone();
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let mut action = None;
        for (index, (name, label, suffix)) in STOCK_SCOPE_CONTROLS.iter().enumerate() {
            let local = scope_control_rect(index);
            let control = Rect::from_min_max(
                point(local.min.x, local.min.y),
                point(local.max.x, local.max.y),
            );
            painter.text(
                point(local.min.x, local.min.y - 8.0),
                Align2::LEFT_CENTER,
                *label,
                crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
                INK,
            );
            if let Some(spec) = module.and_then(|module| module["params"].get(*name)) {
                let edit = match *name {
                    "x_scale" | "y_scale" => {
                        self.choice_parameter(ui, node, name, spec, control, &SCOPE_SCALE_CHOICES)
                    }
                    "xy" => self.choice_parameter(ui, node, name, spec, control, &["TIME", "XY"]),
                    "trigger_disabled" => self.choice_parameter(
                        ui,
                        node,
                        name,
                        spec,
                        control,
                        &["ENABLED", "DISABLED"],
                    ),
                    "time" => self.stock_scope_time_control(ui, node, spec, control),
                    _ => self.compact_parameter(ui, node, name, spec, control, suffix),
                };
                if let Some(edit) = edit {
                    if matches!(*name, "x_scale" | "y_scale" | "x_offset" | "y_offset") {
                        if let Some(view) = self.stock_scope_views.get_mut(&node.id) {
                            view.automatic = false;
                            view.fitted = None;
                        }
                    }
                    action = Some(stock_scope_parameter_action(edit, self.observed_revision));
                }
            }
        }
        let local = scope_control_rect(8);
        let control = Rect::from_min_max(
            point(local.min.x, local.min.y),
            point(local.max.x, local.max.y),
        );
        painter.text(
            point(local.min.x, local.min.y - 8.0),
            Align2::LEFT_CENTER,
            "VIEW · NATIVE",
            crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
            INK,
        );
        let token = self.scope_identities.get(&node.id).copied();
        let enabled = ui.is_enabled() && token.is_some();
        let automatic = self
            .stock_scope_views
            .get(&node.id)
            .map_or(true, |view| view.automatic);
        let response = ui.interact(
            control,
            ui.id().with((&node.id, "stock-scope-native-view", token)),
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                enabled,
                automatic,
                format!("{} native Scope automatic view", node.id),
            )
        });
        crate::panel_skin::readout(
            &painter,
            control.center(),
            if automatic { "AUTO" } else { "MANUAL" },
            control.width(),
            zoom,
        );
        if enabled && response.clicked() {
            if let Some(view) = self.stock_scope_views.get_mut(&node.id) {
                view.automatic = !view.automatic;
                view.fitted = None;
            }
        }
        response.on_hover_text("Native view only: AUTO fits completed volts for weak signals. MANUAL uses the saved source ranges and offsets. This switch never edits the source patch.");
        action
    }

    fn stock_scope_time_control(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        spec: &Value,
        rect: Rect,
    ) -> Option<RackAction> {
        let min_raw = spec["min"].as_f64().unwrap_or(-50.0_f64.log2()) as f32;
        let max_raw = spec["max"].as_f64().unwrap_or(-0.005_f64.log2()) as f32;
        let raw = node
            .params
            .get("time")
            .copied()
            .unwrap_or(spec["default"].as_f64().unwrap_or(1.0) as f32)
            .clamp(min_raw, max_raw);
        let mut millis = stock_scope_nominal_ms(raw);
        let id = ui.id().with((&node.id, "stock-scope-time"));
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect).id_salt(id), |ui| {
                crate::panel_skin::control_style(ui, self.zoom);
                ui.style_mut().override_font_id =
                    Some(crate::panel_skin::print_font(ui.ctx(), 10.0 * self.zoom));
                ui.add_sized(
                    rect.size(),
                    egui::DragValue::new(&mut millis)
                        .range(5.0..=50_000.0)
                        .speed(1.0)
                        .max_decimals(3)
                        .suffix(" ms/screen"),
                )
            })
            .inner;
        response.widget_info(|| {
            egui::WidgetInfo::slider(
                true,
                millis,
                format!("{} nominal screen milliseconds", node.id),
            )
        });
        let revision_id = id.with("opening-revision");
        if response.drag_started() || response.gained_focus() {
            let revision = self.observed_revision;
            ui.data_mut(|data| data.insert_temp(revision_id, revision));
        }
        if response.drag_started() {
            self.gesture = self.gesture.wrapping_add(1);
            ui.data_mut(|data| data.insert_temp(id, format!("rack-value-{}", self.gesture)));
        }
        let history_group = if response.dragged() {
            ui.data(|data| data.get_temp::<String>(id))
        } else {
            None
        };
        if response.drag_stopped() {
            ui.data_mut(|data| data.remove::<String>(id));
        }
        if let Some(cue) = self
            .construction_animation
            .parameter(&node.id, &node.kind, "time")
        {
            self.register_construction_anchor(
                ui,
                node,
                "time",
                crate::construction_motion::Anchor {
                    rect,
                    old_text: format!(
                        "{} ms/screen",
                        scope_measurement_number(stock_scope_nominal_ms(cue.old))
                    ),
                    new_text: format!(
                        "{} ms/screen",
                        scope_measurement_number(stock_scope_nominal_ms(cue.new))
                    ),
                    marker: None,
                },
                &[response.id],
            );
        }
        let action = if response.changed()
            && millis.is_finite()
            && (5.0..=50_000.0).contains(&millis)
        {
            let value = stock_scope_time_raw(millis).clamp(min_raw, max_raw);
            if value.to_bits() != raw.to_bits() {
                self.select_primary(node.id.clone());
                let revision = ui
                    .data(|data| data.get_temp::<u64>(revision_id))
                    .unwrap_or(self.observed_revision);
                let mut args = json!({"id":node.id,"parameter":"time","value":value,"expected_revision":revision});
                if let Some(group) = history_group {
                    args["history_group"] = json!(group);
                }
                Some(RackAction::Command("set_parameter", args))
            } else {
                None
            }
        } else {
            None
        };
        if response.lost_focus() {
            ui.data_mut(|data| data.remove::<u64>(revision_id));
        }
        response.on_hover_text("Nominal source TIME in ms per screen, 5–50,000. Type an exact millisecond value; the stored source control is log2 inverse seconds. Stored bins may span earlier settings.");
        action
    }
}
