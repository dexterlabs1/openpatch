// Patch-derived configuration is printed; only callback telemetry illuminates lamps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SegmentGardenGroup {
    first: usize,
    end: usize,
    triggered: bool,
    sequencer: bool,
}
fn segment_garden_type(node: &Node, stage: usize) -> usize {
    node.params
        .get(&format!("type{}", stage + 1))
        .copied()
        .unwrap_or(0.0)
        .round()
        .clamp(0.0, 2.0) as usize
}
fn segment_garden_loop(node: &Node, stage: usize) -> bool {
    node.params
        .get(&format!("loop{}", stage + 1))
        .copied()
        .unwrap_or(if stage == 0 { 1.0 } else { 0.0 })
        >= 0.5
}
fn segment_garden_groups(node: &Node, patch: &Patch) -> Vec<SegmentGardenGroup> {
    let patched: [bool; 6] = std::array::from_fn(|i| {
        patch
            .cables
            .iter()
            .any(|c| c.to == node.id && c.input == format!("gate{}", i + 1))
    });
    let mut groups = Vec::new();
    let mut first = 0;
    while first < 6 {
        let end = if patched[first] {
            ((first + 1)..6).find(|i| patched[*i]).unwrap_or(6)
        } else {
            first + 1
        };
        let sequencer = end - first >= 3
            && segment_garden_type(node, first) != 1
            && !segment_garden_loop(node, first)
            && ((first + 1)..end).all(|i| segment_garden_type(node, i) == 1);
        groups.push(SegmentGardenGroup {
            first,
            end,
            triggered: patched[first],
            sequencer,
        });
        first = end;
    }
    groups
}
fn segment_garden_stage(name: &str) -> usize {
    name.chars()
        .last()
        .and_then(|c| c.to_digit(10))
        .expect("stage parameter") as usize
        - 1
}
fn segment_garden_control(name: &str) -> (Pos2, f32) {
    let x = 60.0 + segment_garden_stage(name) as f32 * 120.0;
    if name.starts_with("level") {
        (pos2(x, 162.0), 27.0)
    } else {
        (pos2(x, 245.0), 18.0)
    }
}
fn segment_garden_value_center(name: &str) -> Pos2 {
    pos2(
        segment_garden_control(name).0.x,
        if name.starts_with("level") {
            202.0
        } else {
            277.0
        },
    )
}
fn segment_garden_roles(
    node: &Node,
    group: SegmentGardenGroup,
    stage: usize,
) -> (&'static str, &'static str) {
    if group.sequencer && stage == group.first {
        return ("RESET", "DIRECTION");
    }
    match segment_garden_type(node, stage) {
        1 => ("LEVEL", "GLIDE"),
        2 => (
            "LEVEL",
            if !group.triggered {
                "DELAY"
            } else if group.end - group.first > 1 {
                if segment_garden_loop(node, stage)
                    && (group.first..group.end).all(|i| i == stage || !segment_garden_loop(node, i))
                {
                    "HELD"
                } else {
                    "TIME"
                }
            } else if segment_garden_loop(node, stage) {
                "CHANCE"
            } else {
                "TIME"
            },
        ),
        _ if group.end - group.first > 1 => (
            "TIME",
            if (group.first..stage).any(|i| segment_garden_type(node, i) == 0)
                && stage + 1 < group.end
                && segment_garden_type(node, stage + 1) == 0
            {
                "LEVEL"
            } else {
                "SHAPE"
            },
        ),
        _ => (
            if !group.triggered && segment_garden_loop(node, stage) {
                "RATE"
            } else if group.triggered && segment_garden_loop(node, stage) {
                "DIVISION"
            } else {
                "TIME"
            },
            "SHAPE",
        ),
    }
}
fn paint_segment_garden_control_print(
    painter: &egui::Painter,
    rect: Rect,
    node: &Node,
    patch: &Patch,
    name: &str,
    zoom: f32,
) {
    let stage = segment_garden_stage(name);
    let group = segment_garden_groups(node, patch)
        .into_iter()
        .find(|g| stage >= g.first && stage < g.end)
        .unwrap();
    let roles = segment_garden_roles(node, group, stage);
    painter.text(
        rect.min
            + vec2(
                segment_garden_control(name).0.x,
                if name.starts_with("level") {
                    122.0
                } else {
                    216.0
                },
            ) * zoom,
        Align2::CENTER_CENTER,
        if name.starts_with("level") {
            roles.0
        } else {
            roles.1
        },
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
}
fn segment_garden_port(_input: bool, index: usize) -> Pos2 {
    let stage = index % 6;
    pos2(
        35.0 + stage as f32 * 120.0 + if index >= 6 { 50.0 } else { 0.0 },
        if _input { 317.0 } else { 362.0 },
    )
}
fn segment_garden_port_label(port: &str) -> &'static str {
    if port.starts_with("gate") {
        "GATE"
    } else if port.starts_with("cv") {
        "CV"
    } else if port.starts_with("active") {
        "ACTIVE"
    } else {
        "OUT"
    }
}
fn segment_garden_lamp(lights: Option<&ModuleLightState>, index: usize) -> f32 {
    lights
        .filter(|s| index < usize::from(s.count))
        .map_or(0.0, |s| {
            let components = s.brightness[index];
            components[0].max(components[1]).clamp(0.0, 1.0)
        })
}
impl Rack {
    // Shared surface context keeps catalog-driven seed limits and patch grouping coherent.
    #[allow(clippy::too_many_arguments)]
    fn segment_garden_panel(
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
        let mut action = None;
        for group in segment_garden_groups(node, patch) {
            let left = 12.0 + group.first as f32 * 120.0;
            let right = group.end as f32 * 120.0 - 12.0;
            painter.line_segment([p(left, 67.0), p(right, 67.0)], Stroke::new(0.8 * z, MUTED));
            for x in [left, right] {
                painter.line_segment([p(x, 63.0), p(x, 67.0)], Stroke::new(0.8 * z, MUTED));
            }
            let role = if group.sequencer {
                if segment_garden_type(node, group.first) == 0 {
                    "NOTE SEQ"
                } else {
                    "CV SEQ"
                }
            } else if group.triggered {
                "GATE"
            } else {
                "FREE"
            };
            let range = if group.end - group.first == 1 {
                format!("{}", group.first + 1)
            } else {
                format!("{}–{}", group.first + 1, group.end)
            };
            painter.text(
                p((left + right) * 0.5, 55.0),
                Align2::CENTER_CENTER,
                format!("CONFIGURED {range} · {role}"),
                crate::panel_skin::print_font(ui.ctx(), 8.0 * z),
                INK,
            );
        }
        for stage in 0..6 {
            let x = stage as f32 * 120.0;
            if stage > 0 {
                painter.line_segment(
                    [p(x, 77.0), p(x, 385.0)],
                    Stroke::new(0.5 * z, Color32::from_rgba_unmultiplied(100, 94, 72, 75)),
                );
            }
            painter.text(
                p(x + 17.0, 109.0),
                Align2::CENTER_CENTER,
                format!("{}", stage + 1),
                crate::panel_skin::print_font(ui.ctx(), 10.0 * z),
                INK,
            );
            let name = format!("type{}", stage + 1);
            let selected = segment_garden_type(node, stage);
            let mut ids = Vec::new();
            for (value, (label, color)) in [
                ("RAMP", Color32::from_rgb(63, 122, 72)),
                ("STEP", Color32::from_rgb(160, 124, 32)),
                ("HOLD", Color32::from_rgb(166, 62, 47)),
            ]
            .into_iter()
            .enumerate()
            {
                let r = screen(Rect::from_min_size(
                    pos2(x + 8.0 + value as f32 * 35.0, 78.0),
                    vec2(34.0, 20.0),
                ));
                painter.rect_filled(
                    r,
                    2.0 * z,
                    if selected == value {
                        color
                    } else {
                        Color32::from_rgb(226, 217, 189)
                    },
                );
                painter.rect_stroke(
                    r,
                    2.0 * z,
                    Stroke::new(0.7 * z, color),
                    egui::StrokeKind::Inside,
                );
                painter.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    label,
                    crate::panel_skin::print_font(ui.ctx(), 9.0 * z),
                    if selected == value {
                        Color32::from_rgb(255, 249, 225)
                    } else {
                        INK
                    },
                );
                let response =
                    ui.interact(r, ui.id().with((&node.id, &name, value)), Sense::click());
                ids.push(response.id);
                if interactive && response.clicked() && selected != value {
                    action = Some(RackAction::Command(
                        "set_parameter",
                        json!({"id":node.id,"parameter":name,"value":value,"expected_revision":revision}),
                    ));
                }
                response.on_hover_text("Configured stage type: green Ramp, yellow Step, red Hold. These are selectors, not engine activity lamps.");
            }
            if let Some(cue) = self
                .construction_animation
                .parameter(&node.id, &node.kind, &name)
            {
                let labels = ["RAMP", "STEP", "HOLD"];
                self.register_construction_anchor(
                    ui,
                    node,
                    &name,
                    crate::construction_motion::Anchor {
                        rect: screen(Rect::from_min_size(pos2(x + 8.0, 78.0), vec2(104.0, 20.0))),
                        marker: None,
                        old_text: labels[(cue.old as usize).min(2)].into(),
                        new_text: labels[(cue.new as usize).min(2)].into(),
                    },
                    &ids,
                );
            }
            let name = format!("loop{}", stage + 1);
            let looping = segment_garden_loop(node, stage);
            let r = screen(Rect::from_min_size(pos2(x + 34.0, 103.0), vec2(52.0, 12.0)));
            painter.rect_filled(
                r,
                2.0 * z,
                if looping {
                    Color32::from_rgb(82, 93, 67)
                } else {
                    Color32::from_rgb(219, 210, 183)
                },
            );
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                if looping { "LOOP ●" } else { "LOOP ○" },
                crate::panel_skin::print_font(ui.ctx(), 8.0 * z),
                if looping {
                    Color32::from_rgb(255, 249, 225)
                } else {
                    INK
                },
            );
            let response = ui.interact(r, ui.id().with((&node.id, &name)), Sense::click());
            if interactive && response.clicked() {
                action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":name,"value":if looping {0} else {1},"expected_revision":revision}),
                ));
            }
            if let Some(cue) = self
                .construction_animation
                .parameter(&node.id, &node.kind, &name)
            {
                self.register_construction_anchor(
                    ui,
                    node,
                    &name,
                    crate::construction_motion::Anchor {
                        rect: r,
                        marker: None,
                        old_text: if cue.old >= 0.5 { "LOOP" } else { "ONCE" }.into(),
                        new_text: if cue.new >= 0.5 { "LOOP" } else { "ONCE" }.into(),
                    },
                    &[response.id],
                );
            }
            response.on_hover_text("Configured loop boundary. Gate patching discovers groups; one boundary loops a stage, paired boundaries delimit a section. A grouped HOLD at the sole loop point reads HELD: its secondary time control is ignored while it holds for the gate. Grouped HOLD stages otherwise use TIME. CHANCE applies only to a triggered singleton HOLD with LOOP. Live stage activity comes only from engine telemetry.");
            let strength = segment_garden_lamp(self.module_lights.get(&node.id), 6 + stage);
            self.lighting.circle(
                &painter,
                p(x + 103.0, 109.0),
                2.2 * z,
                Color32::from_rgb(103, 181, 111).gamma_multiply(0.12 + strength * 0.88),
                strength,
                0.12,
            );
        }
        let strength = segment_garden_lamp(self.module_lights.get(&node.id), 12);
        self.lighting.circle(
            &painter,
            p(453.0, 24.0),
            2.2 * z,
            Color32::from_rgb(215, 78, 56).gamma_multiply(0.12 + strength * 0.88),
            strength,
            0.12,
        );
        painter.text(
            p(467.0, 24.0),
            Align2::LEFT_CENTER,
            "EDGE",
            crate::panel_skin::print_font(ui.ctx(), 8.0 * z),
            MUTED,
        );
        ui.interact(screen(Rect::from_min_max(pos2(446.0, 16.0), pos2(511.0, 33.0))), ui.id().with((&node.id, "segment-edge")), Sense::hover()).on_hover_text("Sticky engine edge-overload indicator. Dark without telemetry. Output pairs below show real bipolar voltage; the small stage lamps show actual active stages. CONFIGURED grouping reflects physical gate cables, including patched 0 V.");
        painter.text(
            p(544.0, 24.0),
            Align2::RIGHT_CENTER,
            "SEED",
            crate::panel_skin::print_font(ui.ctx(), 8.0 * z),
            INK,
        );
        if let Some(spec) = module.and_then(|m| m["params"].get("seed")) {
            ui.add_enabled_ui(interactive, |ui| {
                if let Some(RackAction::Command(command, mut args)) = self.compact_parameter(
                    ui,
                    node,
                    "seed",
                    spec,
                    screen(Rect::from_min_size(pos2(552.0, 15.0), vec2(105.0, 18.0))),
                    "",
                ) {
                    args["expected_revision"] = json!(revision);
                    action = Some(RackAction::Command(command, args));
                }
            });
        }
        action
    }
}
