//! Native rack surface. Graph edits are returned to the owning session; layout
//! lives separately so moving a panel never rebuilds the audio graph.
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, PointerButton, Pos2, Rect, Sense, Stroke, Vec2,
};
use openpatch_core::{Node, Patch};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const HP: f32 = 15.0;
const ROW: f32 = 420.0;
const PANEL_HEIGHT: f32 = 396.0;
const PAPER: Color32 = Color32::from_rgb(216, 217, 211);
const INK: Color32 = Color32::from_rgb(35, 39, 40);
const MUTED: Color32 = Color32::from_rgb(94, 98, 96);
const CABLES: [Color32; 6] = [
    Color32::from_rgb(229, 74, 57),
    Color32::from_rgb(232, 181, 52),
    Color32::from_rgb(57, 155, 217),
    Color32::from_rgb(88, 174, 114),
    Color32::from_rgb(182, 107, 205),
    Color32::from_rgb(236, 132, 61),
];

pub enum RackAction {
    Command(&'static str, Value),
    OpenBrowser,
    LayoutChanged,
}

struct ParameterEdit {
    id: String,
    parameter: String,
    text: String,
    min: f32,
    max: f32,
    focus: bool,
}

pub struct Rack {
    pub selected: Option<String>,
    positions: BTreeMap<String, Pos2>,
    pan: Vec2,
    zoom: f32,
    pending: Option<String>,
    dragging_cable: bool,
    editing: Option<ParameterEdit>,
    suppress_reveal_once: bool,
}

impl Default for Rack {
    fn default() -> Self {
        Self {
            selected: None,
            positions: BTreeMap::new(),
            pan: vec2(24.0, 12.0),
            zoom: 1.0,
            pending: None,
            dragging_cable: false,
            editing: None,
            suppress_reveal_once: false,
        }
    }
}

impl Rack {
    pub fn reset_view(&mut self) {
        self.pan = vec2(24.0, 12.0);
        self.zoom = 1.0;
    }
    pub fn zoom(&self) -> f32 {
        self.zoom
    }
    pub fn set_zoom(&mut self, zoom: f32) {
        if zoom.is_finite() {
            self.zoom = zoom.clamp(0.4, 1.8);
        }
    }

    pub fn layout(&self) -> Value {
        let positions: BTreeMap<_, _> = self
            .positions
            .iter()
            .map(|(id, point)| (id, [point.x, point.y]))
            .collect();
        json!({"version":1,"zoom":self.zoom,"pan":[self.pan.x,self.pan.y],"positions":positions})
    }

