//! Native rack surface. Graph edits are returned to the owning session; layout
//! lives separately so moving a panel never rebuilds the audio graph.
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, PointerButton, Pos2, Rect, Sense, Stroke, Vec2,
};
use openpatch_core::{Cable, ModuleTelemetry, Node, Patch};
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
    Inspect {
        lane: usize,
        id: String,
        port: String,
    },
}

struct ParameterEdit {
    id: String,
    parameter: String,
    text: String,
    min: f32,
    max: f32,
    step: Option<f32>,
    display_scale: f32,
    focus: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum CableEnd {
    Output { node: String, port: String },
    Input { node: String, port: String },
}

#[derive(Clone, Debug)]
struct PendingCable {
    origin: CableEnd,
    // The original stays in the patch until the complete replacement validates.
    original: Option<Cable>,
}

impl PendingCable {
    fn new(origin: CableEnd, patch: &Patch) -> Self {
        let original = match &origin {
            CableEnd::Input { node, port } => patch
                .cables
                .iter()
                .find(|cable| cable.to == *node && cable.input == *port)
                .cloned(),
            CableEnd::Output { .. } => None,
        };
        Self { origin, original }
    }
}

pub struct Rack {
    pub selected: Option<String>,
    positions: BTreeMap<String, Pos2>,
    pan: Vec2,
    zoom: f32,
    pending: Option<PendingCable>,
    dragging_cable: bool,
    editing: Option<ParameterEdit>,
    suppress_reveal_once: bool,
    gesture: u64,
    layout_revision: u64,
    sequence_states: BTreeMap<String, ModuleTelemetry>,
    performance_available: bool,
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
            gesture: 0,
            layout_revision: 0,
            sequence_states: BTreeMap::new(),
            performance_available: false,
        }
    }
}

impl Rack {
    pub fn set_sequence_states(
        &mut self,
        states: BTreeMap<String, ModuleTelemetry>,
        available: bool,
    ) {
        self.sequence_states = states;
        self.performance_available = available;
    }

    pub fn layout_revision(&self) -> u64 {
        self.layout_revision
    }

