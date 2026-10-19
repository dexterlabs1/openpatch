// Printed configuration never substitutes for native clock, lock or output telemetry.
const RHYTHM_TERRAIN_KNOBS: [&str; 7] = ["x", "y", "chaos", "fill1", "fill2", "fill3", "tempo"];
fn rhythm_terrain_control(name: &str) -> (Pos2, f32) {
    let index = RHYTHM_TERRAIN_KNOBS
        .iter()
        .position(|n| *n == name)
        .expect("rhythm knob");
    (pos2(60.0 + index as f32 * 100.0, 165.0), 28.0)
}
fn rhythm_terrain_value_center(name: &str) -> Pos2 {
    pos2(rhythm_terrain_control(name).0.x, 210.0)
}
fn rhythm_terrain_drums(node: &Node) -> bool {
    node.params.get("mode").copied().unwrap_or(1.0) >= 0.5
}
fn rhythm_terrain_flag(node: &Node, name: &str) -> bool {
    node.params
        .get(name)
        .copied()
        .unwrap_or(if name == "tap_tempo" { 1.0 } else { 0.0 })
        >= 0.5
}
fn rhythm_terrain_length(node: &Node, name: &str) -> u32 {
    ((node
        .params
        .get(name)
        .copied()
        .unwrap_or(if name == "chaos" { 0.0 } else { 0.5 })
        .clamp(0.0, 1.0)
        * 255.0)
        .round() as u32
        >> 3)
        + 1
}
fn rhythm_terrain_control_label<'a>(node: &Node, name: &'a str) -> &'a str {
    match name {
        "x" => {
            if rhythm_terrain_drums(node) {
                "MAP X"
            } else {
                "LENGTH 1"
            }
        }
        "y" => {
            if rhythm_terrain_drums(node) {
                "MAP Y"
            } else {
                "LENGTH 2"
            }
        }
        "chaos" => {
            if !rhythm_terrain_drums(node) {
                "LENGTH 3"
            } else if rhythm_terrain_flag(node, "swing") {
                "SWING AMOUNT"
            } else {
                "RANDOMNESS"
            }
        }
        "fill1" => "FILL 1",
        "fill2" => "FILL 2",
        "fill3" => "FILL 3",
        "tempo" => "TEMPO",
        other => other,
    }
}
fn paint_rhythm_terrain_control_print(
    painter: &egui::Painter,
    rect: Rect,
    node: &Node,
    name: &str,
    zoom: f32,
) {
    painter.text(
        rect.min + vec2(rhythm_terrain_control(name).0.x, 122.0) * zoom,
        Align2::CENTER_CENTER,
        rhythm_terrain_control_label(node, name),
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
}
fn rhythm_terrain_selector(name: &str) -> Option<(Rect, &'static str, &'static [&'static str])> {
    let (x, width, label, choices): (_, _, _, &[&str]) = match name {
        "mode" => (20.0, 112.0, "PATTERN", &["EUCLIDEAN", "DRUM MAP"]),
        "resolution" => (
            144.0,
            104.0,
            "CLOCK INPUT",
            &["4 PPQN", "8 PPQN", "24 PPQN"],
        ),
        "gate_mode" => (260.0, 88.0, "OUTPUT", &["TRIGGERS", "GATES"]),
        "alternate" => (360.0, 100.0, "AUXILIARY", &["INDIVIDUAL", "ALTERNATE"]),
        "swing" => (472.0, 92.0, "CHAOS ROLE", &["RANDOM", "SWING"]),
        "tap_tempo" => (576.0, 124.0, "MANUAL BUTTON", &["RESET", "TAP TEMPO"]),
        _ => return None,
    };
    Some((
        Rect::from_min_size(pos2(x, 76.0), vec2(width, 24.0)),
        label,
        choices,
    ))
}
fn rhythm_terrain_seed_rect() -> Rect {
    Rect::from_min_size(pos2(580.0, 15.0), vec2(92.0, 18.0))
}
fn rhythm_terrain_button_rect(release: bool) -> Rect {
    Rect::from_center_size(
        pos2(if release { 641.0 } else { 535.0 }, 261.0),
        vec2(92.0, 24.0),
    )
}
fn rhythm_terrain_port(_input: bool, index: usize) -> Pos2 {
    pos2(
        62.0 + index as f32 * 85.0,
        if _input { 318.0 } else { 372.0 },
    )
}
fn rhythm_terrain_port_label<'a>(node: &Node, port: &'a str, input: bool) -> &'a str {
    if input {
        return match port {
            "clock" => "CLOCK",
            "reset" => "RESET",
            "x_cv" => "X CV",
            "y_cv" => "Y CV",
            "chaos_cv" => "CHAOS CV",
            "fill1_cv" => "FILL 1 CV",
            "fill2_cv" => "FILL 2 CV",
            "fill3_cv" => "FILL 3 CV",
            other => other,
        };
    }
    let drums = rhythm_terrain_drums(node);
    let alternate = rhythm_terrain_flag(node, "alternate");
    match port {
        "trig1" => {
            if drums {
                "BD"
            } else {
                "1"
            }
        }
        "trig2" => {
            if drums {
                "SD"
            } else {
                "2"
            }
        }
        "trig3" => {
            if drums {
                "HH"
            } else {
                "3"
            }
        }
        "aux1" => {
            if alternate {
                if drums {
                    "COMMON ACC"
                } else {
                    "COMMON CYCLE"
                }
            } else if drums {
                "ACC 1"
            } else {
                "CYCLE 1"
            }
        }
        "aux2" => {
            if alternate {
                "CLOCK"
            } else if drums {
                "ACC 2"
            } else {
                "CYCLE 2"
            }
        }
        "aux3" => {
            if alternate {
                if drums {
                    "BAR"
                } else {
                    "ALL CYCLE"
                }
            } else if drums {
                "ACC 3"
            } else {
                "CYCLE 3"
            }
        }
        "clock" => "NATIVE CLK",
        "random" => "NATIVE RND",
        other => other,
    }
}
fn rhythm_terrain_lamp(lights: Option<&ModuleLightState>, index: usize) -> f32 {
    event_fork_lamp(lights, index)
}
fn rhythm_terrain_status(lights: Option<&ModuleLightState>) -> String {
    let Some(lights) = lights else {
        return "CLOCK UNAVAILABLE".into();
    };
    let lock = if rhythm_terrain_lamp(Some(lights), 8) > 0.0 {
        " · LOCK"
    } else {
        ""
    };
    match lights.clock_frequency.filter(|v| v.is_finite() && *v > 0.0) {
        Some(hz) => format!("EFFECTIVE {:.1} BPM{lock}", hz * 60.0),
        None => format!("EXTERNAL CLOCK{lock}"),
    }
}
impl Rack {
    #[allow(clippy::too_many_arguments)]
    fn rhythm_terrain_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
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
        painter.text(p(572.0, 24.0), Align2::RIGHT_CENTER, "SEED", font(8.0), INK);
        let seed_bounds = screen(rhythm_terrain_seed_rect());
        let seed_id = ui.id().with(("rhythm-terrain-seed", &node.id));
        let mut seed_edit = random_seed_edit(node, revision);
        seed_edit.max = 65_535.0;
        seed_edit.raw_max = 65_535.0;
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
            response.on_hover_text("Exact integer seed, 1–65,535. Shared precision edits retain their opening document revision.");
        }
        for name in [
            "mode",
            "resolution",
            "gate_mode",
            "alternate",
            "swing",
            "tap_tempo",
        ] {
            let (local, label, choices) = rhythm_terrain_selector(name).unwrap();
            painter.text(
                p(local.center().x, 61.0),
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
        if !rhythm_terrain_drums(node) {
            for name in ["x", "y", "chaos"] {
                painter.text(
                    p(rhythm_terrain_control(name).0.x, 230.0),
                    Align2::CENTER_CENTER,
                    format!("CONFIG {} STEPS", rhythm_terrain_length(node, name)),
                    font(8.0),
                    MUTED,
                );
            }
        } else {
            painter.text(
                p(160.0, 230.0),
                Align2::CENTER_CENTER,
                "X / Y SCAN THE DRUM MAP",
                font(8.0),
                MUTED,
            );
        }
        painter.text(
            p(460.0, 230.0),
            Align2::CENTER_CENTER,
            "THREE INDEPENDENT DENSITIES",
            font(8.0),
            MUTED,
        );
        let lights = self.module_lights.get(&node.id);
        painter.text(
            p(24.0, 261.0),
            Align2::LEFT_CENTER,
            rhythm_terrain_status(lights),
            font(9.0),
            INK,
        );
        let next = lights
            .and_then(|s| s.waveform_position)
            .filter(|v| v.is_finite())
            .map(|v| format!("NEXT {:02}", (v.clamp(0.0, 1.0) * 32.0).round() as u32))
            .unwrap_or_else(|| "NEXT —".into());
        painter.text(p(348.0, 261.0), Align2::CENTER_CENTER, next, font(9.0), INK);
        let strength = rhythm_terrain_lamp(lights, 9);
        self.lighting.circle(
            &painter,
            p(417.0, 261.0),
            2.2 * z,
            Color32::from_rgb(215, 78, 56).gamma_multiply(0.12 + strength * 0.88),
            strength,
            0.12,
        );
        painter.text(
            p(427.0, 261.0),
            Align2::LEFT_CENTER,
            "EDGE",
            font(8.0),
            MUTED,
        );
        let enabled = interactive
            && ui.is_enabled()
            && self.shortcuts_enabled
            && self.performance_available
            && !node.bypassed
            && ui.input(|i| i.focused);
        let tap = rhythm_terrain_flag(node, "tap_tempo");
        let active = self
            .module_controls
            .get(&node.id)
            .is_some_and(|s| s.pressed_mask & 1 != 0)
            || lights.is_some_and(|s| s.manual);
        for release in [false, true] {
            let bounds = screen(rhythm_terrain_button_rect(release));
            let label = if release {
                "RELEASE TAP"
            } else if tap {
                "TAP"
            } else {
                "RESET"
            };
            let response = ui.interact(
                bounds,
                ui.id().with((&node.id, "rhythm-runtime", release)),
                Sense::click_and_drag(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    enabled,
                    format!("Rhythm Terrain {} {label}", node.id),
                )
            });
            paint_atlas_manual_button(
                &painter,
                bounds,
                z,
                active && !release && tap,
                enabled && response.hovered(),
                enabled,
                (label, 9.0),
            );
            {
                self.register_construction_anchor(
                    ui,
                    node,
                    if release { "release_tap" } else { "action" },
                    crate::construction_motion::Anchor {
                        rect: bounds,
                        old_text: String::new(),
                        new_text: label.into(),
                        marker: None,
                    },
                    &[response.id],
                );
            }
            if enabled && release && response.clicked() {
                action = Some(RackAction::Command(
                    "trigger_module",
                    json!({"id":node.id,"action":"release_tap","expected_revision":revision}),
                ));
            } else if enabled
                && !release
                && tap
                && self.held_manual.is_none()
                && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
            {
                if response.clicked_by(PointerButton::Primary) {
                    action = Some(RackAction::ManualClick {
                        id: node.id.clone(),
                        press: "tap_press",
                        release: "tap_release",
                    });
                } else if response.is_pointer_button_down_on() {
                    self.held_manual = Some(HeldManual {
                        id: node.id.clone(),
                        kind: node.kind.clone(),
                        release: "tap_release",
                        button_rect: bounds,
                    });
                    action = Some(RackAction::Command(
                        "trigger_module",
                        json!({"id":node.id,"action":"tap_press","expected_revision":revision}),
                    ));
                }
            } else if enabled && !release && !tap && response.clicked() {
                action = Some(RackAction::Command(
                    "trigger_module",
                    json!({"id":node.id,"action":"reset","expected_revision":revision}),
                ));
            }
            response.on_hover_text(if release { "Release the native tap tempo lock. Stored TEMPO controls become effective again." } else { "TAP follows the source press/hold/release action; RESET resets immediately when selected. Tempo below 40 selects external clock. At 40 or above the internal clock ignores CLOCK unless tap locked. Effective BPM and LOCK report actual engine state." });
        }
        ui.interact(screen(Rect::from_min_max(pos2(20.0, 245.0), pos2(479.0, 277.0))), ui.id().with((&node.id,"rhythm-status")), Sense::hover())
            .on_hover_text("Clock, NEXT source step, tap lock and sticky edge-queue overload come only from native telemetry. Controls remain saved while tap locked. Normalized controls add CV / 5 then round to 0–255. Euclidean lengths use (ADC >> 3) + 1; configured length hints omit live CV. NATIVE CLK and NATIVE RND expose source clock/random bits as native extra outputs; they are not physical Grids outputs.");
        action
    }
}