    /// Validate the entire sidecar before replacing any current layout state.
    pub fn restore_layout(&mut self, value: &Value) -> Result<(), String> {
        if value["version"] != 1 {
            return Err("Unsupported rack layout version".into());
        }
        let zoom = finite_number(&value["zoom"], 0.4, 1.8)?;
        let pan = coordinate(&value["pan"])?;
        let values = value["positions"]
            .as_object()
            .ok_or("Rack layout needs a positions object")?;
        if values.len() > openpatch_core::MAX_NODES {
            return Err("Too many module positions in rack layout".into());
        }
        let mut positions = BTreeMap::new();
        for (id, point) in values {
            if id.is_empty() || id.len() > 64 {
                return Err("Invalid module ID in rack layout".into());
            }
            positions.insert(id.clone(), coordinate(point)?);
        }
        self.positions = positions;
        self.pan = pan.to_vec2();
        self.zoom = zoom;
        self.pending = None;
        self.dragging_cable = false;
        self.suppress_reveal_once = true;
        Ok(())
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        patch: &Patch,
        catalog: &Value,
        device_rate: u32,
    ) -> Option<RackAction> {
        // Capture this before reconcile assigns new panels their first position.
        // Restored sidecars own their viewport, including partial layouts.
        let suppress_reveal = std::mem::take(&mut self.suppress_reveal_once);
        let newly_selected = self
            .selected
            .as_ref()
            .filter(|id| !self.positions.contains_key(*id))
            .cloned();
        self.reconcile(patch);
        let mut action = None;
        let (canvas, response) = ui.allocate_exact_size(
            ui.available_size().max(vec2(1.0, 1.0)),
            Sense::click_and_drag(),
        );
        if !suppress_reveal {
            if let Some(node) =
                newly_selected.and_then(|id| patch.nodes.iter().find(|node| node.id == id))
            {
                self.reveal_module(node, canvas);
            }
        }
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Other,
                true,
                "Modular synthesizer rack. Right-click to add modules; middle-drag to pan.",
            )
        });
        let painter = ui.painter().with_clip_rect(canvas);
        if canvas.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or(Pos2::ZERO))) {
            let (scroll, zoom_delta, command) = ui.input(|i| {
                (
                    i.smooth_scroll_delta,
                    i.zoom_delta(),
                    i.modifiers.command || i.modifiers.ctrl,
                )
            });
            if zoom_delta != 1.0 {
                let pointer = ui
                    .input(|i| i.pointer.hover_pos())
                    .unwrap_or(canvas.center());
                let local = pointer - canvas.min;
                let world = (local - self.pan) / self.zoom;
                self.set_zoom(self.zoom * zoom_delta);
                self.pan = local - world * self.zoom;
            } else if !command {
                self.pan += scroll;
            }
        }
        if response.dragged_by(PointerButton::Middle) {
            self.pan += response.drag_delta();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pending = None;
            self.dragging_cable = false;
            self.editing = None;
        }
        if !ui.ctx().wants_keyboard_input()
            && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            if let Some(id) = self.selected.take() {
                action = Some(RackAction::Command("remove_module", json!({"id":id})));
            }
        }
        paint_background(&painter, canvas, self.pan, self.zoom);

        let mut outputs = BTreeMap::<String, Pos2>::new();
        let mut inputs = BTreeMap::<(String, String), Pos2>::new();
        let mut input_targets = Vec::new();
        let mut pointer_on_module = false;
        for node in &patch.nodes {
            let position = self.positions[&node.id];
            let width = panel_width(&node.kind);
            let rect = Rect::from_min_size(
                canvas.min + self.pan + position.to_vec2() * self.zoom,
                vec2(width, PANEL_HEIGHT) * self.zoom,
            );
            let point = |x, y| rect.min + vec2(x, y) * self.zoom;
            let module = catalog["modules"]
                .as_array()
                .and_then(|modules| modules.iter().find(|item| item["kind"] == node.kind));
            let ports = module
                .and_then(|item| item["inputs"].as_array())
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let output_point = point(width * 0.5, 374.0);
            outputs.insert(node.id.clone(), output_point);
            for (index, port) in ports.iter().filter_map(Value::as_str).enumerate() {
                let location = point(width * (index + 1) as f32 / (ports.len() + 1) as f32, 332.0);
                inputs.insert((node.id.clone(), port.to_string()), location);
                input_targets.push((node.id.clone(), port.to_string(), location));
            }
            pointer_on_module |=
                ui.input(|i| i.pointer.hover_pos().is_some_and(|p| rect.contains(p)));
            if !rect.intersects(canvas) {
                continue;
            }
            let panel_response =
                ui.interact(rect, ui.id().with(("module", &node.id)), Sense::click());
            panel_response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    true,
                    self.selected.as_ref() == Some(&node.id),
                    format!("{} module {}", title(&node.kind), node.id),
                )
            });
            if panel_response.clicked() {
                self.selected = Some(node.id.clone());
            }
            panel_response.context_menu(|ui| {
                ui.strong(format!("{}  /  {}", title(&node.kind), node.id));
                ui.separator();
                if ui.button("Use as audio output").clicked() {
                    action = Some(RackAction::Command("set_output", json!({"id":node.id})));
                    ui.close();
                }
                if ui.button("Remove module").clicked() {
                    action = Some(RackAction::Command("remove_module", json!({"id":node.id})));
                    self.selected = None;
                    ui.close();
                }
            });
            paint_panel(
                &painter,
                rect,
                node,
                self.selected.as_ref() == Some(&node.id),
                patch.output.as_ref() == Some(&node.id),
                self.zoom,
            );
            let header = Rect::from_min_max(point(15.0, 19.0), point(width - 15.0, 69.0));
            let header_response = ui.interact(
                header,
                ui.id().with(("module-header", &node.id)),
                Sense::click_and_drag(),
            );
            header_response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("Move {} module", node.id),
                )
            });
            if header_response.clicked() || header_response.drag_started() {
                self.selected = Some(node.id.clone());
            }
            if header_response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            if header_response.dragged_by(PointerButton::Primary) {
                *self.positions.get_mut(&node.id).unwrap() +=
                    header_response.drag_delta() / self.zoom;
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if header_response.drag_stopped_by(PointerButton::Primary) {
                let other: Vec<_> = patch
                    .nodes
                    .iter()
                    .filter(|other| other.id != node.id)
                    .map(|other| (self.positions[&other.id], panel_width(&other.kind)))
                    .collect();
                let snapped = settle(self.positions[&node.id], width, &other);
                self.positions.insert(node.id.clone(), snapped);
                if action.is_none() {
                    action = Some(RackAction::LayoutChanged);
                }
            }
            header_response.context_menu(|ui| {
                ui.strong(format!("{}  /  {}", title(&node.kind), node.id));
                ui.separator();
                if ui.button("Use as audio output").clicked() {
                    action = Some(RackAction::Command("set_output", json!({"id":node.id})));
                    ui.close();
                }
                if ui.button("Remove module").clicked() {
                    action = Some(RackAction::Command("remove_module", json!({"id":node.id})));
                    self.selected = None;
                    ui.close();
                }
            });
            header_response.on_hover_text("Drag to move. Panels snap to rack rows and free space.");

            let parameters = ordered_parameters(node, module);
            let count = parameters.len();
            for (index, (name, spec)) in parameters.iter().enumerate() {
                let min = spec["min"].as_f64().unwrap_or(0.0) as f32;
                let mut max = spec["max"].as_f64().unwrap_or(1.0) as f32;
                if let Some(ratio) = spec["sample_rate_max_ratio"].as_f64() {
                    max = max.min(patch.sample_rate.min(device_rate) as f32 * ratio as f32);
                }
                let default = (spec["default"].as_f64().unwrap_or(0.0) as f32).clamp(min, max);
                let value = node
                    .params
                    .get(*name)
                    .copied()
                    .unwrap_or(default)
                    .clamp(min, max);
                let (y, radius) = match count {
                    1 => (183.0, 42.0),
                    2 => (135.0 + index as f32 * 110.0, 32.0),
                    _ => (114.0 + index as f32 * 78.0, 18.0),
                };
                let center = point(width * 0.5, y);
                let logarithmic = *name == "frequency" || *name == "cutoff";
                let normalized = normalize(value, min, max, logarithmic);
                let knob_rect =
                    Rect::from_center_size(center, Vec2::splat((radius + 10.0) * 2.0 * self.zoom));
                let knob = ui.interact(
                    knob_rect,
                    ui.id().with(("knob", &node.id, name)),
                    Sense::click_and_drag(),
                );
                knob.widget_info(|| {
                    egui::WidgetInfo::slider(true, value as f64, format!("{} {}", node.id, name))
                });
                if knob.drag_started() {
                    self.selected = Some(node.id.clone());
                }
                let modifiers = ui.input(|i| i.modifiers);
                let speed = if modifiers.command || modifiers.ctrl {
                    if modifiers.shift {
                        0.000_02
                    } else {
                        0.000_2
                    }
                } else if modifiers.shift {
                    0.01
                } else {
                    0.003
                };
                let change = -knob.drag_motion().y * speed;
                let new_value = if knob.double_clicked() {
                    Some(default)
                } else if change != 0.0 {
                    Some(denormalize(
                        (normalized + change).clamp(0.0, 1.0),
                        min,
                        max,
                        logarithmic,
                    ))
                } else {
                    None
                };
                if let Some(new_value) = new_value {
                    action = Some(RackAction::Command(
                        "set_parameter",
                        json!({"id":node.id,"parameter":name,"value":new_value}),
                    ));
                }
                if knob.hovered() || knob.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                }
                paint_knob(
                    &painter,
                    center,
                    radius * self.zoom,
                    normalized,
                    knob.hovered() || knob.dragged(),
                    self.zoom,
                );
                painter.text(
                    center - vec2(0.0, (radius + 13.0) * self.zoom),
                    Align2::CENTER_BOTTOM,
                    parameter_label(name),
                    FontId::proportional(10.0 * self.zoom),
                    INK,
                );
                let value_offset = if count >= 3 { 9.0 } else { 15.0 };
                let value_center = center + vec2(0.0, (radius + value_offset) * self.zoom);
                painter.text(
                    value_center,
                    Align2::CENTER_CENTER,
                    format_value(value, spec["unit"].as_str().unwrap_or("")),
                    FontId::monospace(11.0 * self.zoom),
                    INK,
                );
                let value_response = ui.interact(
                    Rect::from_center_size(value_center, vec2(width - 20.0, 19.0) * self.zoom),
                    ui.id().with(("value", &node.id, name)),
                    Sense::click(),
                );
                value_response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        format!("Edit {} {} value, {}", node.id, name, value),
                    )
                });
                if value_response.double_clicked() {
                    self.editing = Some(ParameterEdit {
                        id: node.id.clone(),
                        parameter: name.to_string(),
                        text: value.to_string(),
                        min,
                        max,
                        focus: true,
                    });
                }
                knob.on_hover_text(format!(
                    "{}: {}\nDrag vertically; Cmd/Ctrl for fine adjustment. Double-click to reset.",
                    name,
                    format_value(value, spec["unit"].as_str().unwrap_or(""))
                ));
                value_response.on_hover_text("Double-click to type a value");
            }
            if count == 0 {
                paint_center_mark(
                    &painter,
                    rect,
                    &node.kind,
                    self.zoom,
                    patch.output.as_ref() == Some(&node.id),
                );
            }
            for port in ports.iter().filter_map(Value::as_str) {
                let location = inputs[&(node.id.clone(), port.to_string())];
                let connected = patch
                    .cables
                    .iter()
                    .any(|c| c.to == node.id && c.input == port);
                let jack = jack(
                    ui,
                    &painter,
                    location,
                    ui.id().with(("input", &node.id, port)),
                    port,
                    connected,
                    false,
                    self.zoom,
                );
                if jack.clicked() {
                    self.selected = Some(node.id.clone());
                    if let Some(from) = self.pending.take() {
                        action = Some(RackAction::Command(
                            "connect",
                            json!({"from":from,"to":node.id,"input":port}),
                        ));
                        self.dragging_cable = false;
                    }
                }
                if jack.secondary_clicked() && connected {
                    action = Some(RackAction::Command(
                        "disconnect",
                        json!({"to":node.id,"input":port}),
                    ));
                }
            }
            let connected = patch.cables.iter().any(|c| c.from == node.id);
            let jack = jack(
                ui,
                &painter,
                output_point,
                ui.id().with(("output", &node.id)),
                "OUT",
                connected,
                true,
                self.zoom,
            );
            if jack.clicked() {
                self.pending = if self.pending.as_ref() == Some(&node.id) {
                    None
                } else {
                    Some(node.id.clone())
                };
                self.selected = Some(node.id.clone());
            }
            if jack.drag_started_by(PointerButton::Primary) {
                self.pending = Some(node.id.clone());
                self.dragging_cable = true;
                self.selected = Some(node.id.clone());
            }
        }

        // Cables intentionally paint after every module, including their plug caps.
        for (index, cable) in patch.cables.iter().enumerate() {
            if let (Some(&from), Some(&to)) = (
                outputs.get(&cable.from),
                inputs.get(&(cable.to.clone(), cable.input.clone())),
            ) {
                paint_cable(
                    &painter,
                    from,
                    to,
                    CABLES[index % CABLES.len()],
                    self.zoom,
                    true,
                );
            }
        }
        if self.dragging_cable && ui.input(|i| i.pointer.button_released(PointerButton::Primary)) {
            let pointer = ui.input(|i| i.pointer.interact_pos());
            if let Some((to, input, _)) = input_targets.iter().find(|(_, _, location)| {
                pointer.is_some_and(|p| {
                    canvas.contains(p) && p.distance(*location) <= 17.0 * self.zoom
                })
            }) {
                if let Some(from) = self.pending.take() {
                    action = Some(RackAction::Command(
                        "connect",
                        json!({"from":from,"to":to,"input":input}),
                    ));
                }
            } else {
                self.pending = None;
            }
            self.dragging_cable = false;
        }
        if let Some(source) = &self.pending {
            if let (Some(&from), Some(pointer)) =
                (outputs.get(source), ui.input(|i| i.pointer.hover_pos()))
            {
                paint_cable(
                    &painter,
                    from,
                    pointer,
                    CABLES[patch.cables.len() % CABLES.len()],
                    self.zoom,
                    false,
                );
            }
        }
        if !pointer_on_module {
            if response.secondary_clicked() || response.double_clicked() {
                action = Some(RackAction::OpenBrowser);
            }
            if response.clicked() {
                self.selected = None;
                self.pending = None;
            }
        }
        if patch.nodes.is_empty() {
            painter.text(
                canvas.center() - vec2(0.0, 12.0),
                Align2::CENTER_CENTER,
                "Your rack is ready",
                FontId::proportional(24.0),
                Color32::from_gray(171),
            );
            painter.text(
                canvas.center() + vec2(0.0, 22.0),
                Align2::CENTER_CENTER,
                "Right-click or press Enter to add a module",
                FontId::proportional(13.0),
                Color32::from_gray(118),
            );
        }
        self.parameter_editor(ui.ctx(), &mut action);
        action
    }

    fn reconcile(&mut self, patch: &Patch) {
        self.positions
            .retain(|id, _| patch.nodes.iter().any(|node| &node.id == id));
        if self
            .selected
            .as_ref()
            .is_some_and(|id| !patch.nodes.iter().any(|node| &node.id == id))
        {
            self.selected = None;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|id| !patch.nodes.iter().any(|node| &node.id == id))
        {
            self.pending = None;
            self.dragging_cable = false;
        }
        for node in &patch.nodes {
            if self.positions.contains_key(&node.id) {
                continue;
            }
            let mut x = 0.0_f32;
            for other in &patch.nodes {
                if let Some(position) = self.positions.get(&other.id) {
                    if position.y.abs() < 1.0 {
                        x = x.max(position.x + panel_width(&other.kind));
                    }
                }
            }
            self.positions.insert(node.id.clone(), pos2(x, 0.0));
        }
    }

    fn reveal_module(&mut self, node: &Node, canvas: Rect) {
        let inset = 12.0_f32
            .min(canvas.width() * 0.25)
            .min(canvas.height() * 0.25);
        let viewport = canvas.shrink(inset);
        let min = canvas.min + self.pan + self.positions[&node.id].to_vec2() * self.zoom;
        // When zoom makes the panel taller than the viewport, prioritize its
        // header instead of changing the user's zoom or hiding its top edge.
        let size = (vec2(panel_width(&node.kind), PANEL_HEIGHT) * self.zoom).min(viewport.size());
        let rect = Rect::from_min_size(min, size);
        self.pan.x += if rect.left() < viewport.left() {
            viewport.left() - rect.left()
        } else if rect.right() > viewport.right() {
            viewport.right() - rect.right()
        } else {
            0.0
        };
        self.pan.y += if rect.top() < viewport.top() {
            viewport.top() - rect.top()
        } else if rect.bottom() > viewport.bottom() {
            viewport.bottom() - rect.bottom()
        } else {
            0.0
        };
    }

    fn parameter_editor(&mut self, ctx: &egui::Context, action: &mut Option<RackAction>) {
        let Some(edit) = &mut self.editing else {
            return;
        };
        let mut close = false;
        egui::Window::new(format!("{} / {}", edit.id, edit.parameter)).id(egui::Id::new("rack-parameter-editor")).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(format!("Range: {} to {}", edit.min, edit.max));
            let input = ui.add(egui::TextEdit::singleline(&mut edit.text).desired_width(220.0));
            if edit.focus { input.request_focus(); edit.focus = false; }
            let parsed = edit.text.parse::<f32>().ok().filter(|value| value.is_finite() && *value >= edit.min && *value <= edit.max);
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                if ui.add_enabled(parsed.is_some(), egui::Button::new("Apply")).clicked() || (enter && parsed.is_some()) {
                    *action = Some(RackAction::Command("set_parameter", json!({"id":edit.id,"parameter":edit.parameter,"value":parsed.unwrap()}))); close = true;
                }
                if ui.button("Cancel").clicked() { close = true; }
            });
            if parsed.is_none() { ui.colored_label(Color32::LIGHT_RED, "Enter a finite value within the range."); }
        });
        if close {
            self.editing = None;
        }
    }
}