    pub fn reset_view(&mut self) {
        if self.pan != vec2(24.0, 12.0) || self.zoom != 1.0 {
            self.layout_revision = self.layout_revision.wrapping_add(1);
        }
        self.pan = vec2(24.0, 12.0);
        self.zoom = 1.0;
    }
    pub fn zoom(&self) -> f32 {
        self.zoom
    }
    pub fn set_zoom(&mut self, zoom: f32) {
        if zoom.is_finite() {
            let zoom = zoom.clamp(0.4, 1.8);
            if zoom != self.zoom {
                self.layout_revision = self.layout_revision.wrapping_add(1);
            }
            self.zoom = zoom;
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
        self.layout_revision = self.layout_revision.wrapping_add(1);
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
        let interactive = ui.is_enabled();
        if !interactive {
            self.pending = None;
            self.dragging_cable = false;
            self.editing = None;
        }
        let initial_pan = self.pan;
        // Capture this before reconcile assigns new panels their first position.
        // Restored sidecars own their viewport, including partial layouts.
        let suppress_reveal = std::mem::take(&mut self.suppress_reveal_once);
        let newly_selected = self
            .selected
            .as_ref()
            .filter(|id| !self.positions.contains_key(*id))
            .cloned();
        self.reconcile(patch, catalog);
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
        if interactive && canvas.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or(Pos2::ZERO)))
        {
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
        if interactive && response.dragged_by(PointerButton::Middle) {
            self.pan += response.drag_delta();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pending = None;
            self.dragging_cable = false;
            self.editing = None;
        }
        if interactive
            && !ui.ctx().wants_keyboard_input()
            && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
        {
            if let Some(id) = self.selected.take() {
                action = Some(RackAction::Command("remove_module", json!({"id":id})));
            }
        }
        paint_background(&painter, canvas, self.pan, self.zoom);

        let mut outputs = BTreeMap::<(String, String), Pos2>::new();
        let mut inputs = BTreeMap::<(String, String), Pos2>::new();
        let mut jack_targets = Vec::new();
        let mut pointer_on_module = false;
        for node in &patch.nodes {
            let position = self.positions[&node.id];
            let width = panel_width(&node.kind);
            let rect = Rect::from_min_size(
                canvas.min + self.pan + position.to_vec2() * self.zoom,
                vec2(width, PANEL_HEIGHT) * self.zoom,
            );
            let zoom = self.zoom;
            let point = |x, y| rect.min + vec2(x, y) * zoom;
            let module = catalog["modules"]
                .as_array()
                .and_then(|modules| modules.iter().find(|item| item["kind"] == node.kind));
            let ports = module
                .and_then(|item| item["inputs"].as_array())
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let output_ports: Vec<&str> = module
                .and_then(|module| module["outputs"].as_array())
                .map(|ports| ports.iter().filter_map(Value::as_str).collect())
                .unwrap_or_else(|| vec!["out"]);
            for (index, port) in output_ports.iter().enumerate() {
                let location = point_from_port(
                    rect.min,
                    self.zoom,
                    port_position(&node.kind, false, index, output_ports.len(), width),
                );
                outputs.insert((node.id.clone(), port.to_string()), location);
                jack_targets.push((
                    CableEnd::Output {
                        node: node.id.clone(),
                        port: port.to_string(),
                    },
                    location,
                ));
            }
            for (index, port) in ports.iter().filter_map(Value::as_str).enumerate() {
                let location = point_from_port(
                    rect.min,
                    self.zoom,
                    port_position(&node.kind, true, index, ports.len(), width),
                );
                inputs.insert((node.id.clone(), port.to_string()), location);
                jack_targets.push((
                    CableEnd::Input {
                        node: node.id.clone(),
                        port: port.to_string(),
                    },
                    location,
                ));
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
                monitor_menu(ui, node, &output_ports, patch, &mut action);
                if node.kind == "seq3" {
                    for (direction, label) in [
                        ("left", "Rotate pattern left"),
                        ("right", "Rotate pattern right"),
                    ] {
                        if ui.button(label).clicked() {
                            action = Some(RackAction::Command(
                                "rotate_sequence",
                                json!({"id":node.id,"direction":direction}),
                            ));
                            ui.close();
                        }
                    }
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
            if interactive && header_response.dragged_by(PointerButton::Primary) {
                if header_response.drag_delta() != Vec2::ZERO {
                    self.layout_revision = self.layout_revision.wrapping_add(1);
                }
                *self.positions.get_mut(&node.id).unwrap() +=
                    header_response.drag_delta() / self.zoom;
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if interactive && header_response.drag_stopped_by(PointerButton::Primary) {
                let other: Vec<_> = patch
                    .nodes
                    .iter()
                    .filter(|other| other.id != node.id)
                    .map(|other| (self.positions[&other.id], panel_width(&other.kind)))
                    .collect();
                let snapped = settle(self.positions[&node.id], width, &other);
                self.positions.insert(node.id.clone(), snapped);
                self.layout_revision = self.layout_revision.wrapping_add(1);
                if action.is_none() {
                    action = Some(RackAction::LayoutChanged);
                }
            }
            header_response.context_menu(|ui| {
                ui.strong(format!("{}  /  {}", title(&node.kind), node.id));
                ui.separator();
                monitor_menu(ui, node, &output_ports, patch, &mut action);
                if node.kind == "seq3" {
                    for (direction, label) in [
                        ("left", "Rotate pattern left"),
                        ("right", "Rotate pattern right"),
                    ] {
                        if ui.button(label).clicked() {
                            action = Some(RackAction::Command(
                                "rotate_sequence",
                                json!({"id":node.id,"direction":direction}),
                            ));
                            ui.close();
                        }
                    }
                }
                if ui.button("Remove module").clicked() {
                    action = Some(RackAction::Command("remove_module", json!({"id":node.id})));
                    self.selected = None;
                    ui.close();
                }
            });
            let description = module
                .and_then(|module| module["description"].as_str())
                .unwrap_or("Built-in OpenPatch module");
            let model = module
                .and_then(|module| module["model"].as_str())
                .unwrap_or("legacy");
            let policy = module
                .and_then(|module| {
                    module["polyphony"][if patch.version == 2 {
                        "version_2"
                    } else {
                        "version_1"
                    }]
                    .as_str()
                })
                .unwrap_or("mono");
            header_response.on_hover_text(format!("{description}\nModel: {model}\nChannel policy: {policy}\nDrag to move. Panels snap to rack rows and free space."));

            if node.kind == "seq3" {
                if let Some(edit) = self.sequence_panel(ui, node, module, rect) {
                    action = Some(edit);
                }
            }
            if is_sequential_switch(&node.kind) {
                if let Some(edit) = self.switch_panel(ui, node, rect) {
                    action = Some(edit);
                }
            }
            let parameters = ordered_parameters(node, module);
            let count = parameters.len();
            if node.kind == "poly_constant" {
                painter.text(
                    point(20.0, 93.0),
                    Align2::LEFT_CENTER,
                    "CHANNELS",
                    FontId::proportional(10.0 * self.zoom),
                    INK,
                );
                if let Some(spec) = module.and_then(|module| module["params"].get("channels")) {
                    if let Some(edit) = self.compact_parameter(
                        ui,
                        node,
                        "channels",
                        spec,
                        Rect::from_min_size(
                            point(width - 100.0, 81.0),
                            vec2(75.0, 24.0) * self.zoom,
                        ),
                        "",
                    ) {
                        action = Some(edit);
                    }
                }
                for channel in 0..16 {
                    let parameter = format!("v{}", channel + 1);
                    let x = 20.0 + (channel % 4) as f32 * (width - 40.0) / 4.0;
                    let y = 127.0 + (channel / 4) as f32 * 45.0;
                    painter.text(
                        point(x, y + 12.0),
                        Align2::LEFT_CENTER,
                        format!("{:02}", channel + 1),
                        FontId::monospace(9.0 * self.zoom),
                        MUTED,
                    );
                    if let Some(spec) = module.and_then(|module| module["params"].get(&parameter)) {
                        if let Some(edit) = self.compact_parameter(
                            ui,
                            node,
                            &parameter,
                            spec,
                            Rect::from_min_size(
                                point(x + 20.0, y),
                                vec2((width - 40.0) / 4.0 - 24.0, 24.0) * self.zoom,
                            ),
                            " V",
                        ) {
                            action = Some(edit);
                        }
                    }
                }
                let channels = node
                    .params
                    .get("channels")
                    .copied()
                    .or_else(|| {
                        module
                            .and_then(|module| module["params"]["channels"]["default"].as_f64())
                            .map(|value| value as f32)
                    })
                    .unwrap_or(0.0);
                painter.text(
                    point(width * 0.5, 313.0),
                    Align2::CENTER_CENTER,
                    format!("Configured: {} channels", channels as u32),
                    FontId::proportional(10.0 * self.zoom),
                    MUTED,
                );
            }
            if node.kind == "poly_split" {
                painter.text(
                    point(width * 0.5, 83.0),
                    Align2::CENTER_CENTER,
                    "16 named mono outputs",
                    FontId::proportional(10.0 * self.zoom),
                    MUTED,
                );
            }
            if node.kind == "poly_sum" {
                painter.text(
                    point(width * 0.5, 183.0),
                    Align2::CENTER_CENTER,
                    "SUM CHANNELS",
                    FontId::proportional(12.0 * self.zoom),
                    MUTED,
                );
            }
            if node.kind == "mid_side" {
                for (x, label) in [(width * 0.25, "ENCODER"), (width * 0.75, "DECODER")] {
                    painter.text(
                        point(x, 78.0),
                        Align2::CENTER_CENTER,
                        label,
                        FontId::proportional(10.0 * self.zoom),
                        MUTED,
                    );
                }
                painter.line_segment(
                    [point(width * 0.5, 70.0), point(width * 0.5, 384.0)],
                    Stroke::new(self.zoom, Color32::from_gray(175)),
                );
                painter.text(
                    point(width * 0.5, 204.0),
                    Align2::CENTER_CENTER,
                    "L/R  >  M/S  >  L/R",
                    FontId::proportional(10.0 * self.zoom),
                    MUTED,
                );
            }
            if node.kind == "mult" {
                painter.text(
                    point(width * 0.5, 143.0),
                    Align2::CENTER_CENTER,
                    "EIGHT IDENTICAL OUTPUTS",
                    FontId::proportional(10.0 * self.zoom),
                    MUTED,
                );
            }
            for (index, (name, spec)) in parameters.iter().enumerate() {
                if is_sequential_switch(&node.kind)
                    || (node.kind == "seq3" && sequence_cell(name).is_none())
                {
                    continue;
                }
                if node.kind == "midi_cv" {
                    let x = 20.0 + (index % 2) as f32 * width * 0.5;
                    let y = 90.0 + (index / 2) as f32 * 52.0;
                    painter.text(
                        point(x, y),
                        Align2::LEFT_CENTER,
                        parameter_label(name),
                        FontId::proportional(10.0 * self.zoom),
                        INK,
                    );
                    let control_rect = Rect::from_min_size(
                        point(x, y + 10.0),
                        vec2(width * 0.5 - 40.0, 25.0) * self.zoom,
                    );
                    let edit = match *name {
                        "allocation" => self.choice_parameter(
                            ui,
                            node,
                            name,
                            spec,
                            control_rect,
                            &["Rotate", "Reuse", "Reset"],
                        ),
                        "channel" => self.choice_parameter(
                            ui,
                            node,
                            name,
                            spec,
                            control_rect,
                            &[
                                "All", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11",
                                "12", "13", "14", "15", "16",
                            ],
                        ),
                        _ => self.compact_parameter(ui, node, name, spec, control_rect, ""),
                    };
                    if edit.is_some() {
                        action = edit;
                    }
                    continue;
                }
                if node.kind == "poly_constant" {
                    continue;
                }
                if node.kind == "poly_merge" && *name == "channels" {
                    painter.text(
                        point(20.0, 330.0),
                        Align2::LEFT_CENTER,
                        "CHANNELS",
                        FontId::proportional(10.0 * self.zoom),
                        INK,
                    );
                    if let Some(edit) = self.compact_parameter(
                        ui,
                        node,
                        name,
                        spec,
                        Rect::from_min_size(
                            point(width - 100.0, 318.0),
                            vec2(75.0, 24.0) * self.zoom,
                        ),
                        "",
                    ) {
                        action = Some(edit);
                    }
                    painter.text(
                        point(width * 0.5, 305.0),
                        Align2::CENTER_CENTER,
                        "0 = auto from connected inputs",
                        FontId::proportional(9.0 * self.zoom),
                        MUTED,
                    );
                    continue;
                }
                if node.kind == "multi_lfo" && matches!(*name, "unipolar" | "invert") {
                    let enabled = node.params.get(*name).copied().unwrap_or(0.0) >= 0.5;
                    let rect = Rect::from_min_size(
                        point(
                            if *name == "unipolar" {
                                20.0
                            } else {
                                width * 0.5 + 5.0
                            },
                            279.0,
                        ),
                        vec2(width * 0.5 - 25.0, 25.0) * self.zoom,
                    );
                    let label = if *name == "invert" {
                        "Invert"
                    } else if enabled {
                        "0–10 V"
                    } else {
                        "±5 V"
                    };
                    let button = ui.put(rect, egui::Button::new(label).selected(enabled));
                    if button.clicked() {
                        self.selected = Some(node.id.clone());
                        action = Some(RackAction::Command(
                            "set_parameter",
                            json!({"id":node.id,"parameter":name,"value":if enabled { 0.0 } else { 1.0 }}),
                        ));
                    }
                    button.on_hover_text(if *name == "invert" {
                        "Invert all waveforms within the selected voltage range."
                    } else {
                        "Switch all outputs between bipolar ±5 V and unipolar 0–10 V."
                    });
                    continue;
                }
                if node.kind == "mix6" && matches!(*name, "invert" | "average") {
                    let value = node.params.get(*name).copied().unwrap_or(0.0);
                    let x = if *name == "average" {
                        20.0
                    } else {
                        width * 0.5 + 5.0
                    };
                    let rect = Rect::from_min_size(
                        point(x, 199.0),
                        vec2(width * 0.5 - 25.0, 26.0) * self.zoom,
                    );
                    let button = ui.put(
                        rect,
                        egui::Button::new(if *name == "average" {
                            "Average"
                        } else {
                            "Invert"
                        })
                        .selected(value >= 0.5),
                    );
                    if button.clicked() {
                        self.selected = Some(node.id.clone());
                        action = Some(RackAction::Command(
                            "set_parameter",
                            json!({"id":node.id,"parameter":name,"value":if value >= 0.5 { 0.0 } else { 1.0 }}),
                        ));
                    }
                    button.on_hover_text(if *name == "average" {
                        "Divide by the number of active input jacks, including patched 0 V."
                    } else {
                        "Reverse the output polarity."
                    });
                    continue;
                }
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
                if *name == "scale_mask" {
                    if let Some(mask) = scale_keyboard(
                        ui,
                        &painter,
                        point(0.0, 0.0),
                        width,
                        self.zoom,
                        ui.id().with((&node.id, "scale")),
                        value as u16,
                    ) {
                        self.selected = Some(node.id.clone());
                        action = Some(RackAction::Command(
                            "set_parameter",
                            json!({"id":node.id,"parameter":name,"value":mask}),
                        ));
                    }
                    continue;
                }
                let (knob_position, radius) = if let Some((row, column)) =
                    sequence_cell(name).filter(|_| node.kind == "seq3")
                {
                    (
                        pos2(238.0 + column as f32 * 67.0, 123.0 + row as f32 * 73.0),
                        17.0,
                    )
                } else if node.kind == "quantizer" {
                    (pos2(width * 0.5, 260.0), 23.0)
                } else if node.kind == "multi_lfo" {
                    (
                        pos2(
                            width * (if index.is_multiple_of(2) { 0.25 } else { 0.75 }),
                            125.0 + (index / 2) as f32 * 100.0,
                        ),
                        23.0,
                    )
                } else if node.kind == "mid_side" {
                    (pos2(width * (0.25 + index as f32 * 0.5), 133.0), 27.0)
                } else if node.kind == "mix6" {
                    (pos2(width * 0.5, 125.0), 32.0)
                } else {
                    parameter_position(count, index, width)
                };
                let center = point(knob_position.x, knob_position.y);
                let logarithmic = matches!(
                    *name,
                    "frequency" | "cutoff" | "attack" | "decay" | "release"
                );
                let step = if *name == "waveform" {
                    Some(1.0)
                } else {
                    spec["step"]
                        .as_f64()
                        .filter(|step| *step > 0.0)
                        .map(|step| step as f32)
                };
                let geometric = node.kind == "multi_lfo" && *name == "frequency";
                let clock_ratio = geometric
                    && patch
                        .cables
                        .iter()
                        .any(|c| c.to == node.id && c.input == "clock");
                let display_scale = if clock_ratio { 0.5 } else { 1.0 };
                let display_value = if clock_ratio {
                    format!("{:.4}x", value * display_scale)
                } else {
                    parameter_value(name, value, spec)
                };
                let normalized = if geometric {
                    normalize_geometric(value, min, max)
                } else {
                    normalize(value, min, max, logarithmic)
                };
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
                    self.gesture = self.gesture.wrapping_add(1);
                    ui.data_mut(|data| {
                        data.insert_temp(knob.id, format!("rack-knob-{}", self.gesture))
                    });
                    if step.is_some() {
                        ui.data_mut(|data| data.insert_temp(knob.id, normalized));
                    }
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
                let drag_normalized = if step.is_some() && knob.dragged() {
                    let prior = ui
                        .data(|data| data.get_temp::<f32>(knob.id))
                        .unwrap_or(normalized);
                    let next = (prior + change).clamp(0.0, 1.0);
                    ui.data_mut(|data| data.insert_temp(knob.id, next));
                    next
                } else {
                    (normalized + change).clamp(0.0, 1.0)
                };
                let new_value = if knob.double_clicked() {
                    Some(default)
                } else if change != 0.0 {
                    Some(quantize(
                        if geometric {
                            denormalize_geometric(drag_normalized, min, max)
                        } else {
                            denormalize(drag_normalized, min, max, logarithmic)
                        },
                        min,
                        max,
                        step,
                    ))
                } else {
                    None
                };
                if knob.drag_stopped() {
                    ui.data_mut(|data| {
                        data.remove::<f32>(knob.id);
                        data.remove::<String>(knob.id);
                    });
                }
                if let Some(new_value) = new_value.filter(|new_value| *new_value != value) {
                    let mut args = json!({"id":node.id,"parameter":name,"value":new_value});
                    if knob.dragged() && !knob.double_clicked() {
                        if let Some(group) = ui.data(|data| data.get_temp::<String>(knob.id)) {
                            args["history_group"] = json!(group);
                        }
                    }
                    action = Some(RackAction::Command("set_parameter", args));
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
                    if node.kind == "seq3" {
                        ""
                    } else if clock_ratio {
                        "CLOCK RATIO"
                    } else {
                        parameter_label(name)
                    },
                    FontId::proportional(10.0 * self.zoom),
                    INK,
                );
                let value_offset = if count >= 3 { 9.0 } else { 15.0 };
                let value_center = center + vec2(0.0, (radius + value_offset) * self.zoom);
                painter.text(
                    value_center,
                    Align2::CENTER_CENTER,
                    &display_value,
                    FontId::monospace(11.0 * self.zoom),
                    INK,
                );
                let value_response = ui.interact(
                    Rect::from_center_size(
                        value_center,
                        vec2(
                            if node.kind == "seq3" {
                                54.0
                            } else if count >= 4 || node.kind == "mid_side" {
                                width * 0.5 - 12.0
                            } else {
                                width - 20.0
                            },
                            19.0,
                        ) * self.zoom,
                    ),
                    ui.id().with(("value", &node.id, name)),
                    Sense::click(),
                );
                value_response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        format!("Edit {} {} value, {}", node.id, name, display_value),
                    )
                });
                if value_response.double_clicked() {
                    self.editing = Some(ParameterEdit {
                        id: node.id.clone(),
                        parameter: name.to_string(),
                        text: (value * display_scale).to_string(),
                        min: min * display_scale,
                        max: max * display_scale,
                        step: step.map(|step| step * display_scale),
                        display_scale,
                        focus: true,
                    });
                }
                knob.on_hover_text(format!(
                    "{}: {}\n{}\nDrag vertically; Cmd/Ctrl for fine adjustment. Double-click to reset.",
                    name,
                    display_value,
                    if clock_ratio { "Multiplier of measured clock frequency. Free-runs until two rising edges arrive; clock changes speed without resetting phase." } else { description }
                ));
                value_response.on_hover_text("Double-click to type a value");
            }
            if count == 0 && matches!(node.kind.as_str(), "mixer" | "output" | "audio_out") {
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
                self.cable_jack(
                    &jack,
                    CableEnd::Input {
                        node: node.id.clone(),
                        port: port.to_string(),
                    },
                    patch,
                    catalog,
                    &mut action,
                );
                if jack.secondary_clicked() && connected {
                    self.pending = None;
                    self.dragging_cable = false;
                    action = Some(RackAction::Command(
                        "disconnect",
                        json!({"to":node.id,"input":port}),
                    ));
                }
            }
            for port in &output_ports {
                let output_point = outputs[&(node.id.clone(), port.to_string())];
                let connected = patch
                    .cables
                    .iter()
                    .any(|c| c.from == node.id && c.output == *port);
                let jack = jack(
                    ui,
                    &painter,
                    output_point,
                    ui.id().with(("output", &node.id, port)),
                    port,
                    connected,
                    true,
                    self.zoom,
                );
                self.cable_jack(
                    &jack,
                    CableEnd::Output {
                        node: node.id.clone(),
                        port: port.to_string(),
                    },
                    patch,
                    catalog,
                    &mut action,
                );
                if patch.output.as_ref() == Some(&node.id) && patch.output_port == *port {
                    paint_jack_light(
                        &painter,
                        output_point,
                        self.zoom * 0.8,
                        Color32::from_rgb(91, 174, 119),
                    );
                }
                let metadata = module
                    .and_then(|module| module["output_metadata"].as_array())
                    .and_then(|ports| ports.iter().find(|metadata| metadata["name"] == *port));
                let capacity = if patch.version == 1 {
                    1
                } else {
                    metadata
                        .and_then(|metadata| metadata["maximum_channels"].as_u64())
                        .unwrap_or(16)
                };
                let units = metadata
                    .and_then(|metadata| metadata["units"].as_str())
                    .unwrap_or("signal");
                jack.context_menu(|ui| {
                    for (lane, label) in [(0, "Inspect in scope X"), (1, "Inspect in scope Y")] {
                        if ui.button(label).clicked() {
                            self.pending = None;
                            self.dragging_cable = false;
                            action = Some(RackAction::Inspect {
                                lane,
                                id: node.id.clone(),
                                port: port.to_string(),
                            });
                            ui.close();
                        }
                    }
                });
                jack.on_hover_text(format!("{}.{}\nCapacity: up to {capacity} channels; {units}.\nThis is the port specification, not a live channel meter.", node.id, port));
            }
        }

        let pointer = ui.input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()));
        let nearest = pointer.filter(|p| canvas.contains(*p)).and_then(|pointer| {
            jack_targets
                .iter()
                .filter(|(_, point)| canvas.contains(*point))
                .filter(|(_, point)| pointer.distance(*point) <= (19.0 * self.zoom).max(10.0))
                .min_by(|(_, a), (_, b)| pointer.distance(*a).total_cmp(&pointer.distance(*b)))
        });
        let target_valid =
            self.pending
                .as_ref()
                .zip(nearest)
                .is_some_and(|(pending, (target, _))| {
                    cable_plan(pending, target)
                        .is_some_and(|(cable, old)| connection_allowed(patch, &cable, old, catalog))
                });
        if interactive
            && self.dragging_cable
            && ui.input(|i| i.pointer.button_released(PointerButton::Primary))
        {
            if let Some(pending) = self.pending.take() {
                if let Some((target, _)) = nearest {
                    action = cable_action(patch, &pending, target, catalog);
                }
            }
            self.dragging_cable = false;
        }
        // Connected plugs remain visible until an atomic edit succeeds. The
        // picked-up cable becomes a ghost so cancellation never loses a route.
        for cable in &patch.cables {
            if let (Some(&from), Some(&to)) = (
                outputs.get(&(cable.from.clone(), cable.output.clone())),
                inputs.get(&(cable.to.clone(), cable.input.clone())),
            ) {
                let picked_up = self
                    .pending
                    .as_ref()
                    .and_then(|pending| pending.original.as_ref())
                    .is_some_and(|old| same_cable(old, cable));
                let traced = nearest.is_some_and(|(end, _)| match end {
                    CableEnd::Output { node, port } => *node == cable.from && *port == cable.output,
                    CableEnd::Input { node, port } => *node == cable.to && *port == cable.input,
                });
                let color = cable_color(&cable.from, &cable.output).gamma_multiply(if picked_up {
                    0.24
                } else if nearest.is_some() && !traced && self.pending.is_none() {
                    0.48
                } else {
                    1.0
                });
                paint_cable(
                    &painter,
                    from,
                    to,
                    color,
                    self.zoom,
                    true,
                    configured_bus(patch, catalog, &cable.from),
                );
                if traced && !picked_up {
                    paint_jack_light(
                        &painter,
                        to,
                        self.zoom,
                        cable_color(&cable.from, &cable.output),
                    );
                }
            }
        }
        if let Some(pending) = &self.pending {
            if let Some(pointer) = pointer {
                let original_source = pending.original.as_ref().and_then(|old| {
                    outputs
                        .get(&(old.from.clone(), old.output.clone()))
                        .copied()
                });
                let origin = match &pending.origin {
                    CableEnd::Output { node, port } => {
                        outputs.get(&(node.clone(), port.clone())).copied()
                    }
                    CableEnd::Input { node, port } => {
                        inputs.get(&(node.clone(), port.clone())).copied()
                    }
                };
                let changing_source =
                    nearest.is_some_and(|(end, _)| matches!(end, CableEnd::Output { .. }));
                let anchor = if changing_source {
                    origin
                } else {
                    original_source.or(origin)
                };
                let color = match &pending.origin {
                    CableEnd::Output { node, port } => cable_color(node, port),
                    _ => pending
                        .original
                        .as_ref()
                        .map_or(CABLES[2], |old| cable_color(&old.from, &old.output)),
                };
                if let Some(anchor) = anchor {
                    let endpoint = nearest.map_or(pointer, |(_, point)| *point);
                    let bus = match &pending.origin {
                        CableEnd::Output { node, .. } => configured_bus(patch, catalog, node),
                        _ => pending
                            .original
                            .as_ref()
                            .is_some_and(|old| configured_bus(patch, catalog, &old.from)),
                    };
                    paint_cable(&painter, anchor, endpoint, color, self.zoom, false, bus);
                    paint_jack_light(&painter, anchor, self.zoom, color);
                }
                if let Some((_, target)) = nearest {
                    let light = if target_valid {
                        Color32::from_rgb(71, 195, 139)
                    } else {
                        Color32::from_rgb(229, 88, 75)
                    };
                    paint_jack_light(&painter, *target, self.zoom, light);
                    ui.ctx().set_cursor_icon(if target_valid {
                        egui::CursorIcon::PointingHand
                    } else {
                        egui::CursorIcon::NotAllowed
                    });
                }
                let message = if nearest.is_some() && !target_valid {
                    "Cannot connect here. Original cable is kept. Esc cancels."
                } else if pending.original.is_some() {
                    "Move to an input, or choose a new OUT. Esc cancels."
                } else if matches!(pending.origin, CableEnd::Input { .. }) {
                    "Choose an OUT to connect this input. Esc cancels."
                } else {
                    "Choose an input. Occupied inputs will be replaced. Esc cancels."
                };
                let galley = painter.layout_no_wrap(
                    message.into(),
                    FontId::proportional(11.0),
                    Color32::from_gray(225),
                );
                let rect = Rect::from_min_size(
                    canvas.left_bottom() + vec2(14.0, -38.0),
                    galley.size() + vec2(20.0, 12.0),
                );
                painter.rect_filled(rect, 4.0, Color32::from_black_alpha(220));
                painter.galley(rect.min + vec2(10.0, 6.0), galley, Color32::from_gray(225));
            }
        }

        if !pointer_on_module {
            if response.secondary_clicked() || response.double_clicked() {
                if self.pending.take().is_none() {
                    action = Some(RackAction::OpenBrowser);
                }
                self.dragging_cable = false;
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
        if self.pan != initial_pan {
            self.layout_revision = self.layout_revision.wrapping_add(1);
        }
        self.parameter_editor(ui.ctx(), &mut action);
        if interactive {
            action
        } else {
            None
        }
    }

    fn switch_panel(&mut self, ui: &mut egui::Ui, node: &Node, rect: Rect) -> Option<RackAction> {
        let painter = ui.painter().clone();
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let configured_length = node.params.get("steps").copied().unwrap_or(4.0) as u8;
        let state = self.sequence_states.get(&node.id).copied();
        let length = state.map_or(configured_length, |s| s.length);
        let selected = state.map(|s| s.step);
        let declick = node.params.get("declick").copied().unwrap_or(0.0) != 0.0;
        let mut action = None;
        painter.text(
            point(90.0, 80.0),
            Align2::CENTER_CENTER,
            "STEPS",
            FontId::proportional(10.0 * zoom),
            INK,
        );
        for (index, steps) in [2, 3, 4].into_iter().enumerate() {
            if panel_button(
                ui,
                &painter,
                Rect::from_min_size(
                    point(24.0 + index as f32 * 44.0, 94.0),
                    vec2(38.0, 24.0) * zoom,
                ),
                ui.id().with((&node.id, "length", steps)),
                &steps.to_string(),
                configured_length == steps,
                true,
                zoom,
            ) {
                self.selected = Some(node.id.clone());
                action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":"steps","value":steps}),
                ));
            }
        }
        if panel_button(
            ui,
            &painter,
            Rect::from_min_size(point(24.0, 128.0), vec2(132.0, 24.0) * zoom),
            ui.id().with((&node.id, "declick")),
            "De-click",
            declick,
            true,
            zoom,
        ) {
            self.selected = Some(node.id.clone());
            action = Some(RackAction::Command(
                "set_parameter",
                json!({"id":node.id,"parameter":"declick","value":if declick {0.0} else {1.0}}),
            ));
        }
        let status = selected.map_or("Waiting for engine".into(), |step| {
            format!("Route {step} / {length}")
        });
        painter.text(
            point(90.0, 174.0),
            Align2::CENTER_CENTER,
            status,
            FontId::monospace(9.0 * zoom),
            MUTED,
        );
        let many_x = if node.kind == "switch_1to4" {
            130.0
        } else {
            50.0
        };
        let main_x = 180.0 - many_x;
        for step in 1..=4 {
            let y = 262.0 + (step - 1) as f32 * 36.0;
            let active = selected == Some(step);
            let disabled = step > length;
            let color = if disabled {
                Color32::from_rgb(175, 75, 68)
            } else if active {
                Color32::from_rgb(202, 158, 55)
            } else {
                Color32::from_gray(158)
            };
            painter.line_segment(
                [point(main_x, 316.0), point(many_x, y)],
                Stroke::new(
                    if active { 1.8 * zoom } else { 0.7 * zoom },
                    color.gamma_multiply(if active { 0.9 } else { 0.35 }),
                ),
            );
            paint_jack_light(&painter, point(many_x - 22.0, y), zoom * 0.65, color);
        }
        action
    }

    fn sequence_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        rect: Rect,
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
        painter.line_segment(
            [point(207.0, 72.0), point(207.0, 385.0)],
            Stroke::new(zoom, Color32::from_gray(174)),
        );
        for (name, label, x, y, w, suffix) in [
            ("tempo", "TEMPO", 20.0, 99.0, 175.0, " BPM"),
            ("steps", "LENGTH", 20.0, 152.0, 175.0, " steps"),
            ("tempo_amount", "TEMPO CV", 20.0, 206.0, 80.0, "×"),
            ("steps_amount", "LENGTH CV", 114.0, 206.0, 80.0, "×"),
        ] {
            painter.text(
                point(x, y - 12.0),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(10.0 * zoom),
                INK,
            );
            if let Some(spec) = specs.and_then(|p| p.get(name)) {
                if let Some(edit) = self.compact_parameter(
                    ui,
                    node,
                    name,
                    spec,
                    Rect::from_min_size(point(x, y), vec2(w, 24.0) * zoom),
                    suffix,
                ) {
                    action = Some(edit);
                }
            }
        }
        for (name, label, y) in [
            ("passthrough", "Pass clock gate", 245.0),
            ("running", "Start running on load", 275.0),
        ] {
            let enabled = value(name, if name == "running" { 1.0 } else { 0.0 }) != 0.0;
            if panel_button(
                ui,
                &painter,
                Rect::from_min_size(point(20.0, y), vec2(175.0, 23.0) * zoom),
                ui.id().with((&node.id, name)),
                label,
                enabled,
                true,
                zoom,
            ) {
                action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":name,"value":if enabled{0.0}else{1.0}}),
                ));
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
                    point(20.0 + index as f32 * 60.0, 309.0),
                    vec2(55.0, 25.0) * zoom,
                ),
                ui.id().with((&node.id, "perform", event)),
                label,
                *event == "run" && state.is_some_and(|s| s.running),
                self.performance_available,
                zoom,
            ) {
                action = Some(RackAction::Command(
                    "trigger_module",
                    json!({"id":node.id,"action":event}),
                ));
            }
        }
        let text = state.map_or("Waiting for live engine".into(), |s| {
            format!(
                "{} · step {} / {}",
                if s.running { "Running" } else { "Stopped" },
                s.step,
                s.length
            )
        });
        painter.text(
            point(107.0, 345.0),
            Align2::CENTER_CENTER,
            text,
            FontId::monospace(9.0 * zoom),
            MUTED,
        );
        for row in 0..3 {
            painter.text(
                point(203.0, 123.0 + row as f32 * 73.0),
                Align2::RIGHT_CENTER,
                format!("{}", row + 1),
                FontId::monospace(9.0 * zoom),
                MUTED,
            );
        }
        for column in 0..8 {
            let x = 238.0 + column as f32 * 67.0;
            let current = state.is_some_and(|s| s.step as usize == column + 1);
            if column >= length || current {
                painter.rect_filled(
                    Rect::from_min_max(point(x - 29.0, 72.0), point(x + 29.0, 303.0)),
                    3.0,
                    if current {
                        Color32::from_rgb(177, 202, 182)
                    } else {
                        Color32::from_rgb(194, 196, 191)
                    },
                );
            }
            let gate = format!("gate{}", column + 1);
            let enabled = value(&gate, 1.0) != 0.0;
            if panel_button(
                ui,
                &painter,
                Rect::from_center_size(point(x, 85.0), vec2(31.0, 22.0) * zoom),
                ui.id().with((&node.id, "gate", column)),
                &format!("{}", column + 1),
                enabled,
                true,
                zoom,
            ) {
                action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":gate,"value":if enabled{0.0}else{1.0}}),
                ));
            }
            if current {
                painter.circle_filled(
                    point(x + 23.0, 85.0),
                    3.0 * zoom,
                    Color32::from_rgb(51, 137, 80),
                );
            }
        }
        action
    }

    fn choice_parameter(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        parameter: &str,
        spec: &Value,
        rect: Rect,
        choices: &[&str],
    ) -> Option<RackAction> {
        let old = node
            .params
            .get(parameter)
            .copied()
            .unwrap_or(spec["default"].as_f64().unwrap_or(0.0) as f32) as usize;
        let mut value = old.min(choices.len() - 1);
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect)
                .id_salt((&node.id, parameter)),
            |ui| {
                *ui.visuals_mut() = egui::Visuals::light();
                ui.style_mut().override_font_id = Some(FontId::proportional(12.0 * self.zoom));
                egui::ComboBox::from_id_salt("choice")
                    .width(rect.width())
                    .selected_text(choices[value])
                    .show_ui(ui, |ui| {
                        for (index, label) in choices.iter().enumerate() {
                            ui.selectable_value(&mut value, index, *label);
                        }
                    });
            },
        );
        (value != old).then(|| {
            RackAction::Command(
                "set_parameter",
                json!({"id":node.id,"parameter":parameter,"value":value}),
            )
        })
    }

    fn compact_parameter(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        parameter: &str,
        spec: &Value,
        rect: Rect,
        suffix: &str,
    ) -> Option<RackAction> {
        let min = spec["min"].as_f64().unwrap_or(0.0) as f32;
        let max = spec["max"].as_f64().unwrap_or(16.0) as f32;
        let step = spec["step"]
            .as_f64()
            .map(|step| step as f32)
            .or_else(|| (parameter == "channels").then_some(1.0));
        let mut value = node
            .params
            .get(parameter)
            .copied()
            .unwrap_or(spec["default"].as_f64().unwrap_or(0.0) as f32)
            .clamp(min, max);
        let id = ui.id().with((&node.id, parameter, "compact"));
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect).id_salt(id), |ui| {
                ui.visuals_mut().override_text_color = Some(INK);
                ui.style_mut().override_font_id = Some(FontId::monospace(10.0 * self.zoom));
                ui.spacing_mut().button_padding = vec2(2.0, 1.0) * self.zoom;
                ui.spacing_mut().interact_size = vec2(8.0, 18.0) * self.zoom;
                ui.visuals_mut().widgets.inactive.bg_fill = Color32::from_rgb(234, 235, 228);
                ui.visuals_mut().widgets.hovered.bg_fill = Color32::from_rgb(244, 246, 238);
                ui.visuals_mut().widgets.active.bg_fill = Color32::from_rgb(181, 215, 195);
                // DragValue paints a Button at rest and a TextEdit while typing.
                // Set both surfaces so the dark panel text stays legible.
                let visuals = ui.visuals_mut();
                visuals.widgets.inactive.weak_bg_fill = visuals.widgets.inactive.bg_fill;
                visuals.widgets.hovered.weak_bg_fill = visuals.widgets.hovered.bg_fill;
                visuals.widgets.active.weak_bg_fill = visuals.widgets.active.bg_fill;
                visuals.text_edit_bg_color = Some(Color32::from_rgb(244, 246, 238));
                ui.add_sized(
                    rect.size(),
                    egui::DragValue::new(&mut value)
                        .range(min..=max)
                        .speed(if step.is_some() { 0.1 } else { 0.02 })
                        .max_decimals(if step.is_some() { 0 } else { 2 })
                        .suffix(suffix),
                )
            })
            .inner;
        response.widget_info(|| {
            egui::WidgetInfo::slider(true, value as f64, format!("{} {}", node.id, parameter))
        });
        if response.drag_started() {
            self.gesture = self.gesture.wrapping_add(1);
            ui.data_mut(|data| data.insert_temp(id, format!("rack-value-{}", self.gesture)));
        }
        let group = if response.dragged() {
            ui.data(|data| data.get_temp::<String>(id))
        } else {
            None
        };
        if response.drag_stopped() {
            ui.data_mut(|data| data.remove::<String>(id));
        }
        if response.changed() {
            self.selected = Some(node.id.clone());
            let mut args =
                json!({"id":node.id,"parameter":parameter,"value":quantize(value,min,max,step)});
            if let Some(group) = group {
                args["history_group"] = json!(group);
            }
            Some(RackAction::Command("set_parameter", args))
        } else {
            None
        }
    }

    fn cable_jack(
        &mut self,
        response: &egui::Response,
        end: CableEnd,
        patch: &Patch,
        catalog: &Value,
        action: &mut Option<RackAction>,
    ) {
        if response.clicked() {
            if let Some(pending) = self.pending.take() {
                if pending.origin != end {
                    if cable_plan(&pending, &end).is_some() {
                        *action = cable_action(patch, &pending, &end, catalog);
                    } else {
                        self.pending = Some(PendingCable::new(end.clone(), patch));
                    }
                }
            } else {
                self.pending = Some(PendingCable::new(end.clone(), patch));
            }
            self.dragging_cable = false;
            self.selected = Some(match &end {
                CableEnd::Output { node, .. } | CableEnd::Input { node, .. } => node.clone(),
            });
        }
        if response.drag_started_by(PointerButton::Primary) {
            self.pending = Some(PendingCable::new(end.clone(), patch));
            self.dragging_cable = true;
            self.selected = Some(match end {
                CableEnd::Output { node, .. } | CableEnd::Input { node, .. } => node,
            });
        }
    }

    pub(crate) fn reconcile(&mut self, patch: &Patch, catalog: &Value) {
        let old_count = self.positions.len();
        self.positions
            .retain(|id, _| patch.nodes.iter().any(|node| &node.id == id));
        if old_count != self.positions.len() {
            self.layout_revision = self.layout_revision.wrapping_add(1);
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|id| !patch.nodes.iter().any(|node| &node.id == id))
        {
            self.selected = None;
        }
        if self.pending.as_ref().is_some_and(|pending| {
            let origin_exists = match &pending.origin {
                CableEnd::Output { node, port } => port_exists(patch, catalog, node, port, true),
                CableEnd::Input { node, port } => port_exists(patch, catalog, node, port, false),
            };
            !origin_exists
                || pending
                    .original
                    .as_ref()
                    .is_some_and(|old| !patch.cables.iter().any(|cable| same_cable(old, cable)))
        }) {
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
            self.layout_revision = self.layout_revision.wrapping_add(1);
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
            let parsed = edit.text.parse::<f32>().ok().filter(|value| value.is_finite() && *value >= edit.min && *value <= edit.max && edit.step.is_none_or(|step| ((value - edit.min) / step - ((value - edit.min) / step).round()).abs() < 0.000_01));
            if edit.display_scale == 0.5 { ui.label("Enter a clock multiplier (1 = same rate as clock).\nFree-runs until two rising edges arrive."); }
            if edit.parameter == "waveform" { ui.label("0 = Saw, 1 = Pulse"); }
            if matches!(edit.parameter.as_str(), "pulse_width" | "duty" | "sustain") { ui.label("Enter a ratio from 0 to 1."); }
            if matches!(edit.parameter.as_str(), "attack" | "decay" | "release") { ui.label("Enter time in seconds."); }
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                if ui.add_enabled(parsed.is_some(), egui::Button::new("Apply")).clicked() || (enter && parsed.is_some()) {
                    *action = Some(RackAction::Command("set_parameter", json!({"id":edit.id,"parameter":edit.parameter,"value":parsed.unwrap() / edit.display_scale}))); close = true;
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

fn same_cable(a: &Cable, b: &Cable) -> bool {
    a.from == b.from && a.output == b.output && a.to == b.to && a.input == b.input
}

fn port_exists(patch: &Patch, catalog: &Value, node: &str, port: &str, output: bool) -> bool {
    let Some(node) = patch.nodes.iter().find(|item| item.id == node) else {
        return false;
    };
    let Some(module) = catalog["modules"]
        .as_array()
        .and_then(|modules| modules.iter().find(|module| module["kind"] == node.kind))
    else {
        return false;
    };
    if module["min_patch_version"].as_u64().unwrap_or(1) > patch.version as u64 {
        return false;
    }
    module[if output { "outputs" } else { "inputs" }]
        .as_array()
        .map_or(output && port == "out", |ports| {
            ports.iter().any(|item| item == port)
        })
}

fn monitor_menu(
    ui: &mut egui::Ui,
    node: &Node,
    ports: &[&str],
    patch: &Patch,
    action: &mut Option<RackAction>,
) {
    for port in ports {
        if ui
            .selectable_label(
                patch.output.as_ref() == Some(&node.id) && patch.output_port == *port,
                format!("Monitor {}", port.to_uppercase()),
            )
            .clicked()
        {
            *action = Some(RackAction::Command(
                "set_output",
                json!({"id":node.id,"port":port}),
            ));
            ui.close();
        }
    }
}

fn point_from_port(origin: Pos2, zoom: f32, point: Pos2) -> Pos2 {
    origin + point.to_vec2() * zoom
}

fn port_position(kind: &str, input: bool, index: usize, count: usize, width: f32) -> Pos2 {
    if is_sequential_switch(kind) {
        return if input && index < 2 {
            pos2(50.0 + index as f32 * 80.0, 213.0)
        } else if kind == "switch_1to4" {
            if input {
                pos2(50.0, 316.0)
            } else {
                pos2(130.0, 262.0 + index as f32 * 36.0)
            }
        } else if input {
            pos2(50.0, 262.0 + (index - 2) as f32 * 36.0)
        } else {
            pos2(130.0, 316.0)
        };
    }
    if kind == "seq3" {
        return if input {
            pos2(35.0 + index as f32 * 37.0, 374.0)
        } else if (4..12).contains(&index) {
            pos2(238.0 + (index - 4) as f32 * 67.0, 374.0)
        } else {
            pos2(
                238.0 + (if index < 4 { index } else { index - 8 }) as f32 * 67.0,
                330.0,
            )
        };
    }
    if kind == "mid_side" {
        return if input {
            match index {
                0 | 1 => pos2(width * (0.15 + index as f32 * 0.20), 245.0),
                2 | 3 => pos2(width * (0.65 + (index - 2) as f32 * 0.20), 245.0),
                4 => pos2(width * 0.25, 305.0),
                _ => pos2(width * 0.75, 305.0),
            }
        } else {
            pos2(width * [0.15, 0.35, 0.65, 0.85][index], 374.0)
        };
    }
    if kind == "mix6" && input {
        return pos2(
            width * ((index % 3) + 1) as f32 / 4.0,
            270.0 + (index / 3) as f32 * 52.0,
        );
    }
    if kind == "mult" && input {
        return pos2(width * 0.5, 100.0);
    }
    if count > 4 {
        let rows = count.div_ceil(4);
        let first_y = if input {
            112.0
        } else if kind == "midi_cv" {
            220.0
        } else {
            187.0
        };
        let last_y = if input { 286.0 } else { 361.0 };
        return pos2(
            width * ((index % 4) + 1) as f32 / 5.0,
            first_y + (index / 4) as f32 * (last_y - first_y) / (rows - 1).max(1) as f32,
        );
    }
    let y = if input {
        if kind == "poly_split" {
            126.0
        } else {
            332.0
        }
    } else {
        374.0
    };
    pos2(width * (index + 1) as f32 / (count + 1) as f32, y)
}

fn cable_plan<'a>(
    pending: &'a PendingCable,
    target: &CableEnd,
) -> Option<(Cable, Option<&'a Cable>)> {
    if pending.origin == *target {
        return None;
    }
    let cable = match (&pending.origin, target) {
        (
            CableEnd::Output {
                node: from,
                port: output,
            },
            CableEnd::Input { node, port },
        ) => Cable {
            from: from.clone(),
            output: output.clone(),
            to: node.clone(),
            input: port.clone(),
        },
        (
            CableEnd::Input { node, port },
            CableEnd::Output {
                node: from,
                port: output,
            },
        ) => Cable {
            from: from.clone(),
            output: output.clone(),
            to: node.clone(),
            input: port.clone(),
        },
        (CableEnd::Input { .. }, CableEnd::Input { node, port }) => Cable {
            from: pending.original.as_ref()?.from.clone(),
            output: pending.original.as_ref()?.output.clone(),
            to: node.clone(),
            input: port.clone(),
        },
        _ => return None,
    };
    Some((cable, pending.original.as_ref()))
}

fn connection_allowed(patch: &Patch, cable: &Cable, old: Option<&Cable>, catalog: &Value) -> bool {
    let module = |id: &str| {
        let node = patch.nodes.iter().find(|node| node.id == id)?;
        catalog["modules"]
            .as_array()?
            .iter()
            .find(|module| module["kind"] == node.kind)
    };
    let Some(source) = module(&cable.from) else {
        return false;
    };
    let Some(destination) = module(&cable.to) else {
        return false;
    };
    let output_exists = source["outputs"]
        .as_array()
        .map_or(cable.output == "out", |ports| {
            ports.iter().any(|port| port == &cable.output)
        });
    let input_exists = destination["inputs"]
        .as_array()
        .is_some_and(|ports| ports.iter().any(|port| port == &cable.input));
    if !output_exists
        || !input_exists
        || [source, destination]
            .iter()
            .any(|module| module["min_patch_version"].as_u64().unwrap_or(1) > patch.version as u64)
    {
        return false;
    }
    let retained = |existing: &&Cable| {
        !(existing.to == cable.to && existing.input == cable.input)
            && !old.is_some_and(|old| same_cable(old, existing))
    };
    if patch.cables.iter().filter(retained).count() >= openpatch_core::MAX_CABLES {
        return false;
    }
    if patch.version == 2 {
        return true;
    }
    if cable.from == cable.to {
        return false;
    }
    // A candidate creates feedback exactly when its destination can reach its
    // source after removing the routes this atomic edit will replace.
    let mut stack = vec![cable.to.as_str()];
    let mut visited = std::collections::BTreeSet::new();
    while let Some(node) = stack.pop() {
        if node == cable.from {
            return false;
        }
        if !visited.insert(node) {
            continue;
        }
        stack.extend(
            patch
                .cables
                .iter()
                .filter(retained)
                .filter(|edge| edge.from == node)
                .map(|edge| edge.to.as_str()),
        );
    }
    true
}

fn cable_action(
    patch: &Patch,
    pending: &PendingCable,
    target: &CableEnd,
    catalog: &Value,
) -> Option<RackAction> {
    let (cable, old) = cable_plan(pending, target)?;
    if !connection_allowed(patch, &cable, old, catalog) {
        return None;
    }
    if patch
        .cables
        .iter()
        .any(|existing| same_cable(existing, &cable))
        && old.is_none_or(|old| same_cable(old, &cable))
    {
        return None;
    }
    if old.is_none()
        && !patch
            .cables
            .iter()
            .any(|existing| existing.to == cable.to && existing.input == cable.input)
    {
        return Some(RackAction::Command(
            "connect",
            json!({"from":cable.from,"output":cable.output,"to":cable.to,"input":cable.input}),
        ));
    }
    let mut candidate = patch.clone();
    candidate.cables.retain(|existing| {
        !(existing.to == cable.to && existing.input == cable.input)
            && !old.is_some_and(|old| same_cable(old, existing))
    });
    candidate.cables.push(cable);
    Some(RackAction::Command(
        "replace_patch",
        json!({"patch":candidate}),
    ))
}

fn cable_color(source: &str, port: &str) -> Color32 {
    let hash = source
        .bytes()
        .chain([0])
        .chain(port.bytes())
        .fold(2166136261_u32, |hash, byte| {
            (hash ^ byte as u32).wrapping_mul(16777619)
        });
    CABLES[hash as usize % CABLES.len()]
}

/// Only explicit bus sources have a known static channel count. Propagated or
/// auto-sized signals need engine telemetry before their width can be reported.
fn configured_bus(patch: &Patch, catalog: &Value, id: &str) -> bool {
    if patch.version != 2 {
        return false;
    }
    let Some(node) = patch.nodes.iter().find(|node| node.id == id) else {
        return false;
    };
    if !matches!(node.kind.as_str(), "poly_constant" | "poly_merge") {
        return false;
    }
    let channels = node
        .params
        .get("channels")
        .copied()
        .or_else(|| {
            catalog["modules"]
                .as_array()?
                .iter()
                .find(|module| module["kind"] == node.kind)?["params"]["channels"]["default"]
                .as_f64()
                .map(|value| value as f32)
        })
        .unwrap_or(0.0);
    channels > 1.0
}

fn paint_jack_light(painter: &egui::Painter, center: Pos2, zoom: f32, color: Color32) {
    for (radius, alpha) in [(17.0, 18), (14.0, 35)] {
        painter.circle_filled(
            center,
            radius * zoom,
            color.to_opaque().gamma_multiply(alpha as f32 / 255.0),
        );
    }
    painter.circle_stroke(center, 11.4 * zoom, Stroke::new(1.6 * zoom, color));
}

fn scale_keyboard(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    origin: Pos2,
    width: f32,
    zoom: f32,
    id: egui::Id,
    mask: u16,
) -> Option<u16> {
    let key_width = (width - 40.0) / 7.0;
    let mut changed = None;
    painter.text(
        origin + vec2(width * 0.5, 86.0) * zoom,
        Align2::CENTER_CENTER,
        "SCALE",
        FontId::proportional(10.0 * zoom),
        INK,
    );
    for (white_index, (note, label)) in [
        (0, "C"),
        (2, "D"),
        (4, "E"),
        (5, "F"),
        (7, "G"),
        (9, "A"),
        (11, "B"),
    ]
    .into_iter()
    .enumerate()
    {
        let rect = Rect::from_min_size(
            origin + vec2(20.0 + white_index as f32 * key_width, 101.0) * zoom,
            vec2(key_width - 1.0, 76.0) * zoom,
        );
        let enabled = mask & (1 << note) != 0;
        let response = ui.interact(rect, id.with(note), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                true,
                enabled,
                format!("Scale note {label}"),
            )
        });
        painter.rect_filled(
            rect,
            1.0,
            if enabled {
                Color32::from_rgb(167, 205, 180)
            } else {
                Color32::from_gray(236)
            },
        );
        painter.rect_stroke(
            rect,
            1.0,
            Stroke::new(0.8 * zoom, Color32::from_gray(109)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center_bottom() - vec2(0.0, 10.0) * zoom,
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(10.0 * zoom),
            INK,
        );
        if response.clicked() {
            changed = Some(mask ^ (1 << note));
        }
        response.on_hover_text(format!("Toggle {label} in the quantizer scale"));
    }
    for (boundary, note, label) in [
        (1, 1, "C#"),
        (2, 3, "D#"),
        (4, 6, "F#"),
        (5, 8, "G#"),
        (6, 10, "A#"),
    ] {
        let rect = Rect::from_min_size(
            origin + vec2(20.0 + boundary as f32 * key_width - key_width * 0.3, 101.0) * zoom,
            vec2(key_width * 0.6, 46.0) * zoom,
        );
        let enabled = mask & (1 << note) != 0;
        let response = ui.interact(rect, id.with(note), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                true,
                enabled,
                format!("Scale note {label}"),
            )
        });
        painter.rect_filled(
            rect,
            1.0,
            if enabled {
                Color32::from_rgb(58, 126, 86)
            } else {
                Color32::from_rgb(35, 39, 39)
            },
        );
        painter.text(
            rect.center_bottom() - vec2(0.0, 10.0) * zoom,
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(7.0 * zoom),
            Color32::from_gray(236),
        );
        if response.clicked() {
            changed = Some(mask ^ (1 << note));
        }
        response.on_hover_text(format!("Toggle {label} in the quantizer scale"));
    }
    for (x, label, value) in [
        (width * 0.5 - 28.0, "All", 4095),
        (width * 0.5 + 28.0, "None", 0),
    ] {
        let rect = Rect::from_center_size(origin + vec2(x, 194.0) * zoom, vec2(44.0, 18.0) * zoom);
        let response = ui.interact(rect, id.with(label), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("{label} scale notes"),
            )
        });
        painter.rect_filled(
            rect,
            2.0,
            if response.hovered() {
                Color32::from_gray(175)
            } else {
                Color32::from_gray(191)
            },
        );
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(10.0 * zoom),
            INK,
        );
        if response.clicked() {
            changed = Some(value);
        }
    }
    changed
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

