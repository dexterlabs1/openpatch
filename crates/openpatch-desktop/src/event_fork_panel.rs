// Event Fork uses the same face, native parameter gestures and sockets in the
// rack and browser. Configuration guides never stand in for engine activity.
fn event_fork_control(name: &str) -> (Pos2, f32) {
    (
        pos2(if name == "probability1" { 75.0 } else { 225.0 }, 137.0),
        30.0,
    )
}
fn event_fork_value_center(name: &str) -> Pos2 {
    pos2(event_fork_control(name).0.x, 184.0)
}
fn event_fork_selector(name: &str) -> Option<(Rect, &'static str, &'static [&'static str])> {
    let (channel, y, label, choices): (_, _, _, &[&str]) = match name {
        "toggle1" => (0, 219.0, "TOGGLE", &["DIRECT", "TOGGLE"]),
        "toggle2" => (1, 219.0, "TOGGLE", &["DIRECT", "TOGGLE"]),
        "latch1" => (0, 265.0, "LATCH", &["GATED", "LATCH"]),
        "latch2" => (1, 265.0, "LATCH", &["GATED", "LATCH"]),
        _ => return None,
    };
    Some((
        Rect::from_min_size(pos2(19.0 + channel as f32 * 150.0, y), vec2(112.0, 22.0)),
        label,
        choices,
    ))
}
fn event_fork_seed_rect() -> Rect {
    Rect::from_min_size(pos2(71.0, 46.0), vec2(112.0, 20.0))
}
fn event_fork_port(input: bool, index: usize) -> Pos2 {
    let (channel, cv) = if input {
        (index % 2, index >= 2)
    } else {
        (index / 2, !index.is_multiple_of(2))
    };
    pos2(
        44.0 + channel as f32 * 150.0 + if cv { 62.0 } else { 0.0 },
        if input { 327.0 } else { 377.0 },
    )
}
fn event_fork_port_label(port: &str) -> &str {
    match port {
        "gate1" => "GATE 1",
        "gate2" => "GATE 2",
        "probability1_cv" | "probability2_cv" => "P CV",
        "a1" => "A1",
        "b1" => "B1",
        "a2" => "A2",
        "b2" => "B2",
        other => other,
    }
}
fn event_fork_gate2_normalled(node: &Node, patch: &Patch) -> bool {
    !patch
        .cables
        .iter()
        .any(|c| c.to == node.id && c.input == "gate2")
}
fn event_fork_lamp(lights: Option<&ModuleLightState>, index: usize) -> f32 {
    lights
        .filter(|s| index < usize::from(s.count))
        .map(|s| s.brightness[index][0])
        .filter(|v| v.is_finite())
        .unwrap_or(0.0)
        .clamp(0.0, 1.0)
}
fn paint_event_fork_control_print(painter: &egui::Painter, rect: Rect, name: &str, zoom: f32) {
    painter.text(
        rect.min + vec2(event_fork_control(name).0.x, 95.0) * zoom,
        Align2::CENTER_CENTER,
        "PROBABILITY",
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
}
impl Rack {
    #[allow(clippy::too_many_arguments)]
    fn event_fork_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        patch: &Patch,
        module: Option<&Value>,
        rect: Rect,
        revision: u64,
        interactive: bool,
    ) -> Option<RackAction> {
        let z = self.zoom;
        let p = |x, y| rect.min + vec2(x, y) * z;
        let screen = |r: Rect| Rect::from_min_size(p(r.min.x, r.min.y), r.size() * z);
        let painter = ui.painter().clone();
        let font = |size| crate::panel_skin::print_font(painter.ctx(), size * z);
        let mut action = None;
        painter.line_segment(
            [p(150.0, 72.0), p(150.0, 386.0)],
            Stroke::new(0.6 * z, MUTED.gamma_multiply(0.5)),
        );
        painter.text(p(23.0, 56.0), Align2::LEFT_CENTER, "SEED", font(9.0), INK);
        painter.text(
            p(240.0, 56.0),
            Align2::CENTER_CENTER,
            if event_fork_gate2_normalled(node, patch) {
                "G2 ← G1 NORMAL"
            } else {
                "G2 INDEPENDENT"
            },
            font(8.0),
            INK,
        );
        for channel in 0..2 {
            let x = 75.0 + channel as f32 * 150.0;
            painter.text(
                p(x, 78.0),
                Align2::CENTER_CENTER,
                format!("CHANNEL {}", channel + 1),
                font(10.0),
                INK,
            );
            let toggle = node
                .params
                .get(&format!("toggle{}", channel + 1))
                .copied()
                .unwrap_or(0.0)
                >= 0.5;
            painter.text(
                p(x, 199.0),
                Align2::CENTER_CENTER,
                if toggle {
                    "A STAY · B SWITCH"
                } else {
                    "A ← CHOICE → B"
                },
                font(8.0),
                MUTED,
            );
        }
        // The shared precision editor retains the seed's opening revision,
        // rejects stale typed edits and participates in native control traversal.
        let seed_bounds = screen(event_fork_seed_rect());
        let seed_id = ui.id().with(("event-fork-seed", &node.id));
        let seed_edit = random_seed_edit(node, revision);
        if self.precision_collects(node) {
            self.register_precision_control(seed_edit.clone(), seed_bounds);
        }
        if let Some(cue) = self
            .construction_animation
            .parameter(&node.id, &node.kind, "seed")
        {
            self.register_construction_anchor(
                ui,
                node,
                "seed",
                crate::construction_motion::Anchor {
                    rect: seed_bounds,
                    old_text: format!("{:.0}", cue.old),
                    new_text: seed_edit.text.clone(),
                    marker: None,
                },
                &[seed_id],
            );
        }
        paint_atlas_voice_readout(
            &painter,
            seed_bounds.center(),
            &seed_edit.text,
            seed_bounds.width(),
            z,
        );
        if interactive {
            let response = ui.interact(seed_bounds, seed_id, Sense::click());
            self.precision_affordance(ui, &response);
            if response.clicked() {
                consume_parameter_activation(ui.ctx());
                self.open_precision_editor(seed_edit, seed_bounds);
                self.select_primary(node.id.clone());
            }
            response.on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Edit integer seed, 1–16,777,215. Typed edits use the shared guarded parameter editor and retain their opening document revision.");
        }
        for name in ["toggle1", "toggle2", "latch1", "latch2"] {
            let (local, label, choices) = event_fork_selector(name).unwrap();
            painter.text(
                p(local.center().x, local.top() - 10.0),
                Align2::CENTER_CENTER,
                label,
                font(9.0),
                INK,
            );
            if let Some(spec) = module.and_then(|m| m["params"].get(name)) {
                ui.add_enabled_ui(interactive, |ui| {
                    if let Some(edit) =
                        self.choice_parameter(ui, node, name, spec, screen(local), choices)
                    {
                        action = Some(stock_scope_parameter_action(edit, revision));
                    }
                });
            }
        }
        ui.interact(screen(Rect::from_min_max(pos2(190.0, 46.0), pos2(288.0, 66.0))),
            ui.id().with((&node.id, "event-fork-normal")), Sense::hover())
            .on_hover_text("Gate 2 is physically normalled from Gate 1 until any cable is connected to Gate 2, including a silent cable. Each channel samples its probability at the input edge. Probability CV adds volts / 5 before the source ADC quantization. DIRECT chooses A or B; TOGGLE uses A to stay and B to switch. LATCH holds the chosen output after the input gate falls. Lamps show actual A1/B1/A2/B2 engine output voltage and remain dark without telemetry.");
        action
    }
}