fn finite_number(value: &Value, min: f32, max: f32) -> Result<f32, String> {
    let value = value
        .as_f64()
        .ok_or("Rack layout contains a nonnumeric coordinate")? as f32;
    if !value.is_finite() || value < min || value > max {
        return Err("Rack layout coordinate is outside its valid range".into());
    }
    Ok(value)
}
fn coordinate(value: &Value) -> Result<Pos2, String> {
    let pair = value
        .as_array()
        .filter(|pair| pair.len() == 2)
        .ok_or("Rack layout coordinates need two numbers")?;
    Ok(pos2(
        finite_number(&pair[0], -100_000.0, 100_000.0)?,
        finite_number(&pair[1], -100_000.0, 100_000.0)?,
    ))
}
fn panel_width(kind: &str) -> f32 {
    match kind {
        "oscillator" => HP * 14.0,
        "lfo" | "lowpass" => HP * 12.0,
        "gain" | "mixer" => HP * 10.0,
        "constant" | "output" => HP * 8.0,
        _ => HP * 12.0,
    }
}
fn title(kind: &str) -> &str {
    match kind {
        "oscillator" => "SINE VCO",
        "lfo" => "LFO",
        "gain" => "VCA",
        "lowpass" => "LOW-PASS",
        "mixer" => "MIXER",
        "constant" => "DC",
        "output" => "AUDIO",
        other => other,
    }
}
fn parameter_label(name: &str) -> &str {
    match name {
        "frequency" => "FREQUENCY",
        "amplitude" => "LEVEL",
        "offset" => "OFFSET",
        "gain" => "GAIN",
        "cutoff" => "CUTOFF",
        "value" => "VOLTAGE",
        name => name,
    }
}
fn ordered_parameters<'a>(node: &Node, module: Option<&'a Value>) -> Vec<(&'a str, &'a Value)> {
    let Some(params) = module.and_then(|module| module["params"].as_object()) else {
        return Vec::new();
    };
    let order: &[&str] = if node.kind == "lfo" {
        &["frequency", "amplitude", "offset"]
    } else {
        &["frequency", "amplitude", "gain", "cutoff", "value"]
    };
    order
        .iter()
        .filter_map(|name| {
            params
                .get_key_value(*name)
                .map(|(name, spec)| (name.as_str(), spec))
        })
        .collect()
}
fn normalize(value: f32, min: f32, max: f32, logarithmic: bool) -> f32 {
    if max <= min {
        return 0.0;
    }
    if logarithmic {
        (value.clamp(min, max) - min).ln_1p() / (max - min).ln_1p()
    } else {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    }
}
fn denormalize(value: f32, min: f32, max: f32, logarithmic: bool) -> f32 {
    let value = value.clamp(0.0, 1.0);
    if logarithmic {
        (min + (value * (max - min).ln_1p()).exp_m1()).clamp(min, max)
    } else {
        min + value * (max - min)
    }
}
fn format_value(value: f32, unit: &str) -> String {
    if unit == "Hz" {
        if value >= 1000.0 {
            format!("{:.2} kHz", value / 1000.0)
        } else if value >= 100.0 {
            format!("{value:.1} Hz")
        } else {
            format!("{value:.2} Hz")
        }
    } else {
        format!("{value:.3}")
    }
}