fn is_sequential_switch(kind: &str) -> bool {
    matches!(kind, "switch_1to4" | "switch_4to1")
}

fn sequence_cell(name: &str) -> Option<(usize, usize)> {
    let (row, column) = name.strip_prefix("row")?.split_once('_')?;
    let row = row.parse::<usize>().ok()?;
    let column = column.parse::<usize>().ok()?;
    ((1..=3).contains(&row) && (1..=8).contains(&column))
        .then_some((row.saturating_sub(1), column.saturating_sub(1)))
}
#[allow(clippy::too_many_arguments)]
fn panel_button(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    id: egui::Id,
    label: &str,
    selected: bool,
    enabled: bool,
    zoom: f32,
) -> bool {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label)
    });
    let fill = if !enabled {
        Color32::from_gray(190)
    } else if selected {
        Color32::from_rgb(133, 183, 151)
    } else if response.hovered() {
        Color32::from_rgb(239, 241, 232)
    } else {
        Color32::from_rgb(229, 230, 220)
    };
    painter.rect_filled(rect, 2.0, fill);
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(0.7 * zoom, Color32::from_gray(151)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(10.0 * zoom),
        if enabled { INK } else { MUTED },
    );
    let clicked = enabled && response.clicked();
    response.on_hover_text(if label == "Start running on load" { "Sets the saved initial run state and applies it now. The Run performance button changes only runtime state." } else { label });
    clicked
}

