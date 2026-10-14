//! Native rack surface. Graph and layout edits are returned to the owning
//! session; a committed move can join session history without resetting DSP.
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, PointerButton, Pos2, Rect, Sense, Stroke, Vec2,
};
use openpatch_core::{Cable, ModuleControlState, ModuleLightState, ModuleTelemetry, Node, Patch};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    atomic::{AtomicU32, Ordering},
    OnceLock,
};

#[path = "cable_menu.rs"]
mod cable_menu;
use cable_menu::CableMenu;

const HP: f32 = 15.0;
const ROW: f32 = 420.0;
const PANEL_HEIGHT: f32 = 396.0;
const MAX_PAN: f32 = 1_000_000.0;
const PAPER: Color32 = Color32::from_rgb(220, 216, 205);
const INK: Color32 = crate::panel_skin::INK;
const MUTED: Color32 = Color32::from_rgb(95, 91, 82);
const CABLES: [Color32; 6] = [
    Color32::from_rgb(211, 101, 83),
    Color32::from_rgb(223, 164, 67),
    Color32::from_rgb(72, 171, 167),
    Color32::from_rgb(75, 139, 189),
    Color32::from_rgb(169, 112, 173),
    Color32::from_rgb(195, 138, 80),
];
const PULSE_PRESS: [&str; 10] = [
    "pulse1_press",
    "pulse2_press",
    "pulse3_press",
    "pulse4_press",
    "pulse5_press",
    "pulse6_press",
    "pulse7_press",
    "pulse8_press",
    "pulse9_press",
    "pulse10_press",
];
const PULSE_RELEASE: [&str; 10] = [
    "pulse1_release",
    "pulse2_release",
    "pulse3_release",
    "pulse4_release",
    "pulse5_release",
    "pulse6_release",
    "pulse7_release",
    "pulse8_release",
    "pulse9_release",
    "pulse10_release",
];
const PUSH_PRESS: [&str; 2] = ["push_press", "hold_press"];
const PUSH_RELEASE: [&str; 2] = ["push_release", "hold_release"];

pub(crate) enum RackAction {
    Command(&'static str, Value),
    ManualClick {
        id: String,
        press: &'static str,
        release: &'static str,
    },
    OpenBrowser,
    Preset {
        id: String,
        kind: String,
        revision: u64,
        mode: PresetMode,
    },
    Duplicate {
        ids: Vec<String>,
        cable_mode: DuplicateCableMode,
        revision: u64,
        select_copies: bool,
    },
    MoveModules(ModuleMove),
    Inspect {
        lane: usize,
        id: String,
        port: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModuleMove {
    pub revision: u64,
    pub before: BTreeMap<String, Pos2>,
    pub after: BTreeMap<String, Pos2>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PresetMode {
    Save,
    Browse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuplicateCableMode {
    None,
    Inputs,
    All,
}

impl DuplicateCableMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Inputs => "inputs",
            Self::All => "all",
        }
    }
}

struct ParameterEdit {
    id: String,
    kind: String,
    parameter: String,
    revision: u64,
    surface: ParameterEditSurface,
    menu_popup: Option<egui::Id>,
    stale: bool,
    text: String,
    min: f32,
    max: f32,
    step: Option<f32>,
    display_scale: f32,
    display_unit: String,
    slew_min: Option<f32>,
    gain_power: Option<f32>,
    raw_max: f32,
    focus: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParameterEditSurface {
    Window,
    KnobMenu,
}

enum ParameterEditOutcome {
    Keep,
    Apply(f32),
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
enum CableEnd {
    Output { node: String, port: String },
    Input { node: String, port: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CableOperation {
    Create,
    Move,
    Clone,
}

#[derive(Clone, Debug)]
struct PendingCable {
    origin: CableEnd,
    operation: CableOperation,
    shift_at_press: bool,
    // The source stays live until a guarded edit commits. Clone never removes it.
    original: Option<Cable>,
    companions: Vec<Cable>,
    color: Option<[u8; 3]>,
    revision: u64,
    stack_requested: bool,
}

impl PendingCable {
    #[cfg(test)]
    fn new(origin: CableEnd, patch: &Patch) -> Self {
        Self::at_revision(origin, patch, 0)
    }

    fn at_revision(origin: CableEnd, patch: &Patch, revision: u64) -> Self {
        let original = if matches!(origin, CableEnd::Input { .. }) {
            ordered_endpoint_cables(patch, &origin)
                .last()
                .map(|cable| (*cable).clone())
        } else {
            None
        };
        Self {
            origin,
            operation: if original.is_some() {
                CableOperation::Move
            } else {
                CableOperation::Create
            },
            original,
            companions: Vec::new(),
            color: None,
            revision,
            stack_requested: false,
            shift_at_press: false,
        }
    }

    fn allow_duplicate(&self, shift: bool) -> bool {
        self.stack_requested || (shift && !self.shift_at_press)
    }

    fn moved_original(&self) -> Option<&Cable> {
        (self.operation == CableOperation::Move)
            .then_some(self.original.as_ref())
            .flatten()
    }
}

struct HeldManual {
    id: String,
    kind: String,
    release: &'static str,
    button_rect: Rect,
}

struct GroupDrag {
    anchor: String,
    origins: BTreeMap<String, Pos2>,
    delta: Vec2,
    revision: u64,
}

#[derive(Clone, Copy)]
enum RenderMode {
    Interactive {
        drag_interactive: bool,
        allow_manual_press: bool,
    },
    Preview,
}
impl RenderMode {
    fn is_interactive(self) -> bool {
        matches!(self, Self::Interactive { .. })
    }
}
struct RenderedModules<'a> {
    outputs: BTreeMap<(String, String), Pos2>,
    inputs: BTreeMap<(String, String), Pos2>,
    jack_targets: Vec<(CableEnd, Pos2)>,
    pointer_on_module: bool,
    stack_badges: Vec<(Pos2, usize)>,
    input_groups: BTreeMap<(&'a str, &'a str), Vec<&'a Cable>>,
    cable_ordinals: Vec<usize>,
    action: Option<RackAction>,
}

/// Cached metadata and presentation state only: no DSP engine or live graph.
pub(crate) struct ModulePreview {
    source: Value,
    patch: Patch,
    renderer: Rack,
}
impl ModulePreview {
    pub(crate) fn new(module: &Value) -> Self {
        let kind = module["kind"].as_str().unwrap_or("").to_owned();
        let node = Node {
            score: None,
            id: String::new(),
            kind,
            bypassed: false,
            params: module["params"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(name, spec)| {
                    spec["default"].as_f64().map(|v| (name.clone(), v as f32))
                })
                .collect(),
        };
        let mut renderer = Rack {
            shortcuts_enabled: false,
            pan: Vec2::ZERO,
            ..Rack::default()
        };
        renderer.positions.insert(node.id.clone(), Pos2::ZERO);
        Self {
            source: module.clone(),
            patch: Patch {
                version: 2,
                nodes: vec![node],
                ..Patch::default()
            },
            renderer,
        }
    }

    pub(crate) fn matches(&self, module: &Value) -> bool {
        &self.source == module
    }

    pub(crate) fn size(&self) -> Vec2 {
        vec2(panel_width(&self.patch.nodes[0].kind), PANEL_HEIGHT)
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui, bounds: Rect, catalog: &Value) {
        if !bounds.is_positive()
            || !bounds.min.is_finite()
            || !bounds.max.is_finite()
            || !ui.is_rect_visible(bounds)
        {
            return;
        }
        let size = self.size();
        let scale = (bounds.width() / size.x).min(bounds.height() / size.y);
        let rect = Rect::from_center_size(bounds.center(), size * scale);
        self.renderer.zoom = scale;
        // Disabled egui controls retain their actual normal rendering, but can
        // neither receive focus nor handle pointer/keyboard input. There are no
        // preview menus, rack shortcuts, cable gestures, or parameter editors.
        let mut style = (**ui.style()).clone();
        style.visuals.disabled_alpha = 1.0;
        style.visuals.widgets.noninteractive = style.visuals.widgets.inactive;
        let mut preview_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("actual-module-preview", &self.patch.nodes[0].kind))
                .max_rect(rect)
                .style(style)
                .disabled(),
        );
        preview_ui.set_clip_rect(ui.clip_rect().intersect(bounds));
        let rendered = self.renderer.render_modules(
            &mut preview_ui,
            &self.patch,
            catalog,
            rect,
            self.patch.sample_rate,
            0,
            RenderMode::Preview,
        );
        debug_assert!(rendered.action.is_none());
    }
}

pub struct Rack {
    pub selected: Option<String>,
    selection: BTreeSet<String>,
    group_drag: Option<GroupDrag>,
    reveal_group_once: bool,
    reveal_module_once: Option<String>,
    positions: BTreeMap<String, Pos2>,
    pan: Vec2,
    zoom: f32,
    viewport: Option<Rect>,
    // Center zoom may arrive from the toolbar before this frame measures the
    // resized rack. Correct that pivot once the new canvas is available.
    pending_center_zoom_ratio: f32,
    positions_locked: bool,
    pending: Option<PendingCable>,
    cable_press: Option<PendingCable>,
    cable_menu: Option<CableMenu>,
    observed_revision: u64,
    dragging_cable: bool,
    editing: Option<ParameterEdit>,
    requested_parameter_menu: Option<(String, String)>,
    requested_cable_menu: Option<CableEnd>,
    shortcuts_enabled: bool,
    suppress_reveal_once: bool,
    gesture: u64,
    layout_revision: u64,
    sequence_states: BTreeMap<String, ModuleTelemetry>,
    performance_available: bool,
    module_lights: BTreeMap<String, ModuleLightState>,
    module_controls: BTreeMap<String, ModuleControlState>,
    held_manual: Option<HeldManual>,
    construction_animation: crate::cable_animation::Tracker,
}

impl Default for Rack {
    fn default() -> Self {
        Self {
            selected: None,
            selection: BTreeSet::new(),
            group_drag: None,
            reveal_group_once: false,
            reveal_module_once: None,
            positions: BTreeMap::new(),
            pan: vec2(24.0, 12.0),
            zoom: 1.0,
            viewport: None,
            pending_center_zoom_ratio: 1.0,
            positions_locked: false,
            pending: None,
            cable_press: None,
            cable_menu: None,
            observed_revision: 0,
            dragging_cable: false,
            editing: None,
            requested_parameter_menu: None,
            requested_cable_menu: None,
            shortcuts_enabled: true,
            suppress_reveal_once: false,
            gesture: 0,
            layout_revision: 0,
            sequence_states: BTreeMap::new(),
            performance_available: false,
            module_lights: BTreeMap::new(),
            module_controls: BTreeMap::new(),
            held_manual: None,
            construction_animation: crate::cable_animation::Tracker::default(),
        }
    }
}

impl Rack {
    pub(crate) fn clear_construction_animation(&mut self) {
        self.construction_animation.clear();
    }

    #[cfg(test)]
    pub(crate) fn construction_progress_at(
        &self,
        cable: &Cable,
        now: std::time::Instant,
    ) -> Option<f32> {
        self.construction_animation.progress(cable, now)
    }

    pub(crate) fn observe_construction_transition(
        &mut self,
        before: &Patch,
        after: &Patch,
        previous_revision: u64,
        revision: u64,
        construction: &Value,
        enabled: bool,
    ) {
        // Only an exact, observed construction commit gets visual feedback.
        // Gaps, human edits and history traversal show the current graph directly.
        if enabled
            && previous_revision.checked_add(1) == Some(revision)
            && construction["last_revision"].as_u64() == Some(revision)
        {
            self.construction_animation
                .observe(before, after, std::time::Instant::now());
        } else {
            self.construction_animation.clear();
        }
    }

    /// Open a real knob context menu on the next paint for the native smoke fixture.
    pub(crate) fn request_cable_menu_for_smoke(&mut self, node: &str, port: &str, input: bool) {
        self.requested_cable_menu = Some(if input {
            CableEnd::Input {
                node: node.into(),
                port: port.into(),
            }
        } else {
            CableEnd::Output {
                node: node.into(),
                port: port.into(),
            }
        });
    }

    pub(crate) fn request_parameter_menu_for_smoke(&mut self, id: &str, parameter: &str) {
        self.requested_parameter_menu = Some((id.to_owned(), parameter.to_owned()));
    }

    pub(crate) fn set_shortcuts_enabled(&mut self, enabled: bool) {
        if !enabled {
            self.cancel_module_drag();
        }
        self.shortcuts_enabled = enabled;
    }

    pub(crate) fn select_only(&mut self, id: String) {
        self.selection.clear();
        self.selection.insert(id.clone());
        self.selected = Some(id);
    }

    fn select_primary(&mut self, id: String) {
        if !self.selection.contains(&id) {
            self.selection.clear();
        }
        self.selection.insert(id.clone());
        self.selected = Some(id);
    }

    fn toggle_selection(&mut self, id: String) {
        if self.selection.remove(&id) {
            if self.selected.as_ref() == Some(&id) {
                self.selected = self.selection.iter().next().cloned();
            }
        } else {
            self.selection.insert(id.clone());
            self.selected = Some(id);
        }
    }

    pub(crate) fn selected_ids(&self) -> Vec<String> {
        self.selection.iter().cloned().collect()
    }

    pub(crate) fn selection_count(&self) -> usize {
        self.selection.len()
    }

    pub(crate) fn reconcile_selection(&mut self, patch: &Patch) {
        self.selection
            .retain(|id| patch.nodes.iter().any(|node| &node.id == id));
        if let Some(id) = self
            .selected
            .clone()
            .filter(|id| patch.nodes.iter().any(|node| &node.id == id))
        {
            // Existing callers may set the public primary directly when adding a module.
            if !self.selection.contains(&id) {
                self.selection.clear();
                self.selection.insert(id);
            }
        } else {
            self.selected = self.selection.iter().next().cloned();
        }
        if self
            .group_drag
            .as_ref()
            .is_some_and(|drag| drag.origins.keys().any(|id| !self.selection.contains(id)))
        {
            self.cancel_module_drag();
        }
    }

    fn context_ids(&self, id: &str) -> Vec<String> {
        if self.selection.contains(id) {
            self.selected_ids()
        } else {
            vec![id.to_owned()]
        }
    }

    fn shortcut_target(
        &self,
        patch: &Patch,
        canvas: Rect,
        pointer: Option<Pos2>,
    ) -> Option<(Vec<String>, bool)> {
        let pointer = pointer.filter(|position| canvas.contains(*position))?;
        if let Some(node) = patch.nodes.iter().rev().find(|node| {
            self.positions.get(&node.id).is_some_and(|position| {
                Rect::from_min_size(
                    canvas.min + self.pan + position.to_vec2() * self.zoom,
                    vec2(panel_width(&node.kind), PANEL_HEIGHT) * self.zoom,
                )
                .contains(pointer)
            })
        }) {
            Some((
                self.context_ids(&node.id),
                !self.selection.contains(&node.id),
            ))
        } else {
            Some((self.selected_ids(), false))
        }
    }

    fn shortcut_ids(&self, patch: &Patch, canvas: Rect, pointer: Option<Pos2>) -> Vec<String> {
        self.shortcut_target(patch, canvas, pointer)
            .map_or_else(Vec::new, |(ids, _)| ids)
    }

    fn duplicate_shortcut_target(
        &self,
        patch: &Patch,
        canvas: Rect,
        pointer: Option<Pos2>,
    ) -> Option<(Vec<String>, bool)> {
        pointer.filter(|position| canvas.contains(*position))?;
        if self.selection.is_empty() {
            self.shortcut_target(patch, canvas, pointer)
        } else {
            Some((self.selected_ids(), false))
        }
    }

    /// Deletion keeps the current selection even when the pointer is elsewhere.
    /// Hover is only a fallback when no modules are selected.
    fn delete_shortcut_ids(
        &self,
        patch: &Patch,
        canvas: Rect,
        pointer: Option<Pos2>,
    ) -> Vec<String> {
        if !self.selection.is_empty() {
            self.selected_ids()
        } else {
            self.shortcut_target(patch, canvas, pointer)
                .map_or_else(Vec::new, |(ids, _)| ids)
        }
    }

    /// Predict the same default positions as reconcile without changing layout.
    fn positions_for_patch(&self, patch: &Patch) -> BTreeMap<String, Pos2> {
        let mut positions = self.positions.clone();
        for node in &patch.nodes {
            if !positions.contains_key(&node.id) {
                let x = patch
                    .nodes
                    .iter()
                    .filter_map(|other| {
                        positions
                            .get(&other.id)
                            .filter(|p| p.y.abs() < 1.0)
                            .map(|p| p.x + panel_width(&other.kind))
                    })
                    .fold(0.0, f32::max);
                positions.insert(node.id.clone(), pos2(x, 0.0));
            }
        }
        positions
    }

    /// Pure proposal against the observed graph/layout. Never publishes partial clone positions.
    pub(crate) fn duplicate_placement(
        &self,
        patch: &Patch,
        mappings: &[(String, String)],
    ) -> Result<BTreeMap<String, Pos2>, String> {
        let positions = self.positions_for_patch(patch);
        let mut source = Vec::new();
        for (id, new_id) in mappings {
            let node = patch
                .nodes
                .iter()
                .find(|node| &node.id == id)
                .ok_or("Selected module no longer exists")?;
            source.push((new_id.clone(), positions[id], panel_width(&node.kind)));
        }
        if source.is_empty() {
            return Err("Select a module to duplicate".into());
        }
        let min = pos2(
            source
                .iter()
                .map(|(_, p, _)| p.x)
                .fold(f32::INFINITY, f32::min),
            source
                .iter()
                .map(|(_, p, _)| p.y)
                .fold(f32::INFINITY, f32::min),
        );
        let right = source
            .iter()
            .map(|(_, p, w)| p.x + w)
            .fold(f32::NEG_INFINITY, f32::max);
        let members: Vec<_> = source
            .iter()
            .map(|(_, p, w)| (pos2(p.x - min.x, p.y - min.y), *w))
            .collect();
        let others: Vec<_> = patch
            .nodes
            .iter()
            .map(|n| (positions[&n.id], panel_width(&n.kind)))
            .collect();
        let origin = settle_group(pos2(right, min.y), &members, &others)
            .ok_or("No free rack position within project bounds")?;
        Ok(source
            .into_iter()
            .zip(members)
            .map(|((id, _, _), (relative, _))| (id, origin + relative.to_vec2()))
            .collect())
    }

    /// Propose a library drop on the rack grid. The pointer is in screen space;
    /// the returned panel origin is in world space. Graph and layout stay intact.
    pub(crate) fn library_placement(
        &self,
        patch: &Patch,
        kind: &str,
        canvas: Rect,
        pointer: Pos2,
    ) -> Result<Pos2, String> {
        if !pointer.is_finite()
            || !canvas.min.is_finite()
            || !canvas.max.is_finite()
            || !canvas.is_positive()
            || !canvas.contains(pointer)
            || !self.zoom.is_finite()
            || self.zoom <= 0.0
            || !self.pan.is_finite()
        {
            return Err("Drop inside the rack canvas".into());
        }
        static KINDS: OnceLock<BTreeSet<String>> = OnceLock::new();
        let kinds = KINDS.get_or_init(|| {
            openpatch_core::catalog()["modules"]
                .as_array()
                .unwrap()
                .iter()
                .map(|module| module["kind"].as_str().unwrap().to_owned())
                .collect()
        });
        if !kinds.contains(kind) {
            return Err("Unknown library module kind".into());
        }
        let width = panel_width(kind);
        let world = (pointer - canvas.min - self.pan) / self.zoom;
        if !world.is_finite() {
            return Err("Drop position is outside rack bounds".into());
        }
        let positions = self.positions_for_patch(patch);
        let others: Vec<_> = patch
            .nodes
            .iter()
            .map(|node| (positions[&node.id], panel_width(&node.kind)))
            .collect();
        settle_group(
            pos2(world.x - width * 0.5, world.y),
            &[(Pos2::ZERO, width)],
            &others,
        )
        .ok_or_else(|| "No free rack position within project bounds".into())
    }

    /// Apply a previously validated proposal only after the shared add succeeds.
    /// Drops preserve the viewport; click-add may request a one-shot reveal.
    pub(crate) fn place_added_module(&mut self, id: String, position: Pos2, reveal: bool) {
        if !position.is_finite() || position.x.abs() > 100_000.0 || position.y.abs() > 100_000.0 {
            return;
        }
        self.positions.insert(id.clone(), position);
        self.select_only(id.clone());
        self.pending = None;
        self.cable_press = None;
        self.dragging_cable = false;
        self.group_drag = None;
        self.editing = None;
        self.reveal_group_once = false;
        self.reveal_module_once = reveal.then_some(id);
        self.suppress_reveal_once = !reveal;
        self.layout_revision = self.layout_revision.wrapping_add(1);
    }

    /// A clipped ghost uses the actual panel dimensions and creates no widgets.
    pub(crate) fn paint_library_preview(
        &self,
        painter: &egui::Painter,
        kind: &str,
        canvas: Rect,
        position: Pos2,
    ) {
        if !position.is_finite()
            || !canvas.min.is_finite()
            || !canvas.max.is_finite()
            || !canvas.is_positive()
            || !self.zoom.is_finite()
            || self.zoom <= 0.0
            || !self.pan.is_finite()
        {
            return;
        }
        let rect = Rect::from_min_size(
            canvas.min + self.pan + position.to_vec2() * self.zoom,
            vec2(panel_width(kind), PANEL_HEIGHT) * self.zoom,
        );
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(canvas));
        painter.rect_filled(rect, 3.0, Color32::from_black_alpha(36));
        painter.rect_stroke(
            rect,
            3.0,
            Stroke::new(1.5_f32, Color32::from_rgba_unmultiplied(226, 175, 88, 210)),
            egui::StrokeKind::Inside,
        );
    }

    pub(crate) fn apply_duplicate_placement(
        &mut self,
        positions: BTreeMap<String, Pos2>,
        select_copies: bool,
    ) {
        if select_copies {
            self.selection = positions.keys().cloned().collect();
            self.selected = self.selection.iter().next().cloned();
            self.reveal_module_once = None;
        } else {
            self.reveal_module_once = positions.keys().next().cloned();
        }
        self.positions.extend(positions);
        self.pending = None;
        self.cable_press = None;
        self.group_drag = None;
        self.reveal_group_once = select_copies;
        self.suppress_reveal_once = false;
        self.layout_revision = self.layout_revision.wrapping_add(1);
    }

    pub fn set_sequence_states(
        &mut self,
        states: BTreeMap<String, ModuleTelemetry>,
        available: bool,
    ) {
        self.sequence_states = states;
        self.performance_available = available;
    }

    pub fn set_module_states(
        &mut self,
        lights: BTreeMap<String, ModuleLightState>,
        controls: BTreeMap<String, ModuleControlState>,
    ) {
        self.module_lights = lights;
        self.module_controls = controls;
    }

    fn release_manual_if_needed(&mut self, ui: &egui::Ui, patch: &Patch) -> Option<RackAction> {
        let held = self.held_manual.as_ref()?;
        let eligible = ui.is_enabled()
            && self.shortcuts_enabled
            && self.performance_available
            && self.editing.is_none()
            && ui.input(|i| {
                i.focused
                    && i.pointer.primary_down()
                    && !i.pointer.button_released(PointerButton::Primary)
                    && (!matches!(held.kind.as_str(), "pulses" | "push" | "gates")
                        || i.pointer
                            .latest_pos()
                            .is_some_and(|position| held.button_rect.contains(position)))
                    && !i
                        .raw
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::WindowFocused(false)))
            })
            && patch
                .nodes
                .iter()
                .any(|n| n.id == held.id && n.kind == held.kind && !n.bypassed);
        if eligible {
            return None;
        }
        let held = self.held_manual.take().unwrap();
        Some(RackAction::Command(
            "trigger_module",
            json!({"id":held.id,"action":held.release}),
        ))
    }

    pub fn layout_revision(&self) -> u64 {
        self.layout_revision
    }

    pub fn reset_view(&mut self) {
        self.pending_center_zoom_ratio = 1.0;
        if self.pan != vec2(24.0, 12.0) || self.zoom != 1.0 {
            self.layout_revision = self.layout_revision.wrapping_add(1);
        }
        self.pan = vec2(24.0, 12.0);
        self.zoom = 1.0;
    }
    pub fn zoom(&self) -> f32 {
        self.zoom
    }
    /// Zoom about the latest rack viewport center. Before the first frame,
    /// preserve the origin because no viewport has been measured yet.
    pub fn set_zoom(&mut self, zoom: f32) {
        let pivot = self
            .viewport
            .map_or(Vec2::ZERO, |canvas| canvas.size() * 0.5);
        let previous_zoom = self.zoom;
        self.set_zoom_at(zoom, pivot);
        if self.viewport.is_some() {
            self.pending_center_zoom_ratio *= self.zoom / previous_zoom;
        }
    }

    fn set_zoom_at(&mut self, zoom: f32, pivot: Vec2) {
        if !zoom.is_finite() || !pivot.is_finite() {
            return;
        }
        let zoom = zoom.clamp(0.25, 4.0);
        if zoom == self.zoom {
            return;
        }
        if self.viewport.is_some() {
            let world = (pivot - self.pan) / self.zoom;
            self.pan = bounded_pan(pivot - world * zoom);
        }
        self.zoom = zoom;
        self.layout_revision = self.layout_revision.wrapping_add(1);
    }

    /// Adjacent half-octave steps, including recovery from arbitrary slider zoom.
    pub fn zoom_step(&mut self, zoom_in: bool) {
        let exponent = 2.0 * self.zoom.log2();
        let exponent = if zoom_in {
            (exponent + 0.01).floor() + 1.0
        } else {
            (exponent - 0.01).ceil() - 1.0
        };
        self.set_zoom(2.0_f32.powf(exponent * 0.5));
    }

    pub fn positions_locked(&self) -> bool {
        self.positions_locked
    }

    /// A host preference: lock mouse movement without restricting graph edits,
    /// selection, controls, insertion, or undo of previously committed moves.
    pub fn set_positions_locked(&mut self, locked: bool) {
        self.positions_locked = locked;
        if locked {
            self.cancel_module_drag();
        }
    }

    /// Host zoom shortcuts must not change geometry during a live gesture.
    pub fn navigation_idle(&self) -> bool {
        self.pending.is_none()
            && self.cable_press.is_none()
            && !self.dragging_cable
            && self.group_drag.is_none()
            && self.held_manual.is_none()
            && self.editing.is_none()
    }

    pub fn layout(&self) -> Value {
        let positions: BTreeMap<_, _> = self
            .positions
            .iter()
            .map(|(id, point)| {
                let committed = self
                    .group_drag
                    .as_ref()
                    .and_then(|drag| drag.origins.get(id))
                    .unwrap_or(point);
                (id, [committed.x, committed.y])
            })
            .collect();
        json!({"version":1,"zoom":self.zoom,"pan":[self.pan.x,self.pan.y],"positions":positions})
    }

    pub(crate) fn cancel_module_drag(&mut self) {
        if let Some(drag) = self.group_drag.take() {
            for (id, origin) in drag.origins {
                if let Some(position) = self.positions.get_mut(&id) {
                    *position = origin;
                }
            }
        }
    }

    /// Install a reviewed move only after the graph owner validates its revision
    /// and accepts it. Geometry validation precedes every write so a stale
    /// sidecar cannot partially shift a group. A scratch rack need not have
    /// observed a graph frame to perform this validation.
    pub(crate) fn apply_module_move(&mut self, proposal: &ModuleMove) -> Result<(), String> {
        if self.group_drag.is_some() {
            return Err("A module move is still in progress".into());
        }
        if proposal.before.is_empty() || proposal.before.keys().ne(proposal.after.keys()) {
            return Err("Module move has mismatched members".into());
        }
        for (id, before) in &proposal.before {
            if self.positions.get(id) != Some(before) {
                return Err(format!("Module {id} moved or was removed"));
            }
            let after = proposal.after[id];
            if !after.is_finite() || after.x.abs() > 100_000.0 || after.y.abs() > 100_000.0 {
                return Err("Module move is outside rack bounds".into());
            }
        }
        if proposal.before != proposal.after {
            self.positions.extend(proposal.after.clone());
            self.layout_revision = self.layout_revision.wrapping_add(1);
        }
        Ok(())
    }

    /// Validate the entire sidecar before replacing any current layout state.
    pub fn restore_layout(&mut self, value: &Value) -> Result<(), String> {
        if value["version"] != 1 {
            return Err("Unsupported rack layout version".into());
        }
        let zoom = finite_number(&value["zoom"], 0.25, 4.0)?;
        let pan_values = value["pan"]
            .as_array()
            .filter(|values| values.len() == 2)
            .ok_or("Rack pan needs two coordinates")?;
        let pan = vec2(
            finite_number(&pan_values[0], -MAX_PAN, MAX_PAN)?,
            finite_number(&pan_values[1], -MAX_PAN, MAX_PAN)?,
        );
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
        self.pan = pan;
        self.zoom = zoom;
        self.pending_center_zoom_ratio = 1.0;
        self.pending = None;
        self.cable_press = None;
        self.dragging_cable = false;
        self.suppress_reveal_once = true;
        self.reveal_group_once = false;
        self.reveal_module_once = None;
        self.group_drag = None;
        Ok(())
    }

    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        patch: &Patch,
        catalog: &Value,
        device_rate: u32,
        revision: u64,
    ) -> Option<RackAction> {
        let animation_now = std::time::Instant::now();
        if self.construction_animation.tick(animation_now) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        let navigation_idle_at_frame_start = self.navigation_idle();
        if self.observed_revision != revision {
            self.cancel_module_drag();
            self.pending = None;
            self.cable_press = None;
            self.dragging_cable = false;
            self.observed_revision = revision;
        }
        if !ui.input(|i| i.focused) || !self.shortcuts_enabled || !ui.is_enabled() {
            if let Some(edit) = self.editing.take() {
                if let Some(id) = edit.menu_popup {
                    egui::Popup::close_id(ui.ctx(), id);
                }
            }
        }
        if let Some(edit) = &mut self.editing {
            if (edit.revision != revision || !parameter_target_matches(edit, patch, catalog))
                && !edit.stale
            {
                if let Some(id) = edit.menu_popup {
                    egui::Popup::close_id(ui.ctx(), id);
                }
                edit.stale = true;
                edit.surface = ParameterEditSurface::Window;
            }
        }
        let released_manual = self.release_manual_if_needed(ui, patch);
        let interactive = ui.is_enabled();
        let gesture_interrupted = ui.input(|i| {
            i.events
                .iter()
                .any(|event| matches!(event, egui::Event::WindowFocused(false)))
                || (i
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::PointerGone))
                    && !i.events.iter().any(|event| {
                        matches!(
                            event,
                            egui::Event::PointerButton {
                                button: PointerButton::Primary,
                                pressed: false,
                                ..
                            }
                        )
                    }))
        });
        let drag_interactive = interactive
            && self.shortcuts_enabled
            && !gesture_interrupted
            && ui.input(|i| i.focused)
            && !egui::Popup::is_any_open(ui.ctx())
            && !ui.input(|i| i.key_pressed(egui::Key::Escape));
        if self.positions_locked {
            self.cancel_module_drag();
        }
        if !drag_interactive {
            self.pending = None;
            self.cable_press = None;
            self.dragging_cable = false;
            self.cancel_module_drag();
        }
        if !interactive {
            self.pending = None;
            self.cable_press = None;
            self.dragging_cable = false;
            self.editing = None;
            self.group_drag = None;
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
        let popup_open_at_frame_start = egui::Popup::is_any_open(ui.ctx());
        let mut action = None;
        let (canvas, response) = ui.allocate_exact_size(
            ui.available_size().max(vec2(1.0, 1.0)),
            Sense::click_and_drag(),
        );
        if let Some(previous) = self.viewport {
            let center_shift = (canvas.size() - previous.size()) * 0.5;
            self.pan =
                bounded_pan(self.pan + center_shift * (1.0 - self.pending_center_zoom_ratio));
        }
        self.pending_center_zoom_ratio = 1.0;
        self.viewport = Some(canvas);
        if !suppress_reveal {
            if std::mem::take(&mut self.reveal_group_once) {
                self.reveal_selected_group(patch, canvas);
            }
            if let Some(node) = self
                .reveal_module_once
                .take()
                .and_then(|id| patch.nodes.iter().find(|node| node.id == id))
            {
                self.reveal_module(node, canvas);
            }
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
        let navigation_enabled = interactive
            && self.shortcuts_enabled
            && ui.input(|i| i.focused)
            && !ui.ctx().wants_keyboard_input()
            && !popup_open_at_frame_start
            && navigation_idle_at_frame_start
            && self.navigation_idle();
        if navigation_enabled
            && !ui.input(|i| i.pointer.any_down())
            && canvas.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or(Pos2::ZERO)))
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
                self.set_zoom_at(self.zoom * zoom_delta, pointer - canvas.min);
            } else if !command {
                self.pan += scroll;
            }
        }
        if navigation_enabled
            && !ui.input(|i| {
                i.pointer.primary_down()
                    || i.pointer.secondary_down()
                    || i.pointer.button_down(PointerButton::Middle)
            })
            && canvas.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or(Pos2::ZERO)))
        {
            let (zoom_out, zoom_in) = ui.input(|i| {
                (
                    i.pointer.button_pressed(PointerButton::Extra1),
                    i.pointer.button_pressed(PointerButton::Extra2),
                )
            });
            if zoom_out != zoom_in {
                let factor = if zoom_in {
                    std::f32::consts::SQRT_2
                } else {
                    std::f32::consts::FRAC_1_SQRT_2
                };
                let pivot = ui
                    .input(|i| i.pointer.hover_pos())
                    .unwrap_or(canvas.center())
                    - canvas.min;
                self.set_zoom_at(self.zoom * factor, pivot);
            }
        }
        if navigation_enabled
            && response.dragged_by(PointerButton::Middle)
            && !ui.input(|i| i.pointer.primary_down() || i.pointer.secondary_down())
        {
            self.pan += response.drag_delta();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if navigation_enabled && !ui.input(|i| i.pointer.any_down()) {
            let delta = ui.input(keyboard_pan_delta);
            if delta != Vec2::ZERO {
                self.pan += delta;
                ui.ctx().request_repaint();
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.cancel_module_drag();
            self.pending = None;
            self.cable_press = None;
            self.dragging_cable = false;
            self.editing = None;
        }
        if interactive
            && self.shortcuts_enabled
            && ui.input(|i| i.focused)
            && !ui.ctx().wants_keyboard_input()
            && !egui::Popup::is_any_open(ui.ctx())
            && self.editing.is_none()
            && self.pending.is_none()
            && self.group_drag.is_none()
            && !ui.input(|i| i.pointer.primary_down())
        {
            let pointer = ui.input(|i| i.pointer.hover_pos());
            let shortcut_ids = self.shortcut_ids(patch, canvas, pointer);
            if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A)) {
                self.selection = patch.nodes.iter().map(|node| node.id.clone()).collect();
                self.selected = self
                    .selected
                    .clone()
                    .filter(|id| self.selection.contains(id))
                    .or_else(|| self.selection.iter().next().cloned());
                ui.input_mut(|i| {
                    i.consume_key(i.modifiers, egui::Key::A);
                });
            }
            let duplicate_target = self.duplicate_shortcut_target(patch, canvas, pointer);
            if ui.input(|i| i.modifiers.command && !i.modifiers.alt && i.key_pressed(egui::Key::D))
                && duplicate_target
                    .as_ref()
                    .is_some_and(|(ids, _)| !ids.is_empty())
            {
                let (ids, hovered_unselected) = duplicate_target.unwrap();
                action = Some(RackAction::Duplicate {
                    ids,
                    cable_mode: if ui.input(|i| i.modifiers.shift) {
                        if hovered_unselected {
                            DuplicateCableMode::All
                        } else {
                            DuplicateCableMode::Inputs
                        }
                    } else {
                        DuplicateCableMode::None
                    },
                    revision: self.observed_revision,
                    select_copies: !hovered_unselected,
                });
                ui.input_mut(|i| {
                    i.consume_key(i.modifiers, egui::Key::D);
                });
            } else if ui.input(|i| {
                i.modifiers.command
                    && !i.modifiers.shift
                    && !i.modifiers.alt
                    && i.key_pressed(egui::Key::I)
            }) && !shortcut_ids.is_empty()
            {
                action = Some(module_state_action(
                    ModuleStateAction::Initialize,
                    &shortcut_ids,
                    self.observed_revision,
                ));
                ui.input_mut(|i| {
                    i.consume_key(i.modifiers, egui::Key::I);
                });
            } else if ui.input(|i| {
                i.modifiers.command
                    && !i.modifiers.shift
                    && !i.modifiers.alt
                    && i.key_pressed(egui::Key::R)
            }) && !shortcut_ids.is_empty()
            {
                action = Some(module_state_action(
                    ModuleStateAction::Randomize,
                    &shortcut_ids,
                    self.observed_revision,
                ));
                ui.input_mut(|i| {
                    i.consume_key(i.modifiers, egui::Key::R);
                });
            }
        }
        if interactive
            && self.shortcuts_enabled
            && ui.input(|i| i.focused)
            && !ui.ctx().wants_keyboard_input()
            && !egui::Popup::is_any_open(ui.ctx())
            && navigation_idle_at_frame_start
            && self.navigation_idle()
            && !ui.input(|i| i.pointer.any_down())
        {
            // OS repeats arrive as pressed key events. Merely holding a key
            // across frames must not remove another module without a repeat.
            let key = ui.input(|i| {
                i.events.iter().find_map(|event| match event {
                    egui::Event::Key {
                        key: key @ (egui::Key::Delete | egui::Key::Backspace),
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.is_none() => Some(*key),
                    _ => None,
                })
            });
            if let Some(key) = key {
                let ids =
                    self.delete_shortcut_ids(patch, canvas, ui.input(|i| i.pointer.hover_pos()));
                if !ids.is_empty() {
                    action = Some(remove_selection_action(&ids, self.observed_revision));
                    ui.input_mut(|i| {
                        i.consume_key(egui::Modifiers::NONE, key);
                    });
                }
            }
        }
        paint_background(&painter, canvas, self.pan, self.zoom);

        let RenderedModules {
            outputs,
            inputs,
            jack_targets,
            pointer_on_module,
            stack_badges,
            input_groups,
            cable_ordinals,
            action: module_action,
        } = self.render_modules(
            ui,
            patch,
            catalog,
            canvas,
            device_rate,
            revision,
            RenderMode::Interactive {
                drag_interactive,
                allow_manual_press: released_manual.is_none(),
            },
        );
        if module_action.is_some() {
            action = module_action;
        }

        let pointer = ui.input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()));
        let nearest_jack = |pointer: Pos2| {
            if !canvas.contains(pointer) {
                return None;
            }
            jack_targets
                .iter()
                .filter(|(_, point)| canvas.contains(*point))
                .filter(|(_, point)| pointer.distance(*point) <= (19.0 * self.zoom).max(10.0))
                .min_by(|(_, a), (_, b)| pointer.distance(*a).total_cmp(&pointer.distance(*b)))
        };
        let nearest = pointer.and_then(nearest_jack);
        let target_valid =
            self.pending
                .as_ref()
                .zip(nearest)
                .is_some_and(|(pending, (target, _))| {
                    let shift = ui.input(|i| i.modifiers.shift);
                    if !pending.companions.is_empty() {
                        return pending.origin == *target
                            || cable_group_action(patch, pending, target, catalog, shift)
                                .is_some();
                    }
                    cable_plan(pending, target).is_some_and(|(cable, old)| {
                        old.is_some_and(|old| same_route(old, &cable))
                            || ((!pending.allow_duplicate(shift) || patch.version >= 2)
                                && (pending.allow_duplicate(shift)
                                    || !patch.cables.iter().any(|c| {
                                        !old.is_some_and(|old| same_cable(old, c))
                                            && same_route(c, &cable)
                                    }))
                                && connection_allowed_mode(patch, &cable, old, catalog, true))
                    })
                });
        if interactive
            && self.dragging_cable
            && ui.input(|i| i.pointer.button_released(PointerButton::Primary))
        {
            if let Some(pending) = self.pending.take() {
                // An actual release outside the canvas deletes a moved cable.
                // Losing the pointer without a release is cancellation instead.
                if let Some((position, modifiers)) = ui.input(|i| {
                    i.events.iter().rev().find_map(|event| match event {
                        egui::Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: false,
                            modifiers,
                        } => Some((*pos, *modifiers)),
                        _ => None,
                    })
                }) {
                    if position.is_finite() {
                        action = cable_release_action(
                            patch,
                            &pending,
                            nearest_jack(position).map(|(target, _)| target),
                            catalog,
                            modifiers.shift,
                        );
                    }
                }
            }
            self.dragging_cable = false;
        }
        // Connected plugs remain visible until an atomic edit succeeds. The
        // picked-up cable becomes a ghost so cancellation never loses a route.
        for (cable_index, cable) in patch.cables.iter().enumerate() {
            if let (Some(&from), Some(&to)) = (
                outputs.get(&(cable.from.clone(), cable.output.clone())),
                inputs.get(&(cable.to.clone(), cable.input.clone())),
            ) {
                let picked_up = self
                    .pending
                    .as_ref()
                    .filter(|p| p.operation == CableOperation::Move)
                    .is_some_and(|p| {
                        p.original
                            .iter()
                            .chain(&p.companions)
                            .any(|old| same_cable(old, cable))
                    });
                let traced = nearest.is_some_and(|(end, _)| match end {
                    CableEnd::Output { node, port } => *node == cable.from && *port == cable.output,
                    CableEnd::Input { node, port } => *node == cable.to && *port == cable.input,
                });
                let color = cable_instance_color(cable).gamma_multiply(if picked_up {
                    0.24
                } else if nearest.is_some() && !traced && self.pending.is_none() {
                    0.48
                } else {
                    1.0
                });
                let progress = if picked_up || traced {
                    None
                } else {
                    self.construction_animation.progress(cable, animation_now)
                };
                paint_committed_cable_curves(
                    &painter,
                    from,
                    to,
                    color,
                    self.zoom,
                    configured_bus(patch, catalog, &cable.from),
                    {
                        let count = input_groups[&(cable.to.as_str(), cable.input.as_str())].len();
                        let ordinal = cable_ordinals[cable_index];
                        if count > 1 {
                            (ordinal as f32 / (count - 1) as f32 - 0.5) * 40.0
                        } else {
                            0.0
                        }
                    },
                    progress,
                );
            }
        }
        // Curves retain native array order. Caps and their top lights use each
        // endpoint's independent order, with pickup promotion still provisional.
        let provisional = self
            .pending
            .as_ref()
            .filter(|pending| pending.operation == CableOperation::Move)
            .map(|pending| {
                let mut candidate = patch.clone();
                pending_promotion(&mut candidate, pending);
                candidate
            });
        let ordered_patch = provisional.as_ref().unwrap_or(patch);
        for output in [true, false] {
            let locations = if output { &outputs } else { &inputs };
            let end = if output {
                openpatch_core::CableEndpoint::Output
            } else {
                openpatch_core::CableEndpoint::Input
            };
            for ((node, port), &point) in locations {
                let order =
                    openpatch_core::cable_endpoint_order(&ordered_patch.cables, end, node, port);
                for (ordinal, index) in order.iter().enumerate() {
                    let cable = &ordered_patch.cables[*index];
                    let picked_up = self
                        .pending
                        .as_ref()
                        .filter(|pending| pending.operation == CableOperation::Move)
                        .is_some_and(|pending| {
                            pending
                                .original
                                .iter()
                                .chain(&pending.companions)
                                .any(|old| same_cable(old, cable))
                        });
                    let route_traced = nearest.is_some_and(|(target, _)| match target {
                        CableEnd::Output { node, port } => {
                            *node == cable.from && *port == cable.output
                        }
                        CableEnd::Input { node, port } => *node == cable.to && *port == cable.input,
                    });
                    let color = cable_instance_color(cable).gamma_multiply(if picked_up {
                        0.24
                    } else if nearest.is_some() && !route_traced && self.pending.is_none() {
                        0.48
                    } else {
                        1.0
                    });
                    paint_cable_plug(&painter, point, color, self.zoom);
                    let traced = nearest.is_some_and(|(target, _)| match target {
                        CableEnd::Output {
                            node: target_node,
                            port: target_port,
                        } => output && target_node == node && target_port == port,
                        CableEnd::Input {
                            node: target_node,
                            port: target_port,
                        } => !output && target_node == node && target_port == port,
                    });
                    if ordinal + 1 == order.len() && traced && !picked_up {
                        paint_jack_light(&painter, point, self.zoom, cable_instance_color(cable));
                    }
                }
            }
        }
        for (center, count) in stack_badges {
            painter.circle_filled(center, 8.0 * self.zoom, INK);
            painter.circle_stroke(center, 8.0 * self.zoom, Stroke::new(self.zoom, PAPER));
            painter.text(
                center,
                Align2::CENTER_CENTER,
                count.to_string(),
                FontId::monospace(8.0 * self.zoom),
                PAPER,
            );
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
                let picked_output =
                    matches!(pending.origin, CableEnd::Output { .. }) && pending.original.is_some();
                let anchor = if picked_output {
                    pending
                        .original
                        .as_ref()
                        .and_then(|old| inputs.get(&(old.to.clone(), old.input.clone())).copied())
                } else if pending.original.is_some() {
                    original_source
                } else {
                    origin
                };
                let color = match &pending.origin {
                    CableEnd::Output { node, port } => pending
                        .original
                        .as_ref()
                        .map_or_else(|| cable_color(node, port), cable_instance_color),
                    _ => pending
                        .original
                        .as_ref()
                        .map_or(CABLES[2], cable_instance_color),
                };
                let color = pending
                    .color
                    .map_or(color, |[r, g, b]| Color32::from_rgb(r, g, b));
                if let Some(anchor) = anchor {
                    let endpoint = nearest.map_or(pointer, |(_, point)| *point);
                    let bus = match &pending.origin {
                        CableEnd::Output { node, .. } => configured_bus(patch, catalog, node),
                        _ => pending
                            .original
                            .as_ref()
                            .is_some_and(|old| configured_bus(patch, catalog, &old.from)),
                    };
                    if pending.operation == CableOperation::Move {
                        paint_cable_curves(&painter, anchor, endpoint, color, self.zoom, bus, 0.0);
                    } else {
                        paint_cable(
                            &painter, anchor, endpoint, color, self.zoom, false, bus, 0.0,
                        );
                        paint_jack_light(&painter, anchor, self.zoom, color);
                    }
                }
                for cable in &pending.companions {
                    let anchor = match pending.origin {
                        CableEnd::Input { .. } => {
                            outputs.get(&(cable.from.clone(), cable.output.clone()))
                        }
                        CableEnd::Output { .. } => {
                            inputs.get(&(cable.to.clone(), cable.input.clone()))
                        }
                    };
                    if let Some(&anchor) = anchor {
                        let endpoint = nearest.map_or(pointer, |(_, point)| *point);
                        let color = cable_instance_color(cable);
                        paint_cable_curves(
                            &painter,
                            anchor,
                            endpoint,
                            color,
                            self.zoom,
                            configured_bus(patch, catalog, &cable.from),
                            0.0,
                        );
                    }
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
                let incomplete = incomplete_cable_drop(
                    patch,
                    pending,
                    nearest.map(|(target, _)| target),
                    ui.input(|i| i.modifiers.shift),
                );
                let message = if !pending.companions.is_empty() {
                    if nearest.is_none()
                        || nearest.is_some_and(|(target, _)| cable_plan(pending, target).is_none())
                    {
                        "Drop deletes every cable in this bundle. Esc cancels and keeps them."
                    } else if !target_valid {
                        "The patch cannot accept this bundle. Existing cables stay; Esc cancels."
                    } else {
                        "Move bundle: compatible routes move; duplicate conflicts are deleted. Shift permits duplicates; Esc cancels."
                    }
                } else if !target_valid && incomplete && pending.operation == CableOperation::Move {
                    "Drop deletes this cable. Esc cancels and keeps the original."
                } else if nearest.is_some() && !target_valid && incomplete {
                    "Drop discards this new cable. Existing cables stay; Esc cancels."
                } else if patch.version < 2 && nearest.is_some() && !target_valid {
                    "Stacking or feedback requires engine v2. Unsupported edits keep the original; Esc cancels."
                } else if nearest.is_some() && !target_valid {
                    "The patch cannot accept this connection. Original cable is kept; Esc cancels."
                } else if pending.operation == CableOperation::Clone {
                    "Clone cable: move the grabbed end to another matching jack. Original stays; Esc cancels."
                } else if picked_output {
                    "Move this OUT plug. Drop on empty space to delete this cable; Esc cancels."
                } else if pending.operation == CableOperation::Move {
                    "Move this IN plug. Drop on empty space to delete this cable; Esc cancels."
                } else if matches!(pending.origin, CableEnd::Input { .. }) {
                    "Choose an OUT for a new cable. Existing cables stay. Esc cancels."
                } else {
                    "Choose an IN for a new cable. Existing cables stay. Esc cancels."
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
            if !popup_open_at_frame_start && response.clicked() {
                self.selected = None;
                self.selection.clear();
                self.pending = None;
                self.cable_press = None;
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
        self.pan = bounded_pan(self.pan);
        if self.pan != initial_pan {
            self.layout_revision = self.layout_revision.wrapping_add(1);
        }
        if self.editing.as_ref().is_some_and(|edit| {
            edit.surface == ParameterEditSurface::KnobMenu
                && edit
                    .menu_popup
                    .is_none_or(|id| !egui::Popup::is_id_open(ui.ctx(), id))
        }) {
            self.editing = None;
        }
        self.parameter_editor(ui.ctx(), &mut action);
        released_manual.or(if interactive { action } else { None })
    }

    /// Shared actual module drawing. The preview caller supplies an isolated,
    /// catalog-default graph and a disabled child UI, never the live rack.
    #[allow(clippy::too_many_arguments)]
    fn render_modules<'a>(
        &mut self,
        ui: &mut egui::Ui,
        patch: &'a Patch,
        catalog: &Value,
        canvas: Rect,
        device_rate: u32,
        revision: u64,
        mode: RenderMode,
    ) -> RenderedModules<'a> {
        let interactive = mode.is_interactive() && ui.is_enabled();
        let (drag_interactive, allow_manual_press) = match mode {
            RenderMode::Interactive {
                drag_interactive,
                allow_manual_press,
            } => (drag_interactive, allow_manual_press),
            RenderMode::Preview => (false, false),
        };
        let popup_open_at_frame_start =
            !mode.is_interactive() || egui::Popup::is_any_open(ui.ctx());
        let painter = ui
            .painter()
            .with_clip_rect(ui.clip_rect().intersect(canvas));
        let press = ui.input(|i| {
            i.events.iter().rev().find_map(|event| match event {
                egui::Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    ..
                } => Some(*pos),
                _ => None,
            })
        });
        let press_target = press
            .filter(|pos| drag_interactive && ui.ctx().layer_id_at(*pos) == Some(ui.layer_id()))
            .and_then(|pos| {
                self.cable_press_target(
                    patch,
                    catalog,
                    canvas,
                    canvas.intersect(ui.clip_rect()),
                    pos,
                )
            });
        let mut action = None;
        let mut outputs = BTreeMap::<(String, String), Pos2>::new();
        let mut inputs = BTreeMap::<(String, String), Pos2>::new();
        let mut jack_targets = Vec::new();
        let mut pointer_on_module = false;
        let mut stack_badges = Vec::new();
        // One ordered grouping pass serves picking, menus, counts and curve fans.
        let mut input_groups = BTreeMap::<(&str, &str), Vec<&Cable>>::new();
        let mut cable_ordinals = Vec::with_capacity(patch.cables.len());
        for cable in &patch.cables {
            let group = input_groups
                .entry((cable.to.as_str(), cable.input.as_str()))
                .or_default();
            cable_ordinals.push(group.len());
            group.push(cable);
        }
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
            let description = module
                .and_then(|module| module["description"].as_str())
                .unwrap_or("Built-in OpenPatch module");
            if !mode.is_interactive() {
                paint_panel(
                    &painter,
                    rect,
                    node,
                    self.selection.contains(&node.id),
                    patch.output.as_ref() == Some(&node.id),
                    self.zoom,
                );
            }
            if mode.is_interactive() {
                let header_rect = Rect::from_min_max(point(15.0, 19.0), point(width - 15.0, 69.0));
                if interactive
                    && !popup_open_at_frame_start
                    && !egui::Popup::is_any_open(ui.ctx())
                    && ui.input(|i| {
                        i.pointer.button_pressed(PointerButton::Primary)
                            && i.pointer.interact_pos().is_some_and(|p| {
                                rect.contains(p) && !(i.modifiers.shift && header_rect.contains(p))
                            })
                    })
                {
                    self.select_primary(node.id.clone());
                }
                let panel_response =
                    ui.interact(rect, ui.id().with(("module", &node.id)), Sense::click());
                panel_response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::Button,
                        true,
                        self.selection.contains(&node.id),
                        format!("{} module {}", title(&node.kind), node.id),
                    )
                });
                if !popup_open_at_frame_start && panel_response.clicked() {
                    self.select_primary(node.id.clone());
                }
                panel_response.context_menu(|ui| {
                    ui.strong(format!("{}  /  {}", title(&node.kind), node.id));
                    ui.separator();
                    monitor_menu(ui, node, &output_ports, patch, &mut action);
                    utility_menu(ui, node, &mut action);
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
                    preset_menu(
                        ui,
                        node,
                        &self.context_ids(&node.id),
                        self.observed_revision,
                        &mut action,
                    );
                    selection_menu(
                        ui,
                        &self.context_ids(&node.id),
                        self.observed_revision,
                        self.selection.contains(&node.id),
                        patch,
                        &mut action,
                    );
                });
                paint_panel(
                    &painter,
                    rect,
                    node,
                    self.selection.contains(&node.id),
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
                        format!(
                            "{} {} module",
                            if self.positions_locked {
                                "Select"
                            } else {
                                "Move"
                            },
                            node.id
                        ),
                    )
                });
                if !popup_open_at_frame_start && header_response.clicked() {
                    if ui.input(|i| i.modifiers.shift) {
                        self.toggle_selection(node.id.clone());
                    } else {
                        self.select_primary(node.id.clone());
                    }
                }
                if drag_interactive
                    && !self.positions_locked
                    && header_response.drag_started_by(PointerButton::Primary)
                {
                    self.select_primary(node.id.clone());
                    self.group_drag = Some(GroupDrag {
                        anchor: node.id.clone(),
                        origins: self
                            .selection
                            .iter()
                            .map(|id| (id.clone(), self.positions[id]))
                            .collect(),
                        delta: Vec2::ZERO,
                        revision,
                    });
                }
                if header_response.hovered() && !self.positions_locked {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
                if drag_interactive
                    && !self.positions_locked
                    && header_response.dragged_by(PointerButton::Primary)
                {
                    if let Some(drag) = self
                        .group_drag
                        .as_mut()
                        .filter(|drag| drag.anchor == node.id)
                    {
                        drag.delta += header_response.drag_delta() / self.zoom;
                        for (id, origin) in &drag.origins {
                            self.positions.insert(id.clone(), *origin + drag.delta);
                        }
                    }
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                }
                if drag_interactive
                    && !self.positions_locked
                    && header_response.drag_stopped_by(PointerButton::Primary)
                {
                    if let Some(drag) = self.group_drag.take().filter(|drag| drag.anchor == node.id)
                    {
                        let min = pos2(
                            drag.origins
                                .values()
                                .map(|p| p.x)
                                .fold(f32::INFINITY, f32::min),
                            drag.origins
                                .values()
                                .map(|p| p.y)
                                .fold(f32::INFINITY, f32::min),
                        );
                        let members: Vec<_> = drag
                            .origins
                            .iter()
                            .map(|(id, p)| {
                                (
                                    pos2(p.x - min.x, p.y - min.y),
                                    panel_width(
                                        &patch.nodes.iter().find(|n| &n.id == id).unwrap().kind,
                                    ),
                                )
                            })
                            .collect();
                        let others: Vec<_> = patch
                            .nodes
                            .iter()
                            .filter(|n| !drag.origins.contains_key(&n.id))
                            .map(|n| (self.positions[&n.id], panel_width(&n.kind)))
                            .collect();
                        let settled = (drag.delta != Vec2::ZERO)
                            .then(|| settle_group(min + drag.delta, &members, &others))
                            .flatten();
                        let after = settled.map(|origin| {
                            drag.origins
                                .keys()
                                .zip(members.iter())
                                .map(|(id, (relative, _))| {
                                    (id.clone(), origin + relative.to_vec2())
                                })
                                .collect::<BTreeMap<_, _>>()
                        });
                        for (id, origin) in &drag.origins {
                            if let Some(position) = self.positions.get_mut(id) {
                                *position = *origin;
                            }
                        }
                        if let Some(after) = after.filter(|after| *after != drag.origins) {
                            if action.is_none() {
                                action = Some(RackAction::MoveModules(ModuleMove {
                                    revision: drag.revision,
                                    before: drag.origins,
                                    after,
                                }));
                            }
                        }
                    }
                }
                header_response.context_menu(|ui| {
                    ui.strong(format!("{}  /  {}", title(&node.kind), node.id));
                    ui.separator();
                    monitor_menu(ui, node, &output_ports, patch, &mut action);
                    utility_menu(ui, node, &mut action);
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
                    preset_menu(
                        ui,
                        node,
                        &self.context_ids(&node.id),
                        self.observed_revision,
                        &mut action,
                    );
                    selection_menu(
                        ui,
                        &self.context_ids(&node.id),
                        self.observed_revision,
                        self.selection.contains(&node.id),
                        patch,
                        &mut action,
                    );
                });
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
                header_response.on_hover_text(format!("{} · {description}\nModel: {model}\nChannel policy: {policy}\nDrag to move the selection. Shift-click headers to toggle modules. Cmd/Ctrl+A selects the rack. Panels snap together to free rack rows.", node.id));
            }
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
            if matches!(node.kind.as_str(), "vca_mix" | "fade") {
                self.mixing_panel(ui, node, rect);
            }
            if is_signal_utility(&node.kind) {
                if let Some(edit) = self.signal_panel(ui, node, rect, allow_manual_press) {
                    action = Some(edit);
                }
            }
            if node.kind == "pulses" {
                if let Some(edit) = self.pulses_panel(ui, node, rect, allow_manual_press) {
                    action = Some(edit);
                }
            }
            if node.kind == "push" {
                if let Some(edit) = self.push_panel(ui, node, rect, allow_manual_press) {
                    action = Some(edit);
                }
            }
            if node.kind == "gates" {
                if let Some(edit) = self.gates_panel(ui, node, rect, allow_manual_press) {
                    action = Some(edit);
                }
            }
            if is_voltage_utility(&node.kind) {
                if let Some(edit) = self.utility_panel(ui, node, module, patch, rect) {
                    action = Some(edit);
                }
            }
            if node.kind == "score_sequencer" {
                let length = node.score.as_ref().map_or(64, |s| s.length);
                painter.text(
                    point(25.0, 180.0),
                    Align2::LEFT_CENTER,
                    format!("{length} STEPS / FOUR TRACKS"),
                    crate::panel_skin::print_font(painter.ctx(), 11.0 * self.zoom),
                    INK,
                );
                for track in 0..4 {
                    painter.text(
                        point(25.0, 202.0 + track as f32 * 17.0),
                        Align2::LEFT_CENTER,
                        format!("{}", track + 1),
                        crate::panel_skin::print_font(painter.ctx(), 10.0 * self.zoom),
                        INK,
                    );
                    for step in 0..usize::from(length) {
                        let on = node
                            .score
                            .as_ref()
                            .and_then(|s| s.tracks.get(track))
                            .is_some_and(|t| t.steps.iter().any(|s| usize::from(s.step) == step));
                        painter.rect_filled(
                            Rect::from_min_size(
                                point(42.0 + step as f32 * 3.5, 197.0 + track as f32 * 17.0),
                                vec2(2.5, 10.0) * self.zoom,
                            ),
                            0.0,
                            if on {
                                crate::panel_skin::family_accent("score_sequencer")
                            } else {
                                Color32::from_rgb(164, 165, 155)
                            },
                        );
                    }
                }
            }
            // Saved `holding` seeds the runtime latch; the two performance
            // buttons are PUSH's only on-panel controls.
            let parameters = if node.kind == "push" {
                Vec::new()
            } else {
                ordered_parameters(node, module)
            };
            let count = parameters.len();
            if node.kind == "poly_constant" {
                painter.text(
                    point(20.0, 93.0),
                    Align2::LEFT_CENTER,
                    "CHANNELS",
                    crate::panel_skin::print_font(painter.ctx(), 10.0 * self.zoom),
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
                    crate::panel_skin::print_font(painter.ctx(), 10.0 * self.zoom),
                    MUTED,
                );
            }
            if node.kind == "poly_split" {
                painter.text(
                    point(width * 0.5, 83.0),
                    Align2::CENTER_CENTER,
                    "16 named mono outputs",
                    crate::panel_skin::print_font(painter.ctx(), 10.0 * self.zoom),
                    MUTED,
                );
            }
            if node.kind == "poly_sum" {
                painter.text(
                    point(width * 0.5, 183.0),
                    Align2::CENTER_CENTER,
                    "SUM CHANNELS",
                    crate::panel_skin::print_font(painter.ctx(), 12.0 * self.zoom),
                    MUTED,
                );
            }
            if node.kind == "mid_side" {
                for (x, label) in [(width * 0.25, "ENCODER"), (width * 0.75, "DECODER")] {
                    painter.text(
                        point(x, 78.0),
                        Align2::CENTER_CENTER,
                        label,
                        crate::panel_skin::print_font(painter.ctx(), 10.0 * self.zoom),
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
                    crate::panel_skin::print_font(painter.ctx(), 10.0 * self.zoom),
                    MUTED,
                );
            }
            if node.kind == "mult" {
                painter.text(
                    point(width * 0.5, 143.0),
                    Align2::CENTER_CENTER,
                    "EIGHT IDENTICAL OUTPUTS",
                    crate::panel_skin::print_font(painter.ctx(), 10.0 * self.zoom),
                    MUTED,
                );
            }
            for (index, (name, spec)) in parameters.iter().enumerate() {
                if is_voltage_utility(&node.kind)
                    || (node.kind == "vca_mix" && matches!(*name, "channel_exp" | "mix_exp"))
                    || (node.kind == "fade" && *name == "law")
                    || (node.kind == "rescale"
                        && matches!(*name, "multiplier" | "reflect_min" | "reflect_max"))
                    || is_sequential_switch(&node.kind)
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
                        crate::panel_skin::print_font(painter.ctx(), 10.0 * self.zoom),
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
                        crate::panel_skin::print_font(painter.ctx(), 10.0 * self.zoom),
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
                        crate::panel_skin::print_font(painter.ctx(), 9.0 * self.zoom),
                        MUTED,
                    );
                    continue;
                }
                if node.kind == "multi_vco" && matches!(*name, "linear_fm" | "soft_sync") {
                    let enabled = node.params.get(*name).copied().unwrap_or(0.0) >= 0.5;
                    let rect = Rect::from_min_size(
                        point(
                            if *name == "linear_fm" {
                                20.0
                            } else {
                                width * 0.5 + 5.0
                            },
                            279.0,
                        ),
                        vec2(width * 0.5 - 25.0, 25.0) * self.zoom,
                    );
                    let response = ui.put(
                        rect,
                        egui::Button::new(if *name == "linear_fm" { "LFM" } else { "SOFT" })
                            .selected(enabled),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::Checkbox,
                            true,
                            enabled,
                            format!("{} {}", node.id, name),
                        )
                    });
                    if response.clicked() {
                        self.select_primary(node.id.clone());
                        action = Some(RackAction::Command(
                            "set_parameter",
                            json!({"id":node.id,"parameter":name,"value":if enabled{0.0}else{1.0}}),
                        ));
                    }
                    response.on_hover_text(if *name=="linear_fm"{"Use linear FM. Off selects exponential FM. This mode is saved with the patch."}else{"Use soft sync. Off selects hard sync. This mode is saved with the patch."});
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
                        self.select_primary(node.id.clone());
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
                        self.select_primary(node.id.clone());
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
                        self.select_primary(node.id.clone());
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
                } else if node.kind == "audio_stereo" {
                    (pos2(width * 0.5, 130.0 + index as f32 * 120.0), 30.0)
                } else if node.kind == "score_sequencer" {
                    (pos2(60.0 + index as f32 * 120.0, 130.0), 24.0)
                } else if node.kind == "stereo_delay" {
                    (
                        pos2(
                            width * ((index % 4) as f32 + 0.5) / 4.0,
                            110.0 + (index / 4) as f32 * 80.0,
                        ),
                        17.0,
                    )
                } else if matches!(
                    node.kind.as_str(),
                    "stereo_reverb" | "sidechain_compressor" | "drum_voice"
                ) {
                    (
                        pos2(
                            width * ((index % 3) as f32 + 0.5) / 3.0,
                            110.0 + (index / 3) as f32 * 80.0,
                        ),
                        17.0,
                    )
                } else if node.kind == "vca_mix" {
                    (
                        pos2(
                            if *name == "mix_level" {
                                345.0
                            } else {
                                37.5 + index as f32 * 75.0
                            },
                            126.0,
                        ),
                        if *name == "mix_level" { 27.0 } else { 22.0 },
                    )
                } else if node.kind == "fade" {
                    (
                        pos2(120.0, if *name == "position" { 121.0 } else { 236.0 }),
                        if *name == "position" { 31.0 } else { 24.0 },
                    )
                } else if node.kind == "compare" {
                    (pos2(width * 0.5, 125.0), 27.0)
                } else if node.kind == "process" {
                    (pos2(width * 0.5, 118.0), 25.0)
                } else if node.kind == "gates" {
                    (pos2(78.0, 182.0), 29.0)
                } else if node.kind == "rescale" {
                    let column = usize::from(matches!(*name, "offset" | "max"));
                    let row = usize::from(matches!(*name, "min" | "max"));
                    (
                        pos2(
                            width * (0.25 + column as f32 * 0.5),
                            127.0 + row as f32 * 116.0,
                        ),
                        25.0,
                    )
                } else if node.kind == "quantizer" {
                    (pos2(width * 0.5, 260.0), 23.0)
                } else if matches!(node.kind.as_str(), "multi_lfo" | "multi_vco") {
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
                } else if matches!(node.kind.as_str(), "analog_vco" | "ladder_vcf") {
                    voice_parameter_position(index, width)
                } else {
                    parameter_position(count, index, width)
                };
                let center = point(knob_position.x, knob_position.y);
                let logarithmic = logarithmic_parameter(&node.kind, name);
                let step = if *name == "waveform" {
                    Some(1.0)
                } else {
                    spec["step"]
                        .as_f64()
                        .filter(|step| *step > 0.0)
                        .map(|step| step as f32)
                };
                let geometric =
                    matches!(node.kind.as_str(), "multi_lfo" | "multi_vco") && *name == "frequency";
                let clock_ratio = node.kind == "multi_lfo"
                    && geometric
                    && patch
                        .cables
                        .iter()
                        .any(|c| c.to == node.id && c.input == "clock");
                let gain_power = (node.kind == "vca_mix").then_some(if *name == "mix_level" {
                    1.0
                } else {
                    2.0
                });
                let fade_percent = node.kind == "fade";
                let vco_percent = node.kind == "multi_vco"
                    && matches!(*name, "pulse_width" | "fm_amount" | "pwm_amount");
                let slew_min = (node.kind == "process" && *name == "slew").then_some(min);
                let multiplier = node.params.get("multiplier").copied().unwrap_or(1.0);
                let scaled_gain = node.kind == "rescale" && *name == "gain";
                let display_scale = if clock_ratio {
                    0.5
                } else if scaled_gain {
                    if multiplier == 1.0 {
                        100.0
                    } else {
                        multiplier
                    }
                } else if fade_percent || vco_percent {
                    100.0
                } else {
                    1.0
                };
                let display_value = if let Some(power) = gain_power {
                    gain_db_display(value, power)
                } else if fade_percent {
                    format!("{:.1}%", value * 100.0)
                } else if clock_ratio {
                    format!("{:.4}x", value * display_scale)
                } else if let Some(min) = slew_min {
                    format!("{:.2} ms/V", slew_display(value, min))
                } else if scaled_gain {
                    format!(
                        "{:.2}{}",
                        value * display_scale,
                        if multiplier == 1.0 { "%" } else { "x" }
                    )
                } else {
                    parameter_value(name, value, spec)
                };
                let make_edit = |surface| ParameterEdit {
                    id: node.id.clone(),
                    kind: node.kind.clone(),
                    parameter: name.to_string(),
                    revision,
                    surface,
                    menu_popup: None,
                    stale: false,
                    text: if let Some(power) = gain_power {
                        gain_db(value, power).to_string()
                    } else {
                        slew_min
                            .map_or(value * display_scale, |min| slew_display(value, min))
                            .to_string()
                    },
                    min: if gain_power.is_some() {
                        f32::NEG_INFINITY
                    } else if slew_min.is_some() {
                        0.0
                    } else {
                        min * display_scale
                    },
                    max: if let Some(power) = gain_power {
                        gain_db(max, power)
                    } else if slew_min.is_some() {
                        max.exp2() * 1000.0
                    } else {
                        max * display_scale
                    },
                    step: step.map(|step| step * display_scale),
                    display_scale,
                    display_unit: if gain_power.is_some() {
                        "dB"
                    } else if slew_min.is_some() {
                        "ms/V"
                    } else if clock_ratio || (scaled_gain && multiplier != 1.0) {
                        "x"
                    } else if display_scale == 100.0 {
                        "%"
                    } else {
                        spec["unit"].as_str().unwrap_or("")
                    }
                    .into(),
                    slew_min,
                    gain_power,
                    raw_max: max,
                    focus: true,
                };
                let normalized = if geometric {
                    normalize_geometric(value, min, max)
                } else {
                    normalize(value, min, max, logarithmic)
                };
                let knob_rect =
                    Rect::from_center_size(center, Vec2::splat((radius + 10.0) * 2.0 * self.zoom));
                let mut knob_highlighted = false;
                if mode.is_interactive() {
                    let knob = ui.interact(
                        knob_rect,
                        ui.id().with(("knob", &node.id, name)),
                        Sense::click_and_drag(),
                    );
                    knob.widget_info(|| {
                        egui::WidgetInfo::slider(
                            true,
                            value as f64,
                            format!("{} {}", node.id, name),
                        )
                    });
                    if self
                        .requested_parameter_menu
                        .as_ref()
                        .is_some_and(|(id, parameter)| id == &node.id && parameter == name)
                    {
                        egui::Popup::open_id(ui.ctx(), knob.id.with("popup"));
                        self.requested_parameter_menu = None;
                    }
                    let mut parameter_popup = egui::Popup::context_menu(&knob)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
                    // A programmatic smoke opening has no saved pointer anchor.
                    // Real right-clicks retain their fixed pointer position.
                    if !knob.secondary_clicked()
                        && egui::Popup::position_of_id(ui.ctx(), knob.id.with("popup")).is_none()
                    {
                        parameter_popup = parameter_popup
                            .anchor(egui::PopupAnchor::Position(knob.rect.right_bottom()));
                    }
                    parameter_popup.show(|ui| {
                        let same_target = self.editing.as_ref().is_some_and(|edit| {
                            edit.surface == ParameterEditSurface::KnobMenu
                                && edit.id == node.id
                                && edit.parameter == *name
                        });
                        if !same_target {
                            self.editing = Some(make_edit(ParameterEditSurface::KnobMenu));
                        }
                        self.editing.as_mut().unwrap().menu_popup = Some(knob.id.with("popup"));
                        ui.set_min_width(244.0);
                        ui.strong(parameter_label(name));
                        ui.label(format!("Current: {display_value}"));
                        let outcome = parameter_editor_contents(ui, self.editing.as_mut().unwrap());
                        match outcome {
                            ParameterEditOutcome::Keep => {}
                            ParameterEditOutcome::Apply(raw) => {
                                let edit = self.editing.take().unwrap();
                                action = Some(parameter_edit_action(&edit, raw));
                                ui.close();
                            }
                            ParameterEditOutcome::Cancel => {
                                self.editing = None;
                                ui.close();
                            }
                        }
                    });
                    if knob.drag_started() {
                        self.select_primary(node.id.clone());
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
                    knob_highlighted = knob.hovered() || knob.dragged();
                    if knob_highlighted {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    }
                    knob.on_hover_text(format!(
                    "{}: {}\n{}\nDrag vertically; Cmd/Ctrl for fine adjustment. Double-click to reset; right-click to type a value.",
                    name, display_value,
                    if clock_ratio { "Multiplier of measured clock frequency. Free-runs until two rising edges arrive; clock changes speed without resetting phase." } else { description }
                ));
                }
                paint_knob(
                    &painter,
                    center,
                    radius * self.zoom,
                    normalized,
                    knob_highlighted,
                    self.zoom,
                );
                painter.text(
                    center - vec2(0.0, (radius + 13.0) * self.zoom),
                    Align2::CENTER_BOTTOM,
                    if node.kind == "seq3" {
                        ""
                    } else if clock_ratio {
                        "CLOCK RATIO"
                    } else if node.kind == "vca_mix" {
                        match *name {
                            "level1" => "CH 1",
                            "level2" => "CH 2",
                            "level3" => "CH 3",
                            "level4" => "CH 4",
                            _ => "MASTER",
                        }
                    } else {
                        parameter_label(name)
                    },
                    crate::panel_skin::heading_font(painter.ctx(), 11.0 * self.zoom),
                    INK,
                );
                let value_offset = if node.kind == "vca_mix" {
                    11.0
                } else if node.kind == "fade" {
                    10.0
                } else if count >= 3 {
                    9.0
                } else {
                    15.0
                };
                let value_center = center + vec2(0.0, (radius + value_offset) * self.zoom);
                let readout_width = if node.kind == "score_sequencer" {
                    96.0
                } else if node.kind == "stereo_delay" {
                    width / 4.0 - 12.0
                } else if matches!(
                    node.kind.as_str(),
                    "stereo_reverb" | "sidechain_compressor" | "drum_voice"
                ) {
                    width / 3.0 - 12.0
                } else if node.kind == "vca_mix" {
                    70.0
                } else if node.kind == "seq3" {
                    54.0
                } else if node.kind == "gates" {
                    110.0
                } else if count >= 4
                    || node.kind == "mid_side"
                    || (index > 0 && matches!(node.kind.as_str(), "analog_vco" | "ladder_vcf"))
                {
                    width * 0.5 - 12.0
                } else {
                    width - 20.0
                };
                crate::panel_skin::readout(
                    &painter,
                    value_center,
                    &display_value,
                    readout_width * self.zoom,
                    self.zoom,
                );
                if mode.is_interactive() {
                    let value_response = ui.interact(
                        Rect::from_center_size(value_center, vec2(readout_width, 19.0) * self.zoom),
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
                        self.editing = Some(make_edit(ParameterEditSurface::Window));
                    }
                    value_response.on_hover_text("Double-click to type a value");
                }
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
                let cables = input_groups
                    .get(&(node.id.as_str(), port))
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let connected = !cables.is_empty();
                if !mode.is_interactive() {
                    paint_jack(
                        &painter,
                        location,
                        port,
                        false,
                        false,
                        self.zoom,
                        is_voltage_utility(&node.kind),
                    );
                    continue;
                }
                let jack = jack(
                    ui,
                    &painter,
                    location,
                    ui.id().with(("input", &node.id, port)),
                    port,
                    connected,
                    false,
                    self.zoom,
                    is_voltage_utility(&node.kind),
                );
                if drag_interactive {
                    self.cable_jack(
                        &jack,
                        CableEnd::Input {
                            node: node.id.clone(),
                            port: port.to_string(),
                        },
                        patch,
                        catalog,
                        &mut action,
                        press_target.as_ref()
                            == Some(&CableEnd::Input {
                                node: node.id.clone(),
                                port: port.to_string(),
                            }),
                    );
                }
                if cables.len() > 1 {
                    stack_badges.push((
                        stack_badge_position(location, &node.kind, self.zoom),
                        cables.len(),
                    ));
                }
                let end = CableEnd::Input {
                    node: node.id.clone(),
                    port: port.into(),
                };
                if jack.secondary_clicked() {
                    self.open_cable_menu(end.clone(), patch);
                }
                if self.requested_cable_menu.as_ref() == Some(&end) {
                    self.open_cable_menu(end.clone(), patch);
                    egui::Popup::open_id(ui.ctx(), jack.id.with("popup"));
                    self.requested_cable_menu = None;
                }
                let mut popup = egui::Popup::context_menu(&jack)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
                if !jack.secondary_clicked()
                    && egui::Popup::position_of_id(ui.ctx(), jack.id.with("popup")).is_none()
                {
                    popup = popup.anchor(egui::PopupAnchor::Position(jack.rect.right_bottom()));
                }
                popup.show(|ui| {
                    self.port_cable_menu(ui, &end, patch, &mut action);
                });
                let mut details = format!(
                    "{}.{} · {} cable{}\n{}",
                    node.id,
                    port,
                    cables.len(),
                    if cables.len() == 1 { "" } else { "s" },
                    if patch.version == 2 {
                        "Drag moves the top IN plug. Cmd/Ctrl adds a cable; Cmd/Ctrl+Shift clones; Shift-click deletes the top cable. Other cables stay."
                    } else {
                        "Drag moves the top IN plug. Shift-click deletes it. Adding another cable to an occupied input requires engine v2."
                    }
                );
                for cable in ordered_endpoint_cables(patch, &end)
                    .into_iter()
                    .rev()
                    .take(8)
                {
                    details.push_str(&format!(
                        "\n{}: {}.{}",
                        cable.id.as_deref().unwrap_or("legacy"),
                        cable.from,
                        cable.output
                    ));
                }
                if cables.len() > 8 {
                    details.push_str("\nMore cables in the right-click list.");
                }
                jack.on_hover_text(details);
            }
            for (port_index, port) in output_ports.iter().enumerate() {
                let output_point = outputs[&(node.id.clone(), port.to_string())];
                let connected = patch
                    .cables
                    .iter()
                    .any(|c| c.from == node.id && c.output == *port);
                if !mode.is_interactive() {
                    paint_jack(
                        &painter,
                        output_point,
                        port,
                        false,
                        true,
                        self.zoom,
                        is_voltage_utility(&node.kind) || node.kind == "pulses",
                    );
                    self.paint_output_signal_light(&painter, node, port_index, output_point);
                    continue;
                }
                let jack = jack(
                    ui,
                    &painter,
                    output_point,
                    ui.id().with(("output", &node.id, port)),
                    port,
                    connected,
                    true,
                    self.zoom,
                    is_voltage_utility(&node.kind) || node.kind == "pulses",
                );
                if drag_interactive {
                    self.cable_jack(
                        &jack,
                        CableEnd::Output {
                            node: node.id.clone(),
                            port: port.to_string(),
                        },
                        patch,
                        catalog,
                        &mut action,
                        press_target.as_ref()
                            == Some(&CableEnd::Output {
                                node: node.id.clone(),
                                port: port.to_string(),
                            }),
                    );
                }
                self.paint_output_signal_light(&painter, node, port_index, output_point);
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
                let end = CableEnd::Output {
                    node: node.id.clone(),
                    port: port.to_string(),
                };
                if jack.secondary_clicked() {
                    self.open_cable_menu(end.clone(), patch);
                }
                if self.requested_cable_menu.as_ref() == Some(&end) {
                    self.open_cable_menu(end.clone(), patch);
                    egui::Popup::open_id(ui.ctx(), jack.id.with("popup"));
                    self.requested_cable_menu = None;
                }
                let mut popup = egui::Popup::context_menu(&jack)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
                if !jack.secondary_clicked()
                    && egui::Popup::position_of_id(ui.ctx(), jack.id.with("popup")).is_none()
                {
                    popup = popup.anchor(egui::PopupAnchor::Position(jack.rect.right_bottom()));
                }
                popup.show(|ui| {
                    self.port_cable_menu(ui, &end, patch, &mut action);
                    ui.separator();
                    for (lane, label) in [(0, "Inspect in scope X"), (1, "Inspect in scope Y")] {
                        if ui.button(label).clicked() {
                            self.pending = None;
                            self.cable_press = None;
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
                jack.on_hover_text(format!("{}.{}\nCapacity: up to {capacity} channels; {units}.\nThis is the port specification, not a live channel meter.\nDrag moves the top OUT plug. Cmd/Ctrl-drag adds a cable; Cmd/Ctrl+Shift clones; Shift-click deletes the top cable.", node.id, port));
            }
        }

        RenderedModules {
            outputs,
            inputs,
            jack_targets,
            pointer_on_module,
            stack_badges,
            input_groups,
            cable_ordinals,
            action,
        }
    }

    fn paint_output_signal_light(
        &self,
        painter: &egui::Painter,
        node: &Node,
        port_index: usize,
        output_point: Pos2,
    ) {
        let light_index = match node.kind.as_str() {
            "logic" => Some(port_index),
            "compare" if port_index >= 4 => Some(port_index - 4),
            "gates" => Some(port_index),
            _ => None,
        };
        if let Some(index) = light_index {
            let brightness = self
                .module_lights
                .get(&node.id)
                .filter(|s| index < usize::from(s.count))
                .map_or([0.0; 2], |s| s.brightness[index]);
            paint_signal_light(
                painter,
                output_point + vec2(0.0, 17.0) * self.zoom,
                brightness,
                self.zoom,
            );
        }
    }

    fn mixing_panel(&self, ui: &mut egui::Ui, node: &Node, rect: Rect) {
        let zoom = self.zoom;
        let painter = ui.painter();
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        if node.kind == "vca_mix" {
            for x in [75.0, 150.0, 225.0, 300.0] {
                painter.line_segment(
                    [point(x, 77.0), point(x, 378.0)],
                    Stroke::new(
                        if x == 300.0 { 1.0 } else { 0.5 } * zoom,
                        Color32::from_gray(178),
                    ),
                );
            }
            let state = self.module_lights.get(&node.id).filter(|s| s.count >= 4);
            for channel in 0..4 {
                let center = point(37.5 + channel as f32 * 75.0, 183.0);
                let value = state
                    .map_or(0.0, |s| s.brightness[channel][0])
                    .clamp(0.0, 1.0);
                for segment in 0..12 {
                    let lit = state.is_some() && value > (segment as f32 / 12.0);
                    let color = if segment >= 10 {
                        Color32::from_rgb(223, 151, 58)
                    } else {
                        Color32::from_rgb(76, 163, 112)
                    };
                    painter.rect_filled(
                        Rect::from_min_size(
                            center + vec2(-25.0 + segment as f32 * 4.2, -3.0) * zoom,
                            vec2(3.1, 6.0) * zoom,
                        ),
                        0.5,
                        color.gamma_multiply(if lit { 1.0 } else { 0.16 }),
                    );
                }
                ui.interact(Rect::from_center_size(center,vec2(54.0,13.0)*zoom),ui.id().with((&node.id,"strip-vu",channel)),Sense::hover()).on_hover_text("DSP strip VU: signed channel sum / 5 V, peak smoothing, -24 to 0 dB brightness. Polyphonic lanes can cancel. This is the signal meter, not the knob gain.");
            }
            painter.text(
                point(150.0, 194.0),
                Align2::CENTER_CENTER,
                if state.is_some() {
                    "POST-CV STRIP VU"
                } else {
                    "VU UNAVAILABLE"
                },
                crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
                MUTED,
            );
            let channel_exp = node.params.get("channel_exp").copied().unwrap_or(0.0) >= 0.5;
            let mix_exp = node.params.get("mix_exp").copied().unwrap_or(0.0) >= 0.5;
            painter.text(
                point(150.0, 254.0),
                Align2::CENTER_CENTER,
                if channel_exp {
                    "CHANNEL CV: EXP"
                } else {
                    "CHANNEL CV: LINEAR"
                },
                crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
                MUTED,
            );
            painter.text(
                point(345.0, 215.0),
                Align2::CENTER_CENTER,
                "SUM",
                crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
                MUTED,
            );
            painter.text(
                point(345.0, 244.0),
                Align2::CENTER_CENTER,
                if mix_exp { "EXP CV" } else { "LINEAR CV" },
                crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
                MUTED,
            );
            painter.text(
                point(345.0, 182.0),
                Align2::CENTER_CENTER,
                "Right-click",
                crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
                MUTED,
            );
            painter.text(
                point(345.0, 194.0),
                Align2::CENTER_CENTER,
                "CV response",
                crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
                MUTED,
            );
        } else {
            let square_root = node.params.get("law").copied().unwrap_or(0.0) >= 0.5;
            painter.text(
                point(120.0, 292.0),
                Align2::CENTER_CENTER,
                if square_root {
                    "SQUARE-ROOT LAW"
                } else {
                    "LINEAR LAW"
                },
                crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
                MUTED,
            );
            painter.text(
                point(120.0, 177.0),
                Align2::CENTER_CENTER,
                "A  <  MORPH  >  B",
                crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
                MUTED,
            );
        }
    }

    fn signal_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        allow_press: bool,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let painter = ui.painter().clone();
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let mut action = None;
        if matches!(node.kind.as_str(), "logic" | "process") {
            let (x, y, label, press, release) = if node.kind == "logic" {
                (180.0, 170.0, "MANUAL B", "b_press", "b_release")
            } else {
                (120.0, 185.0, "MANUAL GATE", "gate_press", "gate_release")
            };
            let button_rect = Rect::from_center_size(point(x, y), vec2(100.0, 27.0) * zoom);
            let enabled = ui.is_enabled()
                && self.performance_available
                && !node.bypassed
                && ui.input(|i| i.focused);
            let observed = self
                .module_controls
                .get(&node.id)
                .map(|s| s.pressed)
                .or_else(|| self.module_lights.get(&node.id).map(|s| s.manual));
            let actual = observed.unwrap_or(false);
            let label = if observed.is_none() {
                if enabled {
                    if node.kind == "logic" {
                        "B: WAITING"
                    } else {
                        "GATE: WAITING"
                    }
                } else if node.kind == "logic" {
                    "B: OFFLINE"
                } else {
                    "GATE: OFFLINE"
                }
            } else {
                label
            };
            let response = ui.interact(
                button_rect,
                ui.id().with((&node.id, "manual-button")),
                Sense::click_and_drag(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    enabled,
                    format!("{} {} momentary", node.id, label),
                )
            });
            painter.rect_filled(
                button_rect,
                3.0,
                if actual {
                    Color32::from_rgb(224, 195, 104)
                } else if enabled && response.hovered() {
                    Color32::from_rgb(239, 241, 232)
                } else {
                    Color32::from_rgb(193, 196, 187)
                },
            );
            painter.rect_stroke(
                button_rect,
                3.0,
                Stroke::new(zoom, Color32::from_gray(129)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                button_rect.center() - vec2(5.0, 0.0) * zoom,
                Align2::CENTER_CENTER,
                label,
                crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
                if enabled { INK } else { MUTED },
            );
            paint_signal_light(
                &painter,
                button_rect.right_center() - vec2(10.0, 0.0) * zoom,
                [if actual { 1.0 } else { 0.0 }, 0.0],
                zoom,
            );
            if enabled
                && allow_press
                && self.held_manual.is_none()
                && response.is_pointer_button_down_on()
                && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
            {
                self.held_manual = Some(HeldManual {
                    id: node.id.clone(),
                    kind: node.kind.clone(),
                    release,
                    button_rect,
                });
                self.select_primary(node.id.clone());
                action = Some(RackAction::Command(
                    "trigger_module",
                    json!({"id":node.id,"action":press}),
                ));
            }
            response.on_hover_text(if enabled {"Hold to force the gate high. Releasing anywhere restores the patched gate. Indicator follows the audio engine."} else {"Momentary control requires a live audio engine and an active module."});
        }
        if matches!(node.kind.as_str(), "logic" | "compare") {
            painter.text(
                point(120.0, if node.kind == "logic" { 207.0 } else { 246.0 }),
                Align2::CENTER_CENTER,
                if self.module_lights.contains_key(&node.id) {
                    "YELLOW: MONO   BLUE: POLY"
                } else {
                    "LIGHTS UNAVAILABLE"
                },
                crate::panel_skin::print_font(painter.ctx(), 7.5 * zoom),
                MUTED,
            );
        }
        if node.kind == "rescale" {
            let states = self.module_lights.get(&node.id);
            for (index, x, name) in [(0, 180.0, "max"), (1, 60.0, "min")] {
                let brightness = states
                    .filter(|s| index < usize::from(s.count))
                    .map_or([0.0; 2], |s| s.brightness[index]);
                let center = point(x, 294.0);
                paint_signal_light(&painter, center, brightness, zoom);
                ui.interact(Rect::from_center_size(center,vec2(30.0,14.0)*zoom),ui.id().with((&node.id,"bound-light",index)),Sense::hover()).on_hover_text(format!("Pre-bound {name} crossing: yellow mono, blue poly. Brightness follows DSP, before clipping or reflection."));
                let reflect = node
                    .params
                    .get(&format!("reflect_{name}"))
                    .copied()
                    .unwrap_or(0.0)
                    >= 0.5;
                painter.text(
                    point(x, 311.0),
                    Align2::CENTER_CENTER,
                    if reflect { "REFLECT" } else { "CLIP" },
                    crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
                    MUTED,
                );
            }
            painter.text(
                point(120.0, 331.0),
                Align2::CENTER_CENTER,
                if self.module_lights.contains_key(&node.id) {
                    format!(
                        "Multiplier {}x · right-click to set",
                        node.params.get("multiplier").copied().unwrap_or(1.0) as u32
                    )
                } else {
                    "Lights unavailable · right-click settings".into()
                },
                crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
                MUTED,
            );
        }
        action
    }

    fn pulses_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        allow_press: bool,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let painter = ui.painter().clone();
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let enabled = ui.is_enabled()
            && self.shortcuts_enabled
            && self.performance_available
            && !node.bypassed
            && ui.input(|i| i.focused);
        for (x, label) in [(92.0, "FIRE"), (155.0, "TRIG"), (205.0, "GATE")] {
            painter.text(
                point(x, 80.0),
                Align2::CENTER_CENTER,
                label,
                crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
                MUTED,
            );
        }
        let pressed_mask = self
            .module_controls
            .get(&node.id)
            .map_or(0, |controls| controls.pressed_mask);
        let mut action = None;
        for row in 0..10 {
            let y = 100.0 + row as f32 * 29.0;
            let active = pressed_mask & (1_u16 << row) != 0;
            let button_rect = Rect::from_center_size(point(92.0, y), vec2(62.0, 21.0) * zoom);
            let response = ui.interact(
                button_rect,
                ui.id().with((&node.id, "pulse-button", row)),
                Sense::click_and_drag(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    enabled,
                    format!(
                        "PULSES {} row {} momentary trigger and gate",
                        node.id,
                        row + 1
                    ),
                )
            });
            painter.text(
                point(35.0, y),
                Align2::CENTER_CENTER,
                format!("{:02}", row + 1),
                FontId::monospace(9.0 * zoom),
                MUTED,
            );
            painter.rect_filled(
                button_rect.translate(vec2(0.8, 1.5) * zoom),
                3.0 * zoom,
                Color32::from_black_alpha(80),
            );
            painter.rect_filled(
                button_rect,
                3.0 * zoom,
                if active {
                    Color32::from_rgb(212, 166, 86)
                } else if enabled && response.hovered() {
                    Color32::from_rgb(246, 237, 210)
                } else {
                    Color32::from_rgb(190, 186, 175)
                },
            );
            painter.rect_stroke(
                button_rect,
                3.0 * zoom,
                Stroke::new(zoom, Color32::from_rgb(89, 85, 76)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                button_rect.center(),
                Align2::CENTER_CENTER,
                "PUSH",
                crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
                if enabled { INK } else { MUTED },
            );
            let brightness = self
                .module_lights
                .get(&node.id)
                .filter(|lights| row < usize::from(lights.count))
                .map_or(0.0, |lights| lights.brightness[row][0])
                .clamp(0.0, 1.0);
            painter.circle_filled(point(132.0, y), 4.0 * zoom, Color32::from_rgb(71, 66, 58));
            painter.circle_filled(
                point(132.0, y),
                2.8 * zoom,
                Color32::from_rgb(238, 177, 65).gamma_multiply(0.12 + 0.88 * brightness),
            );
            if row < 9 {
                painter.line_segment(
                    [point(24.0, y + 14.0), point(216.0, y + 14.0)],
                    Stroke::new(0.55 * zoom, Color32::from_white_alpha(28)),
                );
            }
            if enabled
                && allow_press
                && self.held_manual.is_none()
                && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
            {
                if response.clicked_by(PointerButton::Primary) {
                    self.select_primary(node.id.clone());
                    action = Some(RackAction::ManualClick {
                        id: node.id.clone(),
                        press: PULSE_PRESS[row],
                        release: PULSE_RELEASE[row],
                    });
                } else if response.is_pointer_button_down_on() {
                    self.held_manual = Some(HeldManual {
                        id: node.id.clone(),
                        kind: node.kind.clone(),
                        release: PULSE_RELEASE[row],
                        button_rect,
                    });
                    self.select_primary(node.id.clone());
                    action = Some(RackAction::Command(
                        "trigger_module",
                        json!({"id":node.id,"action":PULSE_PRESS[row]}),
                    ));
                }
            }
            response.on_hover_text(if enabled {
                format!(
                    "Row {}: press for TRIG {}, hold for GATE {}. Moving outside or releasing the button ends the gate.",
                    row + 1,
                    row + 1,
                    row + 1
                )
            } else {
                "Momentary controls require a live audio engine and an active module.".into()
            });
        }
        painter.text(
            point(120.0, 384.0),
            Align2::CENTER_CENTER,
            if node.bypassed {
                "BYPASSED"
            } else if !self.module_controls.contains_key(&node.id) {
                if self.performance_available {
                    "WAITING FOR ENGINE"
                } else {
                    "AUDIO OFFLINE"
                }
            } else if !self.performance_available {
                "LAST ENGINE STATE"
            } else {
                "TRIGGER / HELD GATE"
            },
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            MUTED,
        );
        action
    }

    fn push_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        allow_press: bool,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let painter = ui.painter().clone();
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let enabled = ui.is_enabled()
            && self.shortcuts_enabled
            && self.performance_available
            && !node.bypassed
            && ui.input(|i| i.focused);
        for (y, label) in [(78.0, "REMOTE"), (159.0, "MANUAL")] {
            painter.text(
                point(90.0, y),
                Align2::CENTER_CENTER,
                label,
                crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
                MUTED,
            );
        }
        let pressed_mask = self
            .module_controls
            .get(&node.id)
            .map_or(0, |controls| controls.pressed_mask);
        let lights = self.module_lights.get(&node.id).copied();
        let mut action = None;
        for (index, x, label, led_label, led_color) in [
            (0, 49.0, "PUSH", "GATE", Color32::from_rgb(238, 177, 65)),
            (1, 131.0, "HOLD", "LATCH", Color32::from_rgb(75, 165, 178)),
        ] {
            let active = pressed_mask & (1_u16 << index) != 0;
            let button_rect = Rect::from_center_size(point(x, 209.0), vec2(72.0, 48.0) * zoom);
            let response = ui.interact(
                button_rect,
                ui.id().with((&node.id, "push-button", index)),
                Sense::click_and_drag(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    enabled,
                    format!("PUSH {} {} performance button", node.id, label),
                )
            });
            painter.rect_filled(
                button_rect.translate(vec2(1.3, 2.5) * zoom),
                4.0 * zoom,
                Color32::from_black_alpha(95),
            );
            painter.rect_filled(
                button_rect,
                4.0 * zoom,
                if active {
                    Color32::from_rgb(212, 166, 86)
                } else if enabled && response.hovered() {
                    Color32::from_rgb(246, 237, 210)
                } else {
                    Color32::from_rgb(190, 186, 175)
                },
            );
            painter.rect_stroke(
                button_rect,
                4.0 * zoom,
                Stroke::new(1.2 * zoom, Color32::from_rgb(89, 85, 76)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                button_rect.center(),
                Align2::CENTER_CENTER,
                label,
                crate::panel_skin::print_font(painter.ctx(), 12.0 * zoom),
                if enabled { INK } else { MUTED },
            );
            let brightness = lights
                .filter(|state| index < usize::from(state.count))
                .map_or(0.0, |state| state.brightness[index][0])
                .clamp(0.0, 1.0);
            painter.circle_filled(point(x, 265.0), 6.0 * zoom, Color32::from_rgb(71, 66, 58));
            painter.circle_filled(
                point(x, 265.0),
                4.4 * zoom,
                led_color.gamma_multiply(0.12 + 0.88 * brightness),
            );
            painter.text(
                point(x, 283.0),
                Align2::CENTER_CENTER,
                led_label,
                crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
                MUTED,
            );
            if enabled
                && allow_press
                && self.held_manual.is_none()
                && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
            {
                if response.clicked_by(PointerButton::Primary) {
                    self.select_primary(node.id.clone());
                    action = Some(RackAction::ManualClick {
                        id: node.id.clone(),
                        press: PUSH_PRESS[index],
                        release: PUSH_RELEASE[index],
                    });
                } else if response.is_pointer_button_down_on() {
                    self.held_manual = Some(HeldManual {
                        id: node.id.clone(),
                        kind: node.kind.clone(),
                        release: PUSH_RELEASE[index],
                        button_rect,
                    });
                    self.select_primary(node.id.clone());
                    action = Some(RackAction::Command(
                        "trigger_module",
                        json!({"id":node.id,"action":PUSH_PRESS[index]}),
                    ));
                }
            }
            response.on_hover_text(if enabled {
                if index == 0 {
                    "Momentary Push control. Hold can invert the resulting gate. The Gate lamp follows the audio engine. Moving outside or releasing ends the press."
                } else {
                    "Press Hold to toggle the runtime latch. The Latch lamp shows the actual latched state; Hold can invert the resulting gate. Moving outside or releasing ends the physical press."
                }
            } else {
                "Performance controls require a live audio engine and an active module. Lamps show the last reported engine state."
            });
        }
        painter.text(
            point(90.0, 302.0),
            Align2::CENTER_CENTER,
            if node.bypassed {
                "BYPASSED"
            } else if self.module_controls.contains_key(&node.id) && !self.performance_available {
                "LAST ENGINE STATE"
            } else if !self.performance_available {
                "AUDIO OFFLINE"
            } else if !self.module_controls.contains_key(&node.id) {
                "WAITING FOR ENGINE"
            } else {
                "HOLD INVERTS GATE"
            },
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            MUTED,
        );
        action
    }

    fn gates_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        allow_press: bool,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let painter = ui.painter().clone();
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let enabled = ui.is_enabled()
            && self.shortcuts_enabled
            && self.performance_available
            && !node.bypassed
            && ui.input(|i| i.focused);
        painter.line_segment(
            [point(24.0, 247.0), point(216.0, 247.0)],
            Stroke::new(0.8 * zoom, Color32::from_rgb(146, 140, 126)),
        );
        let button_rect = Rect::from_center_size(point(184.0, 182.0), vec2(72.0, 43.0) * zoom);
        let response = ui.interact(
            button_rect,
            ui.id().with((&node.id, "gates-reset")),
            Sense::click_and_drag(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                enabled,
                format!("GATES {} momentary Reset button", node.id),
            )
        });
        let active = self
            .module_controls
            .get(&node.id)
            .is_some_and(|controls| controls.pressed_mask & 1 != 0);
        painter.text(
            point(184.0, 146.0),
            Align2::CENTER_CENTER,
            "MANUAL",
            crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
            INK,
        );
        painter.rect_filled(
            button_rect.translate(vec2(1.2, 2.4) * zoom),
            4.0 * zoom,
            Color32::from_black_alpha(95),
        );
        painter.rect_filled(
            button_rect,
            4.0 * zoom,
            if active {
                Color32::from_rgb(212, 166, 86)
            } else if enabled && response.hovered() {
                Color32::from_rgb(246, 237, 210)
            } else {
                Color32::from_rgb(190, 186, 175)
            },
        );
        painter.rect_stroke(
            button_rect,
            4.0 * zoom,
            Stroke::new(1.2 * zoom, Color32::from_rgb(89, 85, 76)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            button_rect.center(),
            Align2::CENTER_CENTER,
            "RESET",
            crate::panel_skin::print_font(painter.ctx(), 11.0 * zoom),
            if enabled { INK } else { MUTED },
        );
        painter.text(
            point(184.0, 229.0),
            Align2::CENTER_CENTER,
            if node.bypassed {
                "BYPASSED"
            } else if !self.performance_available {
                "AUDIO OFFLINE"
            } else if !self.module_controls.contains_key(&node.id) {
                "WAITING"
            } else {
                "MOMENTARY"
            },
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            MUTED,
        );
        let action = if enabled
            && allow_press
            && self.held_manual.is_none()
            && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
        {
            if response.clicked_by(PointerButton::Primary) {
                self.select_primary(node.id.clone());
                Some(RackAction::ManualClick {
                    id: node.id.clone(),
                    press: "reset_press",
                    release: "reset_release",
                })
            } else if response.is_pointer_button_down_on() {
                self.held_manual = Some(HeldManual {
                    id: node.id.clone(),
                    kind: node.kind.clone(),
                    release: "reset_release",
                    button_rect,
                });
                self.select_primary(node.id.clone());
                Some(RackAction::Command(
                    "trigger_module",
                    json!({"id":node.id,"action":"reset_press"}),
                ))
            } else {
                None
            }
        } else {
            None
        };
        response.on_hover_text(if enabled {
            "Momentary reset. Press to reset the timed gate state; moving outside or releasing ends the manual reset. The RESET input accepts a remote pulse."
        } else {
            "Momentary reset requires a live audio engine and an active module."
        });
        action
    }

    fn utility_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        patch: &Patch,
        rect: Rect,
    ) -> Option<RackAction> {
        let painter = ui.painter().clone();
        let zoom = self.zoom;
        let width = rect.width() / zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let rows = match node.kind.as_str() {
            "attenuverter8" => 8,
            "mutes10" => 10,
            _ => 3,
        };
        let mut action = None;
        for (x, label) in [
            (29.0, "IN"),
            (
                115.0,
                if node.kind == "mutes10" {
                    "STATE"
                } else {
                    "LEVEL"
                },
            ),
            (
                width - 29.0,
                if node.kind == "cv_mix" {
                    "SOURCE"
                } else {
                    "OUT"
                },
            ),
        ] {
            painter.text(
                point(x, 79.0),
                Align2::CENTER_CENTER,
                label,
                crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
                MUTED,
            );
        }
        for row in 0..rows {
            let y = utility_row_y(&node.kind, row);
            painter.line_segment(
                [point(15.0, y + 15.0), point(width - 15.0, y + 15.0)],
                Stroke::new(0.5 * zoom, Color32::from_gray(185)),
            );
            painter.text(
                point(53.0, y),
                Align2::CENTER_CENTER,
                format!("{:02}", row + 1),
                FontId::monospace(8.0 * zoom),
                MUTED,
            );
            let parameter = format!(
                "{}{}",
                if node.kind == "mutes10" {
                    "mute"
                } else if node.kind == "cv_mix" {
                    "level"
                } else {
                    "gain"
                },
                row + 1
            );
            let control = Rect::from_min_size(point(70.0, y - 11.0), vec2(92.0, 22.0) * zoom);
            if node.kind == "mutes10" {
                let muted = node.params.get(&parameter).copied().unwrap_or(0.0) >= 0.5;
                if panel_button(
                    ui,
                    &painter,
                    control,
                    ui.id().with((&node.id, &parameter)),
                    if muted { "MUTED" } else { "PASS" },
                    muted,
                    true,
                    zoom,
                ) {
                    self.select_primary(node.id.clone());
                    action = Some(RackAction::Command(
                        "set_parameter",
                        json!({"id":node.id,"parameter":parameter,"value":if muted {0.0} else {1.0}}),
                    ));
                }
                painter.text(
                    point(181.0, y),
                    Align2::CENTER_CENTER,
                    if muted { "X" } else { ">" },
                    crate::panel_skin::print_font(painter.ctx(), 12.0 * zoom),
                    if muted {
                        Color32::from_rgb(161, 76, 58)
                    } else {
                        MUTED
                    },
                );
            } else {
                let source = utility_source_row(patch, node, row);
                let (suffix, scale) = if source.is_some() {
                    (" %", 100.0)
                } else {
                    (" V", 10.0)
                };
                if let Some(spec) = module.and_then(|m| m["params"].get(&parameter)) {
                    if let Some(edit) = self.compact_parameter_scaled(
                        ui, node, &parameter, spec, control, suffix, scale,
                    ) {
                        action = Some(edit);
                    }
                }
                let label = source.map_or("10V".into(), |source| {
                    if source == row {
                        "IN".into()
                    } else {
                        format!("in{}", source + 1)
                    }
                });
                let explanation = source.map_or("Unpatched: normalled to +10 V. The level sets the output voltage.".into(), |source| if source == row { "Patched input: level is a percentage of this signal.".into() } else { format!("Unpatched: inherits input {} above. Level is a percentage of that signal.", source + 1) });
                let hint = Rect::from_center_size(
                    point(
                        if node.kind == "cv_mix" {
                            width - 29.0
                        } else {
                            181.0
                        },
                        y,
                    ),
                    vec2(32.0, 22.0) * zoom,
                );
                painter.text(
                    hint.center(),
                    Align2::CENTER_CENTER,
                    label,
                    FontId::monospace(8.0 * zoom),
                    MUTED,
                );
                ui.interact(
                    hint,
                    ui.id().with((&node.id, row, "normal")),
                    Sense::hover(),
                )
                .on_hover_text(explanation);
            }
        }
        let footer = match node.kind.as_str() {
            "attenuverter8" => "Unpatched: input above; first = +10 V",
            "mutes10" => "Unpatched: input above; first = 0 V",
            _ => "Unpatched = +10 V · bipolar sum",
        };
        painter.text(
            point(
                width * 0.5,
                if node.kind == "cv_mix" { 320.0 } else { 375.0 },
            ),
            Align2::CENTER_CENTER,
            footer,
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            MUTED,
        );
        if node.kind == "cv_mix" {
            painter.text(
                point(width * 0.5, 347.0),
                Align2::CENTER_CENTER,
                "SUM OUT",
                crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
                INK,
            );
        }
        action
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
            crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
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
                self.select_primary(node.id.clone());
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
            self.select_primary(node.id.clone());
            action = Some(RackAction::Command(
                "set_parameter",
                json!({"id":node.id,"parameter":"declick","value":if declick {0.0} else {1.0}}),
            ));
        }
        let status = if node.bypassed {
            "Bypassed · frozen".into()
        } else {
            selected.map_or("Waiting for engine".into(), |step| {
                format!("Route {step} / {length}")
            })
        };
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
                crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
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
                self.performance_available && !node.bypassed,
                zoom,
            ) {
                action = Some(RackAction::Command(
                    "trigger_module",
                    json!({"id":node.id,"action":event}),
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
                crate::panel_skin::control_style(ui, self.zoom);
                let font = crate::panel_skin::print_font(ui.ctx(), 12.0 * self.zoom);
                ui.style_mut().override_font_id = Some(font);
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
        self.compact_parameter_scaled(ui, node, parameter, spec, rect, suffix, 1.0)
    }

    #[allow(clippy::too_many_arguments)]
    fn compact_parameter_scaled(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        parameter: &str,
        spec: &Value,
        rect: Rect,
        suffix: &str,
        scale: f32,
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
        value *= scale;
        let id = ui.id().with((&node.id, parameter, "compact"));
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect).id_salt(id), |ui| {
                // Retain the established active gold cue while matching the
                // inactive and text-edit surfaces to the panel readouts.
                ui.visuals_mut().widgets.active.bg_fill = Color32::from_rgb(226, 197, 143);
                crate::panel_skin::control_style(ui, self.zoom);
                ui.style_mut().override_font_id = Some(FontId::monospace(10.0 * self.zoom));
                ui.spacing_mut().button_padding = vec2(2.0, 1.0) * self.zoom;
                ui.spacing_mut().interact_size = vec2(8.0, 18.0) * self.zoom;
                ui.add_sized(
                    rect.size(),
                    egui::DragValue::new(&mut value)
                        .range(min * scale..=max * scale)
                        .speed(if step.is_some() { 0.1 } else { 0.02 * scale })
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
            self.select_primary(node.id.clone());
            let mut args = json!({"id":node.id,"parameter":parameter,"value":quantize(value / scale,min,max,step)});
            if let Some(group) = group {
                args["history_group"] = json!(group);
            }
            Some(RackAction::Command("set_parameter", args))
        } else {
            None
        }
    }

    fn begin_cable(
        &self,
        end: CableEnd,
        patch: &Patch,
        shift: bool,
        command: bool,
    ) -> PendingCable {
        let mut pending = PendingCable::at_revision(end, patch, self.observed_revision);
        pending.shift_at_press = shift;
        if matches!(pending.origin, CableEnd::Output { .. }) {
            pending.original = ordered_endpoint_cables(patch, &pending.origin)
                .last()
                .map(|cable| (*cable).clone());
        }
        if command && !shift {
            pending.original = None;
        }
        pending.operation = if pending.original.is_none() {
            CableOperation::Create
        } else if command && shift {
            CableOperation::Clone
        } else {
            CableOperation::Move
        };
        pending
    }

    fn cable_press_target(
        &self,
        patch: &Patch,
        catalog: &Value,
        canvas: Rect,
        clip: Rect,
        pos: Pos2,
    ) -> Option<CableEnd> {
        if !clip.contains(pos) {
            return None;
        }
        let mut nearest: Option<(f32, CableEnd)> = None;
        for node in &patch.nodes {
            let Some(module) = catalog["modules"]
                .as_array()
                .and_then(|modules| modules.iter().find(|m| m["kind"] == node.kind))
            else {
                continue;
            };
            for input in [true, false] {
                let Some(ports) = module[if input { "inputs" } else { "outputs" }].as_array()
                else {
                    continue;
                };
                for (index, port) in ports.iter().filter_map(Value::as_str).enumerate() {
                    let center = canvas.min
                        + self.pan
                        + (self.positions[&node.id].to_vec2()
                            + port_position(
                                &node.kind,
                                input,
                                index,
                                ports.len(),
                                panel_width(&node.kind),
                            )
                            .to_vec2())
                            * self.zoom;
                    let hit =
                        Rect::from_center_size(center, Vec2::splat((29.0 * self.zoom).max(18.0)))
                            .intersect(clip);
                    let distance = center.distance_sq(pos);
                    if hit.contains(pos) && nearest.as_ref().is_none_or(|(old, _)| distance < *old)
                    {
                        nearest = Some((
                            distance,
                            if input {
                                CableEnd::Input {
                                    node: node.id.clone(),
                                    port: port.into(),
                                }
                            } else {
                                CableEnd::Output {
                                    node: node.id.clone(),
                                    port: port.into(),
                                }
                            },
                        ));
                    }
                }
            }
        }
        nearest.map(|(_, end)| end)
    }

    fn cable_jack(
        &mut self,
        response: &egui::Response,
        end: CableEnd,
        patch: &Patch,
        catalog: &Value,
        action: &mut Option<RackAction>,
        press_eligible: bool,
    ) {
        // Capture intent on the press: modifiers may be released before egui
        // recognizes a drag, and an earlier click may have left a draft here.
        let press_modifiers = response.ctx.input(|i| {
            i.events.iter().rev().find_map(|event| match event {
                egui::Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers,
                } if response.interact_rect.contains(*pos) => Some(*modifiers),
                _ => None,
            })
        });
        if let Some(modifiers) = press_modifiers.filter(|_| press_eligible) {
            if modifiers.shift && !modifiers.command && !modifiers.ctrl && !modifiers.alt {
                let picked = self.begin_cable(end.clone(), patch, false, false);
                *action = delete_moved_cable(&picked);
                self.pending = None;
                self.cable_press = None;
                self.dragging_cable = false;
                return; // Includes empty ports and coalesced click events.
            }
            self.cable_press = if modifiers == egui::Modifiers::NONE
                && self
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.origin == end)
            {
                self.pending.clone()
            } else {
                Some(self.begin_cable(
                    end.clone(),
                    patch,
                    modifiers.shift,
                    !modifiers.alt && (modifiers.command || modifiers.ctrl),
                ))
            };
        }
        if response.clicked() {
            let pressed = self.cable_press.take();
            if let Some(pending) = self.pending.take() {
                if pending.origin != end || pending.operation == CableOperation::Move {
                    if cable_plan(&pending, &end).is_some() {
                        *action = cable_action_mode(
                            patch,
                            &pending,
                            &end,
                            catalog,
                            response.ctx.input(|i| i.modifiers.shift),
                        );
                    } else {
                        self.pending = pressed;
                    }
                }
            } else {
                self.pending = pressed;
            }
            self.dragging_cable = false;
            self.select_primary(match &end {
                CableEnd::Output { node, .. } | CableEnd::Input { node, .. } => node.clone(),
            });
        }
        if response.drag_started_by(PointerButton::Primary) {
            if let Some(pressed) = self.cable_press.take() {
                if pressed.operation != CableOperation::Move
                    || pressed.stack_requested
                    || self
                        .pending
                        .as_ref()
                        .is_none_or(|pending| pending.origin != end)
                {
                    self.pending = Some(pressed);
                }
                self.dragging_cable = self.pending.is_some();
            }
            self.select_primary(match end {
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
        self.reconcile_selection(patch);
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
                || pending
                    .companions
                    .iter()
                    .any(|old| !patch.cables.iter().any(|cable| same_cable(old, cable)))
        }) {
            self.pending = None;
            self.cable_press = None;
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

    fn reveal_selected_group(&mut self, patch: &Patch, canvas: Rect) {
        let nodes: Vec<_> = patch
            .nodes
            .iter()
            .filter(|n| self.selection.contains(&n.id))
            .collect();
        if nodes.is_empty() {
            return;
        }
        let min = pos2(
            nodes
                .iter()
                .map(|n| self.positions[&n.id].x)
                .fold(f32::INFINITY, f32::min),
            nodes
                .iter()
                .map(|n| self.positions[&n.id].y)
                .fold(f32::INFINITY, f32::min),
        );
        let max = pos2(
            nodes
                .iter()
                .map(|n| self.positions[&n.id].x + panel_width(&n.kind))
                .fold(f32::NEG_INFINITY, f32::max),
            nodes
                .iter()
                .map(|n| self.positions[&n.id].y + PANEL_HEIGHT)
                .fold(f32::NEG_INFINITY, f32::max),
        );
        let viewport = canvas.shrink(12.0);
        let start = canvas.min + self.pan + min.to_vec2() * self.zoom;
        let size = ((max - min) * self.zoom).min(viewport.size());
        let rect = Rect::from_min_size(start, size);
        if rect.left() < viewport.left() {
            self.pan.x += viewport.left() - rect.left();
        } else if rect.right() > viewport.right() {
            self.pan.x += viewport.right() - rect.right();
        }
        if rect.top() < viewport.top() {
            self.pan.y += viewport.top() - rect.top();
        } else if rect.bottom() > viewport.bottom() {
            self.pan.y += viewport.bottom() - rect.bottom();
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
        if edit.surface != ParameterEditSurface::Window {
            return;
        }
        let mut outcome = ParameterEditOutcome::Keep;
        egui::Window::new(format!("{} / {}", edit.id, edit.parameter))
            .id(egui::Id::new("rack-parameter-editor"))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                outcome = parameter_editor_contents(ui, edit);
            });
        match outcome {
            ParameterEditOutcome::Keep => {}
            ParameterEditOutcome::Apply(raw) => {
                let edit = self.editing.take().unwrap();
                *action = Some(parameter_edit_action(&edit, raw));
            }
            ParameterEditOutcome::Cancel => self.editing = None,
        }
    }
}

fn bounded_pan(pan: Vec2) -> Vec2 {
    vec2(
        pan.x.clamp(-MAX_PAN, MAX_PAN),
        pan.y.clamp(-MAX_PAN, MAX_PAN),
    )
}

/// Screen-space camera movement; rack translation is its inverse.
fn keyboard_pan_delta(input: &egui::InputState) -> Vec2 {
    let x = i32::from(input.key_down(egui::Key::ArrowLeft))
        - i32::from(input.key_down(egui::Key::ArrowRight));
    let y = i32::from(input.key_down(egui::Key::ArrowUp))
        - i32::from(input.key_down(egui::Key::ArrowDown));
    let command = input.modifiers.command || input.modifiers.ctrl;
    let speed = match (command, input.modifiers.shift) {
        (false, false) => 1920.0,
        (true, false) => 480.0,
        (false, true) => 7680.0,
        (true, true) => 120.0,
    };
    vec2(x as f32, y as f32) * speed * input.stable_dt.min(0.1)
}

fn parameter_target_matches(edit: &ParameterEdit, patch: &Patch, catalog: &Value) -> bool {
    patch
        .nodes
        .iter()
        .any(|node| node.id == edit.id && node.kind == edit.kind)
        && catalog["modules"].as_array().is_some_and(|modules| {
            modules.iter().any(|module| {
                module["kind"].as_str() == Some(edit.kind.as_str())
                    && module["params"].get(edit.parameter.as_str()).is_some()
            })
        })
}

fn parameter_edit_raw_value(edit: &ParameterEdit, parsed: f32) -> f32 {
    if let Some(power) = edit.gain_power {
        if parsed == f32::NEG_INFINITY {
            0.0
        } else {
            10.0_f32.powf(parsed / (20.0 * power)).min(edit.raw_max)
        }
    } else if let Some(min) = edit.slew_min {
        if parsed == 0.0 {
            min
        } else {
            (parsed / 1000.0).log2().max(min)
        }
    } else {
        parsed / edit.display_scale
    }
}

fn parameter_edit_action(edit: &ParameterEdit, value: f32) -> RackAction {
    RackAction::Command(
        "set_parameter",
        json!({"id":edit.id,"parameter":edit.parameter,"value":value,"expected_revision":edit.revision}),
    )
}

fn parameter_editor_contents(ui: &mut egui::Ui, edit: &mut ParameterEdit) -> ParameterEditOutcome {
    ui.label(format!("Range: {} to {}", edit.min, edit.max));
    let mut field = egui::TextEdit::singleline(&mut edit.text)
        .desired_width(244.0)
        .char_limit(512)
        .show(ui);
    let input = &field.response;
    if edit.focus {
        input.request_focus();
        field
            .state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(edit.text.chars().count()),
            )));
        field.state.store(ui.ctx(), input.id);
        edit.focus = false;
    }
    let result = parsed_parameter_result(edit);
    let parsed = result.as_ref().ok().copied();
    if let Some(value) = parsed {
        ui.label(format!("Result: {value} {}", edit.display_unit).trim_end());
    }
    ui.weak("Numbers, expressions, or notes · e.g. 440*2, A4, C4v")
        .on_hover_text("Notes yield Hz; notes ending in v yield volts (C4v = 0).\nThe result uses this control's displayed units.");
    if edit.gain_power.is_some() {
        ui.label("Enter dB. -inf sets the gain to zero.");
    }
    if edit.slew_min.is_some() {
        ui.label("Enter milliseconds per volt. 0 = instant (slew CV ignored).");
    }
    if edit.display_scale == 0.5 {
        ui.label("Enter a clock multiplier (1 = same rate as clock).\nFree-runs until two rising edges arrive.");
    }
    if edit.parameter == "frequency" && edit.display_scale != 0.5 {
        ui.label("Enter frequency in Hz.");
    }
    if edit.parameter == "waveform" {
        ui.label("0 = Saw, 1 = Pulse");
    }
    if edit.display_scale == 100.0
        && matches!(
            edit.parameter.as_str(),
            "pulse_width" | "fm_amount" | "pwm_amount"
        )
    {
        ui.label("Enter a percentage.");
    } else if matches!(edit.parameter.as_str(), "pulse_width" | "duty" | "sustain") {
        ui.label("Enter a ratio from 0 to 1.");
    }
    if matches!(edit.parameter.as_str(), "attack" | "decay" | "release") {
        ui.label("Enter time in seconds.");
    }
    if edit.stale {
        ui.colored_label(
            Color32::LIGHT_RED,
            "Patch changed. Reopen this control to edit its current value.",
        );
    }
    let enter = (input.has_focus() || input.lost_focus())
        && ui.input(|i| i.focused && i.key_pressed(egui::Key::Enter));
    let mut apply = false;
    let mut cancel = false;
    ui.horizontal(|ui| {
        apply = ui
            .add_enabled(parsed.is_some() && !edit.stale, egui::Button::new("Apply"))
            .clicked();
        cancel = ui.button("Cancel").clicked();
    });
    if let Err(error) = result {
        ui.colored_label(Color32::LIGHT_RED, error);
    }
    if cancel || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        ParameterEditOutcome::Cancel
    } else if (apply || enter) && !edit.stale {
        parsed.map_or(ParameterEditOutcome::Keep, |value| {
            ParameterEditOutcome::Apply(parameter_edit_raw_value(edit, value))
        })
    } else {
        ParameterEditOutcome::Keep
    }
}

fn remove_selection_action(ids: &[String], revision: u64) -> RackAction {
    if ids.len() == 1 {
        RackAction::Command(
            "remove_module",
            json!({"id":ids[0],"expected_revision":revision}),
        )
    } else {
        RackAction::Command(
            "apply_edits",
            json!({"edits":ids.iter().map(|id|json!({"tool":"remove_module","arguments":{"id":id}})).collect::<Vec<_>>(),"expected_revision":revision}),
        )
    }
}

#[derive(Clone, Copy)]
enum ModuleStateAction {
    Initialize,
    Randomize,
}

fn randomize_seed() -> u32 {
    static NEXT: OnceLock<AtomicU32> = OnceLock::new();
    NEXT.get_or_init(|| {
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        AtomicU32::new(time.as_nanos() as u32)
    })
    .fetch_add(0x9e37_79b9, Ordering::Relaxed)
}

fn module_state_action(kind: ModuleStateAction, ids: &[String], revision: u64) -> RackAction {
    match kind {
        ModuleStateAction::Initialize => RackAction::Command(
            "initialize_modules",
            json!({"ids":ids,"expected_revision":revision}),
        ),
        ModuleStateAction::Randomize => RackAction::Command(
            "randomize_modules",
            json!({"ids":ids,"seed":randomize_seed(),"expected_revision":revision}),
        ),
    }
}

fn preset_menu(
    ui: &mut egui::Ui,
    node: &Node,
    ids: &[String],
    revision: u64,
    action: &mut Option<RackAction>,
) {
    if ids.len() != 1 {
        ui.add_enabled(false, egui::Button::new("Presets"))
            .on_disabled_hover_text("Presets work with one module at a time. Select only this module to save or browse presets.");
        return;
    }
    ui.menu_button("Presets", |ui| {
        for (mode, label, help) in [
            (
                PresetMode::Save,
                "Save preset…",
                "Capture this module's current controls, then save a new .opreset file.",
            ),
            (
                PresetMode::Browse,
                "Browse presets…",
                "Find and preview presets compatible with this module.",
            ),
        ] {
            if ui.button(label).on_hover_text(help).clicked() {
                *action = preset_action(node, ids, revision, mode);
                ui.close();
            }
        }
    });
}

fn preset_action(
    node: &Node,
    ids: &[String],
    revision: u64,
    mode: PresetMode,
) -> Option<RackAction> {
    (ids.len() == 1).then(|| RackAction::Preset {
        id: node.id.clone(),
        kind: node.kind.clone(),
        revision,
        mode,
    })
}

fn selection_menu(
    ui: &mut egui::Ui,
    ids: &[String],
    revision: u64,
    selected_context: bool,
    patch: &Patch,
    action: &mut Option<RackAction>,
) {
    ui.separator();
    if ids.len() > 1 {
        ui.strong(format!("{} selected modules", ids.len()));
    }
    for (kind, label, shortcut, tooltip) in [
        (
            ModuleStateAction::Initialize,
            "Initialize",
            "Cmd/Ctrl+I",
            "Restore default controls and reset supported module-specific state (such as sequencer position and LFO timing). Cables, bypass and panel positions stay as they are.",
        ),
        (
            ModuleStateAction::Randomize,
            "Randomize",
            "Cmd/Ctrl+R",
            "Randomize eligible controls; undo restores their previous values. Runtime sequencer and switch positions stay as they are, along with cables, bypass and panel positions.",
        ),
    ] {
        if ui
            .add(egui::Button::new(label).shortcut_text(shortcut))
            .on_hover_text(tooltip)
            .clicked()
        {
            *action = Some(module_state_action(kind, ids, revision));
            ui.close();
        }
    }
    ui.separator();
    let context_mode = if selected_context {
        DuplicateCableMode::Inputs
    } else {
        DuplicateCableMode::All
    };
    for (mode, label, shortcut) in [
        (DuplicateCableMode::None, "Duplicate", "Cmd/Ctrl+D"),
        (
            context_mode,
            if selected_context {
                "Duplicate with inputs"
            } else {
                "Duplicate with cables"
            },
            "Cmd/Ctrl+Shift+D",
        ),
    ] {
        let supported = mode != DuplicateCableMode::All || duplicate_all_supported(patch, ids);
        if ui.add_enabled(supported, egui::Button::new(label).shortcut_text(shortcut))
            .on_disabled_hover_text("Copying outgoing cables requires engine v2. Duplicate without cables remains available.")
            .clicked()
        {
            *action = Some(RackAction::Duplicate {
                ids: ids.to_vec(),
                cable_mode: mode,
                revision,
                select_copies: selected_context,
            });
            ui.close();
        }
    }
    if ui
        .button(if ids.len() > 1 {
            "Remove selected modules"
        } else {
            "Remove module"
        })
        .clicked()
    {
        *action = Some(remove_selection_action(ids, revision));
        ui.close();
    }
}

pub(crate) fn duplicate_all_supported(patch: &Patch, ids: &[String]) -> bool {
    patch.version >= 2
        || (ids.len() == 1
            && !patch
                .cables
                .iter()
                .any(|cable| cable.from == ids[0] && cable.to != ids[0]))
}

fn cable_endpoint(end: &CableEnd) -> openpatch_core::CableEndpoint {
    match end {
        CableEnd::Input { .. } => openpatch_core::CableEndpoint::Input,
        CableEnd::Output { .. } => openpatch_core::CableEndpoint::Output,
    }
}

fn ordered_endpoint_cables<'a>(patch: &'a Patch, end: &CableEnd) -> Vec<&'a Cable> {
    let (node, port) = match end {
        CableEnd::Input { node, port } | CableEnd::Output { node, port } => (node, port),
    };
    openpatch_core::cable_endpoint_order(&patch.cables, cable_endpoint(end), node, port)
        .into_iter()
        .map(|index| &patch.cables[index])
        .collect()
}

fn pending_promotion(patch: &mut Patch, pending: &PendingCable) -> bool {
    let selected: Vec<_> = pending
        .original
        .iter()
        .chain(&pending.companions)
        .filter_map(|old| patch.cables.iter().position(|cable| same_cable(old, cable)))
        .collect();
    openpatch_core::promote_cable_endpoints(
        &mut patch.cables,
        cable_endpoint(&pending.origin),
        &selected,
    )
    .unwrap_or(false)
}

fn cable_promotion_action(patch: &Patch, pending: &PendingCable) -> Option<RackAction> {
    let mut candidate = patch.clone();
    if !pending_promotion(&mut candidate, pending) {
        return None;
    }
    let ids: Vec<_> = pending
        .original
        .iter()
        .chain(&pending.companions)
        .filter_map(|cable| cable.id.clone())
        .collect();
    Some(RackAction::Command(
        "promote_cables",
        json!({"cable_ids":ids,
        "end":if matches!(pending.origin, CableEnd::Input { .. }) { "input" } else { "output" },
        "expected_revision":pending.revision}),
    ))
}

fn same_route(a: &Cable, b: &Cable) -> bool {
    a.from == b.from && a.output == b.output && a.to == b.to && a.input == b.input
}
fn same_cable(a: &Cable, b: &Cable) -> bool {
    match (&a.id, &b.id) {
        (Some(a_id), Some(b_id)) => a_id == b_id && same_route(a, b),
        (None, None) => same_route(a, b),
        _ => false,
    }
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

fn paint_signal_light(painter: &egui::Painter, center: Pos2, brightness: [f32; 2], zoom: f32) {
    for (index, base) in [
        Color32::from_rgb(242, 199, 59),
        Color32::from_rgb(62, 155, 238),
    ]
    .into_iter()
    .enumerate()
    {
        let strength = brightness[index].clamp(0.0, 1.0);
        let point = center + vec2(if index == 0 { -3.5 } else { 3.5 }, 0.0) * zoom;
        painter.circle_filled(point, 3.0 * zoom, Color32::from_gray(78));
        painter.circle_filled(
            point,
            2.2 * zoom,
            base.gamma_multiply(0.12 + strength * 0.88),
        );
    }
}

fn is_signal_utility(kind: &str) -> bool {
    matches!(kind, "logic" | "compare" | "process" | "rescale")
}

fn parsed_parameter_result(edit: &ParameterEdit) -> Result<f32, String> {
    let mut text = edit.text.trim();
    if edit.gain_power.is_some()
        && text.len() > 2
        && text
            .get(text.len().saturating_sub(2)..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case("db"))
    {
        text = text[..text.len() - 2].trim_end();
    }
    let evaluated = crate::expression::evaluate(text).map_err(|error| error.to_string())?;
    let gain_zero = edit.gain_power.is_some() && evaluated == f64::NEG_INFINITY;
    let value = evaluated as f32;
    if !gain_zero && (!evaluated.is_finite() || !value.is_finite()) {
        return Err("Enter a finite result within this control's range.".into());
    }
    let allowance = if edit.gain_power.is_some() {
        0.000_01
    } else {
        0.0
    };
    if value < edit.min || value > edit.max + allowance {
        return Err(format!(
            "Result must be between {} and {}.",
            edit.min, edit.max
        ));
    }
    if let Some(step) = edit.step {
        let steps = (value - edit.min) / step;
        if (steps - steps.round()).abs() >= 0.000_01 {
            return Err(format!(
                "Result must use steps of {step} from {}.",
                edit.min
            ));
        }
    }
    Ok(value)
}

fn gain_db(value: f32, power: f32) -> f32 {
    if value <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * power * value.log10()
    }
}
fn gain_db_display(value: f32, power: f32) -> String {
    if value <= 0.0 {
        "-inf dB".into()
    } else {
        format!("{:.2} dB", gain_db(value, power))
    }
}

fn slew_display(value: f32, min: f32) -> f32 {
    if value <= min {
        0.0
    } else {
        value.exp2() * 1000.0
    }
}

fn is_voltage_utility(kind: &str) -> bool {
    matches!(kind, "attenuverter8" | "mutes10" | "cv_mix")
}

fn utility_row_y(kind: &str, row: usize) -> f32 {
    match kind {
        "mutes10" => 96.0 + row as f32 * 29.0,
        "cv_mix" => 132.0 + row as f32 * 70.0,
        _ => 107.0 + row as f32 * 35.0,
    }
}

/// The connected jack supplying this row, including a patched zero-volt source.
fn utility_source_row(patch: &Patch, node: &Node, row: usize) -> Option<usize> {
    let connected = |row: usize| {
        patch
            .cables
            .iter()
            .any(|c| c.to == node.id && c.input == format!("in{}", row + 1))
    };
    if node.kind == "attenuverter8" {
        (0..=row).rev().find(|&row| connected(row))
    } else {
        connected(row).then_some(row)
    }
}

fn utility_menu(ui: &mut egui::Ui, node: &Node, action: &mut Option<RackAction>) {
    if ui
        .selectable_label(node.bypassed, "Bypass module")
        .clicked()
    {
        *action = Some(RackAction::Command(
            "set_bypass",
            json!({"id":node.id,"bypassed":!node.bypassed}),
        ));
        ui.close();
    }
    if node.kind == "vca_mix" {
        ui.separator();
        for (parameter, label) in [
            ("channel_exp", "Exponential channel CV"),
            ("mix_exp", "Exponential mix CV"),
        ] {
            let enabled = node.params.get(parameter).copied().unwrap_or(0.0) >= 0.5;
            if ui.selectable_label(enabled, label).clicked() {
                *action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":parameter,"value":if enabled{0.0}else{1.0}}),
                ));
                ui.close();
            }
        }
    }
    if node.kind == "fade" {
        ui.separator();
        ui.label("Pan law");
        let law = node.params.get("law").copied().unwrap_or(0.0);
        for (value, label) in [(0, "Linear law"), (1, "Square-root law")] {
            if ui.selectable_label(law == value as f32, label).clicked() {
                *action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":"law","value":value}),
                ));
                ui.close();
            }
        }
    }
    if node.kind == "rescale" {
        ui.separator();
        ui.label("Gain multiplier");
        let current = node.params.get("multiplier").copied().unwrap_or(1.0);
        for multiplier in [1, 10, 100, 1000] {
            if ui
                .selectable_label(current == multiplier as f32, format!("{multiplier}x"))
                .clicked()
            {
                *action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":"multiplier","value":multiplier}),
                ));
                ui.close();
            }
        }
        ui.separator();
        for (parameter, label) in [
            ("reflect_min", "Reflect minimum"),
            ("reflect_max", "Reflect maximum"),
        ] {
            let enabled = node.params.get(parameter).copied().unwrap_or(0.0) >= 0.5;
            if ui.selectable_label(enabled, label).clicked() {
                *action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":parameter,"value":if enabled{0.0}else{1.0}}),
                ));
                ui.close();
            }
        }
    }
    if node.kind == "mutes10" && ui.button("Invert all mutes").clicked() {
        *action = Some(RackAction::Command("invert_mutes", json!({"id":node.id})));
        ui.close();
    }
}

fn point_from_port(origin: Pos2, zoom: f32, point: Pos2) -> Pos2 {
    origin + point.to_vec2() * zoom
}

fn port_position(kind: &str, input: bool, index: usize, count: usize, width: f32) -> Pos2 {
    match kind {
        "stereo_delay" | "stereo_reverb" | "sidechain_compressor" | "drum_voice" => {
            return pos2(
                width * (index + 1) as f32 / (count + 1) as f32,
                if input { 342.0 } else { 382.0 },
            );
        }
        "score_sequencer" if input => return pos2(60.0 + index as f32 * 80.0, 373.0),
        "score_sequencer" if index < 12 => {
            return pos2(
                310.0 + (index % 4) as f32 * 78.0,
                220.0 + (index / 4) as f32 * 62.0,
            )
        }
        "score_sequencer" => return pos2(60.0 + (index - 12) as f32 * 80.0, 292.0),
        "gates" if input => return pos2(45.0 + index as f32 * 75.0, 105.0),
        "gates" => {
            return pos2(
                45.0 + (index % 3) as f32 * 75.0,
                if index < 3 { 282.0 } else { 350.0 },
            )
        }
        "push" => {
            return pos2(
                if index == 0 { 49.0 } else { 131.0 },
                if input { 117.0 } else { 356.0 },
            )
        }
        "pulses" if !input => {
            return pos2(
                if index < 10 { 155.0 } else { 205.0 },
                100.0 + (index % 10) as f32 * 29.0,
            )
        }
        "vca_mix" if input => {
            return if index < 4 {
                pos2(37.5 + index as f32 * 75.0, 226.0)
            } else if index < 8 {
                pos2(37.5 + (index - 4) as f32 * 75.0, 294.0)
            } else {
                pos2(345.0, 294.0)
            }
        }
        "vca_mix" => {
            return pos2(
                if index < 4 {
                    37.5 + index as f32 * 75.0
                } else {
                    345.0
                },
                360.0,
            )
        }
        "fade" if input => return pos2(45.0 + index as f32 * 75.0, 330.0),
        "fade" => return pos2(75.0 + index as f32 * 90.0, 374.0),
        "logic" if input => return pos2(width * (0.25 + index as f32 * 0.5), 115.0),
        "logic" => {
            return pos2(
                width * ((index % 4) as f32 + 0.5) / 4.0,
                265.0 + (index / 4) as f32 * 88.0,
            )
        }
        "compare" if input => return pos2(width * (0.25 + index as f32 * 0.5), 214.0),
        "compare" => {
            return pos2(
                width * ((index % 4) as f32 + 0.5) / 4.0,
                285.0 + (index / 4) as f32 * 76.0,
            )
        }
        "process" if input => return pos2(width * (index as f32 + 1.0) / 4.0, 226.0),
        "process" => {
            return pos2(
                width * ((index % 3) as f32 + 1.0) / 4.0,
                286.0 + (index / 3) as f32 * 74.0,
            )
        }
        "rescale" => return pos2(width * if input { 0.25 } else { 0.75 }, 360.0),
        _ => {}
    }
    if is_voltage_utility(kind) {
        return if kind == "cv_mix" && !input {
            pos2(width * 0.5, 366.0)
        } else {
            pos2(
                if input { 29.0 } else { width - 29.0 },
                utility_row_y(kind, index),
            )
        };
    }
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
    let mut cable = match (&pending.origin, target, pending.operation) {
        (
            CableEnd::Output {
                node: from,
                port: output,
            },
            CableEnd::Input { node, port },
            CableOperation::Create,
        )
        | (
            CableEnd::Input { node, port },
            CableEnd::Output {
                node: from,
                port: output,
            },
            CableOperation::Create,
        ) => Cable {
            input_order: None,
            output_order: None,
            color: pending.color,
            id: None,
            from: from.clone(),
            output: output.clone(),
            to: node.clone(),
            input: port.clone(),
        },
        (
            CableEnd::Output { .. },
            CableEnd::Output { node, port },
            CableOperation::Move | CableOperation::Clone,
        ) => {
            let mut c = pending.original.clone()?;
            c.from = node.clone();
            c.output = port.clone();
            c
        }
        (
            CableEnd::Input { .. },
            CableEnd::Input { node, port },
            CableOperation::Move | CableOperation::Clone,
        ) => {
            let mut c = pending.original.clone()?;
            c.to = node.clone();
            c.input = port.clone();
            c
        }
        _ => return None,
    };
    if pending.operation == CableOperation::Clone {
        let color = cable_instance_color(pending.original.as_ref()?);
        cable.color = Some([color.r(), color.g(), color.b()]);
        cable.id = None;
        cable.input_order = None;
        cable.output_order = None;
    }
    Some((cable, pending.moved_original()))
}

fn delete_moved_cable(pending: &PendingCable) -> Option<RackAction> {
    if !pending.companions.is_empty() {
        let ids = pending
            .original
            .iter()
            .chain(&pending.companions)
            .map(|c| c.id.clone())
            .collect::<Option<Vec<_>>>()?;
        return Some(RackAction::Command(
            "disconnect_cables",
            json!({"cable_ids":ids,"expected_revision":pending.revision}),
        ));
    }
    let old = pending.moved_original()?;
    let mut args = old.id.as_ref().map_or_else(
        || json!({"to":old.to,"input":old.input}),
        |id| json!({"cable_id":id}),
    );
    args["expected_revision"] = json!(pending.revision);
    Some(RackAction::Command("disconnect", args))
}

// Invalid physical drops discard an incomplete cable in Rack. Keep this distinct
// from format/capacity/publication errors, which cancel the proposed operation.
fn incomplete_cable_drop(
    patch: &Patch,
    pending: &PendingCable,
    target: Option<&CableEnd>,
    shift: bool,
) -> bool {
    let Some((cable, old)) = target.and_then(|target| cable_plan(pending, target)) else {
        return true;
    };
    if old.is_some_and(|old| same_route(old, &cable)) {
        return false;
    }
    !pending.allow_duplicate(shift)
        && patch.cables.iter().any(|existing| {
            !old.is_some_and(|old| same_cable(old, existing)) && same_route(existing, &cable)
        })
}

fn cable_release_action(
    patch: &Patch,
    pending: &PendingCable,
    target: Option<&CableEnd>,
    catalog: &Value,
    shift: bool,
) -> Option<RackAction> {
    if !pending.companions.is_empty() {
        return match target.filter(|target| cable_plan(pending, target).is_some()) {
            Some(target) => cable_group_action(patch, pending, target, catalog, shift),
            None => delete_moved_cable(pending),
        };
    }
    if incomplete_cable_drop(patch, pending, target, shift) {
        return delete_moved_cable(pending);
    }
    cable_action_mode(patch, pending, target?, catalog, shift)
}

#[cfg(test)]
fn connection_allowed(patch: &Patch, cable: &Cable, old: Option<&Cable>, catalog: &Value) -> bool {
    connection_allowed_mode(patch, cable, old, catalog, true)
}
fn connection_allowed_mode(
    patch: &Patch,
    cable: &Cable,
    old: Option<&Cable>,
    catalog: &Value,
    retain_destination: bool,
) -> bool {
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
        (retain_destination || !(existing.to == cable.to && existing.input == cable.input))
            && !old.is_some_and(|old| same_cable(old, existing))
    };
    if patch.cables.iter().filter(retained).count() >= openpatch_core::MAX_CABLES {
        return false;
    }
    if patch.version == 2 {
        return true;
    }
    if cable.from == cable.to
        || patch
            .cables
            .iter()
            .filter(retained)
            .any(|existing| existing.to == cable.to && existing.input == cable.input)
    {
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

#[cfg(test)]
fn cable_action(
    patch: &Patch,
    pending: &PendingCable,
    target: &CableEnd,
    catalog: &Value,
) -> Option<RackAction> {
    cable_action_mode(patch, pending, target, catalog, false)
}
fn cable_action_mode(
    patch: &Patch,
    pending: &PendingCable,
    target: &CableEnd,
    catalog: &Value,
    shift_stack: bool,
) -> Option<RackAction> {
    if pending.operation == CableOperation::Move
        && (!pending.companions.is_empty()
            || pending
                .original
                .as_ref()
                .is_some_and(|cable| cable.id.is_some()))
    {
        return cable_group_action(patch, pending, target, catalog, shift_stack);
    }
    let (cable, old) = cable_plan(pending, target)?;
    let duplicate = pending.allow_duplicate(shift_stack);
    if old.is_some_and(|old| same_route(old, &cable)) {
        return None;
    }
    if (duplicate && patch.version < 2)
        || (!duplicate
            && patch.cables.iter().any(|existing| {
                !old.is_some_and(|old| same_cable(old, existing)) && same_route(existing, &cable)
            }))
        || !connection_allowed_mode(patch, &cable, old, catalog, true)
    {
        return None;
    }
    let mut connect =
        json!({"from":cable.from,"output":cable.output,"to":cable.to,"input":cable.input});
    if patch.version >= 2 {
        connect["mode"] = json!("stack");
    }
    if let Some(id) = &cable.id {
        connect["cable_id"] = json!(id);
    }
    if let Some(color) = cable.color {
        connect["color"] = json!(color);
    }
    let Some(old) = old else {
        connect["expected_revision"] = json!(pending.revision);
        return Some(RackAction::Command("connect", connect));
    };
    let selector = old.id.as_ref().map_or_else(
        || json!({"to":old.to,"input":old.input}),
        |id| json!({"cable_id":id}),
    );
    Some(RackAction::Command(
        "apply_edits",
        json!({"edits":[
        {"tool":"disconnect","arguments":selector},
        {"tool":"connect","arguments":connect}
    ],"expected_revision":pending.revision}),
    ))
}

/// Validate bundle geometry on the UI thread; the command layer revalidates the
/// captured revision and final graph atomically before publishing any edit.
fn cable_group_action(
    patch: &Patch,
    pending: &PendingCable,
    target: &CableEnd,
    catalog: &Value,
    shift: bool,
) -> Option<RackAction> {
    if pending.operation != CableOperation::Move {
        return None;
    }
    cable_plan(pending, target)?;
    if pending.origin == *target {
        return cable_promotion_action(patch, pending);
    }
    let plug_order = pending
        .original
        .iter()
        .chain(&pending.companions)
        .map(|cable| cable.id.clone())
        .collect::<Option<Vec<_>>>()?;
    // Backend collision winners always follow native array order, independently
    // of the selected bundle's displayed bottom-to-top plug order.
    let originals: Vec<_> = patch
        .cables
        .iter()
        .filter(|cable| {
            pending
                .original
                .iter()
                .chain(&pending.companions)
                .any(|old| same_cable(old, cable))
        })
        .collect();
    if originals.len() != plug_order.len() {
        return None;
    }
    let ids = originals
        .iter()
        .map(|cable| cable.id.clone())
        .collect::<Option<Vec<_>>>()?;
    let allow_duplicate = pending.allow_duplicate(shift);
    if allow_duplicate && patch.version < 2 {
        return None;
    }
    let mut candidate = patch.clone();
    candidate
        .cables
        .retain(|c| !originals.iter().any(|old| same_cable(old, c)));
    for original in originals {
        if !patch.cables.iter().any(|c| same_cable(c, original)) {
            return None;
        }
        let mut cable = original.clone();
        match target {
            CableEnd::Input { node, port } => {
                cable.to = node.clone();
                cable.input = port.clone();
            }
            CableEnd::Output { node, port } => {
                cable.from = node.clone();
                cable.output = port.clone();
            }
        }
        if !allow_duplicate && candidate.cables.iter().any(|c| same_route(c, &cable)) {
            continue;
        }
        if !connection_allowed_mode(&candidate, &cable, None, catalog, true) {
            return None;
        }
        candidate.cables.push(cable);
    }
    let (end, node, port) = match target {
        CableEnd::Input { node, port } => ("input", node, port),
        CableEnd::Output { node, port } => ("output", node, port),
    };
    if !port_exists(patch, catalog, node, port, end == "output") {
        return None;
    }
    Some(RackAction::Command(
        "move_cables",
        json!({"cable_ids":ids,"end":end,"node":node,"port":port,
        "duplicate_mode":if allow_duplicate {"stack"} else {"discard"},
        "promote":true,"plug_order":plug_order,"expected_revision":pending.revision}),
    ))
}

fn stack_badge_position(location: Pos2, kind: &str, zoom: f32) -> Pos2 {
    // Labels sit above ports and generic output labels often sit just below
    // inputs. Keep counts beside sockets; switches reserve their left for LEDs.
    location
        + vec2(
            if is_sequential_switch(kind) {
                18.0
            } else {
                -18.0
            },
            0.0,
        ) * zoom
}

fn cable_instance_color(cable: &Cable) -> Color32 {
    if let Some([r, g, b]) = cable.color {
        return Color32::from_rgb(r, g, b);
    }
    cable.id.as_deref().map_or_else(
        || cable_color(&cable.from, &cable.output),
        |id| cable_color(id, "cable"),
    )
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
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
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
            crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
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
            crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
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
            crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
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
        Color32::from_rgb(184, 181, 172)
    } else if selected {
        Color32::from_rgb(222, 177, 96)
    } else if response.hovered() {
        Color32::from_rgb(246, 241, 227)
    } else {
        Color32::from_rgb(228, 224, 211)
    };
    painter.rect_filled(rect, 2.0, fill);
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(0.7 * zoom, Color32::from_rgb(139, 134, 122)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        if enabled { INK } else { MUTED },
    );
    let clicked = enabled && response.clicked();
    response.on_hover_text(if label == "Start running on load" { "Sets the saved initial run state and applies it now. The Run performance button changes only runtime state." } else { label });
    clicked
}

pub(crate) fn panel_width(kind: &str) -> f32 {
    match kind {
        "score_sequencer" => HP * 40.0,
        "stereo_delay" | "sidechain_compressor" => HP * 28.0,
        "stereo_reverb" | "drum_voice" => HP * 24.0,
        "svf_filter" => HP * 18.0,
        "vca_mix" => HP * 26.0,
        "fade" => HP * 16.0,
        "logic" | "compare" | "process" | "rescale" => HP * 16.0,
        "attenuverter8" | "mutes10" | "cv_mix" | "pulses" | "gates" => HP * 16.0,
        "seq3" => HP * 52.0,
        "switch_1to4" | "switch_4to1" => HP * 12.0,
        "poly_constant" => HP * 24.0,
        "midi_cv" => HP * 24.0,
        "audio_stereo" | "octave" | "push" => HP * 12.0,
        "mid_side" | "mult" | "multi_lfo" | "multi_vco" => HP * 20.0,
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
pub(crate) fn title(kind: &str) -> &str {
    match kind {
        "score_sequencer" => "SCORE",
        "stereo_delay" => "STEREO DELAY",
        "stereo_reverb" => "SPACE",
        "sidechain_compressor" => "DUCK",
        "drum_voice" => "DRUM",
        "svf_filter" => "STATE VARIABLE",
        "vca_mix" => "VCA MIX",
        "fade" => "FADE",
        "logic" => "LOGIC",
        "compare" => "COMPARE",
        "process" => "PROCESS",
        "rescale" => "RESCALE",
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
        "attenuverter8" => "8VERT",
        "mutes10" => "MUTES 10",
        "cv_mix" => "CV MIX",
        "pulses" => "PULSES",
        "push" => "PUSH",
        "gates" => "GATES",
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
        "multi_vco" => "MULTI VCO",
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
pub(crate) fn parameter_label(name: &str) -> &str {
    match name {
        "tempo" => "TEMPO",
        "dc_block" => "DC FILTER",
        "swing" => "SWING",
        "running" => "RUN",
        "time_left" => "LEFT TIME",
        "time_right" => "RIGHT TIME",
        "feedback" => "FEEDBACK",
        "crossfeed" => "CROSSFEED",
        "dry" => "DRY",
        "wet" => "WET",
        "low_cut" => "LOW CUT",
        "high_cut" => "HIGH CUT",
        "mod_rate" => "MOD RATE",
        "mod_depth" => "MOD DEPTH",
        "predelay" => "PREDELAY",
        "size" => "SIZE",
        "damping" => "DAMPING",
        "threshold_db" => "THRESHOLD",
        "ratio" => "RATIO",
        "attack_ms" => "ATTACK",
        "release_ms" => "RELEASE",
        "knee_db" => "KNEE",
        "makeup_db" => "MAKEUP",
        "detector_ms" => "DETECTOR",
        "mode" => "VOICE",
        "pitch" => "PITCH",
        "tone" => "TONE",
        "cutoff_cv_amount" => "CV DEPTH",

        "position" => "POSITION",
        "cv_amount" => "CV DEPTH",
        "b_offset" => "B OFFSET",
        "slew" => "SLEW",
        "min" => "MINIMUM",
        "max" => "MAXIMUM",
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
        "complement" => "COMP",
        "mix_cv" => "MIX CV",
        "not_a" => "NOT A",
        "not_b" => "NOT B",
        "clip_gate" => "CLIP G",
        "lim_gate" => "LIM G",
        "greater" => "A > B",
        "less" => "A < B",
        "sh1" => "S&H 1",
        "sh2" => "S&H 2",
        "track_hold" => "T&H",
        "hold_track" => "H&T",
        "slew_cv" => "SLEW CV",
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
        "score_sequencer" => &["tempo", "swing", "running"],
        "drum_voice" => &["mode", "pitch", "decay", "tone", "drive", "level"],
        "stereo_delay" => &[
            "time_left",
            "time_right",
            "feedback",
            "crossfeed",
            "low_cut",
            "high_cut",
            "mod_rate",
            "mod_depth",
            "dry",
            "wet",
        ],
        "stereo_reverb" => &["predelay", "size", "decay", "damping", "dry", "wet"],
        "sidechain_compressor" => &[
            "threshold_db",
            "ratio",
            "knee_db",
            "attack_ms",
            "release_ms",
            "detector_ms",
            "makeup_db",
        ],
        "vca_mix" => &[
            "level1",
            "level2",
            "level3",
            "level4",
            "mix_level",
            "channel_exp",
            "mix_exp",
        ],
        "fade" => &["position", "cv_amount", "law"],
        "rescale" => &[
            "gain",
            "offset",
            "min",
            "max",
            "multiplier",
            "reflect_min",
            "reflect_max",
        ],
        "compare" => &["b_offset"],
        "process" => &["slew"],
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
        "multi_vco" => &[
            "frequency",
            "pulse_width",
            "fm_amount",
            "pwm_amount",
            "linear_fm",
            "soft_sync",
        ],
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
// A large tuning control above a matched pair of secondary controls. Keep this
// geometry shared by rack rendering, previews, and the parameter hit regions.
fn voice_parameter_position(index: usize, width: f32) -> (Pos2, f32) {
    if index == 0 {
        (pos2(width * 0.5, 138.0), 38.0)
    } else {
        (
            pos2(width * if index == 1 { 0.25 } else { 0.75 }, 252.0),
            23.0,
        )
    }
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
pub(crate) fn parameter_value(name: &str, value: f32, spec: &Value) -> String {
    match name {
        "dc_block" | "running" => if value.round() == 0.0 { "Off" } else { "On" }.into(),
        "mode" => match value.round() as i32 {
            0 => "Kick",
            1 => "Clap",
            _ => "Hi-hat",
        }
        .into(),
        "tempo" => format!("{value:.1} BPM"),
        "swing" => format!("{value:.1}%"),
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
        "gain" => format_value(value, "x"),
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
fn logarithmic_parameter(kind: &str, name: &str) -> bool {
    (kind == "gates" && name == "length")
        || matches!(
            name,
            "frequency" | "cutoff" | "attack" | "decay" | "release"
        )
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
    } else if matches!(unit, "ms" | "dB") {
        format!("{value:.1} {unit}")
    } else if matches!(unit, "V" | "V RMS") {
        format!("{value:.2} {unit}")
    } else if unit == "x" {
        if value == 0.0 || value.abs() >= 0.01 {
            format!("{value:.2}x")
        } else if value.abs() >= 0.001 {
            format!("{value:.3}x")
        } else {
            format!("{value:.2e}x")
        }
    } else {
        format!("{value:.3}")
    }
}

/// Search complete group translations on the HP/row grid. Forbidden x intervals
/// account for each member separately, so empty spaces inside a group stay usable.
fn settle_group(requested: Pos2, members: &[(Pos2, f32)], others: &[(Pos2, f32)]) -> Option<Pos2> {
    if members.is_empty() {
        return None;
    }
    let max_relative_x = members.iter().map(|(p, _)| p.x).fold(0.0, f32::max);
    let max_relative_y = members.iter().map(|(p, _)| p.y).fold(0.0, f32::max);
    let max_x = ((100_000.0 - max_relative_x) / HP).floor() * HP;
    let max_row = ((100_000.0 - max_relative_y) / ROW).floor() as usize;
    if max_x < 0.0 || max_relative_y > 100_000.0 {
        return None;
    }
    let requested = pos2(
        (requested.x / HP).round().clamp(0.0, max_x / HP) * HP,
        (requested.y / ROW).round().max(0.0) * ROW,
    );
    let last_row = others
        .iter()
        .map(|(p, _)| (p.y / ROW).ceil() as usize + 1)
        .max()
        .unwrap_or(0)
        .max((requested.y / ROW) as usize + 1)
        .min(max_row);
    let mut best: Option<(f32, Pos2)> = None;
    for row in 0..=last_row {
        let y = row as f32 * ROW;
        let mut intervals = Vec::new();
        for (relative, width) in members {
            for (other, other_width) in others {
                if (y + relative.y - other.y).abs() < PANEL_HEIGHT {
                    intervals.push((
                        other.x - relative.x - width,
                        other.x + other_width - relative.x,
                    ));
                }
            }
        }
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f32, f32)> = Vec::new();
        for (left, right) in intervals {
            if let Some(last) = merged.last_mut().filter(|last| left < last.1) {
                last.1 = last.1.max(right);
            } else {
                merged.push((left, right));
            }
        }
        let mut candidates = vec![requested.x, 0.0, max_x];
        for (left, right) in &merged {
            candidates.push((left / HP).floor() * HP);
            candidates.push((right / HP).ceil() * HP);
        }
        for x in candidates.into_iter().filter(|x| *x >= 0.0 && *x <= max_x) {
            let index = merged.partition_point(|(_, right)| *right <= x);
            if merged
                .get(index)
                .is_some_and(|(left, right)| *left < x && x < *right)
            {
                continue;
            }
            let point = pos2(x, y);
            let distance = point.distance_sq(requested);
            if best.is_none_or(|(score, old)| {
                distance < score
                    || (distance == score
                        && (point.y - requested.y).abs() < (old.y - requested.y).abs())
            }) {
                best = Some((distance, point));
            }
        }
    }
    best.map(|(_, point)| point)
}

fn paint_background(painter: &egui::Painter, canvas: Rect, pan: Vec2, zoom: f32) {
    painter.rect_filled(canvas, 0, Color32::from_rgb(30, 31, 30));
    // A few broad charcoal striations give the bays depth without a bitmap texture.
    let line_spacing = (18.0 * zoom).max(8.0);
    let mut y = canvas.top();
    while y < canvas.bottom() {
        painter.line_segment(
            [pos2(canvas.left(), y), pos2(canvas.right(), y)],
            Stroke::new(1.0_f32, Color32::from_white_alpha(5)),
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
        painter.rect_filled(rail, 0, Color32::from_rgb(58, 59, 57));
        painter.rect_filled(
            Rect::from_min_max(rail.min, pos2(rail.right(), y - 3.0 * zoom)),
            0,
            Color32::from_black_alpha(68),
        );
        painter.line_segment(
            [rail.left_top(), rail.right_top()],
            Stroke::new(1.0_f32, Color32::from_rgb(105, 106, 101)),
        );
        painter.line_segment(
            [
                pos2(rail.left(), y + 3.0 * zoom),
                pos2(rail.right(), y + 3.0 * zoom),
            ],
            Stroke::new(1.6 * zoom, Color32::from_rgb(23, 24, 23)),
        );
        painter.line_segment(
            [rail.left_bottom(), rail.right_bottom()],
            Stroke::new(1.0_f32, Color32::from_rgb(114, 112, 104)),
        );
        let start = ((-pan.x) / (HP * zoom)).floor() as i32;
        let count = (canvas.width() / (HP * zoom)).ceil() as i32 + 2;
        for slot in start..start + count {
            let x = canvas.left() + pan.x + slot as f32 * HP * zoom;
            let hole = pos2(x + 7.5 * zoom, y);
            painter.circle_filled(hole, 2.8 * zoom, Color32::from_rgb(19, 20, 20));
            painter.line_segment(
                [
                    hole + vec2(-1.5, -2.1) * zoom,
                    hole + vec2(1.5, -2.1) * zoom,
                ],
                Stroke::new(0.7 * zoom, Color32::from_rgb(124, 124, 117)),
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
    crate::panel_skin::metal(painter, rect, zoom);
    if node.bypassed {
        painter.rect_stroke(
            rect.shrink(3.0),
            0,
            Stroke::new(1.8 * zoom, Color32::from_rgb(146, 102, 61)),
            egui::StrokeKind::Inside,
        );
    }
    if selected {
        painter.rect_stroke(
            rect.expand(1.0 * zoom),
            1.0 * zoom,
            Stroke::new(
                3.0 * zoom,
                Color32::from_rgba_unmultiplied(225, 159, 58, 45),
            ),
            egui::StrokeKind::Outside,
        );
        painter.rect_stroke(
            rect.shrink(1.0),
            0,
            Stroke::new(1.5 * zoom, Color32::from_rgb(201, 137, 54)),
            egui::StrokeKind::Inside,
        );
    }
    let point = |x, y| rect.min + vec2(x, y) * zoom;
    let width = rect.width() / zoom;
    for x in [8.0, width - 8.0] {
        for y in [9.0, PANEL_HEIGHT - 9.0] {
            let center = point(x, y);
            painter.circle_filled(
                center + vec2(0.5, 0.8) * zoom,
                4.5 * zoom,
                Color32::from_black_alpha(75),
            );
            painter.circle_filled(center, 3.8 * zoom, Color32::from_rgb(99, 97, 91));
            painter.circle_filled(
                center - vec2(0.4, 0.5) * zoom,
                2.9 * zoom,
                Color32::from_rgb(204, 200, 190),
            );
            painter.circle_stroke(
                center,
                3.7 * zoom,
                Stroke::new(0.8 * zoom, Color32::from_rgb(245, 241, 229)),
            );
            painter.line_segment(
                [
                    center - vec2(1.7, 1.7) * zoom,
                    center + vec2(1.7, 1.7) * zoom,
                ],
                Stroke::new(0.9 * zoom, Color32::from_rgb(68, 67, 63)),
            );
        }
    }
    let label = title(&node.kind);
    let mut font = crate::panel_skin::heading_font(
        painter.ctx(),
        if width < 140.0 { 18.0 } else { 21.0 } * zoom,
    );
    let measured = painter.layout_no_wrap(label.to_owned(), font.clone(), INK);
    let available = (width - 30.0) * zoom;
    font.size *= (available / measured.size().x.max(1.0)).min(1.0);
    let heading = painter.layout_no_wrap(label.to_owned(), font, INK);
    painter.galley(
        point(width * 0.5, 31.0) - heading.size() * 0.5,
        heading,
        INK,
    );
    let mut identity = egui::text::LayoutJob::simple(
        if node.bypassed {
            "BYPASSED"
        } else {
            crate::panel_skin::model_caption(&node.kind)
        }
        .to_owned(),
        crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
        MUTED,
        (width - 28.0) * zoom,
    );
    identity.wrap.max_rows = 1;
    let identity = painter.layout_job(identity);
    painter.galley(
        point(width * 0.5, 51.0) - identity.size() * 0.5,
        identity,
        MUTED,
    );
    painter.line_segment(
        [point(22.0, 65.0), point(width - 22.0, 65.0)],
        Stroke::new(0.65 * zoom, Color32::from_rgb(166, 164, 157)),
    );
    painter.line_segment(
        [point(22.0, 65.0), point(width - 22.0, 65.0)],
        Stroke::new(1.6 * zoom, crate::panel_skin::family_accent(&node.kind)),
    );
    if !matches!(node.kind.as_str(), "pulses" | "svf_filter" | "audio_stereo") {
        let studio_footer = matches!(
            node.kind.as_str(),
            "drum_voice" | "stereo_delay" | "stereo_reverb" | "sidechain_compressor"
        );
        painter.text(
            point(
                if studio_footer {
                    width - 24.0
                } else {
                    width * 0.5
                },
                PANEL_HEIGHT - 7.0,
            ),
            if studio_footer {
                Align2::RIGHT_CENTER
            } else {
                Align2::CENTER_CENTER
            },
            "OPENPATCH",
            FontId::proportional(7.0 * zoom),
            MUTED,
        );
    }
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
    crate::panel_skin::knob(painter, center, radius, normalized, active, zoom);
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
            crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
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
    compact: bool,
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
    paint_jack(painter, center, label, connected, output, zoom, compact);
    if response.hovered() {
        painter.circle_stroke(
            center,
            11.0 * zoom,
            Stroke::new(1.8 * zoom, Color32::from_rgb(221, 157, 64)),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.on_hover_text(if output {
        "Occupied OUT: drag or click another OUT to move the top cable source. Cmd/Ctrl starts a new cable to IN. Empty OUT starts a new cable. Cmd/Ctrl+Shift clones the top cable; Shift-click removes it."
    } else {
        "Drag to OUT to connect. Drag a connected input to another input to move its cable. Right-click to unplug; Esc cancels."
    })
}

fn paint_jack(
    painter: &egui::Painter,
    center: Pos2,
    label: &str,
    connected: bool,
    output: bool,
    zoom: f32,
    compact: bool,
) {
    if output {
        painter.circle_filled(center, 13.0 * zoom, crate::panel_skin::INK);
        painter.circle_stroke(
            center,
            13.0 * zoom,
            Stroke::new(0.6 * zoom, Color32::from_rgb(106, 109, 106)),
        );
    }
    if !compact {
        crate::panel_skin::port_label(
            painter,
            center - vec2(0.0, 19.0) * zoom,
            &port_label(label).to_uppercase(),
            output,
            zoom,
        );
    }
    crate::panel_skin::socket(painter, center, connected, zoom);
}

#[allow(clippy::too_many_arguments)]
fn paint_cable(
    painter: &egui::Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    zoom: f32,
    plugged: bool,
    bus: bool,
    fan: f32,
) {
    paint_cable_curves(painter, from, to, color, zoom, bus, fan);
    paint_cable_plug(painter, from, color, zoom);
    if plugged {
        paint_cable_plug(painter, to, color, zoom);
    }
}

fn paint_cable_curves(
    painter: &egui::Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    zoom: f32,
    bus: bool,
    fan: f32,
) {
    paint_cable_segment(
        painter,
        cable_curve_points(from, to, zoom, fan),
        color,
        zoom,
        bus,
        112,
    );
}

fn cable_curve_points(from: Pos2, to: Pos2, zoom: f32, fan: f32) -> [Pos2; 4] {
    let sag = (from.distance(to) * 0.29 + 38.0 * zoom).clamp(45.0 * zoom, 195.0 * zoom);
    [
        from,
        from + vec2(fan * zoom, sag),
        to + vec2(fan * zoom, sag),
        to,
    ]
}

// De Casteljau subdivision preserves the original curve, including stacked
// fan-out and polyphonic width. No independent easing of endpoint geometry.
fn cable_curve_prefix(points: [Pos2; 4], progress: f32) -> [Pos2; 4] {
    let a = points[0].lerp(points[1], progress);
    let b = points[1].lerp(points[2], progress);
    let c = points[2].lerp(points[3], progress);
    let d = a.lerp(b, progress);
    let e = b.lerp(c, progress);
    [points[0], a, d, d.lerp(e, progress)]
}

#[allow(clippy::too_many_arguments)]
fn paint_committed_cable_curves(
    painter: &egui::Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    zoom: f32,
    bus: bool,
    fan: f32,
    progress: Option<f32>,
) {
    let Some(progress) = progress else {
        paint_cable_curves(painter, from, to, color, zoom, bus, fan);
        return;
    };
    let points = cable_curve_points(from, to, zoom, fan);
    // The entire committed route remains visible; this flourish never represents
    // a pending connection or delays its electrical/hit-test state.
    paint_cable_segment(painter, points, color.gamma_multiply(0.28), zoom, bus, 32);
    if progress > 0.0 {
        let prefix = cable_curve_prefix(points, progress);
        paint_cable_segment(painter, prefix, color, zoom, bus, 112);
        painter.circle_stroke(prefix[3], 4.2 * zoom, Stroke::new(1.2 * zoom, color));
    }
}

fn paint_cable_segment(
    painter: &egui::Painter,
    points: [Pos2; 4],
    color: Color32,
    zoom: f32,
    bus: bool,
    shadow_alpha: u8,
) {
    let curve = |points, width, color| {
        egui::epaint::CubicBezierShape::from_points_stroke(
            points,
            false,
            Color32::TRANSPARENT,
            Stroke::new(width, color),
        )
    };
    painter.add(curve(
        points.map(|point| point + vec2(1.4, 3.2) * zoom),
        (if bus { 10.0 } else { 8.0 }) * zoom,
        Color32::from_black_alpha(shadow_alpha),
    ));
    painter.add(curve(
        points,
        (if bus { 7.8 } else { 6.0 }) * zoom,
        color.gamma_multiply(0.64),
    ));
    painter.add(curve(
        points.map(|point| point - vec2(0.5, 0.8) * zoom),
        (if bus { 3.6 } else { 2.6 }) * zoom,
        color.gamma_multiply(1.13),
    ));
}

fn paint_cable_plug(painter: &egui::Painter, point: Pos2, color: Color32, zoom: f32) {
    painter.circle_filled(
        point + vec2(0.5, 1.8) * zoom,
        7.5 * zoom,
        Color32::from_black_alpha(125),
    );
    painter.rect_filled(
        Rect::from_center_size(point + vec2(0.0, 5.0) * zoom, vec2(7.0, 12.0) * zoom),
        2.0 * zoom,
        color.gamma_multiply(0.52),
    );
    painter.circle_filled(point, 7.2 * zoom, Color32::from_rgb(82, 81, 75));
    painter.circle_filled(point, 6.1 * zoom, color.gamma_multiply(0.58));
    painter.circle_stroke(
        point,
        5.3 * zoom,
        Stroke::new(1.2 * zoom, color.gamma_multiply(1.16)),
    );
    painter.circle_filled(point, 2.0 * zoom, Color32::from_rgb(34, 35, 33));
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
        frame_output(ctx, rack, patch, time, events).0
    }

    fn frame_output(
        ctx: &egui::Context,
        rack: &mut Rack,
        patch: &Patch,
        time: f64,
        events: Vec<egui::Event>,
    ) -> (Option<RackAction>, egui::FullOutput) {
        frame_state(ctx, rack, patch, time, events, true, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn frame_state(
        ctx: &egui::Context,
        rack: &mut Rack,
        patch: &Patch,
        time: f64,
        events: Vec<egui::Event>,
        focused: bool,
        enabled: bool,
    ) -> (Option<RackAction>, egui::FullOutput) {
        frame_revision_state(ctx, rack, patch, time, events, focused, enabled, 0)
    }

    #[allow(clippy::too_many_arguments)]
    fn frame_revision_state(
        ctx: &egui::Context,
        rack: &mut Rack,
        patch: &Patch,
        time: f64,
        events: Vec<egui::Event>,
        focused: bool,
        enabled: bool,
        revision: u64,
    ) -> (Option<RackAction>, egui::FullOutput) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
            time: Some(time),
            modifiers: events
                .iter()
                .rev()
                .find_map(|event| match event {
                    egui::Event::Key { modifiers, .. }
                    | egui::Event::PointerButton { modifiers, .. } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or_default(),
            events,
            focused,
            ..Default::default()
        };
        let mut action = None;
        let output = ctx.run(input, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        action = rack.show(ui, patch, &openpatch_core::catalog(), 48000, revision);
                    });
                });
        });
        (action, output)
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

    /// Compare the real rack body with its preview at identical geometry. This
    /// covers every catalog kind, including specialized buttons, fields, note
    /// keyboards, sequencer cells, lights, meters, labels, knobs and sockets.
    #[test]
    fn every_catalog_preview_matches_actual_default_module_drawing() {
        fn primitives(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
            fn visit(shape: &egui::Shape, result: &mut Vec<String>) {
                match shape {
                    egui::Shape::Vec(children) => {
                        for child in children {
                            visit(child, result);
                        }
                    }
                    egui::Shape::Text(text) => result.push(format!(
                        "text:{:?}:{:?}:{}:{:?}:{:?}:{:?}:{:?}:{}:{}",
                        text.pos,
                        text.galley.size(),
                        text.galley.text(),
                        text.galley.job.sections,
                        text.fallback_color,
                        text.override_text_color,
                        text.underline,
                        text.opacity_factor,
                        text.angle
                    )),
                    egui::Shape::Circle(circle) => result.push(format!("circle:{circle:?}")),
                    egui::Shape::Rect(rect) => result.push(format!("rect:{rect:?}")),
                    egui::Shape::LineSegment { points, stroke } => {
                        result.push(format!("line:{points:?}:{stroke:?}"))
                    }
                    egui::Shape::Mesh(mesh) => {
                        result.push(format!("mesh:{:?}:{:?}", mesh.vertices, mesh.indices))
                    }
                    other => result.push(format!("shape:{other:?}")),
                }
            }
            let mut result = Vec::new();
            for shape in shapes {
                visit(&shape.shape, &mut result);
            }
            result
        }
        let catalog = openpatch_core::catalog();
        let modules = catalog["modules"].as_array().unwrap();
        assert_eq!(
            modules.len(),
            51,
            "update coverage when adding module kinds"
        );
        for (module, scale) in modules
            .iter()
            .flat_map(|module| [0.42, 0.85, 1.0].map(|scale| (module, scale)))
        {
            let kind = module["kind"].as_str().unwrap();
            let rect = Rect::from_min_size(
                pos2(12.0, 12.0),
                vec2(panel_width(kind), PANEL_HEIGHT) * scale,
            );
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut preview = ModulePreview::new(module);
            let input = || egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 500.0))),
                focused: true,
                ..Default::default()
            };
            // Warm the font atlas and first-frame widget layout.
            let _ = ctx.run(input(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    preview.show(ui, rect, &catalog);
                });
            });
            let thumbnail = ctx.run(input(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    preview.show(ui, rect, &catalog);
                });
            });
            // Use the exact fitted bounds: fractional-scale centering has float
            // rounding even when the supplied bounds have the same aspect ratio.
            let fitted = Rect::from_center_size(
                rect.center(),
                vec2(panel_width(kind), PANEL_HEIGHT) * preview.renderer.zoom,
            );
            let actual_id = "rack-instance-42".to_owned();
            preview.patch.nodes[0].id = actual_id.clone();
            preview.renderer.positions.clear();
            preview.renderer.positions.insert(actual_id, Pos2::ZERO);
            let actual = ctx.run(input(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(fitted));
                    child.set_clip_rect(ui.clip_rect().intersect(rect));
                    let rendered = preview.renderer.render_modules(
                        &mut child,
                        &preview.patch,
                        &catalog,
                        fitted,
                        48_000,
                        0,
                        RenderMode::Interactive {
                            drag_interactive: false,
                            allow_manual_press: false,
                        },
                    );
                    assert!(rendered.action.is_none(), "{kind}");
                });
            });
            let thumbnail = primitives(&thumbnail.shapes);
            let actual = primitives(&actual.shapes);
            let difference = thumbnail.iter().zip(&actual).position(|(a, b)| a != b);
            assert!(
                difference.is_none() && thumbnail.len() == actual.len(),
                "{kind} at {scale}: preview has {} primitives, actual {}; first difference {:?}",
                thumbnail.len(),
                actual.len(),
                difference.map(|i| (i, &thumbnail[i], &actual[i]))
            );
        }
    }

    #[test]
    fn studio_panel_text_stays_separate_at_library_and_rack_scales() {
        fn collect(shape: &egui::Shape, result: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Vec(children) => {
                    for child in children {
                        collect(child, result);
                    }
                }
                egui::Shape::Text(text) if !text.galley.text().is_empty() => {
                    result.push((
                        text.galley.text().into(),
                        Rect::from_min_size(text.pos, text.galley.size()),
                    ));
                }
                _ => {}
            }
        }
        let catalog = openpatch_core::catalog();
        for kind in [
            "score_sequencer",
            "drum_voice",
            "svf_filter",
            "stereo_delay",
            "stereo_reverb",
            "sidechain_compressor",
            "audio_stereo",
        ] {
            let module = catalog["modules"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["kind"] == kind)
                .unwrap();
            for scale in [0.42, 0.85, 1.0] {
                let ctx = egui::Context::default();
                crate::panel_skin::install_fonts(&ctx);
                let mut preview = ModulePreview::new(module);
                let rect = Rect::from_min_size(
                    pos2(12.0, 12.0),
                    vec2(panel_width(kind), PANEL_HEIGHT) * scale,
                );
                for _ in 0..2 {
                    let output = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 500.0))),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                preview.show(ui, rect, &catalog);
                            });
                        },
                    );
                    let mut texts = Vec::new();
                    for shape in &output.shapes {
                        collect(&shape.shape, &mut texts);
                    }
                    for (name, _) in ordered_parameters(&preview.patch.nodes[0], Some(module)) {
                        assert!(
                            texts.iter().any(|(text, _)| text == parameter_label(name)),
                            "{kind}: missing parameter label {name}"
                        );
                    }
                    if let Some((_, footer)) = texts.iter().find(|(text, _)| text == "OPENPATCH") {
                        for shape in &output.shapes {
                            if let egui::Shape::Circle(circle) = &shape.shape {
                                assert!(
                                    !footer.intersects(Rect::from_center_size(
                                        circle.center,
                                        Vec2::splat(circle.radius * 2.0)
                                    )),
                                    "{kind} at {scale}: footer intersects a control/socket"
                                );
                            }
                        }
                    }
                    for (index, (label, bounds)) in texts.iter().enumerate() {
                        for (other, other_bounds) in &texts[index + 1..] {
                            assert!(!bounds.shrink(0.2 * scale).intersects(other_bounds.shrink(0.2 * scale)),
                                "{kind} at {scale}: {label:?} {bounds:?} overlaps {other:?} {other_bounds:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn preview_never_changes_defaults_focus_input_or_renderer_state() {
        let catalog = openpatch_core::catalog();
        for module in catalog["modules"].as_array().unwrap() {
            let mut preview = ModulePreview::new(module);
            let before = serde_json::to_value(&preview.patch).unwrap();
            let ctx = egui::Context::default();
            let focus = egui::Id::new("outside-preview-focus");
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 500.0))),
                    focused: true,
                    events: vec![egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let _ = ui.interact(
                            Rect::from_min_size(pos2(900.0, 10.0), vec2(80.0, 30.0)),
                            focus,
                            Sense::click(),
                        );
                        ctx.memory_mut(|memory| memory.request_focus(focus));
                        preview.show(
                            ui,
                            Rect::from_min_size(pos2(12.0, 12.0), vec2(780.0, PANEL_HEIGHT)),
                            &catalog,
                        );
                        assert_eq!(ctx.memory(|memory| memory.focused()), Some(focus));
                        assert!(ctx.input(|input| input.key_pressed(egui::Key::Enter)));
                        assert!(!egui::Popup::is_any_open(ctx));
                    });
                },
            );
            assert!(!output.shapes.is_empty());
            assert_eq!(serde_json::to_value(&preview.patch).unwrap(), before);
            assert_eq!(preview.renderer.layout_revision, 0);
            assert!(preview.renderer.selection.is_empty());
            assert!(preview.renderer.held_manual.is_none());
            assert!(preview.renderer.editing.is_none());
            assert!(preview.renderer.pending.is_none());
        }
    }

    #[test]
    fn multi_vco_controls_modes_and_every_named_jack_have_independent_pointer_targets() {
        let patch:Patch=serde_json::from_value(json!({"version":2,"nodes":[{"id":"vco","kind":"multi_vco"},{"id":"source","kind":"constant"},{"id":"sink","kind":"audio_stereo"}]})).unwrap();
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let source = fixture_jack(&rack, &patch, "source", "out", false);
        for (index, name) in ["pitch", "fm", "sync", "pwm"].into_iter().enumerate() {
            let input = fixture_jack(&rack, &patch, "vco", name, true);
            let action = drag(&ctx, &mut rack, &patch, 1.0 + index as f64, source, input);
            assert!(
                matches!(action,Some(RackAction::Command("connect",ref args)) if args["to"]=="vco"&&args["input"]==name),
                "input {name}"
            );
        }
        let sink = fixture_jack(&rack, &patch, "sink", "left", true);
        for (index, name) in ["sine", "triangle", "saw", "square"]
            .into_iter()
            .enumerate()
        {
            let output = fixture_jack(&rack, &patch, "vco", name, false);
            let action = drag(&ctx, &mut rack, &patch, 6.0 + index as f64, output, sink);
            assert!(
                matches!(action,Some(RackAction::Command("connect",ref args)) if args["from"]=="vco"&&args["output"]==name),
                "output {name}"
            );
        }
        for (index, (name, position)) in [
            ("frequency", pos2(99.0, 137.0)),
            ("pulse_width", pos2(249.0, 137.0)),
            ("fm_amount", pos2(99.0, 237.0)),
            ("pwm_amount", pos2(249.0, 237.0)),
        ]
        .into_iter()
        .enumerate()
        {
            let time = 12.0 + index as f64;
            frame(
                &ctx,
                &mut rack,
                &patch,
                time,
                vec![egui::Event::PointerMoved(position), button(position, true)],
            );
            let moved = position - vec2(0.0, if name == "frequency" { 37.037037 } else { 10.0 });
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                time + 0.1,
                vec![egui::Event::PointerMoved(moved)],
            );
            let Some(RackAction::Command("set_parameter", args)) = action else {
                panic!("knob {name}")
            };
            assert_eq!(args["parameter"], name);
            if name == "frequency" {
                assert!(
                    (args["value"].as_f64().unwrap() - 523.25116).abs() < 0.02,
                    "One-ninth of the nine-octave knob span must double Hz"
                );
            } else {
                assert!(
                    args["value"].as_f64().unwrap() > if name == "pulse_width" { 0.5 } else { 0.0 }
                );
            }
            frame(
                &ctx,
                &mut rack,
                &patch,
                time + 0.2,
                vec![button(moved, false)],
            );
        }
        for (index, name) in ["linear_fm", "soft_sync"].into_iter().enumerate() {
            let target = pos2(106.5 + index as f32 * 135.0, 303.5);
            frame(
                &ctx,
                &mut rack,
                &patch,
                18.0 + index as f64,
                vec![egui::Event::PointerMoved(target), button(target, true)],
            );
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                18.1 + index as f64,
                vec![button(target, false)],
            );
            assert!(
                matches!(action,Some(RackAction::Command("set_parameter",ref args)) if args["parameter"]==name&&args["value"]==1.0),
                "mode {name}"
            );
        }
    }

    #[test]
    fn multi_vco_value_editors_use_hz_and_percentages_without_clock_ratio_conversion() {
        for (name, position, text, scale, expected) in [
            ("frequency", pos2(99.0, 169.0), "440", 1.0, 440.0),
            ("pulse_width", pos2(249.0, 169.0), "35", 100.0, 0.35),
            ("fm_amount", pos2(99.0, 269.0), "-25", 100.0, -0.25),
            ("pwm_amount", pos2(249.0, 269.0), "75", 100.0, 0.75),
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch: Patch = serde_json::from_value(
                json!({"version":2,"nodes":[{"id":"vco","kind":"multi_vco"}]}),
            )
            .unwrap();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            for (time, pressed) in [(1.0, true), (1.04, false), (1.08, true), (1.12, false)] {
                frame(
                    &ctx,
                    &mut rack,
                    &patch,
                    time,
                    vec![
                        egui::Event::PointerMoved(position),
                        button(position, pressed),
                    ],
                );
            }
            let edit = rack
                .editing
                .as_mut()
                .expect("value editor must be distinct from knob and mode buttons");
            assert_eq!(edit.parameter, name);
            assert_eq!(edit.display_scale, scale);
            if name == "frequency" {
                assert!((edit.text.parse::<f32>().unwrap() - 261.62558).abs() < 0.001);
            }
            if name == "pulse_width" {
                assert!((edit.min - 1.0).abs() < 0.001 && (edit.max - 99.0).abs() < 0.001);
            }
            edit.text = text.into();
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
                matches!(action,Some(RackAction::Command("set_parameter",ref args)) if args["parameter"]==name&&(args["value"].as_f64().unwrap()-expected).abs()<0.0001)
            );
        }
    }

    fn right_click_parameter(
        ctx: &egui::Context,
        rack: &mut Rack,
        patch: &Patch,
        revision: u64,
        position: Pos2,
        start_time: f64,
    ) -> egui::FullOutput {
        for (offset, pressed) in [(0.0, true), (0.1, false)] {
            frame_revision_state(
                ctx,
                rack,
                patch,
                start_time + offset,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: PointerButton::Secondary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                true,
                true,
                revision,
            );
        }
        frame_revision_state(
            ctx,
            rack,
            patch,
            start_time + 0.2,
            vec![],
            true,
            true,
            revision,
        )
        .1
    }

    #[test]
    fn knob_secondary_click_opens_direct_numeric_popup_without_module_menu_or_selection_change() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"vco","kind":"multi_vco"},{"id":"other","kind":"constant"}]
        }))
        .unwrap();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
        rack.select_only("other".into());
        let output = right_click_parameter(&ctx, &mut rack, &patch, 7, pos2(99.0, 137.0), 1.0);
        assert_eq!(rack.selected_ids(), vec!["other"]);
        let edit = rack.editing.as_ref().expect("knob popup editor");
        assert_eq!(edit.id, "vco");
        assert_eq!(edit.parameter, "frequency");
        assert_eq!(edit.revision, 7);
        assert_eq!(edit.surface, ParameterEditSurface::KnobMenu);
        let mut text = Vec::new();
        fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(item) => text.push(item.galley.text().to_owned()),
                egui::Shape::Vec(items) => {
                    for item in items {
                        collect(item, text);
                    }
                }
                _ => {}
            }
        }
        for shape in &output.shapes {
            collect(&shape.shape, &mut text);
        }
        assert!(
            text.iter().any(|label| label.starts_with("Current:")),
            "{text:?}"
        );
        assert!(
            text.iter().any(|label| label.starts_with("Range:")),
            "{text:?}"
        );
        assert!(
            !text.iter().any(|label| label.contains("Duplicate with")),
            "{text:?}"
        );
        rack.editing.as_mut().unwrap().text = "440".into();
        let enter = egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.3,
            vec![enter.clone()],
            true,
            true,
            7,
        )
        .0;
        assert!(
            matches!(action,Some(RackAction::Command("set_parameter",args)) if args["id"]=="vco" && args["parameter"]=="frequency" && args["expected_revision"]==7 && args["value"]==440.0 && args.get("history_group").is_none())
        );
        assert!(rack.editing.is_none());
        assert!(
            frame_revision_state(&ctx, &mut rack, &patch, 1.4, vec![enter], true, true, 7)
                .0
                .is_none()
        );
    }

    #[test]
    fn numeric_popup_rejects_invalid_unfocused_and_stale_drafts() {
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"vco","kind":"multi_vco"}]
        }))
        .unwrap();
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 4);
        right_click_parameter(&ctx, &mut rack, &patch, 4, pos2(99.0, 137.0), 1.0);
        rack.editing.as_mut().unwrap().text = "not a number".into();
        let enter = egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        assert!(frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.3,
            vec![enter.clone()],
            true,
            true,
            4
        )
        .0
        .is_none());
        assert!(rack.editing.is_some());
        rack.editing.as_mut().unwrap().text = "440".into();
        assert!(frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.4,
            vec![enter.clone()],
            false,
            true,
            4
        )
        .0
        .is_none());
        assert!(rack.editing.is_none());

        right_click_parameter(&ctx, &mut rack, &patch, 4, pos2(99.0, 137.0), 2.0);
        rack.editing.as_mut().unwrap().text = "880".into();
        assert!(
            frame_revision_state(&ctx, &mut rack, &patch, 2.3, vec![], true, true, 5)
                .0
                .is_none()
        );
        let edit = rack.editing.as_ref().expect("stale draft stays visible");
        assert!(edit.stale);
        assert_eq!(edit.surface, ParameterEditSurface::Window);
        assert_eq!(edit.text, "880");
        assert!(frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            2.4,
            vec![enter.clone()],
            true,
            true,
            5
        )
        .0
        .is_none());
        assert!(rack.editing.is_some());
        assert!(frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            2.5,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            true,
            true,
            5
        )
        .0
        .is_none());
        assert!(rack.editing.is_none());
    }

    fn stacked_fixture() -> Patch {
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"src","kind":"constant","params":{"value":1}},{"id":"alt","kind":"constant","params":{"value":2}},{"id":"target","kind":"gain"},{"id":"dest","kind":"gain"}],"cables":[{"id":"one","from":"src","to":"target","input":"signal"},{"id":"two","from":"src","to":"target","input":"signal"}],"output":"target"})).unwrap()
    }

    fn fixture_jack(rack: &Rack, patch: &Patch, id: &str, port: &str, input: bool) -> Pos2 {
        let node = patch.nodes.iter().find(|n| n.id == id).unwrap();
        let catalog = openpatch_core::catalog();
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == node.kind)
            .unwrap();
        let ports = module[if input { "inputs" } else { "outputs" }]
            .as_array()
            .unwrap();
        let index = ports.iter().position(|p| p == port).unwrap();
        (rack.pan
            + (rack.positions[id].to_vec2()
                + port_position(
                    &node.kind,
                    input,
                    index,
                    ports.len(),
                    panel_width(&node.kind),
                )
                .to_vec2())
                * rack.zoom)
            .to_pos2()
    }

    fn run_cable_action(session: &mut openpatch::control::Session, action: RackAction) {
        let RackAction::Command(tool, args) = action else {
            panic!("expected cable edit")
        };
        session.execute(tool, args).unwrap();
    }

    #[test]
    fn identical_stacked_cables_move_and_change_source_by_id_without_disturbing_siblings() {
        for change_source in [false, true] {
            let workspace = tempfile::tempdir().unwrap();
            let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
            session.replace_patch(stacked_fixture(), None).unwrap();
            let before = session.patch().clone();
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            frame_revision_state(
                &ctx,
                &mut rack,
                &before,
                0.0,
                vec![],
                true,
                true,
                session.revision(),
            );
            let layout = rack.layout();
            let input = if change_source {
                fixture_jack(&rack, &before, "src", "out", false)
            } else {
                fixture_jack(&rack, &before, "target", "signal", true)
            };
            let target = if change_source {
                fixture_jack(&rack, &before, "alt", "out", false)
            } else {
                fixture_jack(&rack, &before, "dest", "signal", true)
            };
            frame_revision_state(
                &ctx,
                &mut rack,
                &before,
                1.0,
                vec![egui::Event::PointerMoved(input), button(input, true)],
                true,
                true,
                session.revision(),
            );
            frame_revision_state(
                &ctx,
                &mut rack,
                &before,
                1.1,
                vec![egui::Event::PointerMoved(target)],
                true,
                true,
                session.revision(),
            );
            assert_eq!(
                rack.pending
                    .as_ref()
                    .unwrap()
                    .original
                    .as_ref()
                    .unwrap()
                    .id
                    .as_deref(),
                Some("two")
            );
            let action = frame_revision_state(
                &ctx,
                &mut rack,
                &before,
                1.2,
                vec![button(target, false)],
                true,
                true,
                session.revision(),
            )
            .0
            .unwrap();
            let RackAction::Command("move_cables", ref args) = action else {
                panic!("picked move must be atomic")
            };
            assert_eq!(args["cable_ids"], json!(["two"]));
            assert_eq!(args["promote"], true);
            run_cable_action(&mut session, action);
            assert_eq!(session.patch().cables.len(), 2);
            assert!(same_cable(
                session
                    .patch()
                    .cables
                    .iter()
                    .find(|c| c.id.as_deref() == Some("one"))
                    .unwrap(),
                &before.cables[0]
            ));
            let moved = session
                .patch()
                .cables
                .iter()
                .find(|c| c.id.as_deref() == Some("two"))
                .unwrap();
            if change_source {
                assert_eq!(moved.from, "alt");
                assert_eq!(moved.to, "target");
            } else {
                assert_eq!(moved.from, "src");
                assert_eq!(moved.to, "dest");
            }
            frame_revision_state(
                &ctx,
                &mut rack,
                session.patch(),
                2.0,
                vec![],
                true,
                true,
                session.revision(),
            );
            assert_eq!(
                rack.layout(),
                layout,
                "Routing edits must leave rack geometry unchanged"
            );
            let encoded = serde_json::to_value(session.patch()).unwrap();
            let decoded: Patch = serde_json::from_value(encoded).unwrap();
            let mut restored = Rack::default();
            restored.restore_layout(&layout).unwrap();
            frame_revision_state(
                &egui::Context::default(),
                &mut restored,
                &decoded,
                0.0,
                vec![],
                true,
                true,
                session.revision(),
            );
            assert_eq!(restored.layout(), layout);
        }
    }

    #[test]
    fn shift_drop_adds_identical_routes_and_default_drop_keeps_existing_siblings() {
        for stack in [false, true] {
            let workspace = tempfile::tempdir().unwrap();
            let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
            session.replace_patch(stacked_fixture(), None).unwrap();
            let patch = session.patch().clone();
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let revision = session.revision();
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, revision);
            let from = if stack {
                pos2(84.0, 386.0)
            } else {
                pos2(204.0, 386.0)
            };
            let to = fixture_jack(&rack, &patch, "target", "signal", true);
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![
                    egui::Event::PointerMoved(from),
                    egui::Event::PointerButton {
                        pos: from,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers {
                            command: true,
                            ..Default::default()
                        },
                    },
                ],
                true,
                true,
                revision,
            );
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.1,
                vec![egui::Event::PointerMoved(to)],
                true,
                true,
                revision,
            );
            let release = egui::Event::PointerButton {
                pos: to,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: if stack {
                    egui::Modifiers::SHIFT
                } else {
                    egui::Modifiers::NONE
                },
            };
            let action = frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.2,
                vec![release],
                true,
                true,
                revision,
            )
            .0
            .unwrap();
            run_cable_action(&mut session, action);
            if stack {
                assert_eq!(session.patch().cables.len(), 3);
                assert!(session
                    .patch()
                    .cables
                    .iter()
                    .all(|c| c.from == "src" && c.to == "target"));
                let ids: BTreeSet<_> = session
                    .patch()
                    .cables
                    .iter()
                    .map(|c| c.id.as_ref().unwrap())
                    .collect();
                assert_eq!(ids.len(), 3);
            } else {
                assert_eq!(session.patch().cables.len(), 3);
                assert_eq!(
                    serde_json::to_value(&session.patch().cables[0]).unwrap(),
                    serde_json::to_value(&patch.cables[0]).unwrap()
                );
                assert_eq!(
                    serde_json::to_value(&session.patch().cables[1]).unwrap(),
                    serde_json::to_value(&patch.cables[1]).unwrap()
                );
                assert_eq!(session.patch().cables[2].from, "alt");
                assert!(!matches!(
                    session.patch().cables[2].id.as_deref(),
                    Some("one" | "two")
                ));
            }
            session.execute("undo", json!({})).unwrap();
            assert_eq!(
                serde_json::to_value(session.patch()).unwrap(),
                serde_json::to_value(&patch).unwrap()
            );
        }
    }

    #[test]
    fn stacked_input_menu_lists_duplicate_ids_and_removes_only_the_chosen_cable() {
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(stacked_fixture(), None).unwrap();
        let patch = session.patch().clone();
        let revision = session.revision();
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, revision);
        let input = fixture_jack(&rack, &patch, "target", "signal", true);
        for (time, pressed) in [(1.0, true), (1.1, false)] {
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                time,
                vec![
                    egui::Event::PointerMoved(input),
                    egui::Event::PointerButton {
                        pos: input,
                        button: PointerButton::Secondary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                true,
                true,
                revision,
            );
        }
        let (_, output) =
            frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![], true, true, revision);
        let text_position = |output: &egui::FullOutput, label: &str| {
            output
                .shapes
                .iter()
                .find_map(|s| {
                    if let egui::epaint::Shape::Text(t) = &s.shape {
                        (t.galley.text() == label).then_some(t.pos + t.galley.size() * 0.5)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| panic!("Missing menu text {label}"))
        };
        text_position(&output, "one · src.out");
        let entry = text_position(&output, "two · src.out");
        for (time, pressed) in [(1.3, true), (1.4, false)] {
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                time,
                vec![egui::Event::PointerMoved(entry), button(entry, pressed)],
                true,
                true,
                revision,
            );
        }
        let (_, output) =
            frame_revision_state(&ctx, &mut rack, &patch, 1.5, vec![], true, true, revision);
        let remove = text_position(&output, "Delete this cable");
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.6,
            vec![egui::Event::PointerMoved(remove), button(remove, true)],
            true,
            true,
            revision,
        );
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.7,
            vec![button(remove, false)],
            true,
            true,
            revision,
        )
        .0
        .unwrap();
        assert!(
            matches!(&action,RackAction::Command("disconnect",args) if args["cable_id"]=="two"&&args.get("to").is_none())
        );
        run_cable_action(&mut session, action);
        assert_eq!(session.patch().cables.len(), 1);
        assert_eq!(session.patch().cables[0].id.as_deref(), Some("one"));
    }

    #[test]
    fn pending_stack_cancel_and_graph_revision_change_leave_every_original_route_intact() {
        let patch = stacked_fixture();
        let original = serde_json::to_value(&patch).unwrap();
        for stale in [false, true] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
            let layout = rack.layout();
            let input = fixture_jack(&rack, &patch, "target", "signal", true);
            let target = fixture_jack(&rack, &patch, "dest", "signal", true);
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![egui::Event::PointerMoved(input), button(input, true)],
                true,
                true,
                7,
            );
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.1,
                vec![egui::Event::PointerMoved(target)],
                true,
                true,
                7,
            );
            assert!(rack.pending.is_some());
            let events = if stale {
                vec![button(target, false)]
            } else {
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            };
            assert!(frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.2,
                events,
                true,
                true,
                if stale { 8 } else { 7 }
            )
            .0
            .is_none());
            assert!(rack.pending.is_none());
            assert!(!rack.dragging_cable);
            assert_eq!(rack.layout(), layout);
            assert_eq!(serde_json::to_value(&patch).unwrap(), original);
        }
    }

    #[test]
    fn legacy_occupied_input_rejects_both_shift_and_ordinary_new_cables() {
        let mut patch = stacked_fixture();
        patch.version = 1;
        patch.cables.pop();
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let from = pos2(204.0, 386.0);
        let to = fixture_jack(&rack, &patch, "target", "signal", true);
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(from), button(from, true)],
        );
        let (_, output) = frame_output(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![
                egui::Event::PointerMoved(to),
                egui::Event::Key {
                    key: egui::Key::ArrowUp,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::SHIFT,
                },
            ],
        );
        assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::epaint::Shape::Text(t) if t.galley.text().contains("engine v2"))));
        assert!(frame(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![egui::Event::PointerButton {
                pos: to,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::SHIFT
            }]
        )
        .is_none());
        let pending = PendingCable::new(
            CableEnd::Output {
                node: "alt".into(),
                port: "out".into(),
            },
            &patch,
        );
        assert!(cable_action(
            &patch,
            &pending,
            &CableEnd::Input {
                node: "target".into(),
                port: "signal".into()
            },
            &openpatch_core::catalog()
        )
        .is_none());
    }

    fn selection_click(
        ctx: &egui::Context,
        rack: &mut Rack,
        patch: &Patch,
        time: f64,
        position: Pos2,
        modifiers: egui::Modifiers,
    ) {
        for (offset, pressed) in [(0.0, true), (0.1, false)] {
            frame(
                ctx,
                rack,
                patch,
                time + offset,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                ],
            );
        }
    }

    fn selection_key(key: egui::Key, shift: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                command: true,
                ctrl: true,
                shift,
                ..Default::default()
            },
        }
    }

    fn duplicate_key_at(
        ctx: &egui::Context,
        rack: &mut Rack,
        patch: &Patch,
        time: f64,
        pointer: Pos2,
        shift: bool,
    ) -> Option<RackAction> {
        let action = frame(
            ctx,
            rack,
            patch,
            time,
            vec![
                egui::Event::PointerMoved(pointer),
                selection_key(egui::Key::D, shift),
            ],
        );
        frame(
            ctx,
            rack,
            patch,
            time + 0.1,
            vec![egui::Event::Key {
                key: egui::Key::D,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        action
    }

    #[test]
    fn duplicate_shortcuts_prioritize_selection_then_hover_and_require_rack_pointer() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[
                {"id":"a","kind":"constant"},
                {"id":"b","kind":"constant"}
            ]
        }))
        .unwrap();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        rack.select_only("a".into());
        for (time, pointer, shift, mode) in [
            (1.0, pos2(204.0, 44.0), false, DuplicateCableMode::None),
            (2.0, pos2(204.0, 44.0), true, DuplicateCableMode::Inputs),
            (3.0, pos2(800.0, 650.0), true, DuplicateCableMode::Inputs),
        ] {
            assert!(matches!(
                duplicate_key_at(&ctx, &mut rack, &patch, time, pointer, shift),
                Some(RackAction::Duplicate { ids, cable_mode, select_copies: true, .. })
                    if ids == ["a"] && cable_mode == mode
            ));
        }
        assert!(
            duplicate_key_at(&ctx, &mut rack, &patch, 4.0, pos2(1100.0, 900.0), true).is_none()
        );
        rack.selection.clear();
        rack.selected = None;
        for (time, shift, mode) in [
            (5.0, false, DuplicateCableMode::None),
            (6.0, true, DuplicateCableMode::All),
        ] {
            assert!(matches!(
                duplicate_key_at(&ctx, &mut rack, &patch, time, pos2(204.0, 44.0), shift),
                Some(RackAction::Duplicate { ids, cable_mode, select_copies: false, .. })
                    if ids == ["b"] && cable_mode == mode
            ));
        }
        assert!(duplicate_key_at(&ctx, &mut rack, &patch, 7.0, pos2(800.0, 650.0), true).is_none());
        assert!(
            duplicate_key_at(&ctx, &mut rack, &patch, 8.0, pos2(1100.0, 900.0), false).is_none()
        );
        let alt_command = egui::Event::Key {
            key: egui::Key::D,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                command: true,
                ctrl: true,
                alt: true,
                ..Default::default()
            },
        };
        assert!(frame(
            &ctx,
            &mut rack,
            &patch,
            9.0,
            vec![egui::Event::PointerMoved(pos2(204.0, 44.0)), alt_command,]
        )
        .is_none());
    }

    #[test]
    fn version_one_all_mode_needs_no_external_outgoing_cable() {
        let patch: Patch = serde_json::from_value(json!({
            "version":1,
            "nodes":[{"id":"source","kind":"constant"},{"id":"sink","kind":"analog_vco"}],
            "cables":[{"from":"source","to":"sink","input":"pitch"}]
        }))
        .unwrap();
        assert!(!duplicate_all_supported(&patch, &["source".into()]));
        assert!(duplicate_all_supported(&patch, &["sink".into()]));
    }

    #[test]
    fn header_and_panel_duplicate_menus_keep_clicked_module_context() {
        fn text_center(output: &egui::FullOutput, label: &str) -> Option<Pos2> {
            fn in_shape(shape: &egui::Shape, label: &str) -> Option<Pos2> {
                match shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    egui::Shape::Vec(shapes) => {
                        shapes.iter().find_map(|shape| in_shape(shape, label))
                    }
                    _ => None,
                }
            }
            output
                .shapes
                .iter()
                .find_map(|shape| in_shape(&shape.shape, label))
        }
        fn visible_text(output: &egui::FullOutput) -> Vec<String> {
            fn collect(shape: &egui::Shape, texts: &mut Vec<String>) {
                match shape {
                    egui::Shape::Text(text) => texts.push(text.galley.text().to_owned()),
                    egui::Shape::Vec(shapes) => {
                        for shape in shapes {
                            collect(shape, texts);
                        }
                    }
                    _ => {}
                }
            }
            let mut texts = Vec::new();
            for shape in &output.shapes {
                collect(&shape.shape, &mut texts);
            }
            texts
        }
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[
                {"id":"a","kind":"constant"},
                {"id":"b","kind":"constant"}
            ]
        }))
        .unwrap();
        for (menu_position, selected_context) in [
            (pos2(204.0, 44.0), false),
            (pos2(204.0, 300.0), false),
            (pos2(84.0, 44.0), true),
            (pos2(84.0, 300.0), true),
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            rack.select_only("a".into());
            if selected_context {
                rack.toggle_selection("b".into());
            }
            let before = rack.selected_ids();
            for (time, pressed) in [(1.0, true), (1.1, false)] {
                frame(
                    &ctx,
                    &mut rack,
                    &patch,
                    time,
                    vec![
                        egui::Event::PointerMoved(menu_position),
                        egui::Event::PointerButton {
                            pos: menu_position,
                            button: PointerButton::Secondary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert_eq!(
                rack.selected_ids(),
                before,
                "right click must preserve selection"
            );
            frame_output(&ctx, &mut rack, &patch, 1.2, vec![]);
            let (_, output) = frame_output(&ctx, &mut rack, &patch, 1.25, vec![]);
            let label = if selected_context {
                "Duplicate with inputs"
            } else {
                "Duplicate with cables"
            };
            let target = text_center(&output, label).unwrap_or_else(|| panic!(
                "missing {label} at {menu_position:?}, selected_context={selected_context}, selection={:?}, popup_open={}, visible_text={:?}",
                rack.selected_ids(), egui::Popup::is_any_open(&ctx), visible_text(&output)
            ));
            frame(
                &ctx,
                &mut rack,
                &patch,
                1.3,
                vec![egui::Event::PointerMoved(target), button(target, true)],
            );
            let action = frame(&ctx, &mut rack, &patch, 1.4, vec![button(target, false)]);
            assert!(matches!(action,
                Some(RackAction::Duplicate { ids, cable_mode, select_copies, .. })
                    if ids == if selected_context { vec!["a", "b"] } else { vec!["b"] }
                    && cable_mode == if selected_context { DuplicateCableMode::Inputs } else { DuplicateCableMode::All }
                    && select_copies == selected_context
            ));
            assert_eq!(rack.selected_ids(), before);
        }
    }

    #[test]
    fn shift_headers_group_drag_shortcuts_and_empty_click_keep_selection_coherent() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch:Patch=serde_json::from_value(json!({"version":2,"nodes":[{"id":"a","kind":"constant"},{"id":"b","kind":"constant"},{"id":"c","kind":"constant"}]})).unwrap();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        selection_click(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            pos2(84.0, 44.0),
            egui::Modifiers::NONE,
        );
        selection_click(
            &ctx,
            &mut rack,
            &patch,
            2.0,
            pos2(204.0, 44.0),
            egui::Modifiers::SHIFT,
        );
        assert_eq!(rack.selected_ids(), vec!["a", "b"]);
        selection_click(
            &ctx,
            &mut rack,
            &patch,
            3.0,
            pos2(204.0, 44.0),
            egui::Modifiers::SHIFT,
        );
        assert_eq!(rack.selected_ids(), vec!["a"]);
        selection_click(
            &ctx,
            &mut rack,
            &patch,
            4.0,
            pos2(204.0, 44.0),
            egui::Modifiers::SHIFT,
        );
        let action = drag(
            &ctx,
            &mut rack,
            &patch,
            5.0,
            pos2(84.0, 44.0),
            pos2(144.0, 464.0),
        );
        let Some(RackAction::MoveModules(proposal)) = action else {
            panic!("expected group move proposal");
        };
        assert_eq!(rack.positions["a"], pos2(0.0, 0.0));
        rack.apply_module_move(&proposal).unwrap();
        assert_eq!(rack.positions["a"], pos2(60.0, 420.0));
        assert_eq!(rack.positions["b"] - rack.positions["a"], vec2(120.0, 0.0));
        assert_eq!(rack.selected_ids(), vec!["a", "b"]);
        for (time, shift) in [(6.0, false), (7.0, true)] {
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                time,
                vec![selection_key(egui::Key::D, shift)],
            );
            assert!(
                matches!(action,Some(RackAction::Duplicate{ref ids,cable_mode,select_copies:true,..}) if ids==&vec!["a".to_owned(),"b".to_owned()]&&cable_mode==if shift {DuplicateCableMode::Inputs} else {DuplicateCableMode::None})
            );
            frame(
                &ctx,
                &mut rack,
                &patch,
                time + 0.1,
                vec![egui::Event::Key {
                    key: egui::Key::D,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        let action = frame(
            &ctx,
            &mut rack,
            &patch,
            8.0,
            vec![egui::Event::Key {
                key: egui::Key::Delete,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            matches!(action,Some(RackAction::Command("apply_edits",ref args)) if args["edits"].as_array().unwrap().len()==2)
        );
        frame(
            &ctx,
            &mut rack,
            &patch,
            9.0,
            vec![selection_key(egui::Key::A, false)],
        );
        assert_eq!(rack.selected_ids(), vec!["a", "b", "c"]);
        // A control click in a selected member retains the group; outside it selects that member.
        rack.select_only("a".into());
        selection_click(
            &ctx,
            &mut rack,
            &patch,
            10.0,
            pos2(324.0, 195.0),
            egui::Modifiers::NONE,
        );
        assert_eq!(rack.selected_ids(), vec!["c"]);
        selection_click(
            &ctx,
            &mut rack,
            &patch,
            11.0,
            pos2(800.0, 650.0),
            egui::Modifiers::NONE,
        );
        assert!(rack.selected_ids().is_empty());
        assert!(rack.selected.is_none());
    }

    #[test]
    fn pointer_move_previews_without_publishing_layout_until_accepted() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"a","kind":"constant"}]
        }))
        .unwrap();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
        let baseline = rack.layout();
        let layout_revision = rack.layout_revision();
        let from = pos2(84.0, 44.0);
        let to = from + vec2(60.0, 420.0);
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(from), button(from, true)],
            true,
            true,
            7,
        );
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![egui::Event::PointerMoved(to)],
            true,
            true,
            7,
        );
        assert_eq!(rack.positions["a"], pos2(60.0, 420.0));
        assert_eq!(rack.layout(), baseline);
        assert_eq!(rack.layout_revision(), layout_revision);
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![button(to, false)],
            true,
            true,
            7,
        )
        .0;
        let Some(RackAction::MoveModules(proposal)) = action else {
            panic!("expected move proposal");
        };
        assert_eq!(proposal.revision, 7);
        assert_eq!(proposal.before["a"], pos2(0.0, 0.0));
        assert_eq!(proposal.after["a"], pos2(60.0, 420.0));
        assert_eq!(rack.layout(), baseline);
        assert_eq!(rack.layout_revision(), layout_revision);
        rack.apply_module_move(&proposal).unwrap();
        assert_eq!(rack.positions["a"], pos2(60.0, 420.0));
        assert_eq!(rack.layout_revision(), layout_revision + 1);
    }

    #[test]
    fn pointer_drag_cancels_without_layout_write_on_escape_focus_modal_or_graph_change() {
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"a","kind":"constant"}]
        }))
        .unwrap();
        for cause in ["escape", "focus", "disabled", "modal", "revision"] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 3);
            let baseline = rack.layout();
            let layout_revision = rack.layout_revision();
            let from = pos2(84.0, 44.0);
            let to = from + vec2(60.0, 420.0);
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![egui::Event::PointerMoved(from), button(from, true)],
                true,
                true,
                3,
            );
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.1,
                vec![egui::Event::PointerMoved(to)],
                true,
                true,
                3,
            );
            assert_ne!(rack.positions["a"], pos2(0.0, 0.0), "{cause}");
            if cause == "modal" {
                rack.set_shortcuts_enabled(false);
            }
            let events = if cause == "escape" {
                vec![selection_key(egui::Key::Escape, false)]
            } else {
                vec![]
            };
            let action = frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.2,
                events,
                cause != "focus",
                cause != "disabled",
                if cause == "revision" { 4 } else { 3 },
            )
            .0;
            assert!(action.is_none(), "{cause}");
            assert_eq!(rack.positions["a"], pos2(0.0, 0.0), "{cause}");
            assert_eq!(rack.layout(), baseline, "{cause}");
            assert_eq!(rack.layout_revision(), layout_revision, "{cause}");
            rack.set_shortcuts_enabled(true);
            assert!(frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.3,
                vec![button(to, false)],
                true,
                true,
                if cause == "revision" { 4 } else { 3 },
            )
            .0
            .is_none());
        }
    }

    #[test]
    fn pointer_drag_snapping_to_same_slot_or_blocked_group_emits_no_move() {
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"a","kind":"constant"},{"id":"b","kind":"constant"}]
        }))
        .unwrap();
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let baseline = rack.layout();
        let revision = rack.layout_revision();
        let action = drag(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            pos2(84.0, 44.0),
            pos2(76.0, 44.0),
        );
        assert!(action.is_none());
        assert_eq!(rack.layout(), baseline);
        assert_eq!(rack.layout_revision(), revision);

        // Individually valid sidecar coordinates can span more than the entire
        // legal group translation range. The placement search must reject it.
        rack.positions.insert("a".into(), pos2(-100_000.0, 0.0));
        rack.positions.insert("b".into(), pos2(100.0, 0.0));
        rack.select_only("b".into());
        rack.toggle_selection("a".into());
        let baseline = rack.layout();
        let revision = rack.layout_revision();
        let action = drag(
            &ctx,
            &mut rack,
            &patch,
            2.0,
            pos2(184.0, 44.0),
            pos2(244.0, 464.0),
        );
        assert!(action.is_none());
        assert_eq!(rack.layout(), baseline);
        assert_eq!(rack.layout_revision(), revision);
    }

    #[test]
    fn cancel_module_drag_restores_only_members_still_present() {
        let mut rack = Rack {
            positions: BTreeMap::from([
                ("a".into(), pos2(60.0, 420.0)),
                ("unrelated".into(), pos2(100.0, 100.0)),
            ]),
            group_drag: Some(GroupDrag {
                anchor: "a".into(),
                origins: BTreeMap::from([
                    ("a".into(), pos2(0.0, 0.0)),
                    ("removed".into(), pos2(120.0, 0.0)),
                ]),
                delta: vec2(60.0, 420.0),
                revision: 4,
            }),
            ..Default::default()
        };
        let revision = rack.layout_revision();
        rack.cancel_module_drag();
        assert_eq!(rack.positions["a"], pos2(0.0, 0.0));
        assert!(!rack.positions.contains_key("removed"));
        assert_eq!(rack.positions["unrelated"], pos2(100.0, 100.0));
        assert_eq!(rack.layout_revision(), revision);
    }

    #[test]
    fn move_application_validates_all_members_before_writing_any_position() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"a","kind":"constant"},{"id":"b","kind":"constant"}]
        }))
        .unwrap();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 3);
        let before = BTreeMap::from([
            ("a".into(), rack.positions["a"]),
            ("b".into(), rack.positions["b"]),
        ]);
        let after = BTreeMap::from([
            ("a".into(), pos2(15.0, 0.0)),
            ("b".into(), pos2(135.0, 0.0)),
        ]);
        let proposal = ModuleMove {
            revision: 3,
            before: before.clone(),
            after,
        };
        let baseline = rack.positions.clone();
        let layout_revision = rack.layout_revision();
        let mut invalid = proposal.clone();
        invalid.after.insert("b".into(), pos2(f32::NAN, 0.0));
        assert!(rack.apply_module_move(&invalid).is_err());
        invalid = proposal.clone();
        invalid.before.insert("b".into(), pos2(999.0, 0.0));
        assert!(rack.apply_module_move(&invalid).is_err());
        invalid = proposal.clone();
        invalid.after.remove("b");
        assert!(rack.apply_module_move(&invalid).is_err());
        assert_eq!(rack.positions, baseline);
        assert_eq!(rack.layout_revision(), layout_revision);
        rack.apply_module_move(&ModuleMove {
            revision: 3,
            before: before.clone(),
            after: before,
        })
        .unwrap();
        assert_eq!(rack.layout_revision(), layout_revision);
        rack.apply_module_move(&proposal).unwrap();
        assert_eq!(rack.positions["a"], pos2(15.0, 0.0));
        assert_eq!(rack.positions["b"], pos2(135.0, 0.0));
        assert_eq!(rack.layout_revision(), layout_revision + 1);
    }

    #[test]
    fn group_placement_preserves_mixed_width_multirow_offsets_and_avoids_every_panel() {
        let mut rack = Rack::default();
        let patch:Patch=serde_json::from_value(json!({"version":2,"nodes":[{"id":"wide","kind":"vca_mix"},{"id":"small","kind":"constant"},{"id":"blocker","kind":"seq3"}]})).unwrap();
        rack.restore_layout(&json!({"version":1,"zoom":1.0,"pan":[24,12],"positions":{"wide":[0,0],"small":[75,420],"blocker":[390,0]}})).unwrap();
        let before = rack.layout();
        let placement = rack
            .duplicate_placement(
                &patch,
                &[
                    ("wide".into(), "wide-copy".into()),
                    ("small".into(), "small-copy".into()),
                ],
            )
            .unwrap();
        assert_eq!(rack.layout(), before, "Planning must be pure");
        assert_eq!(
            placement["small-copy"] - placement["wide-copy"],
            vec2(75.0, 420.0)
        );
        for (id, width) in [("wide-copy", 390.0), ("small-copy", 120.0)] {
            let p = placement[id];
            assert_eq!(p.x % HP, 0.0);
            assert_eq!(p.y % ROW, 0.0);
            for node in &patch.nodes {
                let q = rack.positions[&node.id];
                assert!(
                    (p.y - q.y).abs() >= PANEL_HEIGHT
                        || p.x + width <= q.x
                        || p.x >= q.x + panel_width(&node.kind)
                );
            }
        }
        rack.apply_duplicate_placement(placement, true);
        assert_eq!(rack.selected_ids(), vec!["small-copy", "wide-copy"]);
        let mut updated = patch;
        updated.nodes.push(Node {
            score: None,
            id: "wide-copy".into(),
            kind: "vca_mix".into(),
            params: Default::default(),
            bypassed: false,
        });
        rack.reconcile_selection(&updated);
        assert_eq!(rack.selected_ids(), vec!["wide-copy"]);
    }

    #[test]
    fn selection_shortcuts_ignore_modal_focus_loss_text_editing_and_active_cable_gestures() {
        for key in [egui::Key::D, egui::Key::I, egui::Key::R] {
            for cause in ["modal", "focus", "browser", "editor", "cable", "drag"] {
                let ctx = egui::Context::default();
                let mut rack = Rack::default();
                let patch: Patch = serde_json::from_value(json!({
                    "version":2,"nodes":[{"id":"a","kind":"constant"}]
                }))
                .unwrap();
                rack.select_only("a".into());
                frame(&ctx, &mut rack, &patch, 0.0, vec![]);
                if cause == "browser" {
                    rack.set_shortcuts_enabled(false);
                }
                if cause == "editor" {
                    rack.editing = Some(ParameterEdit {
                        id: "a".into(),
                        kind: "constant".into(),
                        parameter: "value".into(),
                        revision: 0,
                        surface: ParameterEditSurface::Window,
                        menu_popup: None,
                        stale: false,
                        text: "0".into(),
                        min: -10.0,
                        max: 10.0,
                        step: None,
                        display_scale: 1.0,
                        display_unit: String::new(),
                        slew_min: None,
                        gain_power: None,
                        raw_max: 10.0,
                        focus: true,
                    });
                }
                if cause == "cable" {
                    rack.pending = Some(PendingCable::new(
                        CableEnd::Output {
                            node: "a".into(),
                            port: "out".into(),
                        },
                        &patch,
                    ));
                }
                if cause == "drag" {
                    rack.group_drag = Some(GroupDrag {
                        anchor: "a".into(),
                        origins: BTreeMap::from([("a".into(), rack.positions["a"])]),
                        delta: Vec2::ZERO,
                        revision: 0,
                    });
                }
                let action = frame_state(
                    &ctx,
                    &mut rack,
                    &patch,
                    1.0,
                    vec![selection_key(key, false)],
                    cause != "focus",
                    cause != "modal",
                )
                .0;
                assert!(action.is_none(), "{key:?}: {cause}");
                assert_eq!(rack.selected_ids(), vec!["a"]);
            }
        }
    }

    #[test]
    fn initialize_and_randomize_shortcuts_dispatch_once_for_selected_group() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch: Patch = serde_json::from_value(json!({
            "version":2,
            "nodes":[{"id":"a","kind":"constant"},{"id":"b","kind":"constant"}]
        }))
        .unwrap();
        rack.select_only("a".into());
        rack.toggle_selection("b".into());
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            0.0,
            vec![egui::Event::PointerMoved(pos2(84.0, 300.0))],
            true,
            true,
            17,
        );
        for (time, key, name) in [
            (1.0, egui::Key::I, "initialize_modules"),
            (2.0, egui::Key::R, "randomize_modules"),
        ] {
            let action = frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                time,
                vec![selection_key(key, false)],
                true,
                true,
                17,
            )
            .0;
            match action {
                Some(RackAction::Command(tool, args)) => {
                    assert_eq!(tool, name);
                    assert_eq!(args["ids"], json!(["a", "b"]));
                    assert_eq!(args["expected_revision"], 17);
                    assert_eq!(args.get("seed").is_some(), key == egui::Key::R);
                    if key == egui::Key::R {
                        assert!(args["seed"]
                            .as_u64()
                            .is_some_and(|seed| seed <= u32::MAX as u64));
                    }
                }
                _ => panic!("{key:?} did not issue a single module-state command"),
            }
            assert_eq!(rack.selected_ids(), vec!["a", "b"]);
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                time + 0.1,
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                true,
                true,
                17,
            );
        }
    }

    #[test]
    fn module_state_shortcuts_follow_hover_without_changing_selection() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch: Patch = serde_json::from_value(json!({
            "version":2,
            "nodes":[{"id":"a","kind":"constant"},{"id":"b","kind":"constant"}]
        }))
        .unwrap();
        rack.select_only("a".into());
        frame(
            &ctx,
            &mut rack,
            &patch,
            0.0,
            vec![egui::Event::PointerMoved(pos2(204.0, 300.0))],
        );
        let action = frame(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![selection_key(egui::Key::I, false)],
        );
        assert!(
            matches!(action, Some(RackAction::Command("initialize_modules", ref args))
            if args["ids"] == json!(["b"]))
        );
        assert_eq!(rack.selected_ids(), vec!["a"]);

        rack.selection.clear();
        rack.selected = None;
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![egui::Event::Key {
                key: egui::Key::I,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let action = frame(
            &ctx,
            &mut rack,
            &patch,
            2.0,
            vec![selection_key(egui::Key::R, false)],
        );
        assert!(
            matches!(action, Some(RackAction::Command("randomize_modules", ref args))
            if args["ids"] == json!(["b"]) && args["seed"].as_u64().is_some())
        );
        assert!(rack.selected_ids().is_empty());

        frame(
            &ctx,
            &mut rack,
            &patch,
            2.1,
            vec![
                egui::Event::Key {
                    key: egui::Key::R,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerMoved(pos2(1200.0, 900.0)),
            ],
        );
        assert!(frame(
            &ctx,
            &mut rack,
            &patch,
            3.0,
            vec![selection_key(egui::Key::I, false)]
        )
        .is_none());
    }

    #[test]
    fn vca_mix_and_fade_all_jacks_and_controls_have_independent_targets() {
        let catalog = openpatch_core::catalog();
        for kind in ["vca_mix", "fade"] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch:Patch=serde_json::from_value(json!({"version":2,"nodes":[{"id":"utility","kind":kind},{"id":"source","kind":"constant"},{"id":"sink","kind":"audio_out"}]})).unwrap();
            let module = catalog["modules"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["kind"] == kind)
                .unwrap();
            let inputs = module["inputs"].as_array().unwrap();
            let outputs = module["outputs"].as_array().unwrap();
            let width = panel_width(kind);
            let origin = pos2(24.0, 12.0);
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            for (index, input) in inputs.iter().enumerate() {
                let target =
                    origin + port_position(kind, true, index, inputs.len(), width).to_vec2();
                let action = drag(
                    &ctx,
                    &mut rack,
                    &patch,
                    1.0 + index as f64,
                    pos2(24.0 + width + 60.0, 386.0),
                    target,
                );
                assert!(
                    matches!(action,Some(RackAction::Command("connect",ref args)) if &args["input"]==input),
                    "{kind} input {input}"
                );
            }
            for (index, output) in outputs.iter().enumerate() {
                let target =
                    origin + port_position(kind, false, index, outputs.len(), width).to_vec2();
                let action = drag(
                    &ctx,
                    &mut rack,
                    &patch,
                    15.0 + index as f64,
                    target,
                    pos2(24.0 + width + 180.0, 344.0),
                );
                assert!(
                    matches!(action,Some(RackAction::Command("connect",ref args)) if &args["output"]==output),
                    "{kind} output {output}"
                );
            }
            let controls: Vec<_> = if kind == "vca_mix" {
                (0..5)
                    .map(|i| {
                        (
                            pos2(
                                if i == 4 {
                                    345.0
                                } else {
                                    37.5 + i as f32 * 75.0
                                },
                                126.0,
                            ),
                            if i == 4 {
                                "mix_level".into()
                            } else {
                                format!("level{}", i + 1)
                            },
                        )
                    })
                    .collect()
            } else {
                vec![
                    (pos2(120.0, 121.0), "position".into()),
                    (pos2(120.0, 236.0), "cv_amount".into()),
                ]
            };
            for (index, (point, parameter)) in controls.iter().enumerate() {
                let knob = origin + point.to_vec2();
                frame(
                    &ctx,
                    &mut rack,
                    &patch,
                    25.0 + index as f64,
                    vec![egui::Event::PointerMoved(knob), button(knob, true)],
                );
                let moved = knob - vec2(0.0, 10.0);
                let action = frame(
                    &ctx,
                    &mut rack,
                    &patch,
                    25.1 + index as f64,
                    vec![egui::Event::PointerMoved(moved)],
                );
                assert!(
                    matches!(action,Some(RackAction::Command("set_parameter",ref args)) if args["parameter"]==*parameter),
                    "{kind} knob {parameter}"
                );
                frame(
                    &ctx,
                    &mut rack,
                    &patch,
                    25.2 + index as f64,
                    vec![button(moved, false)],
                );
            }
            let ports: Vec<_> = (0..inputs.len())
                .map(|i| port_position(kind, true, i, inputs.len(), width))
                .chain(
                    (0..outputs.len()).map(|i| port_position(kind, false, i, outputs.len(), width)),
                )
                .collect();
            for (index, point) in ports.iter().enumerate() {
                assert!(point.x >= 20.0 && point.x <= width - 20.0 && point.y <= 374.0);
                for other in &ports[index + 1..] {
                    assert!(point.distance(*other) >= 29.0);
                }
            }
        }
    }

    #[test]
    fn mixing_context_settings_emit_saved_parameter_edits() {
        for (kind, label, parameter) in [
            ("vca_mix", "Exponential channel CV", "channel_exp"),
            ("vca_mix", "Exponential mix CV", "mix_exp"),
            ("fade", "Square-root law", "law"),
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch: Patch =
                serde_json::from_value(json!({"version":2,"nodes":[{"id":"utility","kind":kind}]}))
                    .unwrap();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let header = pos2(144.0, 44.0);
            for (time, pressed) in [(1.0, true), (1.1, false)] {
                frame(
                    &ctx,
                    &mut rack,
                    &patch,
                    time,
                    vec![
                        egui::Event::PointerMoved(header),
                        egui::Event::PointerButton {
                            pos: header,
                            button: PointerButton::Secondary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            let (_, output) = frame_output(&ctx, &mut rack, &patch, 1.2, vec![]);
            let target = output
                .shapes
                .iter()
                .find_map(|s| {
                    if let egui::epaint::Shape::Text(t) = &s.shape {
                        (t.galley.text() == label).then_some(t.pos + t.galley.size() * 0.5)
                    } else {
                        None
                    }
                })
                .expect("context setting label");
            frame(
                &ctx,
                &mut rack,
                &patch,
                1.3,
                vec![egui::Event::PointerMoved(target), button(target, true)],
            );
            let action = frame(&ctx, &mut rack, &patch, 1.4, vec![button(target, false)]);
            assert!(
                matches!(action,Some(RackAction::Command("set_parameter",ref args)) if args["parameter"]==parameter && args["value"]==1.0)
            );
        }
    }

    #[test]
    fn mixer_decibel_editor_handles_squared_channel_gain_master_and_zero() {
        assert_eq!(gain_db_display(0.0, 2.0), "-inf dB");
        assert!((gain_db(0.5, 2.0) + 12.0412).abs() < 0.0001);
        assert!((gain_db(0.5, 1.0) + 6.0206).abs() < 0.0001);
        for (parameter, x, power, text, expected) in [
            ("level4", 262.5, 2.0, "-inf", 0.0),
            ("level2", 112.5, 2.0, "−inf", 0.0),
            ("level1", 37.5, 2.0, "-12.0412", 0.5),
            (
                "level3",
                187.5,
                2.0,
                "+6.02dB",
                std::f32::consts::SQRT_2 as f64,
            ),
            (
                "level3",
                187.5,
                2.0,
                "+6.0206 dB",
                std::f32::consts::SQRT_2 as f64,
            ),
            ("mix_level", 345.0, 1.0, "-6.0206", 0.5),
            ("mix_level", 345.0, 1.0, "+6.0206", 2.0),
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch: Patch = serde_json::from_value(
                json!({"version":2,"nodes":[{"id":"mix","kind":"vca_mix"}]}),
            )
            .unwrap();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let target = pos2(
                24.0 + x,
                12.0 + 126.0 + if parameter == "mix_level" { 38.0 } else { 33.0 },
            );
            for (time, pressed) in [(1.0, true), (1.04, false), (1.08, true), (1.12, false)] {
                frame(
                    &ctx,
                    &mut rack,
                    &patch,
                    time,
                    vec![egui::Event::PointerMoved(target), button(target, pressed)],
                );
            }
            let edit = rack.editing.as_mut().expect("decibel value opens editor");
            assert_eq!(edit.parameter, parameter);
            assert_eq!(edit.gain_power, Some(power));
            edit.text = text.into();
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
                matches!(action,Some(RackAction::Command("set_parameter",ref args)) if (args["value"].as_f64().unwrap()-expected).abs()<0.0001)
            );
        }
    }

    #[test]
    fn signal_utility_first_last_ports_and_knobs_have_distinct_targets() {
        let catalog = openpatch_core::catalog();
        for kind in ["logic", "compare", "process", "rescale"] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch: Patch=serde_json::from_value(json!({"version":2,"nodes":[{"id":"utility","kind":kind},{"id":"source","kind":"constant"},{"id":"sink","kind":"audio_out"}]})).unwrap();
            let module = catalog["modules"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["kind"] == kind)
                .unwrap();
            let inputs = module["inputs"].as_array().unwrap();
            let outputs = module["outputs"].as_array().unwrap();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            for (iteration, index) in [0, inputs.len() - 1].into_iter().enumerate() {
                let target = pos2(24.0, 12.0)
                    + port_position(kind, true, index, inputs.len(), 240.0).to_vec2();
                let action = drag(
                    &ctx,
                    &mut rack,
                    &patch,
                    1.0 + iteration as f64,
                    pos2(324.0, 386.0),
                    target,
                );
                assert!(
                    matches!(action,Some(RackAction::Command("connect",ref args)) if args["input"] == inputs[index]),
                    "{kind} input {index}"
                );
            }
            for (iteration, index) in [0, outputs.len() - 1].into_iter().enumerate() {
                let target = pos2(24.0, 12.0)
                    + port_position(kind, false, index, outputs.len(), 240.0).to_vec2();
                let action = drag(
                    &ctx,
                    &mut rack,
                    &patch,
                    4.0 + iteration as f64,
                    target,
                    pos2(444.0, 344.0),
                );
                assert!(
                    matches!(action,Some(RackAction::Command("connect",ref args)) if args["output"] == outputs[index]),
                    "{kind} output {index}"
                );
            }
            if kind != "logic" {
                let (knob, parameter) = match kind {
                    "compare" => (pos2(144.0, 137.0), "b_offset"),
                    "process" => (pos2(144.0, 130.0), "slew"),
                    _ => (pos2(84.0, 139.0), "gain"),
                };
                frame(
                    &ctx,
                    &mut rack,
                    &patch,
                    8.0,
                    vec![egui::Event::PointerMoved(knob), button(knob, true)],
                );
                let action = frame(
                    &ctx,
                    &mut rack,
                    &patch,
                    8.1,
                    vec![egui::Event::PointerMoved(knob - vec2(0.0, 12.0))],
                );
                assert!(
                    matches!(action,Some(RackAction::Command("set_parameter",ref args)) if args["parameter"]==parameter),
                    "{kind} knob"
                );
            }
            let points: Vec<_> = (0..inputs.len())
                .map(|i| port_position(kind, true, i, inputs.len(), 240.0))
                .chain(
                    (0..outputs.len()).map(|i| port_position(kind, false, i, outputs.len(), 240.0)),
                )
                .collect();
            for (index, a) in points.iter().enumerate() {
                assert!((20.0..=376.0).contains(&a.y));
                for b in &points[index + 1..] {
                    assert!(a.distance(*b) >= 29.0, "{kind} overlapping ports");
                }
            }
        }
    }

    #[test]
    fn momentary_buttons_release_outside_focus_modal_bypass_removal_and_engine_loss() {
        for kind in ["logic", "process"] {
            for cause in [
                "outside",
                "focus",
                "modal",
                "bypass",
                "removal",
                "engine",
                "retype",
                "up-and-down",
            ] {
                let ctx = egui::Context::default();
                let mut rack = Rack::default();
                rack.set_sequence_states(BTreeMap::new(), true);
                let mut patch: Patch = serde_json::from_value(
                    json!({"version":2,"nodes":[{"id":"manual","kind":kind}]}),
                )
                .unwrap();
                let target = pos2(24.0, 12.0)
                    + if kind == "logic" {
                        vec2(180.0, 170.0)
                    } else {
                        vec2(120.0, 185.0)
                    };
                frame(&ctx, &mut rack, &patch, 0.0, vec![]);
                let action = frame(
                    &ctx,
                    &mut rack,
                    &patch,
                    1.0,
                    vec![egui::Event::PointerMoved(target), button(target, true)],
                );
                assert!(
                    matches!(action,Some(RackAction::Command("trigger_module",ref args)) if args["action"]==if kind=="logic"{"b_press"}else{"gate_press"}),
                    "{kind} {cause} press"
                );
                assert!(rack.held_manual.is_some());
                assert!(patch.nodes[0].params.is_empty());
                assert!(
                    frame(&ctx, &mut rack, &patch, 1.1, vec![]).is_none(),
                    "Holding never repeats presses"
                );
                let mut events = vec![];
                let (focused, enabled) = match cause {
                    "outside" => {
                        let outside = pos2(1250.0, 850.0);
                        events = vec![egui::Event::PointerMoved(outside), button(outside, false)];
                        (true, true)
                    }
                    "focus" => {
                        events.push(egui::Event::WindowFocused(false));
                        (false, true)
                    }
                    "modal" => (true, false),
                    "bypass" => {
                        patch.nodes[0].bypassed = true;
                        (true, true)
                    }
                    "removal" => {
                        patch.nodes.clear();
                        (true, true)
                    }
                    "engine" => {
                        rack.set_sequence_states(BTreeMap::new(), false);
                        (true, true)
                    }
                    "retype" => {
                        patch.nodes[0].kind = "rescale".into();
                        (true, true)
                    }
                    _ => {
                        events = vec![button(target, false), button(target, true)];
                        (true, true)
                    }
                };
                let action = frame_state(&ctx, &mut rack, &patch, 1.2, events, focused, enabled).0;
                assert!(
                    matches!(action,Some(RackAction::Command("trigger_module",ref args)) if args["id"]=="manual" && args["action"]==if kind=="logic"{"b_release"}else{"gate_release"}),
                    "{kind} {cause} release"
                );
                assert!(rack.held_manual.is_none(), "{kind} {cause}");
                assert!(
                    frame_state(&ctx, &mut rack, &patch, 1.3, vec![], focused, enabled)
                        .0
                        .is_none(),
                    "Release is emitted once"
                );
            }
        }
    }

    #[test]
    fn pulses_first_and_tenth_buttons_and_jacks_use_their_own_rows() {
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[
                {"id":"manual","kind":"pulses"},
                {"id":"sink","kind":"gain"}
            ]
        }))
        .unwrap();
        for (row, output_name) in [(0, "trig1"), (9, "gate10")] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            rack.set_sequence_states(BTreeMap::new(), true);
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let button_center = pos2(24.0 + 92.0, 12.0 + 100.0 + row as f32 * 29.0);
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![
                    egui::Event::PointerMoved(button_center),
                    button(button_center, true),
                ],
            );
            assert!(matches!(action,
                Some(RackAction::Command("trigger_module", ref args))
                    if args["id"] == "manual" && args["action"] == PULSE_PRESS[row]
            ));
            assert_eq!(
                rack.held_manual.as_ref().unwrap().release,
                PULSE_RELEASE[row]
            );
            assert!(patch.nodes[0].params.is_empty());
            assert!(frame(&ctx, &mut rack, &patch, 1.1, vec![]).is_none());
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                1.2,
                vec![button(button_center, false)],
            );
            assert!(matches!(action,
                Some(RackAction::Command("trigger_module", ref args))
                    if args["action"] == PULSE_RELEASE[row]
            ));
            let source = fixture_jack(&rack, &patch, "manual", output_name, false);
            let sink = fixture_jack(&rack, &patch, "sink", "signal", true);
            let action = drag(&ctx, &mut rack, &patch, 2.0, source, sink);
            assert!(matches!(action,
                Some(RackAction::Command("connect", ref args))
                    if args["from"] == "manual" && args["output"] == output_name
                    && args["to"] == "sink" && args["input"] == "signal"
            ));
        }
    }

    #[test]
    fn pulses_same_frame_pointer_click_sends_ordered_press_and_release_without_hold() {
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"manual","kind":"pulses"}]
        }))
        .unwrap();
        for row in [0, 9] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            rack.set_sequence_states(BTreeMap::new(), true);
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let target = pos2(116.0, 112.0 + row as f32 * 29.0);
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![
                    egui::Event::PointerMoved(target),
                    button(target, true),
                    button(target, false),
                ],
            );
            assert!(matches!(action,
                Some(RackAction::ManualClick { id, press, release })
                    if id == "manual" && press == PULSE_PRESS[row] && release == PULSE_RELEASE[row]
            ));
            assert!(rack.held_manual.is_none());
            assert!(frame(&ctx, &mut rack, &patch, 1.1, vec![]).is_none());
            assert!(patch.nodes[0].params.is_empty());
        }
    }

    #[test]
    fn pulses_held_row_releases_on_exit_focus_modal_bypass_removal_and_engine_loss() {
        for cause in [
            "exit",
            "focus",
            "modal",
            "shortcut-modal",
            "bypass",
            "removal",
            "engine",
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            rack.set_sequence_states(BTreeMap::new(), true);
            let mut patch: Patch = serde_json::from_value(json!({
                "version":2,"nodes":[{"id":"manual","kind":"pulses"}]
            }))
            .unwrap();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let target = pos2(116.0, 112.0 + 9.0 * 29.0);
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![egui::Event::PointerMoved(target), button(target, true)],
            );
            assert!(matches!(action,
                Some(RackAction::Command("trigger_module", ref args))
                    if args["action"] == "pulse10_press"
            ));
            let mut events = vec![];
            let (focused, enabled) = match cause {
                "exit" => {
                    events.push(egui::Event::PointerMoved(target + vec2(70.0, 0.0)));
                    (true, true)
                }
                "focus" => {
                    events.push(egui::Event::WindowFocused(false));
                    (false, true)
                }
                "modal" => (true, false),
                "shortcut-modal" => {
                    rack.set_shortcuts_enabled(false);
                    (true, true)
                }
                "bypass" => {
                    patch.nodes[0].bypassed = true;
                    (true, true)
                }
                "removal" => {
                    patch.nodes.clear();
                    (true, true)
                }
                _ => {
                    rack.set_sequence_states(BTreeMap::new(), false);
                    (true, true)
                }
            };
            let action = frame_state(&ctx, &mut rack, &patch, 1.1, events, focused, enabled).0;
            assert!(
                matches!(action,
                    Some(RackAction::Command("trigger_module", ref args))
                        if args["id"] == "manual" && args["action"] == "pulse10_release"
                ),
                "{cause}"
            );
            assert!(rack.held_manual.is_none(), "{cause}");
        }
    }

    #[test]
    fn pulses_uses_each_telemetry_bit_instead_of_any_held_state() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        // A stopped engine still paints its last telemetry; it only disables presses.
        rack.module_controls.insert(
            "manual".into(),
            ModuleControlState {
                pressed: true,
                pressed_mask: 1 << 9,
            },
        );
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"manual","kind":"pulses"}]
        }))
        .unwrap();
        let (_, output) = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]);
        let active = Color32::from_rgb(212, 166, 86);
        let row_fill = |row: usize| {
            let center = pos2(116.0, 112.0 + row as f32 * 29.0);
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::epaint::Shape::Rect(rect)
                    if rect.rect.center() == center && rect.fill == active)
            })
        };
        assert!(!row_fill(0));
        assert!(row_fill(9));
    }

    #[test]
    fn push_buttons_and_four_jacks_have_independent_targets_without_parameter_edits() {
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[
                {"id":"src","kind":"constant"},
                {"id":"manual","kind":"push","params":{"holding":1}},
                {"id":"sink","kind":"gain"}
            ]
        }))
        .unwrap();
        for (index, input_name, output_name) in [(0, "push", "trig"), (1, "hold", "gate")] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            rack.set_sequence_states(BTreeMap::new(), true);
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            assert_eq!(panel_width("push"), 180.0);
            let center = pos2(24.0 + 120.0 + if index == 0 { 49.0 } else { 131.0 }, 221.0);
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![egui::Event::PointerMoved(center), button(center, true)],
            );
            assert!(matches!(action,
                Some(RackAction::Command("trigger_module", ref args))
                    if args["id"] == "manual" && args["action"] == PUSH_PRESS[index]
            ));
            assert_eq!(
                rack.held_manual.as_ref().unwrap().release,
                PUSH_RELEASE[index]
            );
            let action = frame(&ctx, &mut rack, &patch, 1.1, vec![button(center, false)]);
            assert!(matches!(action,
                Some(RackAction::Command("trigger_module", ref args))
                    if args["action"] == PUSH_RELEASE[index]
            ));
            assert_eq!(patch.nodes[1].params["holding"], 1.0);
            let source = fixture_jack(&rack, &patch, "src", "out", false);
            let input = fixture_jack(&rack, &patch, "manual", input_name, true);
            let action = drag(&ctx, &mut rack, &patch, 2.0, source, input);
            assert!(matches!(action,
                Some(RackAction::Command("connect", ref args))
                    if args["to"] == "manual" && args["input"] == input_name
            ));
            let output = fixture_jack(&rack, &patch, "manual", output_name, false);
            let sink = fixture_jack(&rack, &patch, "sink", "signal", true);
            let action = drag(&ctx, &mut rack, &patch, 3.0, output, sink);
            assert!(matches!(action,
                Some(RackAction::Command("connect", ref args))
                    if args["from"] == "manual" && args["output"] == output_name
                    && args["to"] == "sink"
            ));
        }
    }

    #[test]
    fn push_same_frame_click_pairs_actions_without_held_state_or_retrigger() {
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"manual","kind":"push"}]
        }))
        .unwrap();
        for index in 0..2 {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            rack.set_sequence_states(BTreeMap::new(), true);
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let target = pos2(24.0 + if index == 0 { 49.0 } else { 131.0 }, 221.0);
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![
                    egui::Event::PointerMoved(target),
                    button(target, true),
                    button(target, false),
                ],
            );
            assert!(matches!(action,
                Some(RackAction::ManualClick { id, press, release })
                    if id == "manual" && press == PUSH_PRESS[index] && release == PUSH_RELEASE[index]
            ));
            assert!(rack.held_manual.is_none());
            assert!(frame(&ctx, &mut rack, &patch, 1.1, vec![]).is_none());
        }
    }

    #[test]
    fn push_releases_each_button_on_exit_focus_modal_bypass_and_removal() {
        for index in 0..2 {
            for cause in [
                "exit",
                "focus",
                "modal",
                "shortcut-modal",
                "bypass",
                "removal",
            ] {
                let ctx = egui::Context::default();
                let mut rack = Rack::default();
                rack.set_sequence_states(BTreeMap::new(), true);
                let mut patch: Patch = serde_json::from_value(json!({
                    "version":2,"nodes":[{"id":"manual","kind":"push"}]
                }))
                .unwrap();
                frame(&ctx, &mut rack, &patch, 0.0, vec![]);
                let target = pos2(24.0 + if index == 0 { 49.0 } else { 131.0 }, 221.0);
                let action = frame(
                    &ctx,
                    &mut rack,
                    &patch,
                    1.0,
                    vec![egui::Event::PointerMoved(target), button(target, true)],
                );
                assert!(matches!(action,
                    Some(RackAction::Command("trigger_module", ref args))
                        if args["action"] == PUSH_PRESS[index]
                ));
                let mut events = vec![];
                let (focused, enabled) = match cause {
                    "exit" => {
                        events.push(egui::Event::PointerMoved(target + vec2(0.0, 70.0)));
                        (true, true)
                    }
                    "focus" => {
                        events.push(egui::Event::WindowFocused(false));
                        (false, true)
                    }
                    "modal" => (true, false),
                    "shortcut-modal" => {
                        rack.set_shortcuts_enabled(false);
                        (true, true)
                    }
                    "bypass" => {
                        patch.nodes[0].bypassed = true;
                        (true, true)
                    }
                    _ => {
                        patch.nodes.clear();
                        (true, true)
                    }
                };
                let action = frame_state(&ctx, &mut rack, &patch, 1.1, events, focused, enabled).0;
                assert!(
                    matches!(action,
                        Some(RackAction::Command("trigger_module", ref args))
                            if args["id"] == "manual" && args["action"] == PUSH_RELEASE[index]
                    ),
                    "button {index} {cause}"
                );
                assert!(rack.held_manual.is_none());
            }
        }
    }

    #[test]
    fn push_offline_telemetry_separates_pressed_buttons_from_gate_and_latch() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        rack.module_controls.insert(
            "manual".into(),
            ModuleControlState {
                pressed: true,
                pressed_mask: 1,
            },
        );
        let mut lights = ModuleLightState {
            count: 2,
            ..ModuleLightState::default()
        };
        lights.brightness[1][0] = 1.0;
        rack.module_lights.insert("manual".into(), lights);
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"manual","kind":"push","params":{"holding":0}}]
        }))
        .unwrap();
        let (_, output) = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]);
        let active = Color32::from_rgb(212, 166, 86);
        let active_button = |x: f32| {
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::epaint::Shape::Rect(rect)
                    if rect.rect.center() == pos2(24.0 + x, 221.0) && rect.fill == active)
            })
        };
        assert!(active_button(49.0));
        assert!(!active_button(131.0));
        let lamp_fill = |x: f32, color: Color32| {
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::epaint::Shape::Circle(circle)
                    if circle.center == pos2(24.0 + x, 277.0) && circle.fill == color)
            })
        };
        assert!(lamp_fill(
            49.0,
            Color32::from_rgb(238, 177, 65).gamma_multiply(0.12)
        ));
        assert!(lamp_fill(131.0, Color32::from_rgb(75, 165, 178)));
    }

    #[test]
    fn gates_reset_and_each_named_jack_have_independent_pointer_targets() {
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[
                {"id":"source","kind":"constant"},
                {"id":"gates","kind":"gates","params":{"length":0.1}},
                {"id":"sink","kind":"gain"}
            ]
        }))
        .unwrap();
        assert_eq!(panel_width("gates"), 240.0);
        assert!(logarithmic_parameter("gates", "length"));
        assert_eq!(title("gates"), "GATES");
        for (input, ports) in [
            (true, &["length", "in", "reset"][..]),
            (
                false,
                &["rise", "fall", "flip", "flop", "gate", "delay"][..],
            ),
        ] {
            let centers = ports
                .iter()
                .enumerate()
                .map(|(index, _)| port_position("gates", input, index, ports.len(), 240.0))
                .collect::<Vec<_>>();
            for (index, center) in centers.iter().enumerate() {
                assert!(centers[..index]
                    .iter()
                    .all(|other| center.distance(*other) > 29.0));
            }
        }
        for input_name in ["length", "in", "reset"] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let source = fixture_jack(&rack, &patch, "source", "out", false);
            let target = fixture_jack(&rack, &patch, "gates", input_name, true);
            let action = drag(&ctx, &mut rack, &patch, 1.0, source, target);
            assert!(matches!(action,
                Some(RackAction::Command("connect", ref args))
                    if args["from"] == "source" && args["to"] == "gates"
                    && args["input"] == input_name
            ));
        }
        for output_name in ["rise", "fall", "flip", "flop", "gate", "delay"] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let source = fixture_jack(&rack, &patch, "gates", output_name, false);
            let target = fixture_jack(&rack, &patch, "sink", "signal", true);
            let action = drag(&ctx, &mut rack, &patch, 1.0, source, target);
            assert!(matches!(action,
                Some(RackAction::Command("connect", ref args))
                    if args["from"] == "gates" && args["output"] == output_name
                    && args["to"] == "sink"
            ));
        }
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        rack.set_sequence_states(BTreeMap::new(), true);
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        // The reset button is at panel-local (184, 182), separate from the IN jack.
        let target = fixture_jack(&rack, &patch, "gates", "in", true) + vec2(64.0, 77.0);
        let action = frame(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(target), button(target, true)],
        );
        assert!(matches!(action,
            Some(RackAction::Command("trigger_module", ref args))
                if args["id"] == "gates" && args["action"] == "reset_press"
        ));
        assert_eq!(rack.held_manual.as_ref().unwrap().release, "reset_release");
        assert_eq!(patch.nodes[1].params["length"], 0.1);
        let action = frame(&ctx, &mut rack, &patch, 1.1, vec![button(target, false)]);
        assert!(matches!(action,
            Some(RackAction::Command("trigger_module", ref args))
                if args["action"] == "reset_release"
        ));
    }

    #[test]
    fn gates_fast_reset_click_pairs_actions_without_held_state() {
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"manual","kind":"gates"}]
        }))
        .unwrap();
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        rack.set_sequence_states(BTreeMap::new(), true);
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let target = pos2(24.0 + 184.0, 12.0 + 182.0);
        let action = frame(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![
                egui::Event::PointerMoved(target),
                button(target, true),
                button(target, false),
            ],
        );
        assert!(matches!(action,
            Some(RackAction::ManualClick { id, press, release })
                if id == "manual" && press == "reset_press" && release == "reset_release"
        ));
        assert!(rack.held_manual.is_none());
        assert!(frame(&ctx, &mut rack, &patch, 1.1, vec![]).is_none());
    }

    #[test]
    fn gates_reset_releases_on_exit_focus_modal_bypass_removal_and_engine_loss() {
        for cause in [
            "exit",
            "focus",
            "modal",
            "shortcut-modal",
            "bypass",
            "removal",
            "engine",
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            rack.set_sequence_states(BTreeMap::new(), true);
            let mut patch: Patch = serde_json::from_value(json!({
                "version":2,"nodes":[{"id":"manual","kind":"gates"}]
            }))
            .unwrap();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let target = pos2(24.0 + 184.0, 12.0 + 182.0);
            let action = frame(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![egui::Event::PointerMoved(target), button(target, true)],
            );
            assert!(matches!(action,
                Some(RackAction::Command("trigger_module", ref args))
                    if args["action"] == "reset_press"
            ));
            let mut events = vec![];
            let (focused, enabled) = match cause {
                "exit" => {
                    events.push(egui::Event::PointerMoved(target + vec2(90.0, 0.0)));
                    (true, true)
                }
                "focus" => {
                    events.push(egui::Event::WindowFocused(false));
                    (false, true)
                }
                "modal" => (true, false),
                "shortcut-modal" => {
                    rack.set_shortcuts_enabled(false);
                    (true, true)
                }
                "bypass" => {
                    patch.nodes[0].bypassed = true;
                    (true, true)
                }
                "removal" => {
                    patch.nodes.clear();
                    (true, true)
                }
                _ => {
                    rack.set_sequence_states(BTreeMap::new(), false);
                    (true, true)
                }
            };
            let action = frame_state(&ctx, &mut rack, &patch, 1.1, events, focused, enabled).0;
            assert!(
                matches!(action,
                    Some(RackAction::Command("trigger_module", ref args))
                        if args["id"] == "manual" && args["action"] == "reset_release"
                ),
                "{cause}"
            );
            assert!(rack.held_manual.is_none());
        }
    }

    #[test]
    fn gates_telemetry_uses_reset_mask_and_each_output_light_pair() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        rack.module_controls.insert(
            "manual".into(),
            ModuleControlState {
                pressed: true,
                pressed_mask: 0,
            },
        );
        let mut lights = ModuleLightState {
            count: 6,
            ..ModuleLightState::default()
        };
        lights.brightness[1][1] = 0.7;
        lights.brightness[5][0] = 1.0;
        rack.module_lights.insert("manual".into(), lights);
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"manual","kind":"gates"}]
        }))
        .unwrap();
        let (_, output) = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]);
        assert!(!output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::epaint::Shape::Rect(rect)
                if rect.rect.center() == pos2(208.0, 194.0)
                && rect.fill == Color32::from_rgb(212, 166, 86))
        }));
        for (index, channel, strength) in [(1, 1, 0.7), (5, 0, 1.0)] {
            let jack = port_position("gates", false, index, 6, 240.0);
            let point = pos2(
                24.0 + jack.x + if channel == 0 { -3.5 } else { 3.5 },
                12.0 + jack.y + 17.0,
            );
            let base = if channel == 0 {
                Color32::from_rgb(242, 199, 59)
            } else {
                Color32::from_rgb(62, 155, 238)
            };
            assert!(
                output.shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::epaint::Shape::Circle(circle)
                    if circle.center == point
                    && circle.fill == base.gamma_multiply(0.12 + strength * 0.88))
                }),
                "output light {index}, channel {channel}"
            );
        }
    }

    #[test]
    fn unavailable_or_bypassed_momentary_buttons_do_not_press_and_show_status() {
        for (available, bypassed) in [(false, false), (true, true)] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            rack.set_sequence_states(BTreeMap::new(), available);
            let patch: Patch = serde_json::from_value(
                json!({"version":2,"nodes":[{"id":"logic","kind":"logic","bypassed":bypassed}]}),
            )
            .unwrap();
            let (_, output) = frame_output(&ctx, &mut rack, &patch, 0.0, vec![]);
            assert!(output.shapes.iter().any(
                |s| matches!(&s.shape,egui::epaint::Shape::Text(t) if t.galley.text()=="B: OFFLINE")
            ));
            let target = pos2(204.0, 182.0);
            assert!(frame(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![egui::Event::PointerMoved(target), button(target, true)]
            )
            .is_none());
            assert!(rack.held_manual.is_none());
        }
    }

    #[test]
    fn process_and_rescale_numeric_editors_convert_display_units() {
        for (kind, multiplier, scale, initial, typed, expected) in [
            ("process", 1.0, 1.0, 0.0, "2000", 1.0),
            ("rescale", 1.0, 100.0, 10.0, "20", 0.2),
            ("rescale", 10.0, 10.0, 1.0, "2", 0.2),
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let params = if kind == "rescale" {
                json!({"gain":0.1,"multiplier":multiplier})
            } else {
                json!({})
            };
            let patch: Patch = serde_json::from_value(
                json!({"version":2,"nodes":[{"id":"utility","kind":kind,"params":params}]}),
            )
            .unwrap();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let target = if kind == "process" {
                pos2(144.0, 170.0)
            } else {
                pos2(84.0, 173.0)
            };
            for (time, pressed) in [(1.0, true), (1.04, false), (1.08, true), (1.12, false)] {
                frame(
                    &ctx,
                    &mut rack,
                    &patch,
                    time,
                    vec![egui::Event::PointerMoved(target), button(target, pressed)],
                );
            }
            let edit = rack.editing.as_mut().expect("value opens numeric editor");
            assert_eq!(edit.display_scale, scale);
            assert!((edit.text.parse::<f32>().unwrap() - initial).abs() < 0.0001);
            if kind == "process" {
                assert_eq!(edit.min, 0.0);
                assert!(edit.slew_min.is_some());
            }
            edit.text = typed.into();
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
                matches!(action,Some(RackAction::Command("set_parameter",ref args)) if (args["value"].as_f64().unwrap()-expected).abs()<0.0001)
            );
        }
    }

    #[test]
    fn utility_end_rows_edit_and_patch_without_overlapping_controls() {
        for (kind, rows) in [("attenuverter8", 8), ("mutes10", 10), ("cv_mix", 3)] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
                {"id":"utility","kind":kind,"bypassed":true},
                {"id":"source","kind":"constant"},
                {"id":"sink","kind":"audio_out"}]}))
            .unwrap();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let origin = pos2(24.0, 12.0);
            for (iteration, row) in [0, rows - 1].into_iter().enumerate() {
                let time = 1.0 + iteration as f64 * 50.0;
                let input = origin + port_position(kind, true, row, rows, 240.0).to_vec2();
                let action = drag(&ctx, &mut rack, &patch, time, pos2(324.0, 386.0), input);
                assert!(
                    matches!(action, Some(RackAction::Command("connect", ref args)) if args["input"] == format!("in{}",row+1)),
                    "{kind} row {} input",
                    row + 1
                );
                let output = origin
                    + port_position(
                        kind,
                        false,
                        if kind == "cv_mix" { 0 } else { row },
                        rows,
                        240.0,
                    )
                    .to_vec2();
                let action = drag(
                    &ctx,
                    &mut rack,
                    &patch,
                    time + 10.0,
                    output,
                    pos2(444.0, 344.0),
                );
                assert!(
                    matches!(action, Some(RackAction::Command("connect", ref args)) if args["output"] == if kind == "cv_mix" {"out".into()} else {format!("out{}",row+1)}),
                    "{kind} row {} output",
                    row + 1
                );
                let control = origin + vec2(116.0, utility_row_y(kind, row));
                if kind == "mutes10" {
                    frame(
                        &ctx,
                        &mut rack,
                        &patch,
                        time + 20.0,
                        vec![egui::Event::PointerMoved(control), button(control, true)],
                    );
                    let action = frame(
                        &ctx,
                        &mut rack,
                        &patch,
                        time + 20.1,
                        vec![button(control, false)],
                    );
                    assert!(
                        matches!(action,Some(RackAction::Command("set_parameter",ref args)) if args["parameter"] == format!("mute{}",row+1) && args["value"] == 1.0)
                    );
                } else {
                    frame(
                        &ctx,
                        &mut rack,
                        &patch,
                        time + 20.0,
                        vec![egui::Event::PointerMoved(control), button(control, true)],
                    );
                    let moved = control + vec2(12.0, 0.0);
                    let action = frame(
                        &ctx,
                        &mut rack,
                        &patch,
                        time + 20.1,
                        vec![egui::Event::PointerMoved(moved)],
                    );
                    assert!(
                        matches!(action,Some(RackAction::Command("set_parameter",ref args)) if args["parameter"] == format!("{}{}",if kind == "cv_mix" {"level"} else {"gain"},row+1) && args["value"].as_f64().unwrap() > 0.0)
                    );
                    frame(
                        &ctx,
                        &mut rack,
                        &patch,
                        time + 20.2,
                        vec![button(moved, false)],
                    );
                }
            }
        }
    }

    #[test]
    fn bypass_and_invert_are_available_from_header_and_panel_menus() {
        for (menu_position, label, command) in [
            (pos2(144.0, 44.0), "Bypass module", "set_bypass"),
            (pos2(144.0, 400.0), "Bypass module", "set_bypass"),
            (pos2(144.0, 44.0), "Invert all mutes", "invert_mutes"),
            (pos2(144.0, 400.0), "Invert all mutes", "invert_mutes"),
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch: Patch = serde_json::from_value(
                json!({"version":2,"nodes":[{"id":"mutes","kind":"mutes10"}]}),
            )
            .unwrap();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            for (time, pressed) in [(1.0, true), (1.1, false)] {
                frame(
                    &ctx,
                    &mut rack,
                    &patch,
                    time,
                    vec![
                        egui::Event::PointerMoved(menu_position),
                        egui::Event::PointerButton {
                            pos: menu_position,
                            button: PointerButton::Secondary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            let (_, output) = frame_output(&ctx, &mut rack, &patch, 1.2, vec![]);
            let target = output
                .shapes
                .iter()
                .find_map(|shape| {
                    if let egui::epaint::Shape::Text(text) = &shape.shape {
                        (text.galley.text() == label).then_some(text.pos + text.galley.size() * 0.5)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| panic!("Missing {label} at {menu_position:?}"));
            frame(
                &ctx,
                &mut rack,
                &patch,
                1.3,
                vec![egui::Event::PointerMoved(target), button(target, true)],
            );
            let action = frame(&ctx, &mut rack, &patch, 1.4, vec![button(target, false)]);
            assert!(
                matches!(action,Some(RackAction::Command(name,ref args)) if name == command && args["id"] == "mutes")
            );
            if let Some(RackAction::Command("set_bypass", args)) = action {
                assert_eq!(args["bypassed"], true);
            }
        }
    }

    #[test]
    fn module_state_menu_actions_use_context_selection_and_observed_revision() {
        for (menu_position, label, command) in [
            (pos2(84.0, 44.0), "Initialize", "initialize_modules"),
            (pos2(84.0, 300.0), "Initialize", "initialize_modules"),
            (pos2(84.0, 44.0), "Randomize", "randomize_modules"),
            (pos2(84.0, 300.0), "Randomize", "randomize_modules"),
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch: Patch = serde_json::from_value(json!({
                "version":2,
                "nodes":[{"id":"a","kind":"constant"},{"id":"b","kind":"constant"}]
            }))
            .unwrap();
            rack.select_only("a".into());
            rack.toggle_selection("b".into());
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 23);
            for (time, pressed) in [(1.0, true), (1.1, false)] {
                frame_revision_state(
                    &ctx,
                    &mut rack,
                    &patch,
                    time,
                    vec![
                        egui::Event::PointerMoved(menu_position),
                        egui::Event::PointerButton {
                            pos: menu_position,
                            button: PointerButton::Secondary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    true,
                    true,
                    23,
                );
            }
            let (_, output) =
                frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![], true, true, 23);
            let target = output
                .shapes
                .iter()
                .find_map(|shape| {
                    if let egui::epaint::Shape::Text(text) = &shape.shape {
                        (text.galley.text() == label).then_some(text.pos + text.galley.size() * 0.5)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| panic!("Missing {label} at {menu_position:?}"));
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.3,
                vec![egui::Event::PointerMoved(target), button(target, true)],
                true,
                true,
                23,
            );
            let action = frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.4,
                vec![button(target, false)],
                true,
                true,
                23,
            )
            .0;
            match action {
                Some(RackAction::Command(name, args)) => {
                    assert_eq!(name, command);
                    assert_eq!(args["ids"], json!(["a", "b"]));
                    assert_eq!(args["expected_revision"], 23);
                    assert_eq!(args.get("seed").is_some(), command == "randomize_modules");
                }
                _ => panic!("{label} did not dispatch a module-state command"),
            }
            assert_eq!(rack.selected_ids(), vec!["a", "b"]);
        }
    }

    #[test]
    fn preset_menu_targets_context_module_and_disables_groups() {
        let patch: Patch = serde_json::from_value(json!({
            "version":2,
            "nodes":[{"id":"a","kind":"constant"},{"id":"b","kind":"seq3"}]
        }))
        .unwrap();
        let mut rack = Rack::default();
        rack.select_only("a".into());
        let ids = rack.context_ids("b");
        let action = preset_action(&patch.nodes[1], &ids, 42, PresetMode::Browse);
        assert!(
            matches!(action, Some(RackAction::Preset { id, kind, revision: 42, mode: PresetMode::Browse })
            if id == "b" && kind == "seq3")
        );
        assert_eq!(rack.selected_ids(), vec!["a"]);

        rack.toggle_selection("b".into());
        assert!(preset_action(
            &patch.nodes[1],
            &rack.context_ids("b"),
            42,
            PresetMode::Save
        )
        .is_none());
    }

    #[test]
    fn utility_normaling_tracks_connected_zero_sources_and_stops_at_current_row() {
        let mut patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[{"id":"utility","kind":"attenuverter8"},{"id":"zero","kind":"constant"}]})).unwrap();
        let node = patch.nodes[0].clone();
        assert_eq!(utility_source_row(&patch, &node, 7), None);
        patch.cables.push(Cable {
            input_order: None,
            output_order: None,
            color: None,
            id: None,
            from: "zero".into(),
            output: "out".into(),
            to: "utility".into(),
            input: "in3".into(),
        });
        assert_eq!(utility_source_row(&patch, &node, 1), None);
        assert_eq!(utility_source_row(&patch, &node, 2), Some(2));
        assert_eq!(utility_source_row(&patch, &node, 7), Some(2));
        let mut mix = node;
        mix.kind = "cv_mix".into();
        assert_eq!(utility_source_row(&patch, &mix, 1), None);
        assert_eq!(utility_source_row(&patch, &mix, 2), Some(2));
    }

    #[test]
    fn bypassed_sequencer_disables_performance_buttons_but_keeps_saved_controls() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch: Patch = serde_json::from_value(
            json!({"version":2,"nodes":[{"id":"sequence","kind":"seq3","bypassed":true}]}),
        )
        .unwrap();
        rack.set_sequence_states(BTreeMap::new(), true);
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        for (index, x) in [47.5, 107.5, 167.5].into_iter().enumerate() {
            let target = pos2(24.0 + x, 333.5);
            frame(
                &ctx,
                &mut rack,
                &patch,
                1.0 + index as f64,
                vec![egui::Event::PointerMoved(target), button(target, true)],
            );
            assert!(frame(
                &ctx,
                &mut rack,
                &patch,
                1.1 + index as f64,
                vec![button(target, false)]
            )
            .is_none());
        }
        let target = pos2(100.0, 298.5);
        frame(
            &ctx,
            &mut rack,
            &patch,
            5.0,
            vec![egui::Event::PointerMoved(target), button(target, true)],
        );
        assert!(
            matches!(frame(&ctx,&mut rack,&patch,5.1,vec![button(target,false)]),Some(RackAction::Command("set_parameter",ref args)) if args["parameter"] == "running")
        );
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

    fn apply_replug(patch: &Patch, mut args: Value) -> Patch {
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch.clone(), None).unwrap();
        args["expected_revision"] = json!(session.revision());
        session.execute("apply_edits", args).unwrap();
        session.patch().clone()
    }

    #[test]
    fn replug_transaction_preserves_source_incarnation_and_rejects_partial_or_stale_edits() {
        let workspace = tempfile::tempdir().unwrap();
        let mut patch = openpatch_core::demo_patch();
        patch.version = 2;
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch.clone(), None).unwrap();
        let patch = session.patch().clone();
        let incarnation_tokens = crate::audio::test_module_incarnation_tokens(&mut session);
        let before_tokens = incarnation_tokens(&patch, session.revision());
        assert_eq!(before_tokens.len(), patch.nodes.len());
        let pending = PendingCable::new(
            CableEnd::Input {
                node: "vca".into(),
                port: "signal".into(),
            },
            &patch,
        );
        let target = CableEnd::Input {
            node: "filter".into(),
            port: "signal".into(),
        };
        let Some(RackAction::Command("move_cables", mut args)) =
            cable_action(&patch, &pending, &target, &openpatch_core::catalog())
        else {
            panic!("occupied replug must be an edit transaction")
        };
        assert_eq!(args["cable_ids"].as_array().unwrap().len(), 1);
        let before = session.snapshot();
        args["expected_revision"] = json!(session.revision());
        session.execute("move_cables", args.clone()).unwrap();
        let committed = session.snapshot();
        assert_eq!(
            committed["history"]["undo_steps"].as_u64().unwrap(),
            before["history"]["undo_steps"].as_u64().unwrap() + 1
        );
        assert_eq!(session.patch().output, patch.output);
        assert_eq!(incarnation_tokens(session.patch(),session.revision()),before_tokens,"Recabling must preserve all original node incarnations, including the oscillator source");
        // Even after the proposed disconnect, a bad final connect publishes nothing.
        let moving = PendingCable::new(
            CableEnd::Input {
                node: "filter".into(),
                port: "signal".into(),
            },
            session.patch(),
        );
        let occupied = CableEnd::Input {
            node: "out".into(),
            port: "signal".into(),
        };
        let Some(RackAction::Command("move_cables", mut invalid)) = cable_action(
            session.patch(),
            &moving,
            &occupied,
            &openpatch_core::catalog(),
        ) else {
            panic!("second occupied move must validate")
        };
        let mut stale = invalid.clone();
        stale["expected_revision"] = args["expected_revision"].clone();
        invalid["expected_revision"] = json!(session.revision());
        invalid["port"] = json!("missing-input");
        assert!(session.execute("move_cables", invalid).is_err());
        assert_eq!(session.snapshot(), committed);
        assert!(
            session
                .execute("move_cables", stale)
                .unwrap_err()
                .contains("revision"),
            "App.command's revision guard must reject an otherwise valid stale replug"
        );
        assert_eq!(session.snapshot(), committed);
        session.execute("undo", json!({})).unwrap();
        assert_eq!(
            serde_json::to_value(session.patch()).unwrap(),
            serde_json::to_value(patch).unwrap()
        );
    }

    #[test]
    fn input_origin_connects_and_replug_preserves_destination_siblings_atomically() {
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
        patch.version = 2;
        let original = serde_json::to_value(&patch).unwrap();
        // Drop within the snap zone, beyond the actual socket's painted edge.
        let filter_input = pos2(654.0 + 16.0, 344.0);
        let action = drag(&ctx, &mut rack, &patch, 2.0, vca_input, filter_input);
        let Some(RackAction::Command("apply_edits", args)) = action else {
            panic!("expected one atomic replacement");
        };
        let candidate = apply_replug(&patch, args);
        candidate.validate().unwrap();
        assert!(!candidate
            .cables
            .iter()
            .any(|cable| cable.to == "vca" && cable.input == "signal"));
        assert!(candidate
            .cables
            .iter()
            .any(|cable| cable.from == "tone" && cable.to == "filter" && cable.input == "signal"));
        assert_eq!(candidate.cables.len(), patch.cables.len());
        assert_eq!(
            serde_json::to_value(&patch).unwrap(),
            original,
            "dragging must never mutate the live graph before validation"
        );

        let target = pos2(654.0, 344.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            3.0,
            vec![
                egui::Event::PointerMoved(tone_out),
                egui::Event::PointerButton {
                    pos: tone_out,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers {
                        command: true,
                        ..Default::default()
                    },
                },
            ],
        );
        frame(
            &ctx,
            &mut rack,
            &patch,
            3.1,
            vec![egui::Event::PointerMoved(target)],
        );
        let action = frame(&ctx, &mut rack, &patch, 3.2, vec![button(target, false)]);
        assert!(
            matches!(action, Some(RackAction::Command("connect", _))),
            "a new cable onto an occupied input must preserve its siblings"
        );
    }

    #[test]
    fn graph_failure_and_cancel_keep_routes_but_empty_move_drop_deletes() {
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
        assert!(matches!(
            drag(
                &ctx,
                &mut rack,
                &patch,
                2.0,
                pos2(464.0, 344.0),
                pos2(600.0, 600.0)
            ),
            Some(RackAction::Command("disconnect", _))
        ));
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
                        action = rack.show(ui, &patch, &openpatch_core::catalog(), 48000, 0);
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
                score: None,
                bypassed: false,
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
                    score: None,
                    bypassed: false,
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
            input_order: None,
            output_order: None,
            color: None,
            id: None,
            from: "split".into(),
            output: "out16".into(),
            to: "a".into(),
            input: "signal".into(),
        });
        let action = drag(&ctx, &mut rack, &patch, 2.0, a_signal, b_signal);
        let Some(RackAction::Command("apply_edits", args)) = action else {
            panic!("expected atomic named-output replug");
        };
        let candidate = apply_replug(&patch, args);
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
        let action = drag(&ctx, &mut rack, &patch, 3.0, split_out16, pos2(84.0, 199.0));
        let Some(RackAction::Command("apply_edits", args)) = action else {
            panic!("expected a named source replacement");
        };
        let candidate = apply_replug(&patch, args);
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
                    score: None,
                    bypassed: false,
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
                score: None,
                bypassed: false,
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
                    input_order: None,
                    output_order: None,
                    color: None,
                    id: None,
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
                    score: None,
                    bypassed: false,
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
                score: None,
                bypassed: false,
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
        let Some(RackAction::MoveModules(proposal)) = action else {
            panic!("expected single move proposal");
        };
        assert_eq!(rack.positions["tone"], pos2(0.0, 0.0));
        rack.apply_module_move(&proposal).unwrap();
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
                score: None,
                bypassed: false,
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
            score: None,
            bypassed: false,
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
    fn analog_voice_primary_and_secondary_controls_have_independent_edit_targets() {
        for (kind, parameters) in [
            ("analog_vco", ["frequency", "waveform", "pulse_width"]),
            ("ladder_vcf", ["cutoff", "resonance", "drive"]),
        ] {
            // Actual pointer input verifies both the knobs and numeric wells.
            for (index, parameter, readout) in
                parameters
                    .into_iter()
                    .enumerate()
                    .flat_map(|(index, parameter)| {
                        [false, true].map(|readout| (index, parameter, readout))
                    })
            {
                let ctx = egui::Context::default();
                let mut rack = Rack::default();
                let patch: Patch = serde_json::from_value(json!({"version":2,
                    "nodes":[{"id":"voice","kind":kind}]}))
                .unwrap();
                frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
                let target = [pos2(129.0, 150.0), pos2(76.5, 264.0), pos2(181.5, 264.0)][index];
                if readout {
                    let target = target + vec2(0.0, if index == 0 { 47.0 } else { 32.0 });
                    for (time, pressed) in [(1.0, true), (1.04, false), (1.08, true), (1.12, false)]
                    {
                        frame_revision_state(
                            &ctx,
                            &mut rack,
                            &patch,
                            time,
                            vec![egui::Event::PointerMoved(target), button(target, pressed)],
                            true,
                            true,
                            17,
                        );
                    }
                } else {
                    right_click_parameter(&ctx, &mut rack, &patch, 17, target, 1.0);
                }
                let edit = rack
                    .editing
                    .as_ref()
                    .expect("each knob and readout opens its own editor");
                assert_eq!(edit.parameter, parameter, "{kind}");
                assert_eq!(edit.id, "voice");
            }
        }
    }

    #[test]
    fn waveform_drag_accumulates_small_movements_and_emits_only_valid_steps() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let mut patch = Patch {
            nodes: vec![Node {
                score: None,
                bypassed: false,
                id: "vco".into(),
                kind: "analog_vco".into(),
                params: Default::default(),
            }],
            ..Patch::default()
        };
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let knob = pos2(76.5, 264.0);
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
        let settle = |point, width, others: &[(Pos2, f32)]| {
            settle_group(point, &[(Pos2::ZERO, width)], others).unwrap()
        };
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
    fn library_drop_converts_zoom_pan_and_centers_actual_panel_width_without_mutation() {
        let patch = Patch::new_v2();
        let canvas = Rect::from_min_size(pos2(200.0, 100.0), vec2(6000.0, 4000.0));
        for zoom in [0.25, 0.4, 0.85, 1.0, 1.8, 4.0] {
            for pan in [vec2(24.0, 12.0), vec2(-80.0, 60.0)] {
                for kind in ["constant", "seq3", "multi_vco", "vca_mix"] {
                    let mut rack = Rack {
                        pan,
                        zoom,
                        ..Default::default()
                    };
                    rack.select_only("existing-selection".into());
                    let before = rack.layout();
                    let revision = rack.layout_revision();
                    let pointer =
                        canvas.min + pan + vec2(392.0 + panel_width(kind) * 0.5, ROW + 95.0) * zoom;
                    let position = rack
                        .library_placement(&patch, kind, canvas, pointer)
                        .unwrap();
                    assert_eq!(position, pos2(390.0, ROW), "{kind}, {zoom}, {pan:?}");
                    assert_eq!(rack.layout(), before);
                    assert_eq!(rack.layout_revision(), revision);
                    assert_eq!(rack.selected.as_deref(), Some("existing-selection"));
                }
            }
        }
    }

    #[test]
    fn library_drop_uses_free_grid_gaps_and_adjacent_rows_and_predicts_missing_layout() {
        let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"a","kind":"oscillator"},{"id":"b","kind":"gain"},
            {"id":"c","kind":"seq3"}]}))
        .unwrap();
        let mut rack = Rack::default();
        rack.restore_layout(&json!({"version":1,"zoom":1,"pan":[24,12],"positions":{
            "a":[0,0],"b":[390,0],"c":[0,420]}}))
            .unwrap();
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 1200.0));
        let before = rack.layout();
        let position = rack
            .library_placement(
                &patch,
                "audio_stereo",
                canvas,
                pos2(24.0 + 230.0 + 90.0, 12.0 + 17.0),
            )
            .unwrap();
        assert_eq!(position, pos2(210.0, 0.0));
        let adjacent = rack
            .library_placement(
                &patch,
                "audio_stereo",
                canvas,
                pos2(24.0 + 90.0, 12.0 + ROW),
            )
            .unwrap();
        assert_eq!(adjacent, pos2(0.0, ROW * 2.0));
        for proposed in [position, adjacent] {
            let proposed =
                Rect::from_min_size(proposed, vec2(panel_width("audio_stereo"), PANEL_HEIGHT));
            for node in &patch.nodes {
                let other = Rect::from_min_size(
                    rack.positions[&node.id],
                    vec2(panel_width(&node.kind), PANEL_HEIGHT),
                );
                assert!(!proposed.intersect(other).is_positive());
            }
        }
        assert_eq!(rack.layout(), before);
        let empty_layout = Rack::default();
        let requested = pos2(24.0 + 60.0, 12.0);
        let proposal = empty_layout
            .library_placement(&patch, "constant", canvas, requested)
            .unwrap();
        let mut reconciled = Rack::default();
        reconciled.reconcile(&patch, &openpatch_core::catalog());
        assert_eq!(
            proposal,
            reconciled
                .library_placement(&patch, "constant", canvas, requested)
                .unwrap()
        );
        assert!(empty_layout.positions.is_empty());
    }

    #[test]
    fn library_drop_rejects_invalid_points_canvas_and_kind_without_layout_changes() {
        let rack = Rack::default();
        let patch = Patch::new_v2();
        let canvas = Rect::from_min_size(pos2(200.0, 100.0), vec2(900.0, 700.0));
        let before = rack.layout();
        for pointer in [
            pos2(199.0, 300.0),
            pos2(1101.0, 300.0),
            pos2(400.0, 99.0),
            pos2(400.0, 801.0),
            pos2(f32::NAN, 300.0),
            pos2(400.0, f32::INFINITY),
        ] {
            assert!(rack
                .library_placement(&patch, "constant", canvas, pointer)
                .is_err());
        }
        for canvas in [
            Rect::NOTHING,
            Rect::from_min_max(pos2(f32::NAN, 0.0), pos2(1000.0, 1000.0)),
            Rect::from_min_size(Pos2::ZERO, Vec2::ZERO),
        ] {
            assert!(rack
                .library_placement(&patch, "constant", canvas, pos2(400.0, 300.0))
                .is_err());
        }
        assert!(rack
            .library_placement(&patch, "unknown-kind", canvas, pos2(400.0, 300.0))
            .is_err());
        assert_eq!(rack.layout(), before);
        assert_eq!(rack.layout_revision(), 0);
        assert!(rack.selected.is_none());
    }

    #[test]
    fn successful_library_placement_selects_module_and_reveals_only_when_requested() {
        let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"old","kind":"gain"},{"id":"new","kind":"constant"}]}))
        .unwrap();
        for reveal in [false, true] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            rack.restore_layout(
                &json!({"version":1,"zoom":0.85,"pan":[70,-200],"positions":{"old":[0,0]}}),
            )
            .unwrap();
            rack.select_only("old".into());
            let revision = rack.layout_revision();
            let pan = rack.pan;
            rack.place_added_module("new".into(), pos2(2400.0, ROW * 2.0), reveal);
            assert_eq!(rack.layout_revision(), revision + 1);
            assert_eq!(rack.positions["new"], pos2(2400.0, ROW * 2.0));
            assert_eq!(rack.selected_ids(), vec!["new"]);
            assert_eq!(rack.pan, pan);
            frame(&ctx, &mut rack, &patch, 1.0, vec![]);
            if reveal {
                assert_ne!(rack.pan, pan);
                let start = rack.pan + rack.positions["new"].to_vec2() * rack.zoom;
                assert!(start.x >= 12.0 && start.y >= 12.0);
                assert!(start.x + panel_width("constant") * rack.zoom <= 1088.0);
                assert!(start.y + PANEL_HEIGHT * rack.zoom <= 688.0);
            } else {
                assert_eq!(rack.pan, pan, "drop must keep the viewport");
            }
            rack.pan = vec2(-300.0, 80.0);
            frame(&ctx, &mut rack, &patch, 2.0, vec![]);
            assert_eq!(rack.pan, vec2(-300.0, 80.0), "reveal runs only once");
            assert_eq!(rack.zoom, 0.85);
        }
    }

    #[test]
    fn library_preview_is_clipped_to_canvas_and_matches_real_panel_geometry() {
        let ctx = egui::Context::default();
        let rack = Rack {
            pan: vec2(-80.0, 35.0),
            zoom: 0.85,
            ..Default::default()
        };
        let canvas = Rect::from_min_size(pos2(180.0, 90.0), vec2(500.0, 400.0));
        let position = pos2(15.0, 0.0);
        let before = rack.layout();
        for kind in ["constant", "seq3", "vca_mix"] {
            let expected = Rect::from_min_size(
                canvas.min + rack.pan + position.to_vec2() * rack.zoom,
                vec2(panel_width(kind), PANEL_HEIGHT) * rack.zoom,
            );
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                    ..Default::default()
                },
                |ctx| {
                    let painter = ctx.layer_painter(egui::LayerId::new(
                        egui::Order::Foreground,
                        egui::Id::new("library-preview-test"),
                    ));
                    rack.paint_library_preview(&painter, kind, canvas, position);
                },
            );
            let rectangles: Vec<_> = output.shapes.iter().filter(|shape| matches!(&shape.shape,egui::epaint::Shape::Rect(rect) if rect.rect == expected)).collect();
            assert_eq!(rectangles.len(), 2);
            assert!(rectangles.iter().all(|shape| shape.clip_rect == canvas));
            assert_eq!(rack.layout(), before);
            assert_eq!(rack.layout_revision(), 0);
        }
    }
    fn deletion_fixture() -> Patch {
        serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"a","kind":"constant"},{"id":"b","kind":"constant"},{"id":"c","kind":"constant"}]})).unwrap()
    }
    fn delete_event(key: egui::Key, repeat: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers,
        }
    }
    fn is_removal(action: &Option<RackAction>) -> bool {
        matches!(
            action,
            Some(RackAction::Command("remove_module" | "apply_edits", _))
        )
    }

    #[test]
    fn delete_selection_precedes_hover_and_does_not_require_pointer_inside_canvas() {
        let patch = deletion_fixture();
        for pointer in [Some(pos2(204.0, 160.0)), Some(pos2(-20.0, -20.0)), None] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 73);
            rack.select_only("a".into());
            rack.toggle_selection("c".into());
            rack.set_positions_locked(true);
            let mut events = pointer
                .into_iter()
                .map(egui::Event::PointerMoved)
                .collect::<Vec<_>>();
            events.push(delete_event(
                egui::Key::Delete,
                false,
                egui::Modifiers::NONE,
            ));
            let action =
                frame_revision_state(&ctx, &mut rack, &patch, 1.0, events, true, true, 73).0;
            let Some(RackAction::Command("apply_edits", args)) = action else {
                panic!("selected deletion at {pointer:?}");
            };
            assert_eq!(args["expected_revision"], 73);
            let ids = args["edits"]
                .as_array()
                .unwrap()
                .iter()
                .map(|edit| edit["arguments"]["id"].as_str().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(ids, vec!["a", "c"]);
            assert!(
                !ctx.input(|i| i.key_pressed(egui::Key::Delete)),
                "accepted delete is consumed"
            );
        }
    }

    #[test]
    fn delete_hover_fallback_accepts_backspace_and_os_repeat_but_not_held_key_alone() {
        let patch = deletion_fixture();
        for key in [egui::Key::Delete, egui::Key::Backspace] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 19);
            let hover = pos2(204.0, 160.0);
            let action = frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![
                    egui::Event::PointerMoved(hover),
                    delete_event(key, false, egui::Modifiers::NONE),
                ],
                true,
                true,
                19,
            )
            .0;
            assert!(
                matches!(action,Some(RackAction::Command("remove_module",ref args)) if args["id"]=="b" && args["expected_revision"]==19)
            );
            assert!(
                rack.selection.is_empty(),
                "hover targeting must not create selection"
            );
            assert!(
                frame_revision_state(&ctx, &mut rack, &patch, 1.1, vec![], true, true, 19)
                    .0
                    .is_none()
            );
            assert!(
                ctx.input(|i| i.key_down(key)),
                "held key alone still exists after consumption"
            );
            let action = frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.2,
                vec![delete_event(key, true, egui::Modifiers::NONE)],
                true,
                true,
                19,
            )
            .0;
            assert!(
                matches!(action,Some(RackAction::Command("remove_module",ref args)) if args["id"]=="b")
            );
        }
        for pointer in [None, Some(pos2(900.0, 600.0)), Some(pos2(-10.0, -10.0))] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let mut events = pointer
                .into_iter()
                .map(egui::Event::PointerMoved)
                .collect::<Vec<_>>();
            events.push(delete_event(
                egui::Key::Backspace,
                false,
                egui::Modifiers::NONE,
            ));
            assert!(frame(&ctx, &mut rack, &patch, 1.0, events).is_none());
            assert!(
                ctx.input(|i| i.key_pressed(egui::Key::Backspace)),
                "unhandled key must remain available"
            );
        }
    }

    #[test]
    fn modified_delete_and_backspace_never_remove_modules() {
        let patch = deletion_fixture();
        for key in [egui::Key::Delete, egui::Key::Backspace] {
            for modifiers in [
                egui::Modifiers::COMMAND,
                egui::Modifiers::CTRL,
                egui::Modifiers::SHIFT,
                egui::Modifiers::ALT,
            ] {
                let ctx = egui::Context::default();
                let mut rack = Rack::default();
                frame(&ctx, &mut rack, &patch, 0.0, vec![]);
                rack.select_only("a".into());
                assert!(frame(
                    &ctx,
                    &mut rack,
                    &patch,
                    1.0,
                    vec![delete_event(key, false, modifiers)]
                )
                .is_none());
                assert!(ctx.input(|i| i.key_pressed(key)));
            }
        }
    }

    #[test]
    fn deletion_guards_focus_popups_all_pointer_buttons_and_live_gestures() {
        let patch = deletion_fixture();
        for guard in [
            "blur",
            "disabled",
            "shortcuts",
            "text",
            "popup",
            "pending",
            "cable_press",
            "group",
            "manual",
            "editor",
            "primary",
            "secondary",
            "middle",
            "extra",
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 73);
            rack.select_only("a".into());
            if guard == "shortcuts" {
                rack.set_shortcuts_enabled(false);
            }
            if guard == "pending" || guard == "cable_press" {
                let pending = PendingCable::at_revision(
                    CableEnd::Output {
                        node: "a".into(),
                        port: "out".into(),
                    },
                    &patch,
                    73,
                );
                if guard == "pending" {
                    rack.pending = Some(pending);
                } else {
                    rack.cable_press = Some(pending);
                }
            }
            if guard == "group" {
                rack.group_drag = Some(GroupDrag {
                    anchor: "a".into(),
                    origins: BTreeMap::from([("a".into(), Pos2::ZERO)]),
                    delta: Vec2::ZERO,
                    revision: 73,
                });
            }
            if guard == "manual" {
                rack.held_manual = Some(HeldManual {
                    id: "a".into(),
                    kind: "constant".into(),
                    release: "push_release",
                    button_rect: Rect::NOTHING,
                });
            }
            if guard == "editor" {
                rack.editing = Some(ParameterEdit {
                    id: "a".into(),
                    kind: "constant".into(),
                    parameter: "value".into(),
                    revision: 73,
                    surface: ParameterEditSurface::Window,
                    menu_popup: None,
                    stale: false,
                    text: "0".into(),
                    min: -16.0,
                    max: 16.0,
                    step: None,
                    display_scale: 1.0,
                    display_unit: "V".into(),
                    slew_min: None,
                    gain_power: None,
                    raw_max: 16.0,
                    focus: false,
                });
            }
            let mut events = vec![delete_event(
                egui::Key::Delete,
                false,
                egui::Modifiers::NONE,
            )];
            let button = match guard {
                "primary" => Some(PointerButton::Primary),
                "secondary" => Some(PointerButton::Secondary),
                "middle" => Some(PointerButton::Middle),
                "extra" => Some(PointerButton::Extra1),
                _ => None,
            };
            if let Some(button) = button {
                events.push(egui::Event::PointerMoved(pos2(900.0, 600.0)));
                events.push(egui::Event::PointerButton {
                    pos: pos2(900.0, 600.0),
                    button,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let mut action = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                    time: Some(1.0),
                    events,
                    focused: guard != "blur",
                    ..Default::default()
                },
                |ctx| {
                    if guard == "popup" {
                        egui::Popup::open_id(ctx, egui::Id::new("delete-popup"));
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            if guard == "text" {
                                let mut text = String::new();
                                ui.add(egui::TextEdit::singleline(&mut text))
                                    .request_focus();
                            }
                            ui.add_enabled_ui(guard != "disabled", |ui| {
                                action =
                                    rack.show(ui, &patch, &openpatch_core::catalog(), 48_000, 73);
                            });
                        });
                },
            );
            assert!(!is_removal(&action), "guard {guard}");
        }
    }

    #[test]
    fn context_removal_helper_captures_revision_for_single_and_group_actions() {
        for ids in [vec!["a".into()], vec!["a".into(), "b".into()]] {
            let RackAction::Command(_, args) = remove_selection_action(&ids, 91) else {
                panic!("expected removal command");
            };
            assert_eq!(args["expected_revision"], 91);
        }
    }

    fn navigation_key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }
    fn assert_near_vec(actual: Vec2, expected: Vec2) {
        assert!(
            (actual - expected).length() < 0.001,
            "{actual:?} != {expected:?}"
        );
    }

    #[test]
    fn zoom_limits_center_anchor_resize_and_half_octave_steps() {
        let mut rack = Rack::default();
        assert!(!rack.positions_locked());
        let pan = rack.pan;
        rack.set_zoom(0.0);
        assert_eq!(rack.zoom(), 0.25);
        assert_eq!(
            rack.pan, pan,
            "before viewport measurement the origin is preserved"
        );
        rack.set_zoom(100.0);
        assert_eq!(rack.zoom(), 4.0);
        let layout = rack.layout();
        for zoom in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            rack.set_zoom(zoom);
            assert_eq!(rack.layout(), layout);
        }
        for zoom in [0.25, 4.0] {
            rack.restore_layout(&json!({"version":1,"zoom":zoom,"pan":[31,-20],"positions":{}}))
                .unwrap();
            assert_eq!(rack.zoom(), zoom);
        }
        for zoom in [0.249, 4.001] {
            assert!(rack
                .restore_layout(&json!({"version":1,"zoom":zoom,"pan":[0,0],"positions":{}}))
                .is_err());
        }
        let ctx = egui::Context::default();
        let patch = Patch::new_v2();
        for size in [vec2(900.0, 500.0), vec2(1300.0, 800.0)] {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        rack.show(ui, &patch, &openpatch_core::catalog(), 48_000, 0);
                    });
                },
            );
            let pivot = rack.viewport.unwrap().size() * 0.5;
            let world = (pivot - rack.pan) / rack.zoom();
            rack.set_zoom(if rack.zoom() == 4.0 { 0.25 } else { 4.0 });
            assert_near_vec((pivot - rack.pan) / rack.zoom(), world);
        }
        for (start, zoom_in, expected) in [
            (1.0, true, std::f32::consts::SQRT_2),
            (1.0, false, std::f32::consts::FRAC_1_SQRT_2),
            (0.85, true, 1.0),
            (0.85, false, std::f32::consts::FRAC_1_SQRT_2),
            (4.0, true, 4.0),
            (0.25, false, 0.25),
        ] {
            rack.set_zoom(start);
            rack.zoom_step(zoom_in);
            assert!((rack.zoom() - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn navigation_pan_bounds_round_trip_without_restricting_module_positions() {
        let ctx = egui::Context::default();
        let patch = Patch::new_v2();
        let mut rack = Rack::default();
        rack.restore_layout(&json!({"version":1,"zoom":0.25,"pan":[999999,-999999],"positions":{"edge":[100000,-100000]}})).unwrap();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        rack.set_zoom(4.0);
        assert!(rack.pan.x.abs() <= MAX_PAN && rack.pan.y.abs() <= MAX_PAN);
        frame(
            &ctx,
            &mut rack,
            &patch,
            0.02,
            vec![
                navigation_key(egui::Key::ArrowLeft, egui::Modifiers::SHIFT),
                navigation_key(egui::Key::ArrowDown, egui::Modifiers::SHIFT),
            ],
        );
        let layout = rack.layout();
        let mut restored = Rack::default();
        restored.restore_layout(&layout).unwrap();
        assert_eq!(restored.layout(), layout);
        assert!(restored
            .restore_layout(&json!({"version":1,"zoom":1,"pan":[1000001,0],"positions":{}}))
            .is_err());
        assert!(restored
            .restore_layout(
                &json!({"version":1,"zoom":1,"pan":[0,0],"positions":{"edge":[100001,0]}})
            )
            .is_err());
    }

    #[test]
    fn pointer_zoom_and_extra_buttons_keep_cursor_anchor_without_center_drift() {
        for event in [
            egui::Event::Zoom(1.5),
            egui::Event::PointerButton {
                pos: pos2(230.0, 155.0),
                button: PointerButton::Extra1,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos: pos2(230.0, 155.0),
                button: PointerButton::Extra2,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch = Patch::new_v2();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            rack.set_zoom(0.85);
            let pivot = pos2(230.0, 155.0) - rack.viewport.unwrap().min;
            let world = (pivot - rack.pan) / rack.zoom;
            let factor = match &event {
                egui::Event::Zoom(factor) => *factor,
                egui::Event::PointerButton {
                    button: PointerButton::Extra1,
                    ..
                } => std::f32::consts::FRAC_1_SQRT_2,
                _ => std::f32::consts::SQRT_2,
            };
            frame(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![egui::Event::PointerMoved(pos2(230.0, 155.0)), event],
            );
            assert!((rack.zoom - 0.85 * factor).abs() < 1e-6);
            assert_near_vec((pivot - rack.pan) / rack.zoom, world);
        }
    }

    #[test]
    fn held_arrows_pan_camera_with_modifiers_and_opposing_keys_cancel() {
        for (modifiers, speed) in [
            (egui::Modifiers::NONE, 1920.0),
            (egui::Modifiers::COMMAND, 480.0),
            (egui::Modifiers::CTRL, 480.0),
            (egui::Modifiers::SHIFT, 7680.0),
            (
                egui::Modifiers {
                    shift: true,
                    ctrl: true,
                    ..Default::default()
                },
                120.0,
            ),
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch = Patch::new_v2();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let before = rack.pan;
            frame(
                &ctx,
                &mut rack,
                &patch,
                0.02,
                vec![
                    navigation_key(egui::Key::ArrowRight, modifiers),
                    navigation_key(egui::Key::ArrowUp, modifiers),
                ],
            );
            let dt = ctx.input(|i| i.stable_dt.min(0.1));
            assert_near_vec(rack.pan - before, vec2(-speed, speed) * dt);
            let before = rack.pan;
            // A held key continues on the next frame without a fresh press.
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                    time: Some(0.04),
                    modifiers,
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            rack.show(ui, &patch, &openpatch_core::catalog(), 48_000, 0);
                        });
                },
            );
            let dt = ctx.input(|i| i.stable_dt.min(0.1));
            assert_near_vec(rack.pan - before, vec2(-speed, speed) * dt);
            let before = rack.pan;
            frame(
                &ctx,
                &mut rack,
                &patch,
                0.06,
                vec![
                    navigation_key(egui::Key::ArrowLeft, modifiers),
                    navigation_key(egui::Key::ArrowDown, modifiers),
                ],
            );
            assert_eq!(rack.pan, before);
        }
    }

    #[test]
    fn navigation_respects_focus_popups_disabled_surfaces_and_live_gestures() {
        for guard in [
            "focus",
            "disabled",
            "shortcuts",
            "text",
            "popup",
            "pointer",
            "cable",
            "module",
            "manual",
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch = Patch::new_v2();
            frame(&ctx, &mut rack, &patch, 0.0, vec![]);
            let before = rack.pan;
            if guard == "shortcuts" {
                rack.set_shortcuts_enabled(false);
            }
            if guard == "cable" {
                rack.pending = Some(PendingCable::new(
                    CableEnd::Output {
                        node: "none".into(),
                        port: "out".into(),
                    },
                    &patch,
                ));
            }
            if guard == "module" {
                rack.group_drag = Some(GroupDrag {
                    anchor: "none".into(),
                    origins: BTreeMap::new(),
                    delta: Vec2::ZERO,
                    revision: 0,
                });
            }
            if guard == "manual" {
                rack.held_manual = Some(HeldManual {
                    id: "none".into(),
                    kind: "push".into(),
                    release: "push_release",
                    button_rect: Rect::NOTHING,
                });
            }
            let mut events = vec![navigation_key(egui::Key::ArrowRight, egui::Modifiers::NONE)];
            if guard == "pointer" {
                events.push(button(pos2(900.0, 600.0), true));
            }
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                    time: Some(0.02),
                    events,
                    focused: guard != "focus",
                    ..Default::default()
                },
                |ctx| {
                    if guard == "popup" {
                        egui::Popup::open_id(ctx, egui::Id::new("navigation-popup"));
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            if guard == "text" {
                                let mut text = String::new();
                                ui.add(egui::TextEdit::singleline(&mut text))
                                    .request_focus();
                            }
                            ui.add_enabled_ui(guard != "disabled", |ui| {
                                rack.show(ui, &patch, &openpatch_core::catalog(), 48_000, 0);
                            });
                        });
                },
            );
            assert_eq!(rack.pan, before, "guard {guard}");
        }
    }

    #[test]
    fn position_lock_cancels_group_preview_but_preserves_controls_cables_and_history_moves() {
        let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[{"id":"a","kind":"constant"},{"id":"b","kind":"audio_out"}]})).unwrap();
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        rack.select_only("a".into());
        rack.toggle_selection("b".into());
        let original_positions = rack.positions.clone();
        let start = pos2(60.0, 45.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![egui::Event::PointerMoved(start + vec2(90.0, 30.0))],
        );
        assert_eq!(rack.group_drag.as_ref().unwrap().origins.len(), 2);
        rack.set_positions_locked(true);
        assert!(rack.positions_locked());
        assert!(rack.group_drag.is_none());
        assert_eq!(rack.positions, original_positions);
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![button(start + vec2(90.0, 30.0), false)],
        );
        let positions = rack.positions.clone();
        assert!(drag(
            &ctx,
            &mut rack,
            &patch,
            2.0,
            start,
            start + vec2(100.0, 50.0)
        )
        .is_none());
        assert_eq!(rack.positions, positions);
        assert!(rack.selection.contains("a"));
        let (knob_position, _) = parameter_position(1, 0, panel_width("constant"));
        let knob = rack.viewport.unwrap().min
            + rack.pan
            + (rack.positions["a"].to_vec2() + knob_position.to_vec2()) * rack.zoom;
        let moved = knob - vec2(0.0, 20.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            3.0,
            vec![egui::Event::PointerMoved(knob), button(knob, true)],
        );
        // Parameter edits are emitted while dragging; release is intentionally
        // action-free, unlike the atomic cable drop checked below.
        assert!(matches!(
            frame(
                &ctx,
                &mut rack,
                &patch,
                3.1,
                vec![egui::Event::PointerMoved(moved)]
            ),
            Some(RackAction::Command("set_parameter", _))
        ));
        frame(&ctx, &mut rack, &patch, 3.2, vec![button(moved, false)]);
        let from = fixture_jack(&rack, &patch, "a", "out", false);
        let to = fixture_jack(&rack, &patch, "b", "signal", true);
        assert!(matches!(
            drag(&ctx, &mut rack, &patch, 4.0, from, to),
            Some(RackAction::Command("connect", _))
        ));
        let before = BTreeMap::from([("a".to_owned(), Pos2::ZERO)]);
        let after = BTreeMap::from([("a".to_owned(), pos2(15.0, ROW))]);
        rack.apply_module_move(&ModuleMove {
            revision: 0,
            before,
            after,
        })
        .unwrap();
        assert_eq!(rack.positions["a"], pos2(15.0, ROW));
        assert!(rack
            .library_placement(
                &patch,
                "constant",
                rack.viewport.unwrap(),
                pos2(700.0, 300.0)
            )
            .is_ok());
        rack.set_positions_locked(false);
        frame(&ctx, &mut rack, &patch, 4.8, vec![]);
        rack.select_only("a".into());
        rack.toggle_selection("b".into());
        let header = rack.viewport.unwrap().min
            + rack.pan
            + (rack.positions["a"].to_vec2() + vec2(35.0, 35.0)) * rack.zoom;
        assert!(matches!(
            drag(&ctx, &mut rack, &patch, 5.0, header, header + vec2(45.0, 0.0)),
            Some(RackAction::MoveModules(movement)) if movement.before.len() == 2
        ));
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

    fn numeric_popup_label_center(output: &egui::FullOutput, label: &str) -> Pos2 {
        fn find(shape: &egui::Shape, label: &str) -> Option<Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, label))
            .unwrap_or_else(|| panic!("missing popup button {label}"))
    }

    #[test]
    fn numeric_popup_pointer_apply_cancel_and_outside_click_preserve_selection() {
        for outcome in ["Apply", "Cancel", "outside"] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
                {"id":"vco","kind":"multi_vco"},{"id":"other","kind":"constant"}]}))
            .unwrap();
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
            rack.select_only("other".into());
            let output = right_click_parameter(&ctx, &mut rack, &patch, 7, pos2(99.0, 137.0), 1.0);
            let input_point =
                numeric_popup_label_center(&output, &rack.editing.as_ref().unwrap().text);
            for (time, pressed) in [(1.23, true), (1.24, false)] {
                assert!(frame_revision_state(
                    &ctx,
                    &mut rack,
                    &patch,
                    time,
                    vec![
                        egui::Event::PointerMoved(input_point),
                        button(input_point, pressed)
                    ],
                    true,
                    true,
                    7
                )
                .0
                .is_none());
                assert!(
                    rack.editing.is_some(),
                    "clicking the input must keep the popup open"
                );
            }
            let output = frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.3,
                vec![
                    egui::Event::Key {
                        key: egui::Key::A,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers {
                            command: true,
                            ..Default::default()
                        },
                    },
                    egui::Event::Text("440".into()),
                ],
                true,
                true,
                7,
            )
            .1;
            assert_eq!(rack.editing.as_ref().unwrap().text, "440");
            let point = if outcome == "outside" {
                pos2(99.0, 40.0)
            } else {
                numeric_popup_label_center(&output, outcome)
            };
            let mut actions = Vec::new();
            for (time, pressed) in [(1.4, true), (1.5, false)] {
                if let Some(action) = frame_revision_state(
                    &ctx,
                    &mut rack,
                    &patch,
                    time,
                    vec![egui::Event::PointerMoved(point), button(point, pressed)],
                    true,
                    true,
                    7,
                )
                .0
                {
                    actions.push(action);
                }
            }
            assert_eq!(rack.selected_ids(), vec!["other"], "{outcome}");
            assert!(rack.editing.is_none(), "{outcome}");
            assert!(!egui::Popup::is_any_open(&ctx), "{outcome}");
            if outcome == "Apply" {
                assert_eq!(actions.len(), 1);
                assert!(
                    matches!(&actions[0], RackAction::Command("set_parameter", args)
                    if args["id"] == "vco" && args["parameter"] == "frequency"
                    && args["value"] == 440.0 && args["expected_revision"] == 7
                    && args.get("history_group").is_none())
                );
            } else {
                assert!(actions.is_empty(), "{outcome}");
            }
        }
    }

    #[test]
    fn numeric_popup_modal_disable_closes_popup_and_target_changes_stale_the_draft() {
        for change in ["modal", "shortcuts", "removed", "kind"] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let mut patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
                {"id":"vco","kind":"multi_vco"}]}))
            .unwrap();
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
            right_click_parameter(&ctx, &mut rack, &patch, 7, pos2(99.0, 137.0), 1.0);
            rack.editing.as_mut().unwrap().text = "880".into();
            match change {
                "removed" => patch.nodes.clear(),
                "kind" => patch.nodes[0].kind = "multi_lfo".into(),
                "shortcuts" => rack.set_shortcuts_enabled(false),
                _ => {}
            }
            assert!(frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.3,
                vec![],
                true,
                change != "modal",
                7
            )
            .0
            .is_none());
            assert!(!egui::Popup::is_any_open(&ctx), "{change}");
            if matches!(change, "modal" | "shortcuts") {
                assert!(rack.editing.is_none());
                rack.set_shortcuts_enabled(true);
                frame_revision_state(&ctx, &mut rack, &patch, 1.4, vec![], true, true, 7);
                assert!(
                    rack.editing.is_none(),
                    "reenabling must not resurrect a closed draft"
                );
            } else {
                let edit = rack.editing.as_ref().unwrap();
                assert!(edit.stale);
                assert_eq!(edit.surface, ParameterEditSurface::Window);
                assert_eq!(edit.text, "880");
                let output =
                    frame_revision_state(&ctx, &mut rack, &patch, 1.4, vec![], true, true, 7).1;
                let point = numeric_popup_label_center(&output, "Apply");
                for (time, pressed) in [(1.5, true), (1.6, false)] {
                    assert!(frame_revision_state(
                        &ctx,
                        &mut rack,
                        &patch,
                        time,
                        vec![egui::Event::PointerMoved(point), button(point, pressed)],
                        true,
                        true,
                        7
                    )
                    .0
                    .is_none());
                }
                assert!(rack.editing.as_ref().unwrap().stale);
            }
        }
    }

    fn expression_enter() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn expression_popup_typing_replaces_selection_and_commits_once_at_captured_revision() {
        for (text, expected) in [("220 * 2", 440.0), ("A4", 440.0)] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
                {"id":"vco","kind":"multi_vco"}]}))
            .unwrap();
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
            right_click_parameter(&ctx, &mut rack, &patch, 17, pos2(99.0, 137.0), 1.0);
            assert!(frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.3,
                vec![egui::Event::Text(text.into())],
                true,
                true,
                17
            )
            .0
            .is_none());
            assert_eq!(
                rack.editing.as_ref().unwrap().text,
                text,
                "opening must select the old value so typing replaces it without Cmd+A"
            );
            let action = frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.4,
                vec![expression_enter()],
                true,
                true,
                17,
            )
            .0;
            assert!(
                matches!(action, Some(RackAction::Command("set_parameter", args))
                if args["id"] == "vco" && args["parameter"] == "frequency"
                && args["value"] == expected && args["expected_revision"] == 17
                && args.get("history_group").is_none()),
                "{text}"
            );
            assert!(rack.editing.is_none());
            assert!(frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.5,
                vec![expression_enter()],
                true,
                true,
                17
            )
            .0
            .is_none());
        }
    }

    #[test]
    fn expression_popup_invalid_or_stale_draft_never_emits_a_command() {
        for (text, step, stale) in [
            ("220 *", None, false),
            ("sqrt(-1)", None, false),
            ("1e9", None, false),
            ("0.5", Some(1.0), false),
            ("220 * 2", None, true),
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let kind = if step.is_some() {
                "analog_vco"
            } else {
                "multi_vco"
            };
            let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
                {"id":"vco","kind":kind}]}))
            .unwrap();
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
            let target = if step.is_some() {
                pos2(76.5, 264.0)
            } else {
                pos2(99.0, 137.0)
            };
            right_click_parameter(&ctx, &mut rack, &patch, 17, target, 1.0);
            if step.is_some() {
                assert_eq!(rack.editing.as_ref().unwrap().parameter, "waveform");
                assert_eq!(rack.editing.as_ref().unwrap().step, step);
            }
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.3,
                vec![egui::Event::Text(text.into())],
                true,
                true,
                17,
            );
            assert_eq!(rack.editing.as_ref().unwrap().text, text);
            let revision = if stale { 18 } else { 17 };
            assert!(frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.4,
                vec![],
                true,
                true,
                revision
            )
            .0
            .is_none());
            assert!(
                frame_revision_state(
                    &ctx,
                    &mut rack,
                    &patch,
                    1.5,
                    vec![expression_enter()],
                    true,
                    true,
                    revision
                )
                .0
                .is_none(),
                "{text}"
            );
            assert!(
                rack.editing.is_some(),
                "invalid draft remains editable: {text}"
            );
            assert_eq!(rack.editing.as_ref().unwrap().stale, stale);
        }
    }

    #[test]
    fn expression_legacy_value_double_click_preserves_display_unit_conversions() {
        for (kind, target, text, parameter, expected, clocked) in [
            (
                "vca_mix",
                pos2(61.5, 171.0),
                "gaintodb(0)",
                "level1",
                0.0,
                false,
            ),
            (
                "rescale",
                pos2(84.0, 173.0),
                "dbtogain(-inf)",
                "gain",
                0.0,
                false,
            ),
            (
                "multi_vco",
                pos2(249.0, 169.0),
                "25 + 10",
                "pulse_width",
                0.35,
                false,
            ),
            (
                "process",
                pos2(144.0, 170.0),
                "1000 * 2",
                "slew",
                1.0,
                false,
            ),
            (
                "multi_lfo",
                pos2(99.0, 169.0),
                "1 + 1",
                "frequency",
                4.0,
                true,
            ),
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let mut patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
                {"id":"target","kind":kind},{"id":"clock","kind":"gate"}]}))
            .unwrap();
            if clocked {
                patch.cables.push(Cable {
                    input_order: None,
                    output_order: None,
                    color: None,
                    id: None,
                    from: "clock".into(),
                    output: "out".into(),
                    to: "target".into(),
                    input: "clock".into(),
                });
            }
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 17);
            for (time, pressed) in [(1.0, true), (1.04, false), (1.08, true), (1.12, false)] {
                frame_revision_state(
                    &ctx,
                    &mut rack,
                    &patch,
                    time,
                    vec![egui::Event::PointerMoved(target), button(target, pressed)],
                    true,
                    true,
                    17,
                );
            }
            assert_eq!(
                rack.editing
                    .as_ref()
                    .expect("double click opens value editor")
                    .parameter,
                parameter
            );
            frame_revision_state(&ctx, &mut rack, &patch, 1.3, vec![], true, true, 17);
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.4,
                vec![egui::Event::Text(text.into())],
                true,
                true,
                17,
            );
            assert_eq!(rack.editing.as_ref().unwrap().text, text, "{kind}");
            let action = frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.5,
                vec![expression_enter()],
                true,
                true,
                17,
            )
            .0;
            assert!(
                matches!(action, Some(RackAction::Command("set_parameter", args))
                if args["parameter"] == parameter && args["expected_revision"] == 17
                && (args["value"].as_f64().unwrap() - expected).abs() < 0.0001
                && args.get("history_group").is_none()),
                "{kind}: {text}"
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn output_gesture_start(
        ctx: &egui::Context,
        rack: &mut Rack,
        patch: &Patch,
        revision: u64,
        from: Pos2,
        to: Pos2,
        modifiers: egui::Modifiers,
    ) -> egui::FullOutput {
        assert!(frame_revision_state(
            ctx,
            rack,
            patch,
            1.0,
            vec![
                egui::Event::PointerMoved(from),
                egui::Event::PointerButton {
                    pos: from,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers,
                }
            ],
            true,
            true,
            revision
        )
        .0
        .is_none());
        // The movement frame deliberately has no modifiers: intent belongs to press.
        let (action, output) = frame_revision_state(
            ctx,
            rack,
            patch,
            1.1,
            vec![egui::Event::PointerMoved(to)],
            true,
            true,
            revision,
        );
        assert!(action.is_none());
        output
    }

    #[test]
    fn occupied_output_moves_top_id_to_named_output_with_preview_persistence_and_one_undo() {
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        let mut patch = stacked_fixture();
        patch.nodes[0].kind = "poly_constant".into();
        patch.nodes[0].params.clear();
        patch.nodes[0].params.insert("channels".into(), 4.0);
        patch.nodes[1].kind = "multi_vco".into();
        patch.nodes[1].params.clear();
        session.replace_patch(patch, None).unwrap();
        let before = session.patch().clone();
        let revision = session.revision();
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, revision);
        let layout = rack.layout();
        let from = fixture_jack(&rack, &before, "src", "out", false);
        let to = fixture_jack(&rack, &before, "alt", "sine", false);
        let destination = fixture_jack(&rack, &before, "target", "signal", true);
        let output = output_gesture_start(
            &ctx,
            &mut rack,
            &before,
            revision,
            from,
            to,
            egui::Modifiers::NONE,
        );
        let old = rack.pending.as_ref().unwrap().original.as_ref().unwrap();
        assert_eq!(old.id.as_deref(), Some("two"));
        let color = cable_instance_color(old);
        let curves: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::CubicBezier(curve) => Some(curve),
                _ => None,
            })
            .collect();
        assert!(curves.iter().any(|curve| curve.points[0] == destination
            && curve.points[3] == to && curve.stroke.width == 7.8
            && matches!(curve.stroke.color, egui::epaint::ColorMode::Solid(c) if c == color.gamma_multiply(0.64))), "preview retains destination, ID color and bus width");
        for cable in &before.cables {
            let expected = cable_instance_color(cable)
                .gamma_multiply(if cable.id.as_deref() == Some("two") {
                    0.24
                } else {
                    1.0
                })
                .gamma_multiply(0.64);
            assert!(curves.iter().any(|curve| curve.points[0] == from
                && curve.points[3] == destination && matches!(curve.stroke.color, egui::epaint::ColorMode::Solid(c) if c == expected)));
        }
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &before,
            1.2,
            vec![button(to, false)],
            true,
            true,
            revision,
        )
        .0
        .unwrap();
        let RackAction::Command("move_cables", ref args) = action else {
            panic!("atomic move")
        };
        assert_eq!(args["expected_revision"], revision);
        assert_eq!(args["cable_ids"], json!(["two"]));
        assert_eq!(args["promote"], true);
        run_cable_action(&mut session, action);
        assert_eq!(session.patch().cables.len(), 2);
        assert!(same_cable(&session.patch().cables[0], &before.cables[0]));
        let moved = session
            .patch()
            .cables
            .iter()
            .find(|c| c.id.as_deref() == Some("two"))
            .unwrap();
        assert_eq!(
            (&*moved.from, &*moved.output, &*moved.to, &*moved.input),
            ("alt", "sine", "target", "signal")
        );
        assert_eq!(cable_instance_color(moved), color);
        session
            .execute("save_patch", json!({"filename":"output-move.json"}))
            .unwrap();
        let saved: Patch = serde_json::from_slice(
            &std::fs::read(workspace.path().join("output-move.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(saved).unwrap(),
            serde_json::to_value(session.patch()).unwrap()
        );
        let after = session.patch().clone();
        session.execute("undo", json!({})).unwrap();
        assert_eq!(
            serde_json::to_value(session.patch()).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
        session.execute("redo", json!({})).unwrap();
        assert_eq!(
            serde_json::to_value(session.patch()).unwrap(),
            serde_json::to_value(after).unwrap()
        );
        frame_revision_state(
            &ctx,
            &mut rack,
            session.patch(),
            2.0,
            vec![],
            true,
            true,
            session.revision(),
        );
        assert_eq!(rack.layout(), layout);
    }

    #[test]
    fn cmd_and_ctrl_output_press_create_fresh_cables_even_after_modifier_release_and_old_click() {
        for ctrl in [false, true] {
            for version in [1, 2] {
                let workspace = tempfile::tempdir().unwrap();
                let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
                let mut patch = stacked_fixture();
                patch.version = version;
                if version == 1 {
                    patch.cables.truncate(1);
                }
                session.replace_patch(patch, None).unwrap();
                let before = session.patch().clone();
                let revision = session.revision();
                let ctx = egui::Context::default();
                let mut rack = Rack::default();
                frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, revision);
                let from = fixture_jack(&rack, &before, "src", "out", false);
                let to = fixture_jack(&rack, &before, "dest", "signal", true);
                for (time, pressed) in [(0.5, true), (0.6, false)] {
                    frame_revision_state(
                        &ctx,
                        &mut rack,
                        &before,
                        time,
                        vec![egui::Event::PointerMoved(from), button(from, pressed)],
                        true,
                        true,
                        revision,
                    );
                }
                assert!(rack.pending.as_ref().unwrap().original.is_some());
                let modifiers = egui::Modifiers {
                    command: !ctrl,
                    ctrl,
                    ..Default::default()
                };
                output_gesture_start(&ctx, &mut rack, &before, revision, from, to, modifiers);
                assert!(rack.pending.as_ref().unwrap().original.is_none());
                let action = frame_revision_state(
                    &ctx,
                    &mut rack,
                    &before,
                    1.2,
                    vec![button(to, false)],
                    true,
                    true,
                    revision,
                )
                .0
                .unwrap();
                run_cable_action(&mut session, action);
                assert_eq!(session.patch().cables.len(), before.cables.len() + 1);
                assert!(before.cables.iter().all(|old| session
                    .patch()
                    .cables
                    .iter()
                    .any(|c| same_cable(old, c))));
                assert!(session
                    .patch()
                    .cables
                    .iter()
                    .any(|c| c.from == "src" && c.to == "dest"));
                session.execute("undo", json!({})).unwrap();
                assert_eq!(
                    serde_json::to_value(session.patch()).unwrap(),
                    serde_json::to_value(before).unwrap()
                );
            }
        }
    }

    #[test]
    fn picked_output_invalid_drop_deletes_but_cancel_and_stale_gestures_preserve_routes() {
        for outcome in [
            "input", "same", "empty", "escape", "focus", "modal", "stale", "cycle",
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let mut patch = stacked_fixture();
            if outcome == "cycle" {
                patch.version = 1;
                patch.cables.truncate(1);
            }
            let before = serde_json::to_value(&patch).unwrap();
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
            let from = fixture_jack(&rack, &patch, "src", "out", false);
            let to = match outcome {
                "input" => fixture_jack(&rack, &patch, "dest", "signal", true),
                "empty" => pos2(1000.0, 600.0),
                "same" => from + vec2(8.0, 0.0),
                "cycle" => fixture_jack(&rack, &patch, "target", "out", false),
                _ => fixture_jack(&rack, &patch, "alt", "out", false),
            };
            output_gesture_start(&ctx, &mut rack, &patch, 7, from, to, egui::Modifiers::NONE);
            let events = if outcome == "escape" {
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            } else {
                vec![]
            };
            assert!(frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.15,
                events,
                outcome != "focus",
                outcome != "modal",
                if outcome == "stale" { 8 } else { 7 }
            )
            .0
            .is_none());
            let action = frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.2,
                vec![button(to, false)],
                true,
                true,
                if outcome == "stale" { 8 } else { 7 },
            )
            .0;
            if matches!(outcome, "input" | "empty") {
                assert!(
                    matches!(action, Some(RackAction::Command("disconnect", args))
                    if args["cable_id"] == "two" && args["expected_revision"] == 7),
                    "{outcome}"
                );
            } else {
                assert!(action.is_none(), "{outcome}");
            }
            assert!(rack.pending.is_none(), "{outcome}");
            assert_eq!(serde_json::to_value(&patch).unwrap(), before);
        }
    }

    #[test]
    fn same_frame_output_click_captures_press_modifier_and_connects_fresh() {
        for ctrl in [false, true] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch = stacked_fixture();
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
            let from = fixture_jack(&rack, &patch, "src", "out", false);
            let to = fixture_jack(&rack, &patch, "dest", "signal", true);
            let press = egui::Event::PointerButton {
                pos: from,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers {
                    command: !ctrl,
                    ctrl,
                    ..Default::default()
                },
            };
            assert!(frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![egui::Event::PointerMoved(from), press, button(from, false)],
                true,
                true,
                7
            )
            .0
            .is_none());
            let pending = rack.pending.as_ref().expect("same-frame click opens draft");
            assert!(pending.original.is_none());
            let action = frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.1,
                vec![
                    egui::Event::PointerMoved(to),
                    button(to, true),
                    button(to, false),
                ],
                true,
                true,
                7,
            )
            .0
            .unwrap();
            assert!(matches!(action, RackAction::Command("connect", args)
                if args["from"] == "src" && args["to"] == "dest"
                && args["expected_revision"] == 7 && args.get("cable_id").is_none()));
        }
    }

    #[test]
    fn blocked_initial_output_press_cannot_resume_after_focus_or_modal_recovery() {
        for block in [
            "focus",
            "modal",
            "shortcuts",
            "revision",
            "lost_focus",
            "disabled_modal",
        ] {
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            let patch = stacked_fixture();
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
            let from = fixture_jack(&rack, &patch, "src", "out", false);
            let to = fixture_jack(&rack, &patch, "alt", "out", false);
            if block == "shortcuts" {
                rack.set_shortcuts_enabled(false);
            }
            assert!(frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![egui::Event::PointerMoved(from), button(from, true)],
                block != "focus",
                block != "modal",
                7
            )
            .0
            .is_none());
            if matches!(block, "revision" | "lost_focus" | "disabled_modal") {
                frame_revision_state(
                    &ctx,
                    &mut rack,
                    &patch,
                    1.05,
                    vec![],
                    block != "lost_focus",
                    block != "disabled_modal",
                    if block == "revision" { 8 } else { 7 },
                );
            }
            rack.set_shortcuts_enabled(true);
            let revision = if block == "revision" { 8 } else { 7 };
            assert!(frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.1,
                vec![egui::Event::PointerMoved(to)],
                true,
                true,
                revision
            )
            .0
            .is_none());
            assert!(
                frame_revision_state(
                    &ctx,
                    &mut rack,
                    &patch,
                    1.2,
                    vec![button(to, false)],
                    true,
                    true,
                    revision
                )
                .0
                .is_none(),
                "{block}"
            );
            assert!(rack.pending.is_none(), "{block}");
        }
    }

    #[test]
    fn input_drag_preserves_non_top_cable_selected_by_menu() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = stacked_fixture();
        frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
        let from = fixture_jack(&rack, &patch, "target", "signal", true);
        let to = fixture_jack(&rack, &patch, "dest", "signal", true);
        // This is the draft produced by the input menu's "Move this cable".
        rack.pending = Some(PendingCable {
            operation: CableOperation::Move,
            shift_at_press: false,
            origin: CableEnd::Input {
                node: "target".into(),
                port: "signal".into(),
            },
            companions: Vec::new(),
            color: None,
            original: Some(patch.cables[0].clone()),
            revision: 7,
            stack_requested: false,
        });
        output_gesture_start(&ctx, &mut rack, &patch, 7, from, to, egui::Modifiers::NONE);
        assert_eq!(
            rack.pending
                .as_ref()
                .unwrap()
                .original
                .as_ref()
                .unwrap()
                .id
                .as_deref(),
            Some("one")
        );
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![button(to, false)],
            true,
            true,
            7,
        )
        .0
        .unwrap();
        assert!(matches!(action, RackAction::Command("move_cables", args)
            if args["cable_ids"] == json!(["one"]) && args["promote"] == true));
    }
    include!("cable_gesture_tests.rs");
    include!("cable_menu_tests.rs");
    include!("cable_animation_render_tests.rs");
}