fn settle(position: Pos2, width: f32, others: &[(Pos2, f32)]) -> Pos2 {
    let y = (position.y / ROW).round().max(0.0) * ROW;
    let requested = (position.x / HP).round().max(0.0) * HP;
    let mut candidates = vec![requested, 0.0];
    for (point, other_width) in others {
        if (point.y - y).abs() < PANEL_HEIGHT {
            candidates.push(((point.x + other_width) / HP).ceil() * HP);
            candidates.push(((point.x - width) / HP).floor() * HP);
        }
    }
    candidates.retain(|x| {
        *x >= 0.0
            && others.iter().all(|(point, other_width)| {
                (point.y - y).abs() >= PANEL_HEIGHT
                    || *x + width <= point.x
                    || *x >= point.x + other_width
            })
    });
    let x = candidates
        .into_iter()
        .min_by(|a, b| (a - requested).abs().total_cmp(&(b - requested).abs()))
        .unwrap_or(requested);
    pos2(x, y)
}

fn paint_background(painter: &egui::Painter, canvas: Rect, pan: Vec2, zoom: f32) {
    painter.rect_filled(canvas, 0, Color32::from_rgb(27, 29, 30));
    let line_spacing = (4.0 * zoom).max(2.0);
    let mut y = canvas.top();
    while y < canvas.bottom() {
        painter.line_segment(
            [pos2(canvas.left(), y), pos2(canvas.right(), y)],
            Stroke::new(1.0_f32, Color32::from_white_alpha(3)),
        );
        y += line_spacing;
    }
    let first_row = ((-pan.y) / (ROW * zoom)).floor() as i32 - 1;
    let last_row = ((canvas.height() - pan.y) / (ROW * zoom)).ceil() as i32 + 1;
    for row in first_row..=last_row {
        let y = canvas.top() + pan.y + row as f32 * ROW * zoom;
        let rail = Rect::from_min_size(
            pos2(canvas.left(), y - 9.0 * zoom),
            vec2(canvas.width(), 18.0 * zoom),
        );
        painter.rect_filled(rail, 0, Color32::from_rgb(64, 68, 69));
        painter.line_segment(
            [rail.left_top(), rail.right_top()],
            Stroke::new(1.0_f32, Color32::from_gray(105)),
        );
        painter.line_segment(
            [pos2(rail.left(), y + 1.0), pos2(rail.right(), y + 1.0)],
            Stroke::new(2.0 * zoom, Color32::from_gray(30)),
        );
        let start = ((-pan.x) / (HP * zoom)).floor() as i32;
        let count = (canvas.width() / (HP * zoom)).ceil() as i32 + 2;
        for slot in start..start + count {
            let x = canvas.left() + pan.x + slot as f32 * HP * zoom;
            painter.rect_filled(
                Rect::from_center_size(pos2(x + 7.5 * zoom, y), vec2(6.0, 3.0) * zoom),
                1.0,
                Color32::from_gray(28),
            );
        }
    }
}