fn panel_width(kind: &str) -> f32 {
    match kind {
        "seq3" => HP * 52.0,
        "switch_1to4" | "switch_4to1" => HP * 12.0,
        "poly_constant" => HP * 24.0,
        "midi_cv" => HP * 24.0,
        "audio_stereo" | "octave" => HP * 12.0,
        "mid_side" | "mult" | "multi_lfo" => HP * 20.0,
        "mix6" => HP * 16.0,
        "poly_split" | "poly_merge" => HP * 20.0,
        "poly_sum" => HP * 10.0,
        "quantizer" => HP * 16.0,
        "white_noise" | "attenuverter" | "crossfade" => HP * 12.0,
        "sample_hold" => HP * 10.0,
        "adsr" => HP * 16.0,
        "analog_vco" | "ladder_vcf" => HP * 14.0,
        "analog_vca" | "gate" => HP * 10.0,
        "audio_out" => HP * 8.0,
        "oscillator" => HP * 14.0,
        "lfo" | "lowpass" => HP * 12.0,
        "gain" | "mixer" => HP * 10.0,
        "constant" | "output" => HP * 8.0,
        _ => HP * 12.0,
    }
}
fn title(kind: &str) -> &str {
    match kind {
        "switch_1to4" => "SWITCH 1:4",
        "switch_4to1" => "SWITCH 4:1",
        "poly_constant" => "POLY VOLTAGE",
        "midi_cv" => "MIDI TO CV",
        "audio_stereo" => "STEREO AUDIO",
        "mid_side" => "MID / SIDE",
        "mix6" => "MIX 6",
        "octave" => "OCTAVE",
        "mult" => "MULT",
        "poly_split" => "SPLIT",
        "poly_merge" => "MERGE",
        "poly_sum" => "SUM",
        "quantizer" => "QUANTIZER",
        "white_noise" => "WHITE NOISE",
        "sample_hold" => "S&H",
        "attenuverter" => "ATTENUVERTER",
        "crossfade" => "CROSSFADE",
        "analog_vco" => "VCO",
        "ladder_vcf" => "LADDER",
        "analog_vca" => "OTA VCA",
        "adsr" => "ADSR",
        "gate" => "GATE",
        "audio_out" => "AUDIO",
        "oscillator" => "SINE VCO",
        "multi_lfo" => "LFO",
        "seq3" => "SEQUENCE 3",
        "lfo" => "SINE LFO",
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
        "waveform" => "WAVEFORM",
        "pulse_width" => "PULSE WIDTH",
        "resonance" => "RESONANCE",
        "drive" => "DRIVE",
        "attack" => "ATTACK",
        "decay" => "DECAY",
        "sustain" => "SUSTAIN",
        "release" => "RELEASE",
        "duty" => "DUTY",
        "initial" => "INITIAL",
        "seed" => "SEED",
        "level" => "LEVEL",
        "mix" => "MIX",
        "law" => "CURVE",
        "channels" => "VOICES",
        "channel" => "MIDI CHANNEL",
        "allocation" => "ALLOCATION",
        "clock_div" => "CLOCK DIVIDE",
        "encode_width" | "decode_width" => "WIDTH",
        "octaves" => "OCTAVES",
        "fm_amount" => "FM AMOUNT",
        "pwm_amount" => "PWM AMOUNT",
        name => name,
    }
}
fn port_label(name: &str) -> &str {
    match name {
        "pitch" | "cutoff_cv" => "V/OCT",
        "retrigger" => "RETRIG",
        "offset_cv" => "OFFSET",
        "trigger" => "TRIG",
        "signal" => "IN",
        "velocity" => "VEL",
        "aftertouch" => "AFT",
        "pitchwheel" => "BEND",
        "modwheel" => "MOD",
        "clock_div" => "CLK / N",
        "clock" => "CLK",
        "sine" => "SIN",
        "triangle" => "TRI",
        "saw" => "SAW",
        "square" => "SQR",
        "continue" => "CONT",
        "encode_cv" | "decode_cv" => "WIDTH CV",
        "shift_cv" => "SHIFT CV",
        other => other,
    }
}
fn ordered_parameters<'a>(node: &Node, module: Option<&'a Value>) -> Vec<(&'a str, &'a Value)> {
    let Some(params) = module.and_then(|module| module["params"].as_object()) else {
        return Vec::new();
    };
    let order: &[&str] = match node.kind.as_str() {
        "quantizer" => &["scale_mask", "offset"],
        "midi_cv" => &["channels", "channel", "allocation", "clock_div"],
        "mid_side" => &["encode_width", "decode_width"],
        "mix6" => &["level", "average", "invert"],
        "octave" => &["octaves"],
        "white_noise" => &["level", "seed"],
        "attenuverter" => &["gain", "offset"],
        "crossfade" => &["mix", "law"],
        "analog_vco" => &["frequency", "waveform", "pulse_width"],
        "ladder_vcf" => &["cutoff", "resonance", "drive"],
        "adsr" => &["attack", "decay", "sustain", "release"],
        "gate" => &["frequency", "duty"],
        "lfo" => &["frequency", "amplitude", "offset"],
        "multi_lfo" => &[
            "frequency",
            "pulse_width",
            "fm_amount",
            "pwm_amount",
            "unipolar",
            "invert",
        ],
        _ => &["frequency", "amplitude", "gain", "cutoff", "value"],
    };
    let mut ordered: Vec<_> = order
        .iter()
        .filter_map(|name| {
            params
                .get_key_value(*name)
                .map(|(name, spec)| (name.as_str(), spec))
        })
        .collect();
    // Newly introduced catalog controls remain accessible without a custom order.
    ordered.extend(
        params
            .iter()
            .filter(|(name, _)| !order.contains(&name.as_str()))
            .map(|(name, spec)| (name.as_str(), spec)),
    );
    ordered
}
fn parameter_position(count: usize, index: usize, width: f32) -> (Pos2, f32) {
    match count {
        1 => (pos2(width * 0.5, 183.0), 42.0),
        2 => (pos2(width * 0.5, 135.0 + index as f32 * 110.0), 32.0),
        3 => (pos2(width * 0.5, 114.0 + index as f32 * 78.0), 18.0),
        _ => (
            pos2(
                width * (if index.is_multiple_of(2) { 0.25 } else { 0.75 }),
                139.0 + (index / 2) as f32 * (113.0 / (count.div_ceil(2) - 1).max(1) as f32),
            ),
            if count > 4 { 16.0 } else { 24.0 },
        ),
    }
}
fn quantize(value: f32, min: f32, max: f32, step: Option<f32>) -> f32 {
    step.map_or(value, |step| {
        (min + ((value - min) / step).round() * step).clamp(min, max)
    })
}
fn parameter_value(name: &str, value: f32, spec: &Value) -> String {
    match name {
        "waveform" => {
            if value.round() == 0.0 {
                "Saw".into()
            } else {
                "Pulse".into()
            }
        }
        "pulse_width" | "duty" | "sustain" | "mix" => format!("{:.1}%", value * 100.0),
        "seed" => format!("{value:.0}"),
        "law" => {
            if value.round() == 0.0 {
                "Linear".into()
            } else {
                "Power".into()
            }
        }
        "gain" => format!("{value:.2}x"),
        "fm_amount" | "pwm_amount" => format!("{:+.0}%", value * 100.0),
        "encode_width" | "decode_width" => format!("{:.0}%", value * 100.0),
        "octaves" => format!("{value:+.0} oct"),
        _ => format_value(value, spec["unit"].as_str().unwrap_or("")),
    }
}
fn normalize_geometric(value: f32, min: f32, max: f32) -> f32 {
    (value.clamp(min, max) / min).log2() / (max / min).log2()
}
fn denormalize_geometric(value: f32, min: f32, max: f32) -> f32 {
    (min * (value.clamp(0.0, 1.0) * (max / min).log2()).exp2()).clamp(min, max)
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
        } else if value < 0.1 {
            format!("{value:.4} Hz")
        } else {
            format!("{value:.2} Hz")
        }
    } else if unit == "s" {
        if value < 1.0 {
            format!("{:.1} ms", value * 1000.0)
        } else {
            format!("{value:.2} s")
        }
    } else if matches!(unit, "V" | "V RMS") {
        format!("{value:.2} {unit}")
    } else if unit == "x" {
        format!("{value:.2}x")
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
    for band in 0..8 {
        let shine = Rect::from_min_size(
            rect.min + vec2(0.0, rect.height() * band as f32 / 16.0),
            vec2(rect.width(), rect.height() / 16.0),
        );
        painter.rect_filled(shine, 0, Color32::from_white_alpha(16 - band * 2));
    }
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
        paint_jack_light(
            painter,
            point(width - 19.0, 20.0),
            zoom * 0.45,
            Color32::from_rgb(70, 180, 120),
        );
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
        Rect::from_center_size(center, Vec2::splat((29.0 * zoom).max(18.0))),
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
        port_label(label).to_uppercase(),
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
        "Drag to an input, or click either jack first. Occupied inputs are replaced atomically."
    } else {
        "Drag to OUT to connect. Drag a connected input to another input to move its cable. Right-click to unplug; Esc cancels."
    })
}

