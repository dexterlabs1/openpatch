// Frame Scene shares this authored faceplate between the rack and browser.
// Selection and pointer gestures are UI state; only explicit edits reach Core.
#[derive(Default)]
struct FrameSceneUi {
    selected: BTreeMap<String, u16>,
    drag: Option<FrameSceneDrag>,
    level_drag: Option<FrameSceneLevelDrag>,
}
struct FrameSceneDrag {
    node: String,
    key: openpatch_core::FrameSceneKeyframe,
    pointer: Pos2,
    lane: Option<usize>,
    group: String,
    authority: FrameSceneAuthority,
}
struct FrameSceneLevelDrag {
    node: String,
    parameter: String,
    authority: FrameSceneAuthority,
    valid: bool,
}
struct FrameSceneAuthority {
    key: openpatch_core::FrameSceneKeyframe,
    revision: u64,
    pending: Option<(u64, openpatch_core::FrameSceneKeyframe)>,
}
impl FrameSceneAuthority {
    fn new(key: &openpatch_core::FrameSceneKeyframe, revision: u64) -> Self {
        Self {
            key: key.clone(),
            revision,
            pending: None,
        }
    }
    fn observe(&mut self, key: &openpatch_core::FrameSceneKeyframe, revision: u64) -> bool {
        if revision == self.revision {
            return *key == self.key;
        }
        if let Some((issued, expected)) = &self.pending {
            if issued.checked_add(1) == Some(revision) && key == expected {
                self.key = key.clone();
                self.revision = revision;
                self.pending = None;
                return true;
            }
        }
        false
    }
    fn emitted(&mut self, key: openpatch_core::FrameSceneKeyframe, revision: u64) {
        self.pending = Some((revision, key));
    }
}
const FRAME_SCENE_LEVELS: [&str; 4] = ["level1", "level2", "level3", "level4"];
const FRAME_SCENE_DEFAULTS: [f32; 4] = [1.0, 0.75, 0.5, 0.25];
const FRAME_SCENE_COLORS: [Color32; 4] = [
    Color32::from_rgb(206, 166, 93),
    Color32::from_rgb(119, 173, 160),
    Color32::from_rgb(136, 165, 201),
    Color32::from_rgb(194, 145, 159),
];
fn frame_scene_data(node: &Node) -> Option<&openpatch_core::FrameSceneData> {
    match node.content.as_ref() {
        Some(openpatch_core::ModuleContent::FrameScene(data)) => Some(data),
        _ => None,
    }
}
fn frame_scene_control(index: usize) -> (Pos2, f32) {
    if index == 0 {
        (pos2(64.0, 104.0), 29.0)
    } else if index <= 4 {
        (pos2(170.0 + (index - 1) as f32 * 100.0, 104.0), 29.0)
    } else {
        (pos2(58.0, 220.0), 21.0)
    }
}
fn frame_scene_timeline() -> Rect {
    Rect::from_min_max(pos2(126.0, 196.0), pos2(516.0, 278.0))
}
fn frame_scene_lane_y(lane: usize, value: u16) -> f32 {
    208.0 + lane as f32 * 18.0 + (1.0 - f32::from(value) / 65535.0) * 13.0
}
fn frame_scene_key_x(timestamp: u16) -> f32 {
    141.0 + f32::from(timestamp) / 65535.0 * 367.0
}
fn frame_scene_port(input: bool, index: usize) -> Pos2 {
    pos2(
        if input {
            45.0 + index as f32 * 90.0
        } else {
            60.0 + index as f32 * 84.0
        },
        if input { 330.0 } else { 378.0 },
    )
}
fn frame_scene_port_label(port: &str) -> &str {
    match port {
        "all" => "ALL",
        "frame_cv" => "FRAME CV",
        "in1" | "out1" => "1",
        "in2" | "out2" => "2",
        "in3" | "out3" => "3",
        "in4" | "out4" => "4",
        "mix" => "MIX",
        "step" => "STEP",
        other => other,
    }
}
fn frame_scene_manual_values(node: &Node) -> [u16; 4] {
    std::array::from_fn(|lane| {
        (node
            .params
            .get(FRAME_SCENE_LEVELS[lane])
            .copied()
            .unwrap_or(FRAME_SCENE_DEFAULTS[lane])
            .clamp(0.0, 1.0)
            * 65535.0)
            .round() as u16
    })
}
fn frame_scene_update(
    node: &Node,
    revision: u64,
    id: u16,
    values: [u16; 4],
    group: Option<String>,
) -> RackAction {
    let mut args = json!({"id":node.id,"edit":{"operation":"update","keyframe_id":id,"values":values},"expected_revision":revision});
    if let Some(group) = group {
        args["history_group"] = json!(group);
    }
    RackAction::Command("edit_frame_scene", args)
}
fn frame_scene_position(node: &Node, lights: Option<&ModuleLightState>) -> (f32, bool) {
    if let Some(position) = lights
        .and_then(|s| s.waveform_position)
        .filter(|p| p.is_finite())
    {
        (position.clamp(0.0, 1.0), true)
    } else {
        (
            node.params
                .get("frame")
                .copied()
                .unwrap_or(0.0)
                .clamp(0.0, 1.0),
            false,
        )
    }
}
fn frame_scene_lamp(lights: Option<&ModuleLightState>, index: usize) -> f32 {
    lights
        .filter(|s| index < usize::from(s.count))
        .map_or(0.0, |s| s.brightness[index][1].clamp(0.0, 1.0))
}
impl Rack {
    fn frame_scene_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
        interactive: bool,
    ) -> Option<RackAction> {
        let z = self.zoom;
        let p = |x, y| rect.min + vec2(x, y) * z;
        let screen = |r: Rect| Rect::from_min_max(p(r.min.x, r.min.y), p(r.max.x, r.max.y));
        let painter = ui.painter().clone();
        let ctx = ui.ctx().clone();
        let font = |size| crate::panel_skin::print_font(&ctx, size * z);
        let empty = openpatch_core::FrameSceneData::default();
        let data = frame_scene_data(node).unwrap_or(&empty);
        if !self.frame_scene_ui.selected.contains_key(&node.id) {
            if let Some(key) = data.keyframes.first() {
                self.frame_scene_ui.selected.insert(node.id.clone(), key.id);
            }
        }
        // Never replace an absent ID with an index: undo can restore its identity.
        let selected_id = self.frame_scene_ui.selected.get(&node.id).copied();
        let selected = selected_id.and_then(|id| data.keyframes.iter().find(|key| key.id == id));
        let values = selected.map_or_else(|| frame_scene_manual_values(node), |key| key.values);
        let enabled = interactive
            && ui.is_enabled()
            && self.shortcuts_enabled
            && self.editing.is_none()
            && self.annotation_edit.is_none()
            && !egui::Popup::is_any_open(ui.ctx());
        let history_interrupted =
            self.parameter_history_frame == Some(ui.ctx().cumulative_frame_nr());
        if let Some(drag) = self
            .frame_scene_ui
            .level_drag
            .as_mut()
            .filter(|d| d.node == node.id)
        {
            drag.valid &= enabled
                && !history_interrupted
                && ui.input(|i| i.focused && !i.key_pressed(egui::Key::Escape));
            drag.valid &= selected.is_some_and(|key| drag.authority.observe(key, revision));
        }
        let mut action = None;
        for index in 0..6 {
            let (center, radius) = frame_scene_control(index);
            let name = if index == 0 {
                "frame"
            } else if index <= 4 {
                FRAME_SCENE_LEVELS[index - 1]
            } else {
                "modulation"
            };
            let raw = if (1..=4).contains(&index) {
                f32::from(values[index - 1]) / 65535.0
            } else {
                node.params
                    .get(name)
                    .copied()
                    .unwrap_or(if index == 0 { 0.0 } else { 1.0 })
                    .clamp(if index == 5 { -1.0 } else { 0.0 }, 1.0)
            };
            let normalized = if index == 5 { (raw + 1.0) * 0.5 } else { raw };
            let label = if index == 0 {
                "FRAME".to_owned()
            } else if index <= 4 {
                format!("LEVEL {index}")
            } else {
                "MODULATION".to_owned()
            };
            painter.text(
                p(center.x, center.y - radius - 14.0),
                Align2::CENTER_CENTER,
                label,
                font(10.0),
                INK,
            );
            let value_text = format!("{:+.0}%", raw * 100.0);
            let level_enabled =
                !(1..=4).contains(&index) || data.keyframes.is_empty() || selected.is_some();
            let knob = ui.interact(
                Rect::from_center_size(
                    p(center.x, center.y),
                    Vec2::splat((radius + 3.0) * 2.0 * z),
                ),
                ui.id().with((
                    "frame-scene-knob",
                    &node.id,
                    name,
                    if (1..=4).contains(&index) {
                        selected_id
                    } else {
                        None
                    },
                )),
                Sense::click_and_drag(),
            );
            knob.widget_info(|| {
                egui::WidgetInfo::slider(
                    enabled && level_enabled,
                    f64::from(raw),
                    format!("{} {name}", node.id),
                )
            });
            let keyed_level = (1..=4).contains(&index) && !data.keyframes.is_empty();
            let pressed = knob.is_pointer_button_down_on()
                && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary));
            if enabled && keyed_level && pressed {
                if let Some(key) = selected {
                    self.frame_scene_ui.level_drag = Some(FrameSceneLevelDrag {
                        node: node.id.clone(),
                        parameter: name.into(),
                        authority: FrameSceneAuthority::new(key, revision),
                        valid: true,
                    });
                }
            }
            let pointer_gesture =
                knob.is_pointer_button_down_on() || knob.dragged() || knob.drag_stopped();
            let authority_enabled = if pointer_gesture && keyed_level {
                self.frame_scene_ui.level_drag.as_ref().is_some_and(|drag| {
                    drag.node == node.id
                        && drag.parameter == name
                        && drag.valid
                        && Some(drag.authority.key.id) == selected_id
                })
            } else {
                !self.frame_scene_ui.level_drag.as_ref().is_some_and(|drag| {
                    drag.node == node.id && drag.parameter == name && !drag.valid
                })
            };
            let input = self.parameter_knob_input(
                ui,
                &knob,
                normalized,
                enabled && level_enabled && authority_enabled,
                &value_text,
            );
            if enabled && level_enabled && knob.is_pointer_button_down_on() {
                self.select_primary(node.id.clone());
            }
            let step = ui.input(|i| {
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
            let default = if index == 0 {
                0.0
            } else if index <= 4 {
                FRAME_SCENE_DEFAULTS[index - 1]
            } else {
                1.0
            };
            let n = if input.reset {
                Some(if index == 5 {
                    (default + 1.0) * 0.5
                } else {
                    default
                })
            } else if let Some(high) = input.boundary {
                Some(if high { 1.0 } else { 0.0 })
            } else if input.nudge != 0 {
                Some((normalized + input.nudge as f32 * step).clamp(0.0, 1.0))
            } else {
                input.normalized
            };
            if let Some(n) = n {
                let next = if index == 5 { n * 2.0 - 1.0 } else { n };
                if (next - raw).abs() > f32::EPSILON {
                    if (1..=4).contains(&index) && !data.keyframes.is_empty() {
                        if let Some(key) = selected {
                            let mut next_values = key.values;
                            next_values[index - 1] =
                                (next.clamp(0.0, 1.0) * 65535.0).round() as u16;
                            if next_values != key.values {
                                if let Some(drag) =
                                    self.frame_scene_ui.level_drag.as_mut().filter(|d| {
                                        d.node == node.id && d.parameter == name && d.valid
                                    })
                                {
                                    let mut expected = key.clone();
                                    expected.values = next_values;
                                    drag.authority.emitted(expected, revision);
                                }
                                action = Some(frame_scene_update(
                                    node,
                                    revision,
                                    key.id,
                                    next_values,
                                    input.history_group,
                                ));
                            }
                        }
                    } else {
                        let mut args = json!({"id":node.id,"parameter":name,"value":next,"expected_revision":revision});
                        if let Some(group) = input.history_group {
                            args["history_group"] = json!(group);
                        }
                        action = Some(RackAction::Command("set_parameter", args));
                    }
                }
            }
            paint_atlas_voice_rotary(
                &painter,
                p(center.x, center.y),
                normalized,
                level_enabled && (knob.hovered() || knob.dragged() || knob.has_focus()),
                radius,
                z,
            );
            paint_atlas_voice_readout(
                &painter,
                p(center.x, center.y + radius + 13.0),
                if level_enabled { &value_text } else { "SELECT" },
                if index == 5 { 78.0 } else { 80.0 } * z,
                z,
            );
            if interactive && (index == 0 || index == 5 || data.keyframes.is_empty()) {
                if let Some(cue) = self
                    .construction_animation
                    .parameter(&node.id, &node.kind, name)
                    .cloned()
                {
                    let normalize = |value: f32| {
                        if index == 5 {
                            ((value + 1.0) * 0.5).clamp(0.0, 1.0)
                        } else {
                            value.clamp(0.0, 1.0)
                        }
                    };
                    let value_rect = Rect::from_center_size(
                        p(center.x, center.y + radius + 13.0),
                        vec2(if index == 5 { 78.0 } else { 80.0 }, 19.0) * z,
                    );
                    self.register_construction_anchor(
                        ui,
                        node,
                        name,
                        crate::construction_motion::Anchor {
                            rect: knob.rect.union(value_rect),
                            old_text: format!("{:+.0}%", cue.old * 100.0),
                            new_text: value_text.clone(),
                            marker: Some(crate::construction_motion::Marker::Rotary {
                                center: knob.rect.center(),
                                radius: (radius + 3.0) * z,
                                old: normalize(cue.old),
                                new: normalized,
                            }),
                        },
                        &[knob.id],
                    );
                }
            }
            if index != 5 {
                painter.text(
                    p(center.x, center.y + radius + 28.0),
                    Align2::CENTER_CENTER,
                    "0   ·   50   ·   100%",
                    font(8.0),
                    MUTED,
                );
            }
            knob.on_hover_text(if (1..=4).contains(&index) && !data.keyframes.is_empty() {
                "Authored value of the selected stable frame. Drag edits that keyframe, with one undo gesture. Selection never moves the live frame. Up/Down changes the value; Home/End chooses limits."
            } else if index == 5 { "Signed FRAME CV depth. The saved FRAME knob remains the authoring position used by ADD." }
            else if index == 0 { "Configured frame position. ADD uses this saved position, independent of the live CV playhead. Drag; Cmd/Ctrl for fine adjustment. Up/Down adjusts; Home/End chooses limits." }
            else { "Manual gain while no keyframes are authored. Adding a frame stores the four current authored levels." });
        }
        if ui.input(|i| !i.pointer.primary_down())
            && self
                .frame_scene_ui
                .level_drag
                .as_ref()
                .is_some_and(|d| d.node == node.id)
        {
            self.frame_scene_ui.level_drag = None;
        }
        // Offset is the physical ALL normal voltage, never an inferred input measurement.
        painter.text(
            p(58.0, 274.0),
            Align2::CENTER_CENTER,
            "ALL NORMAL",
            font(9.0),
            INK,
        );
        let offset_rect = screen(Rect::from_min_max(pos2(16.0, 282.0), pos2(100.0, 300.0)));
        let offset_spec =
            json!({"min":0,"max":10,"step":5,"values":[0,5,10],"labels":["0 V","5 V","10 V"]});
        let choices = ParameterChoices::from_spec(&offset_spec).unwrap();
        let offset = node.params.get("offset").copied().unwrap_or(10.0);
        paint_parameter_choices(
            &painter,
            offset_rect,
            choices,
            offset,
            false,
            Color32::from_rgb(136, 112, 74),
            z,
        );
        for i in 0..3 {
            let r = ui.interact(
                choices.segment(offset_rect, i),
                ui.id().with(("frame-scene-offset", &node.id, i)),
                Sense::click(),
            );
            if enabled && r.clicked() && offset != choices.value(i) {
                action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":"offset","value":choices.value(i),"expected_revision":revision}),
                ));
            }
            r.on_hover_text("Normal voltage for an unpatched ALL input. A patched ALL replaces this normal; each channel input can override ALL. MIX excludes individually patched outputs.");
        }
        let timestamp = (node
            .params
            .get("frame")
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, 1.0)
            * 65535.0)
            .round() as u16;
        painter.text(
            p(126.0, 178.0),
            Align2::LEFT_CENTER,
            format!("AUTHORED  {}/64", data.keyframes.len()),
            font(10.0),
            INK,
        );
        for (label, r, available) in [
            (
                "ADD",
                Rect::from_min_max(pos2(286.0, 168.0), pos2(340.0, 188.0)),
                data.keyframes.len() < 64
                    && !data.keyframes.iter().any(|key| key.timestamp == timestamp),
            ),
            (
                "DELETE",
                Rect::from_min_max(pos2(346.0, 168.0), pos2(410.0, 188.0)),
                selected.is_some(),
            ),
        ] {
            let button_rect = screen(r);
            painter.rect_filled(button_rect, 2.0 * z, Color32::from_rgb(237, 233, 222));
            painter.rect_stroke(
                button_rect,
                2.0 * z,
                Stroke::new(0.7 * z, MUTED),
                egui::StrokeKind::Inside,
            );
            painter.text(
                button_rect.center(),
                Align2::CENTER_CENTER,
                label,
                font(9.0),
                if available { INK } else { MUTED },
            );
            let response = ui.interact(
                button_rect,
                ui.id().with(("frame-scene-button", &node.id, label)),
                if available {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            if enabled && available && response.clicked() {
                self.select_primary(node.id.clone());
                if label == "ADD" {
                    let mut id = data.next_id;
                    while data.keyframes.iter().any(|key| key.id == id) {
                        id = id.wrapping_add(1);
                    }
                    self.frame_scene_ui.selected.insert(node.id.clone(), id);
                    action = Some(RackAction::Command(
                        "edit_frame_scene",
                        json!({"id":node.id,"edit":{"operation":"add","timestamp":timestamp,"values":values},"expected_revision":revision}),
                    ));
                } else if let Some(key) = selected {
                    action = Some(RackAction::Command(
                        "edit_frame_scene",
                        json!({"id":node.id,"edit":{"operation":"remove","keyframe_id":key.id},"expected_revision":revision}),
                    ));
                }
            }
            response.on_hover_text(if label == "ADD" { "Store the configured FRAME position and four authored levels. A position must be unique; up to 64 frames." } else { "Delete the selected stable frame. Undo restores its identity and selection." });
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(screen(Rect::from_min_max(pos2(418.0,168.0), pos2(516.0,188.0)))).id_salt((&node.id,"frame-curves")), |ui| {
            crate::panel_skin::control_style(ui, z);
            ui.style_mut().override_font_id = Some(crate::panel_skin::print_font(ui.ctx(), 10.0*z));
            ui.add_enabled_ui(interactive, |ui| { ui.menu_button("CURVES ▾", |ui| {
                ui.set_min_width(300.0);
                ui.label("Per-channel interpolation and response");
                for lane in 0..4 {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}", lane+1));
                        let easing_name = format!("easing{}",lane+1);
                        let old = node.params.get(&easing_name).copied().unwrap_or(1.0).round() as usize;
                        let mut easing = old.min(5);
                        let names = ["Step", "Linear", "Accelerating", "Decelerating", "Smooth", "Bounce"];
                        egui::ComboBox::from_id_salt((&node.id,&easing_name)).selected_text(names[easing]).show_ui(ui, |ui| {
                            for (i,label) in names.iter().enumerate() { ui.selectable_value(&mut easing,i,*label); }
                        });
                        if old != easing { action = Some(RackAction::Command("set_parameter",json!({"id":node.id,"parameter":easing_name,"value":easing,"expected_revision":revision}))); }
                        let response_name = format!("response{}", lane+1);
                        let old = node.params.get(&response_name).copied().unwrap_or(0.0);
                        let mut response_value = old;
                        let response = ui.add(egui::Slider::new(&mut response_value,0.0..=1.0).text("Response"));
                        let id = response.id.with("frame-scene-response-gesture");
                        if response.drag_started() { self.gesture = self.gesture.wrapping_add(1); ui.data_mut(|d|d.insert_temp(id,format!("frame-response-{}",self.gesture))); }
                        if response_value != old {
                            let mut args=json!({"id":node.id,"parameter":response_name,"value":response_value,"expected_revision":revision});
                            if let Some(group)=ui.data(|d|d.get_temp::<String>(id)) { args["history_group"]=json!(group); }
                            action=Some(RackAction::Command("set_parameter",args));
                        }
                        if response.drag_stopped() { ui.data_mut(|d|d.remove::<String>(id)); }
                    });
                }
                ui.label("Curves describe authored interpolation. Lamps and live frame require engine telemetry.");
            }); });
        });
        if let Some(edit) = self.frame_scene_timeline_panel(ui, node, rect, data, revision, enabled)
        {
            action = Some(edit);
        }
        let (position, measured) = frame_scene_position(node, self.module_lights.get(&node.id));
        painter.text(
            p(516.0, 289.0),
            Align2::RIGHT_CENTER,
            format!(
                "{} FRAME {:.1}%",
                if measured { "LIVE" } else { "CONFIGURED" },
                position * 100.0
            ),
            font(9.0),
            INK,
        );
        painter.text(
            p(126.0, 289.0),
            Align2::LEFT_CENTER,
            if data.keyframes.is_empty() {
                "MANUAL LEVELS".to_owned()
            } else if let Some(key) = selected {
                format!("SELECTED #{} · t {}", key.id, key.timestamp)
            } else {
                "SELECT A FRAME".to_owned()
            },
            font(9.0),
            INK,
        );
        let lights = self.module_lights.get(&node.id);
        for (i, index) in [4, 5, 6].into_iter().enumerate() {
            let strength = frame_scene_lamp(lights, index);
            let color = [
                Color32::from_rgb(226, 94, 76),
                Color32::from_rgb(113, 190, 118),
                Color32::from_rgb(102, 156, 220),
            ][i];
            self.lighting.circle(
                &painter,
                p(45.0 + i as f32 * 13.0, 172.0),
                2.1 * z,
                color.gamma_multiply(0.12 + 0.88 * strength),
                strength,
                0.12,
            );
        }
        painter.text(
            p(110.0, 172.0),
            Align2::RIGHT_CENTER,
            "SCENE",
            font(8.0),
            MUTED,
        );
        ui.interact(Rect::from_min_max(p(39.0, 167.0), p(112.0, 177.0)),
            ui.id().with(("frame-scene-palette", &node.id)), Sense::hover())
            .on_hover_text("Actual source palette RGB from engine telemetry. Dark when unavailable; stored keyframes and configured positions do not invent a live color.");
        for (port, index) in [(0, 0), (1, 1), (2, 2), (3, 3), (5, 7)] {
            let center = frame_scene_port(false, port);
            let strength = frame_scene_lamp(lights, index);
            painter.circle_filled(
                p(center.x + 25.0, center.y),
                3.0 * z,
                Color32::from_gray(72),
            );
            self.lighting.circle(
                &painter,
                p(center.x + 25.0, center.y),
                2.2 * z,
                Color32::from_rgb(230, 183, 92).gamma_multiply(0.12 + strength * 0.88),
                strength,
                0.12,
            );
        }
        action
    }
    fn frame_scene_timeline_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        data: &openpatch_core::FrameSceneData,
        revision: u64,
        enabled: bool,
    ) -> Option<RackAction> {
        let z = self.zoom;
        let p = |x, y| rect.min + vec2(x, y) * z;
        let local = frame_scene_timeline();
        let bounds = Rect::from_min_max(p(local.min.x, local.min.y), p(local.max.x, local.max.y));
        let painter = ui.painter();
        painter.rect_filled(bounds, 2.0 * z, Color32::from_rgb(29, 38, 33));
        let selected_id = self.frame_scene_ui.selected.get(&node.id).copied();
        for (lane, color) in FRAME_SCENE_COLORS.into_iter().enumerate() {
            painter.text(
                p(132.0, 214.5 + lane as f32 * 18.0),
                Align2::CENTER_CENTER,
                format!("{}", lane + 1),
                crate::panel_skin::print_font(ui.ctx(), 8.0 * z),
                color,
            );
            painter.line_segment(
                [
                    p(141.0, 223.0 + lane as f32 * 18.0),
                    p(508.0, 223.0 + lane as f32 * 18.0),
                ],
                Stroke::new(0.5 * z, Color32::from_rgb(79, 84, 69)),
            );
            // These are stored authoring points, not a simulated or measured waveform.
            for key in &data.keyframes {
                painter.circle_filled(
                    p(
                        frame_scene_key_x(key.timestamp),
                        frame_scene_lane_y(lane, key.values[lane]),
                    ),
                    if Some(key.id) == selected_id {
                        3.1
                    } else {
                        2.0
                    } * z,
                    color,
                );
            }
        }
        for key in &data.keyframes {
            let x = frame_scene_key_x(key.timestamp);
            painter.line_segment(
                [p(x, 198.0), p(x, 204.0)],
                Stroke::new(
                    if Some(key.id) == selected_id {
                        2.0
                    } else {
                        0.8
                    } * z,
                    if Some(key.id) == selected_id {
                        Color32::WHITE
                    } else {
                        MUTED
                    },
                ),
            );
        }
        let (position, measured) = frame_scene_position(node, self.module_lights.get(&node.id));
        let x = 141.0 + position * 367.0;
        let color = if measured {
            Color32::from_rgb(242, 206, 119)
        } else {
            Color32::from_rgba_unmultiplied(190, 195, 177, 110)
        };
        if measured {
            painter.line_segment([p(x, 196.0), p(x, 278.0)], Stroke::new(z, color));
        } else {
            for y in (198..278).step_by(8) {
                painter.line_segment(
                    [p(x, y as f32), p(x, (y + 4) as f32)],
                    Stroke::new(z, color),
                );
            }
        }
        let response = ui.interact(
            bounds,
            ui.id().with(("frame-scene-timeline", &node.id)),
            Sense::click_and_drag(),
        );
        let history_interrupted =
            self.parameter_history_frame == Some(ui.ctx().cumulative_frame_nr());
        let target_valid = self
            .frame_scene_ui
            .drag
            .as_mut()
            .filter(|d| d.node == node.id)
            .is_none_or(|drag| {
                data.keyframes
                    .iter()
                    .find(|key| key.id == drag.key.id)
                    .is_some_and(|key| drag.authority.observe(key, revision))
            });
        if (!enabled
            || history_interrupted
            || !target_valid
            || ui.input(|i| !i.focused || i.key_pressed(egui::Key::Escape)))
            && self
                .frame_scene_ui
                .drag
                .as_ref()
                .is_some_and(|d| d.node == node.id)
        {
            self.frame_scene_ui.drag = None;
        }
        if enabled
            && !history_interrupted
            && response.is_pointer_button_down_on()
            && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
        {
            if let Some(pointer) = response.interact_pointer_pos() {
                if let Some(key) = data.keyframes.iter().min_by(|a, b| {
                    (p(frame_scene_key_x(a.timestamp), 200.0).x - pointer.x)
                        .abs()
                        .total_cmp(&(p(frame_scene_key_x(b.timestamp), 200.0).x - pointer.x).abs())
                }) {
                    self.frame_scene_ui.selected.insert(node.id.clone(), key.id);
                    self.select_primary(node.id.clone());
                    self.gesture = self.gesture.wrapping_add(1);
                    let local_y = (pointer.y - rect.top()) / z;
                    let lane = if local_y < 207.0 {
                        None
                    } else {
                        Some(((local_y - 207.0) / 18.0).floor().clamp(0.0, 3.0) as usize)
                    };
                    self.frame_scene_ui.drag = Some(FrameSceneDrag {
                        node: node.id.clone(),
                        key: key.clone(),
                        pointer,
                        lane,
                        group: format!("frame-timeline-{}", self.gesture),
                        authority: FrameSceneAuthority::new(key, revision),
                    });
                }
            }
        }
        let mut action = None;
        if enabled && !history_interrupted && (response.dragged() || response.drag_stopped()) {
            if let Some(drag) = self
                .frame_scene_ui
                .drag
                .as_mut()
                .filter(|d| d.node == node.id)
            {
                if let Some(pointer) = response.interact_pointer_pos() {
                    if let Some(current) = data.keyframes.iter().find(|key| key.id == drag.key.id) {
                        let delta = (pointer - drag.pointer) / z;
                        if let Some(lane) = drag.lane {
                            let mut values = current.values;
                            values[lane] =
                                (f32::from(drag.key.values[lane]) - delta.y / 13.0 * 65535.0)
                                    .round()
                                    .clamp(0.0, 65535.0) as u16;
                            if values != current.values {
                                let mut expected = current.clone();
                                expected.values = values;
                                drag.authority.emitted(expected, revision);
                                action = Some(frame_scene_update(
                                    node,
                                    revision,
                                    current.id,
                                    values,
                                    Some(drag.group.clone()),
                                ));
                            }
                        } else {
                            let timestamp =
                                (f32::from(drag.key.timestamp) + delta.x / 367.0 * 65535.0)
                                    .round()
                                    .clamp(0.0, 65535.0) as u16;
                            if timestamp != current.timestamp
                                && !data
                                    .keyframes
                                    .iter()
                                    .any(|key| key.id != current.id && key.timestamp == timestamp)
                            {
                                let mut expected = current.clone();
                                expected.timestamp = timestamp;
                                drag.authority.emitted(expected, revision);
                                action = Some(RackAction::Command(
                                    "edit_frame_scene",
                                    json!({"id":node.id,"edit":{"operation":"update","keyframe_id":current.id,"timestamp":timestamp},"expected_revision":revision,"history_group":drag.group}),
                                ));
                            }
                        }
                    }
                }
            }
        }
        if (response.drag_stopped() || !ui.input(|i| i.pointer.primary_down()))
            && self
                .frame_scene_ui
                .drag
                .as_ref()
                .is_some_and(|d| d.node == node.id)
        {
            self.frame_scene_ui.drag = None;
        }
        response.on_hover_text("Authored four-lane keyframes, up to 64. Click any column to select its stable ID. Drag a top tick horizontally to move its timestamp; drag inside a lane vertically to edit that lane. Level knobs edit the selected frame. Selection never moves playback. Solid line: actual engine frame; dashed line: configured FRAME when telemetry is unavailable.");
        action
    }
}