fn paint_panel(
    painter: &egui::Painter,
    rect: Rect,
    node: &Node,
    selected: bool,
    master: bool,
    zoom: f32,
) {
    painter.rect_filled(
        rect.translate(vec2(3.0, 4.0) * zoom),
        0,
        Color32::from_black_alpha(95),
    );
    painter.rect_filled(rect, 0, PAPER);
    painter.line_segment(
        [rect.left_top(), rect.left_bottom()],
        Stroke::new(1.0_f32, Color32::from_gray(245)),
    );
    painter.line_segment(
        [rect.right_top(), rect.right_bottom()],
        Stroke::new(1.0_f32, Color32::from_gray(110)),
    );
    if selected {
        painter.rect_stroke(
            rect.shrink(1.0),
            0,
            Stroke::new(1.5_f32, Color32::from_rgb(77, 156, 182)),
            egui::StrokeKind::Inside,
        );
    }
    let point = |x, y| rect.min + vec2(x, y) * zoom;
    let width = rect.width() / zoom;
    for x in [8.0, width - 8.0] {
        for y in [9.0, PANEL_HEIGHT - 9.0] {
            let center = point(x, y);
            painter.circle_filled(center, 3.7 * zoom, Color32::from_gray(112));
            painter.circle_stroke(
                center,
                3.7 * zoom,
                Stroke::new(0.8 * zoom, Color32::from_gray(237)),
            );
            painter.line_segment(
                [
                    center - vec2(2.4, 0.0) * zoom,
                    center + vec2(2.4, 0.0) * zoom,
                ],
                Stroke::new(1.0 * zoom, Color32::from_gray(57)),
            );
        }
    }
    painter.text(
        point(width * 0.5, 32.0),
        Align2::CENTER_CENTER,
        title(&node.kind),
        FontId::proportional(if width < 140.0 { 17.0 } else { 20.0 } * zoom),
        INK,
    );
    painter.text(
        point(width * 0.5, 55.0),
        Align2::CENTER_CENTER,
        &node.id,
        FontId::monospace(9.0 * zoom),
        MUTED,
    );
    painter.line_segment(
        [point(22.0, 65.0), point(width - 22.0, 65.0)],
        Stroke::new(1.0_f32, Color32::from_gray(173)),
    );
    painter.text(
        point(width * 0.5, PANEL_HEIGHT - 7.0),
        Align2::CENTER_CENTER,
        "OPENPATCH",
        FontId::proportional(7.0 * zoom),
        MUTED,
    );
    if master {
        painter.circle_filled(
            point(width - 19.0, 20.0),
            3.1 * zoom,
            Color32::from_rgb(55, 154, 100),
        );
    }
}

