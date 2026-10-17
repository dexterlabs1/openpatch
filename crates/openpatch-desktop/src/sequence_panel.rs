impl Rack {
    fn sequence_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        rect: Rect,
        revision: u64,
        interactive: bool,
    ) -> Option<RackAction> {
        let painter = ui.painter().clone();
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let specs = module.and_then(|m| m["params"].as_object());
        let value = |name: &str, fallback: f32| {
            node.params
                .get(name)
                .copied()
                .or_else(|| specs?.get(name)?["default"].as_f64().map(|v| v as f32))
                .unwrap_or(fallback)
        };
        let state = self.sequence_states.get(&node.id).copied();
        let length = state.map_or(value("steps", 8.0) as usize, |s| s.length as usize);
        let mut action = None;
        for (name, label, x) in [
            ("passthrough", "PASS GATE", 20.0),
            ("running", "START RUN", 113.0),
        ] {
            let enabled = value(name, if name == "running" { 1.0 } else { 0.0 }) != 0.0;
            if panel_button(
                ui,
                &painter,
                Rect::from_min_size(point(x, 266.0), vec2(82.0, 22.0) * zoom),
                ui.id().with((&node.id, name)),
                label,
                enabled,
                true,
                zoom,
            ) && interactive
            {
                action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":name,"value":if enabled{0.0}else{1.0},"expected_revision":revision}),
                ));
            }
            if interactive {
                self.atlas_sequence_toggle_anchor(
                    ui,
                    node,
                    name,
                    Rect::from_min_size(point(x, 266.0), vec2(82.0, 22.0) * zoom),
                    ui.id().with((&node.id, name)),
                );
            }
        }
        for (index, (event, label)) in [("clock", "Clock"), ("run", "Run"), ("reset", "Reset")]
            .iter()
            .enumerate()
        {
            if panel_button(
                ui,
                &painter,
                Rect::from_min_size(
                    point(20.0 + index as f32 * 60.0, 299.0),
                    vec2(55.0, 25.0) * zoom,
                ),
                ui.id().with((&node.id, "perform", event)),
                label,
                *event == "run" && state.is_some_and(|s| s.running),
                self.performance_available && !node.bypassed,
                zoom,
            ) && interactive
            {
                action = Some(RackAction::Command(
                    "trigger_module",
                    json!({"id":node.id,"action":event,"expected_revision":revision}),
                ));
            }
        }
        let text = if node.bypassed {
            state.map_or("Bypassed · frozen".into(), |s| {
                format!("Bypassed · frozen at {} / {}", s.step, s.length)
            })
        } else {
            state.map_or("Waiting for live engine".into(), |s| {
                format!(
                    "{} · step {} / {}",
                    if s.running { "Running" } else { "Stopped" },
                    s.step,
                    s.length
                )
            })
        };
        painter.text(
            point(107.0, 334.0),
            Align2::CENTER_CENTER,
            text,
            crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
            MUTED,
        );
        for column in 0..8 {
            let x = 238.0 + column as f32 * 67.0;
            let current = state.is_some_and(|s| s.step as usize == column + 1);
            if column >= length || current {
                painter.rect_filled(
                    Rect::from_min_max(point(x - 29.0, 64.0), point(x + 29.0, 302.0)),
                    3.0,
                    if current {
                        Color32::from_rgb(223, 220, 185)
                    } else {
                        Color32::from_rgb(220, 216, 196)
                    },
                );
            }
            let gate = format!("gate{}", column + 1);
            let enabled = value(&gate, 1.0) != 0.0;
            if panel_button(
                ui,
                &painter,
                Rect::from_center_size(point(x, 73.0), vec2(40.0, 18.0) * zoom),
                ui.id().with((&node.id, "gate", column)),
                &format!("{}", column + 1),
                enabled,
                true,
                zoom,
            ) && interactive
            {
                action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":gate,"value":if enabled{0.0}else{1.0},"expected_revision":revision}),
                ));
            }
            if interactive {
                self.atlas_sequence_toggle_anchor(
                    ui,
                    node,
                    &gate,
                    Rect::from_center_size(point(x, 73.0), vec2(40.0, 18.0) * zoom),
                    ui.id().with((&node.id, "gate", column)),
                );
            }
            let strength = if current { 1.0 } else { 0.0 };
            self.lighting.circle(
                &painter,
                point(x, 54.0),
                3.2 * zoom,
                if state.is_some() {
                    Color32::from_rgb(51, 137, 80).gamma_multiply(0.12 + strength * 0.88)
                } else {
                    Color32::from_rgb(99, 105, 99)
                },
                strength,
                0.12,
            );
        }
        action
    }
}