fn paint_cable(
    painter: &egui::Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    zoom: f32,
    plugged: bool,
    bus: bool,
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
        (if bus { 9.0 } else { 7.0 }) * zoom,
        Color32::from_black_alpha(100),
    ));
    painter.add(curve(
        points,
        (if bus { 7.0 } else { 5.0 }) * zoom,
        color.gamma_multiply(0.62),
    ));
    painter.add(curve(
        points.map(|point| point - vec2(0.0, 0.7) * zoom),
        (if bus { 4.5 } else { 3.0 }) * zoom,
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
        painter.rect_filled(
            Rect::from_center_size(*point + vec2(0.0, 5.0) * zoom, vec2(7.0, 12.0) * zoom),
            2.0 * zoom,
            color.gamma_multiply(0.65),
        );
        painter.circle_filled(*point, 7.1 * zoom, Color32::from_gray(128));
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

    fn drag(
        ctx: &egui::Context,
        rack: &mut Rack,
        patch: &Patch,
        time: f64,
        from: Pos2,
        to: Pos2,
    ) -> Option<RackAction> {
        frame(
            ctx,
            rack,
            patch,
            time,
            vec![egui::Event::PointerMoved(from), button(from, true)],
        );
        frame(
            ctx,
            rack,
            patch,
            time + 0.1,
            vec![egui::Event::PointerMoved(to)],
        );
        frame(ctx, rack, patch, time + 0.2, vec![button(to, false)])
    }

    #[test]
    fn sequential_switch_controls_and_all_routing_jacks_are_independent() {
        for kind in ["switch_1to4", "switch_4to1"] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[{"id":"source","kind":"constant"},{"id":"switch","kind":kind},{"id":"out","kind":"output"}]})).unwrap();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let origin = pos2(144.0, 12.0);
            let length = origin + vec2(43.0, 106.0);
            frame(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![egui::Event::PointerMoved(length), button(length, true)],
            );
            let action = frame(&ctx, &mut rack, &patch, 1.1, vec![button(length, false)]);
            assert!(
                matches!(action, Some(RackAction::Command("set_parameter", ref args)) if args["parameter"] == "steps" && args["value"] == 2),
                "{kind}: step button did not emit a parameter edit"
            );
            let declick = origin + vec2(90.0, 140.0);
            frame(
                &ctx,
                &mut rack,
                &patch,
                2.0,
                vec![egui::Event::PointerMoved(declick), button(declick, true)],
            );
            let action = frame(&ctx, &mut rack, &patch, 2.1, vec![button(declick, false)]);
            assert!(
                matches!(action, Some(RackAction::Command("set_parameter", ref args)) if args["parameter"] == "declick" && args["value"] == 1.0)
            );
            let inputs = if kind == "switch_1to4" { 3 } else { 6 };
            let outputs = if kind == "switch_1to4" { 4 } else { 1 };
            let source = pos2(24.0, 12.0) + port_position("constant", false, 0, 1, 120.0).to_vec2();
            let input = origin + port_position(kind, true, inputs - 1, inputs, 180.0).to_vec2();
            let action = drag(&ctx, &mut rack, &patch, 3.0, source, input);
            assert!(
                matches!(action, Some(RackAction::Command("connect", ref args)) if args["input"] == if kind == "switch_1to4" {"signal"} else {"in4"})
            );
            let output = origin + port_position(kind, false, outputs - 1, outputs, 180.0).to_vec2();
            let destination =
                pos2(324.0, 12.0) + port_position("output", true, 0, 1, 120.0).to_vec2();
            let action = drag(&ctx, &mut rack, &patch, 4.0, output, destination);
            assert!(
                matches!(action, Some(RackAction::Command("connect", ref args)) if args["output"] == if kind == "switch_1to4" {"out4"} else {"out"})
            );
            let ports: Vec<_> = (0..inputs)
                .map(|i| port_position(kind, true, i, inputs, 180.0))
                .chain((0..outputs).map(|i| port_position(kind, false, i, outputs, 180.0)))
                .collect();
            for (i, a) in ports.iter().enumerate() {
                for b in &ports[i + 1..] {
                    assert!(a.distance(*b) >= 36.0);
                }
            }
        }
    }

    #[test]
    fn sequencer_cells_gates_runtime_buttons_and_last_output_have_separate_targets() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch:Patch=serde_json::from_value(json!({"version":2,"nodes":[{"id":"seq","kind":"seq3"},{"id":"out","kind":"audio_out"}]})).unwrap();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let origin = pos2(24.0, 12.0);
        // Last cell is independently editable without hitting a neighbouring value.
        let cell = origin + vec2(707.0, 269.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(cell), button(cell, true)],
        );
        let action = frame(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![egui::Event::PointerMoved(cell - vec2(0.0, 12.0))],
        );
        assert!(
            matches!(action,Some(RackAction::Command("set_parameter",ref args))if args["parameter"]=="row3_8"&&args["value"].as_f64().unwrap()>0.0)
        );
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![button(cell - vec2(0.0, 12.0), false)],
        );
        let gate = origin + vec2(707.0, 85.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            2.0,
            vec![egui::Event::PointerMoved(gate), button(gate, true)],
        );
        let action = frame(&ctx, &mut rack, &patch, 2.1, vec![button(gate, false)]);
        assert!(
            matches!(action,Some(RackAction::Command("set_parameter",ref args))if args["parameter"]=="gate8"&&args["value"]==0.0)
        );
        let clock = origin + vec2(47.0, 320.0);
        for (time, available) in [(3.0, false), (4.0, true)] {
            rack.set_sequence_states(
                BTreeMap::from([(
                    "seq".into(),
                    ModuleTelemetry {
                        step: 3,
                        running: true,
                        length: 8,
                    },
                )]),
                available,
            );
            frame(
                &ctx,
                &mut rack,
                &patch,
                time,
                vec![egui::Event::PointerMoved(clock), button(clock, true)],
            );
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                time + 0.1,
                vec![button(clock, false)],
            );
            if available {
                assert!(
                    matches!(action,Some(RackAction::Command("trigger_module",ref args))if args["action"]=="clock")
                );
            } else {
                assert!(action.is_none());
            }
        }
        let output = origin + port_position("seq3", false, 11, 16, panel_width("seq3")).to_vec2();
        let destination = origin
            + vec2(780.0, 0.0)
            + port_position("audio_out", true, 0, 1, panel_width("audio_out")).to_vec2();
        let action = drag(&ctx, &mut rack, &patch, 5.0, output, destination);
        assert!(
            matches!(action,Some(RackAction::Command("connect",ref args))if args["output"]=="step8"&&args["to"]=="out")
        );
        let points: Vec<_> = (0..16)
            .map(|i| port_position("seq3", false, i, 16, panel_width("seq3")))
            .collect();
        for (i, a) in points.iter().enumerate() {
            for b in &points[i + 1..] {
                assert!(a.distance(*b) >= 40.0);
            }
        }
    }

    #[test]
    fn input_origin_connects_and_replug_replaces_atomically_without_losing_original() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch = openpatch_core::demo_patch();
        patch.cables.clear();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let tone_out = pos2(129.0, 386.0);
        let vca_input = pos2(464.0, 344.0);
        let action = drag(&ctx, &mut rack, &patch, 1.0, vca_input, tone_out);
        assert!(
            matches!(action, Some(RackAction::Command("connect", args)) if args["from"] == "tone" && args["to"] == "vca")
        );

        patch = openpatch_core::demo_patch();
        let original = serde_json::to_value(&patch).unwrap();
        // Drop within the snap zone, beyond the actual socket's painted edge.
        let filter_input = pos2(654.0 + 16.0, 344.0);
        let action = drag(&ctx, &mut rack, &patch, 2.0, vca_input, filter_input);
        let Some(RackAction::Command("replace_patch", args)) = action else {
            panic!("expected one atomic replacement");
        };
        let candidate: Patch = serde_json::from_value(args["patch"].clone()).unwrap();
        candidate.validate().unwrap();
        assert!(!candidate
            .cables
            .iter()
            .any(|cable| cable.to == "vca" && cable.input == "signal"));
        assert!(candidate
            .cables
            .iter()
            .any(|cable| cable.from == "tone" && cable.to == "filter" && cable.input == "signal"));
        assert_eq!(candidate.cables.len(), patch.cables.len() - 1);
        assert_eq!(
            serde_json::to_value(&patch).unwrap(),
            original,
            "dragging must never mutate the live graph before validation"
        );

        let action = drag(&ctx, &mut rack, &patch, 3.0, tone_out, pos2(654.0, 344.0));
        assert!(
            matches!(action, Some(RackAction::Command("replace_patch", _))),
            "dropping onto an occupied input should replace its cable"
        );
    }

    #[test]
    fn invalid_or_cancelled_drag_keeps_routes_and_clears_gesture() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = openpatch_core::demo_patch();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let before = serde_json::to_value(&patch).unwrap();
        // Output feeds back into its own upstream VCA: rejected before dispatch.
        assert!(drag(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            pos2(804.0, 386.0),
            pos2(464.0, 344.0)
        )
        .is_none());
        assert!(rack.pending.is_none());
        assert!(drag(
            &ctx,
            &mut rack,
            &patch,
            2.0,
            pos2(464.0, 344.0),
            pos2(600.0, 600.0)
        )
        .is_none());
        assert!(rack.pending.is_none());
        frame(
            &ctx,
            &mut rack,
            &patch,
            3.0,
            vec![
                egui::Event::PointerMoved(pos2(464.0, 344.0)),
                button(pos2(464.0, 344.0), true),
            ],
        );
        frame(
            &ctx,
            &mut rack,
            &patch,
            3.1,
            vec![egui::Event::PointerMoved(pos2(500.0, 400.0))],
        );
        assert!(rack.pending.is_some());
        frame(
            &ctx,
            &mut rack,
            &patch,
            3.2,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(rack.pending.is_none());
        assert!(!rack.dragging_cable);
        assert!(frame(
            &ctx,
            &mut rack,
            &patch,
            3.3,
            vec![button(pos2(654.0, 344.0), false)]
        )
        .is_none());
        assert_eq!(serde_json::to_value(&patch).unwrap(), before);
    }

    #[test]
    fn disabled_rack_ignores_delete_scroll_pinch_and_pending_cable_release() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = openpatch_core::demo_patch();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        rack.selected = Some(patch.nodes[0].id.clone());
        rack.pending = Some(PendingCable::new(
            CableEnd::Output {
                node: patch.nodes[0].id.clone(),
                port: "out".into(),
            },
            &patch,
        ));
        rack.dragging_cable = true;
        let before = rack.layout();
        let mut action = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                time: Some(1.0),
                events: vec![
                    egui::Event::PointerMoved(pos2(500.0, 300.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: vec2(80.0, 50.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::Zoom(1.5),
                    egui::Event::Key {
                        key: egui::Key::Delete,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                    button(pos2(500.0, 300.0), false),
                ],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_enabled_ui(false, |ui| {
                        action = rack.show(ui, &patch, &openpatch_core::catalog(), 48000);
                    });
                });
            },
        );
        assert!(action.is_none());
        assert_eq!(rack.layout(), before);
        assert_eq!(rack.selected.as_deref(), Some(patch.nodes[0].id.as_str()));
        assert!(rack.pending.is_none());
        assert!(!rack.dragging_cable);
    }

    #[test]
    fn knob_history_group_spans_one_drag_only() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = openpatch_core::demo_patch();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let knob = pos2(129.0, 147.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(knob), button(knob, true)],
        );
        let group = |action| match action {
            Some(RackAction::Command("set_parameter", args)) => {
                args["history_group"].as_str().unwrap().to_string()
            }
            _ => panic!("expected grouped parameter edit"),
        };
        let first = group(frame(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![egui::Event::PointerMoved(knob - vec2(0.0, 10.0))],
        ));
        let second = group(frame(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![egui::Event::PointerMoved(knob - vec2(0.0, 20.0))],
        ));
        assert_eq!(first, second);
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.3,
            vec![button(knob - vec2(0.0, 20.0), false)],
        );
        frame(
            &ctx,
            &mut rack,
            &patch,
            2.0,
            vec![egui::Event::PointerMoved(knob), button(knob, true)],
        );
        let next = group(frame(
            &ctx,
            &mut rack,
            &patch,
            2.1,
            vec![egui::Event::PointerMoved(knob - vec2(0.0, 10.0))],
        ));
        assert_ne!(first, next);
    }

    #[test]
    fn quantizer_keyboard_toggles_white_and_black_notes_by_their_bits() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch = Patch {
            nodes: vec![Node {
                id: "q".into(),
                kind: "quantizer".into(),
                params: Default::default(),
            }],
            ..Patch::default()
        };
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        for (time, position, expected) in [
            (1.0, pos2(58.0, 180.0), 4094),
            (2.0, pos2(72.5, 136.0), 4092),
            (3.0, pos2(172.0, 206.0), 0),
        ] {
            frame(
                &ctx,
                &mut rack,
                &patch,
                time,
                vec![egui::Event::PointerMoved(position), button(position, true)],
            );
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                time + 0.1,
                vec![button(position, false)],
            );
            let Some(RackAction::Command("set_parameter", args)) = action else {
                panic!("expected scale change");
            };
            assert_eq!(args["parameter"], "scale_mask");
            assert_eq!(args["value"].as_u64().unwrap(), expected);
            patch.nodes[0]
                .params
                .insert("scale_mask".into(), expected as f32);
        }
    }

    #[test]
    fn named_output_pointer_drag_and_replug_preserve_the_selected_port() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch = Patch {
            version: 2,
            nodes: [("split", "poly_split"), ("a", "gain"), ("b", "gain")]
                .into_iter()
                .map(|(id, kind)| Node {
                    id: id.into(),
                    kind: kind.into(),
                    params: Default::default(),
                })
                .collect(),
            ..Patch::default()
        };
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let split_out16 = pos2(264.0, 373.0);
        let a_signal = pos2(374.0, 344.0);
        let b_signal = pos2(524.0, 344.0);
        let action = drag(&ctx, &mut rack, &patch, 1.0, split_out16, a_signal);
        assert!(
            matches!(action, Some(RackAction::Command("connect", args)) if args["from"] == "split" && args["output"] == "out16" && args["to"] == "a")
        );
        patch.cables.push(Cable {
            from: "split".into(),
            output: "out16".into(),
            to: "a".into(),
            input: "signal".into(),
        });
        let action = drag(&ctx, &mut rack, &patch, 2.0, a_signal, b_signal);
        let Some(RackAction::Command("replace_patch", args)) = action else {
            panic!("expected atomic named-output replug");
        };
        let candidate: Patch = serde_json::from_value(args["patch"].clone()).unwrap();
        assert_eq!(candidate.cables.len(), 1);
        assert_eq!(candidate.cables[0].output, "out16");
        assert_eq!(candidate.cables[0].to, "b");
        candidate.validate().unwrap();
        let mut invalid = candidate.cables[0].clone();
        invalid.output = "out17".into();
        assert!(!connection_allowed(
            &candidate,
            &invalid,
            None,
            &openpatch_core::catalog()
        ));
        let action = drag(&ctx, &mut rack, &patch, 3.0, a_signal, pos2(84.0, 199.0));
        let Some(RackAction::Command("replace_patch", args)) = action else {
            panic!("expected a named source replacement");
        };
        let candidate: Patch = serde_json::from_value(args["patch"].clone()).unwrap();
        assert_eq!(candidate.cables[0].output, "out1");
        assert_eq!(candidate.cables[0].to, "a");
    }

    #[test]
    fn midi_last_output_drags_to_stereo_right_without_parameter_hit_conflicts() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = Patch {
            version: 2,
            nodes: [("midi", "midi_cv"), ("audio", "audio_stereo")]
                .into_iter()
                .map(|(id, kind)| Node {
                    id: id.into(),
                    kind: kind.into(),
                    params: Default::default(),
                })
                .collect(),
            ..Patch::default()
        };
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let action = drag(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            pos2(312.0, 373.0),
            pos2(504.0, 344.0),
        );
        assert!(matches!(action, Some(RackAction::Command("connect", args))
            if args["from"] == "midi" && args["output"] == "continue"
                && args["to"] == "audio" && args["input"] == "right"));
        let points: Vec<_> = (0..12)
            .map(|index| port_position("midi_cv", false, index, 12, panel_width("midi_cv")))
            .collect();
        for (index, point) in points.iter().enumerate() {
            assert!(point.y >= 220.0);
            for other in &points[index + 1..] {
                assert!(point.distance(*other) >= 65.0);
            }
        }
    }

    #[test]
    fn stereo_routing_panels_patch_named_ports_and_toggle_mix_average() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = Patch {
            version: 2,
            nodes: [
                ("stereo", "mid_side"),
                ("mix", "mix6"),
                ("octave", "octave"),
                ("mult", "mult"),
            ]
            .into_iter()
            .map(|(id, kind)| Node {
                id: id.into(),
                kind: kind.into(),
                params: Default::default(),
            })
            .collect(),
            ..Patch::default()
        };
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let action = drag(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            pos2(219.0, 386.0),
            pos2(504.0, 334.0),
        );
        assert!(matches!(action, Some(RackAction::Command("connect", args))
            if args["from"] == "stereo" && args["output"] == "left" && args["to"] == "mix" && args["input"] == "in6"));
        let action = drag(
            &ctx,
            &mut rack,
            &patch,
            2.0,
            pos2(984.0, 373.0),
            pos2(624.0, 344.0),
        );
        assert!(matches!(action, Some(RackAction::Command("connect", args))
            if args["from"] == "mult" && args["output"] == "out8" && args["to"] == "octave" && args["input"] == "pitch"));
        let position = pos2(391.5, 224.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            3.0,
            vec![egui::Event::PointerMoved(position), button(position, true)],
        );
        let action = frame(&ctx, &mut rack, &patch, 3.1, vec![button(position, false)]);
        assert!(
            matches!(action, Some(RackAction::Command("set_parameter", args))
            if args["id"] == "mix" && args["parameter"] == "average" && args["value"] == 1.0)
        );
    }

    #[test]
    fn lfo_waveform_jacks_and_voltage_switches_are_independently_operable() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[{"id":"lfo","kind":"multi_lfo"},{"id":"out","kind":"audio_stereo"}]}))
        .unwrap();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        for (index, port) in ["sine", "triangle", "saw", "square"]
            .into_iter()
            .enumerate()
        {
            let action = drag(
                &ctx,
                &mut rack,
                &patch,
                1.0 + index as f64,
                pos2(84.0 + index as f32 * 60.0, 386.0),
                pos2(384.0, 344.0),
            );
            assert!(matches!(action, Some(RackAction::Command("connect", args))
                if args["from"] == "lfo" && args["output"] == port && args["input"] == "left"));
        }
        for (index, name) in ["unipolar", "invert"].into_iter().enumerate() {
            let position = pos2(106.5 + index as f32 * 135.0, 303.5);
            frame(
                &ctx,
                &mut rack,
                &patch,
                6.0 + index as f64,
                vec![egui::Event::PointerMoved(position), button(position, true)],
            );
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                6.1 + index as f64,
                vec![button(position, false)],
            );
            assert!(
                matches!(action, Some(RackAction::Command("set_parameter", args))
                if args["parameter"] == name && args["value"] == 1.0)
            );
        }
    }

    #[test]
    fn lfo_clock_ratio_editor_converts_display_units_back_to_patch_units() {
        for (clocked, expected_scale, expected_value) in [(false, 1.0, 2.0), (true, 0.5, 4.0)] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let mut patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
                "nodes":[{"id":"lfo","kind":"multi_lfo"},{"id":"clock","kind":"gate"}]}))
            .unwrap();
            if clocked {
                patch.cables.push(Cable {
                    from: "clock".into(),
                    output: "out".into(),
                    to: "lfo".into(),
                    input: "clock".into(),
                });
            }
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let value = pos2(99.0, 169.0);
            for (time, pressed) in [(1.0, true), (1.04, false), (1.08, true), (1.12, false)] {
                frame(
                    &ctx,
                    &mut rack,
                    &patch,
                    time,
                    vec![egui::Event::PointerMoved(value), button(value, pressed)],
                );
            }
            let edit = rack
                .editing
                .as_mut()
                .expect("double-click opens rate editor");
            assert_eq!(edit.display_scale, expected_scale);
            assert_eq!(edit.text.parse::<f32>().unwrap(), 2.0 * expected_scale);
            edit.text = "2".into();
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                1.3,
                vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert!(
                matches!(action, Some(RackAction::Command("set_parameter", args))
                if args["id"] == "lfo" && args["parameter"] == "frequency" && args["value"] == expected_value)
            );
        }
        assert_eq!(normalize_geometric(2.0, 1.0 / 256.0, 1024.0), 0.5);
        assert_eq!(denormalize_geometric(0.5, 1.0 / 256.0, 1024.0), 2.0);
        assert_eq!(format_value(1.0 / 256.0, "Hz"), "0.0039 Hz");
    }

    #[test]
    fn feedback_pointer_gestures_follow_patch_version() {
        for version in [1, 2] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch = Patch {
                version,
                nodes: vec![Node {
                    id: "loop".into(),
                    kind: "gain".into(),
                    params: Default::default(),
                }],
                ..Patch::default()
            };
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let action = drag(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                pos2(99.0, 386.0),
                pos2(74.0, 344.0),
            );
            if version == 1 {
                assert!(action.is_none());
            } else {
                assert!(
                    matches!(action, Some(RackAction::Command("connect", args)) if args["from"] == "loop" && args["to"] == "loop" && args["output"] == "out")
                );
            }
        }
    }

    #[test]
    fn sixteen_port_panels_have_separate_labeled_jack_regions() {
        for (kind, input) in [("poly_split", false), ("poly_merge", true)] {
            let points: Vec<_> = (0..16)
                .map(|index| port_position(kind, input, index, 16, panel_width(kind)))
                .collect();
            for (index, point) in points.iter().enumerate() {
                assert!(point.x > 20.0 && point.x < panel_width(kind) - 20.0);
                assert!(point.y > 90.0 && point.y < PANEL_HEIGHT - 20.0);
                for other in &points[index + 1..] {
                    assert!(
                        point.distance(*other) >= 50.0,
                        "jack labels and hit targets overlap"
                    );
                }
            }
        }
        assert_eq!(format_value(-2.5, "V"), "-2.50 V");
        assert_eq!(format_value(1.25, "V RMS"), "1.25 V RMS");
    }

    #[test]
    fn poly_voltage_cells_edit_first_and_last_channel_independently() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = Patch {
            version: 2,
            nodes: vec![Node {
                id: "voltages".into(),
                kind: "poly_constant".into(),
                params: Default::default(),
            }],
            ..Patch::default()
        };
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        for (time, point, name) in [
            (1.0, pos2(92.0, 151.0), "v1"),
            (2.0, pos2(332.0, 286.0), "v16"),
        ] {
            frame(
                &ctx,
                &mut rack,
                &patch,
                time,
                vec![egui::Event::PointerMoved(point), button(point, true)],
            );
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                time + 0.1,
                vec![egui::Event::PointerMoved(point - vec2(20.0, 0.0))],
            );
            assert!(
                matches!(action, Some(RackAction::Command("set_parameter", args)) if args["parameter"] == name && args["value"].as_f64().unwrap().is_finite())
            );
            frame(
                &ctx,
                &mut rack,
                &patch,
                time + 0.2,
                vec![button(point - vec2(20.0, 0.0), false)],
            );
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
        assert!(
            matches!(rack.pending.as_ref().map(|pending| &pending.origin), Some(CableEnd::Output { node, port }) if node == "tone" && port == "out")
        );
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
    fn catalog_controls_include_all_models_and_adsr_controls_do_not_overlap() {
        let catalog = openpatch_core::catalog();
        for module in catalog["modules"].as_array().unwrap() {
            let node = Node {
                id: "test".into(),
                kind: module["kind"].as_str().unwrap().into(),
                params: Default::default(),
            };
            assert_eq!(
                ordered_parameters(&node, Some(module)).len(),
                module["params"].as_object().unwrap().len(),
                "{} controls were hidden",
                node.kind
            );
        }
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|module| module["kind"] == "adsr")
            .unwrap();
        let node = Node {
            id: "env".into(),
            kind: "adsr".into(),
            params: Default::default(),
        };
        let names: Vec<_> = ordered_parameters(&node, Some(module))
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(names, ["attack", "decay", "sustain", "release"]);
        let width = panel_width("adsr");
        let controls: Vec<_> = (0..4)
            .map(|index| {
                let (center, radius) = parameter_position(4, index, width);
                let bounds = Rect::from_min_max(
                    center - vec2(width * 0.25 - 6.0, radius + 23.0),
                    center + vec2(width * 0.25 - 6.0, radius + 15.0),
                );
                assert!(bounds.top() > 70.0 && bounds.bottom() < 307.0);
                bounds
            })
            .collect();
        for (index, control) in controls.iter().enumerate() {
            for other in &controls[index + 1..] {
                assert!(!control.intersects(*other));
            }
        }
        assert_eq!(parameter_value("waveform", 0.0, &Value::Null), "Saw");
        assert_eq!(parameter_value("waveform", 1.0, &Value::Null), "Pulse");
        assert_eq!(parameter_value("pulse_width", 0.5, &Value::Null), "50.0%");
        assert_eq!(parameter_value("gain", 1.0, &Value::Null), "1.00x");
        assert_eq!(format_value(0.01, "s"), "10.0 ms");
        assert_eq!(format_value(2.0, "s"), "2.00 s");
    }

    #[test]
    fn waveform_drag_accumulates_small_movements_and_emits_only_valid_steps() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch = Patch {
            nodes: vec![Node {
                id: "vco".into(),
                kind: "analog_vco".into(),
                params: Default::default(),
            }],
            ..Patch::default()
        };
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let knob = pos2(129.0, 204.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(knob), button(knob, true)],
        );
        let mut changed = false;
        for step in 1..=10 {
            let pointer = knob - vec2(0.0, step as f32 * 20.0);
            if let Some(RackAction::Command("set_parameter", args)) = frame(
                &ctx,
                &mut rack,
                &patch,
                1.0 + step as f64 * 0.1,
                vec![egui::Event::PointerMoved(pointer)],
            ) {
                assert_eq!(args["parameter"], "waveform");
                assert_eq!(args["value"], 1.0);
                patch.nodes[0].params.insert("waveform".into(), 1.0);
                changed = true;
            }
        }
        assert!(
            changed,
            "rounding each frame would prevent a stepped control from moving"
        );
        frame(
            &ctx,
            &mut rack,
            &patch,
            2.1,
            vec![button(knob - vec2(0.0, 200.0), false)],
        );
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