fn paint_knob(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    normalized: f32,
    active: bool,
    zoom: f32,
) {
    let angle = |value: f32| (-225.0 + value * 270.0).to_radians();
    for step in 0..=10 {
        let a = angle(step as f32 / 10.0);
        let direction = vec2(a.cos(), a.sin());
        painter.line_segment(
            [
                center + direction * (radius + 6.0 * zoom),
                center + direction * (radius + 9.0 * zoom),
            ],
            Stroke::new(1.0 * zoom, MUTED),
        );
    }
    painter.circle_filled(
        center + vec2(1.0, 3.0) * zoom,
        radius + 2.0 * zoom,
        Color32::from_black_alpha(53),
    );
    painter.circle_filled(center, radius + 1.0 * zoom, Color32::from_gray(45));
    painter.circle_stroke(
        center,
        radius,
        Stroke::new(
            1.3 * zoom,
            Color32::from_gray(if active { 105 } else { 67 }),
        ),
    );
    painter.circle_filled(center, radius * 0.82, Color32::from_rgb(27, 30, 31));
    for step in 0..24 {
        let a = step as f32 * std::f32::consts::TAU / 24.0;
        let direction = vec2(a.cos(), a.sin());
        painter.line_segment(
            [
                center + direction * radius * 0.86,
                center + direction * radius * 0.96,
            ],
            Stroke::new(1.0 * zoom, Color32::from_gray(74)),
        );
    }
    let a = angle(normalized);
    let direction = vec2(a.cos(), a.sin());
    painter.line_segment(
        [
            center + direction * radius * 0.29,
            center + direction * radius * 0.73,
        ],
        Stroke::new(2.8 * zoom, Color32::from_gray(235)),
    );
    painter.circle_filled(
        center - vec2(radius * 0.25, radius * 0.25),
        radius * 0.10,
        Color32::from_white_alpha(8),
    );
}

fn paint_center_mark(painter: &egui::Painter, rect: Rect, kind: &str, zoom: f32, master: bool) {
    let center = rect.min + vec2(rect.width() / zoom * 0.5, 179.0) * zoom;
    if kind == "mixer" {
        painter.line_segment(
            [
                center - vec2(21.0, 0.0) * zoom,
                center + vec2(21.0, 0.0) * zoom,
            ],
            Stroke::new(3.0 * zoom, INK),
        );
        painter.line_segment(
            [
                center - vec2(0.0, 21.0) * zoom,
                center + vec2(0.0, 21.0) * zoom,
            ],
            Stroke::new(3.0 * zoom, INK),
        );
        painter.text(
            center + vec2(0.0, 55.0) * zoom,
            Align2::CENTER_CENTER,
            "A + B",
            FontId::monospace(11.0 * zoom),
            MUTED,
        );
    } else {
        for (i, height) in [14.0, 27.0, 39.0, 22.0, 32.0].iter().enumerate() {
            let x = center.x + (i as f32 - 2.0) * 11.0 * zoom;
            painter.rect_filled(
                Rect::from_center_size(pos2(x, center.y), vec2(5.0, *height * 2.0) * zoom),
                1.0,
                if master {
                    Color32::from_rgb(70, 119, 97)
                } else {
                    MUTED
                },
            );
        }
        painter.text(
            center + vec2(0.0, 68.0) * zoom,
            Align2::CENTER_CENTER,
            if master {
                "MASTER OUTPUT"
            } else {
                "MONO TO DEVICE"
            },
            FontId::proportional(9.0 * zoom),
            MUTED,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn jack(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    center: Pos2,
    id: egui::Id,
    label: &str,
    connected: bool,
    output: bool,
    zoom: f32,
) -> egui::Response {
    let response = ui.interact(
        Rect::from_center_size(center, Vec2::splat(29.0 * zoom)),
        id,
        Sense::click_and_drag(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!(
                "{} {} jack{}",
                label,
                if output { "output" } else { "input" },
                if connected { ", connected" } else { "" }
            ),
        )
    });
    let label_rect =
        Rect::from_center_size(center - vec2(0.0, 19.0) * zoom, vec2(39.0, 13.0) * zoom);
    if output {
        painter.rect_filled(label_rect, 1.0, INK);
    }
    painter.text(
        label_rect.center(),
        Align2::CENTER_CENTER,
        label.to_uppercase(),
        FontId::proportional(9.0 * zoom),
        if output { PAPER } else { INK },
    );
    painter.circle_filled(
        center + vec2(0.0, 1.0) * zoom,
        10.0 * zoom,
        Color32::from_gray(145),
    );
    painter.circle_filled(center, 9.0 * zoom, Color32::from_gray(216));
    painter.circle_stroke(
        center,
        9.0 * zoom,
        Stroke::new(1.2 * zoom, Color32::from_gray(100)),
    );
    painter.circle_filled(center, 6.0 * zoom, Color32::from_gray(39));
    painter.circle_filled(center, 3.6 * zoom, Color32::from_gray(13));
    if response.hovered() {
        painter.circle_stroke(
            center,
            11.0 * zoom,
            Stroke::new(1.8 * zoom, Color32::from_rgb(63, 145, 174)),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.on_hover_text(if output {
        "Drag to an input, or click OUT then click an input."
    } else {
        "Connect a cable here. Right-click to unplug."
    })
}

fn paint_cable(
    painter: &egui::Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    zoom: f32,
    plugged: bool,
) {
    let sag = (from.distance(to) * 0.29 + 38.0 * zoom).clamp(45.0 * zoom, 195.0 * zoom);
    let points = [from, from + vec2(0.0, sag), to + vec2(0.0, sag), to];
    let curve = |points, width, color| {
        egui::epaint::CubicBezierShape::from_points_stroke(
            points,
            false,
            Color32::TRANSPARENT,
            Stroke::new(width, color),
        )
    };
    painter.add(curve(
        points.map(|point| point + vec2(1.0, 3.0) * zoom),
        7.0 * zoom,
        Color32::from_black_alpha(100),
    ));
    painter.add(curve(points, 5.0 * zoom, color.gamma_multiply(0.62)));
    painter.add(curve(
        points.map(|point| point - vec2(0.0, 0.7) * zoom),
        3.0 * zoom,
        color,
    ));
    for (index, point) in [from, to].iter().enumerate() {
        if index == 1 && !plugged {
            continue;
        }
        painter.circle_filled(
            *point + vec2(0.0, 1.0) * zoom,
            7.0 * zoom,
            Color32::from_black_alpha(110),
        );
        painter.circle_filled(*point, 6.1 * zoom, color.gamma_multiply(0.65));
        painter.circle_stroke(*point, 5.3 * zoom, Stroke::new(1.4 * zoom, color));
        painter.circle_filled(*point, 2.0 * zoom, Color32::from_gray(33));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        rack: &mut Rack,
        patch: &Patch,
        time: f64,
        events: Vec<egui::Event>,
    ) -> Option<RackAction> {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut action = None;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    action = rack.show(ui, patch, &openpatch_core::catalog(), 48000);
                });
        });
        action
    }

    fn button(position: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: position,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn pointer_gestures_patch_jacks_adjust_knobs_and_snap_modules() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch = openpatch_core::demo_patch();
        patch.cables.clear();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let output = pos2(129.0, 386.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(output), button(output, true)],
        );
        frame(&ctx, &mut rack, &patch, 1.1, vec![button(output, false)]);
        assert_eq!(rack.pending.as_deref(), Some("tone"));
        let input = pos2(464.0, 344.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            2.0,
            vec![egui::Event::PointerMoved(input), button(input, true)],
        );
        let action = frame(&ctx, &mut rack, &patch, 2.1, vec![button(input, false)]);
        assert!(
            matches!(action, Some(RackAction::Command("connect", args)) if args["from"] == "tone" && args["to"] == "vca" && args["input"] == "signal")
        );
        let knob = pos2(129.0, 147.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            3.0,
            vec![egui::Event::PointerMoved(knob), button(knob, true)],
        );
        let action = frame(
            &ctx,
            &mut rack,
            &patch,
            3.1,
            vec![egui::Event::PointerMoved(knob - vec2(0.0, 25.0))],
        );
        assert!(
            matches!(action, Some(RackAction::Command("set_parameter", args)) if args["id"] == "tone" && args["parameter"] == "frequency" && args["value"].as_f64().unwrap() > 220.0)
        );
        frame(
            &ctx,
            &mut rack,
            &patch,
            3.2,
            vec![button(knob - vec2(0.0, 25.0), false)],
        );
        frame(
            &ctx,
            &mut rack,
            &patch,
            3.3,
            vec![egui::Event::PointerMoved(output), button(output, true)],
        );
        frame(
            &ctx,
            &mut rack,
            &patch,
            3.4,
            vec![egui::Event::PointerMoved(input)],
        );
        let action = frame(&ctx, &mut rack, &patch, 3.5, vec![button(input, false)]);
        assert!(
            matches!(action, Some(RackAction::Command("connect", args)) if args["from"] == "tone" && args["to"] == "vca")
        );
        let header = pos2(129.0, 44.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            4.0,
            vec![egui::Event::PointerMoved(header), button(header, true)],
        );
        let moved = header + vec2(105.0, 420.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            4.1,
            vec![egui::Event::PointerMoved(moved)],
        );
        let action = frame(&ctx, &mut rack, &patch, 4.2, vec![button(moved, false)]);
        assert!(matches!(action, Some(RackAction::LayoutChanged)));
        assert_eq!(rack.positions["tone"], pos2(105.0, 420.0));
    }
    #[test]
    fn knob_mapping_roundtrips_zero_negative_and_frequency_ranges() {
        for (min, max, logarithmic) in [
            (0.0, 20000.0, true),
            (1.0, 20000.0, true),
            (-16.0, 16.0, false),
            (0.0, 1.0, false),
        ] {
            for step in 0..=100 {
                let normalized = step as f32 / 100.0;
                let value = denormalize(normalized, min, max, logarithmic);
                assert!((normalize(value, min, max, logarithmic) - normalized).abs() < 0.00001);
                assert!(value >= min && value <= max);
            }
        }
    }
    #[test]
    fn newly_added_selection_is_revealed_once_without_overriding_restored_views() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch = openpatch_core::demo_patch();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let initial_pan = rack.pan;
        for id in ["new-one", "new-two"] {
            let mut node = patch.nodes[0].clone();
            node.id = id.into();
            patch.nodes.push(node);
            rack.selected = Some(id.into());
            frame(&ctx, &mut rack, &patch, 1.0, vec![]);
            if id == "new-one" {
                assert_eq!(
                    rack.pan, initial_pan,
                    "a visible addition should not move the view"
                );
            }
        }
        assert!(
            rack.pan.x < initial_pan.x,
            "the appended panel was offscreen"
        );
        let selected_min = rack.pan + rack.positions["new-two"].to_vec2() * rack.zoom;
        let selected_max = selected_min + vec2(panel_width("oscillator"), PANEL_HEIGHT) * rack.zoom;
        assert!(selected_min.x >= 12.0 && selected_max.x <= 1088.0);
        assert!(selected_min.y >= 12.0 && selected_max.y <= 688.0);

        rack.pan = vec2(180.0, -230.0);
        frame(&ctx, &mut rack, &patch, 2.0, vec![]);
        assert_eq!(
            rack.pan,
            vec2(180.0, -230.0),
            "ordinary frames must respect manual panning"
        );
        let mut layout = rack.layout();
        layout["positions"]
            .as_object_mut()
            .unwrap()
            .remove("new-two");
        layout["pan"] = json!([-500.0, -170.0]);
        rack.restore_layout(&layout).unwrap();
        for time in [3.0, 4.0] {
            frame(&ctx, &mut rack, &patch, time, vec![]);
            assert_eq!(
                rack.pan,
                vec2(-500.0, -170.0),
                "loading a partial sidecar must keep its viewport"
            );
        }
        assert_eq!(rack.selected.as_deref(), Some("new-two"));
    }
    #[test]
    fn placement_snaps_to_rows_and_nearest_free_hp_without_overlap() {
        let occupied = [(pos2(0.0, 0.0), 210.0), (pos2(390.0, 0.0), 150.0)];
        assert_eq!(
            settle(pos2(230.0, 17.0), 180.0, &occupied),
            pos2(210.0, 0.0)
        );
        assert_eq!(
            settle(pos2(-30.0, 410.0), 180.0, &occupied),
            pos2(0.0, 420.0)
        );
        assert_eq!(settle(pos2(400.0, 0.0), 180.0, &occupied), pos2(540.0, 0.0));
    }
    #[test]
    fn sidecar_restores_atomically_and_rejects_bad_values() {
        let mut rack = Rack::default();
        rack.positions.insert("tone".into(), pos2(210.0, ROW));
        rack.zoom = 0.85;
        let layout = rack.layout();
        let mut restored = Rack::default();
        restored.restore_layout(&layout).unwrap();
        assert_eq!(restored.layout(), layout);
        for bad in [
            json!({"version":2}),
            json!({"version":1,"zoom":0.0,"pan":[0,0],"positions":{}}),
            json!({"version":1,"zoom":1,"pan":[0,0],"positions":{"tone":[1e100,0]}}),
        ] {
            assert!(restored.restore_layout(&bad).is_err());
            assert_eq!(restored.layout(), layout);
        }
    }
}
