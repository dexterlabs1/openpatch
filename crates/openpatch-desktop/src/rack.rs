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
    Arc, OnceLock,
};

#[path = "cable_menu.rs"]
mod cable_menu;
use cable_menu::CableMenu;

include!("parameter_fader_input.rs");
include!("parameter_choice.rs");
include!("stock_lfo_panel.rs");
include!("stock_vco_panel.rs");
include!("stock_seq3_panel.rs");
include!("stock_quantizer_panel.rs");
include!("stock_vca_mix_panel.rs");
include!("stock_process_panel.rs");
include!("stock_noise_panel.rs");
include!("studio_mixer_panel.rs");
include!("cable_autopan.rs");
include!("port_signal.rs");
#[path = "agent_follow.rs"]
mod agent_follow;

#[cfg(test)]
#[path = "panel_toggle_scale_tests.rs"]
mod panel_toggle_scale_tests;

const HP: f32 = 15.0;
const ROW: f32 = 420.0;
// Give Signal Atlas sockets a larger face and target while preserving their
// shared native positions and cable endpoints in rack and browser previews.
const STOCK_VCA_MIX_PORT_SCALE: f32 = 1.07;
// Stock oscillator output rims grow from 13 to about 17 logical points; the
// shared scale also enlarges their input sockets and pointer targets.
const STOCK_OSCILLATOR_PORT_SCALE: f32 = 1.30;
const STOCK_NOISE_PORT_SCALE: f32 = 1.30;
const STOCK_PROCESS_PORT_SCALE: f32 = 1.30;
const REDESIGNED_PORT_SCALE: f32 = 1.30;
fn port_material_scale(kind: &str) -> f32 {
    match kind {
        "stock_vca_mix" => STOCK_VCA_MIX_PORT_SCALE,
        "stock_lfo" | "stock_vco" => STOCK_OSCILLATOR_PORT_SCALE,
        "stock_noise" => STOCK_NOISE_PORT_SCALE,
        "stock_process" => STOCK_PROCESS_PORT_SCALE,
        "analog_vco" | "ladder_vcf" | "fade" | "compare" | "process" | "rescale" | "gate"
        | "attenuverter" | "white_noise" | "oscillator" | "multi_vco" | "multi_lfo"
        | "wavetable_vco" | "wavetable_lfo" | "stock_vcf" | "stock_vca" => REDESIGNED_PORT_SCALE,
        _ => 1.0,
    }
}

// Caption geometry belongs to the panel's jack layout, not to the diameter
// of its socket. All offsets are logical points from an unchanged port center.
#[derive(Clone, Copy)]
enum JackCaptionPlacement {
    Classic,
    ExistingEnlarged,
    Above(f32),
    Right(f32),
    Left(f32),
}

fn jack_caption_placement(kind: &str, port: &str, output: bool) -> JackCaptionPlacement {
    use JackCaptionPlacement::*;
    match (kind, port, output) {
        ("stock_lfo" | "stock_vco" | "stock_noise" | "stock_process", _, _) => ExistingEnlarged,
        ("rescale", "signal", false) => Left(31.0),
        ("attenuverter", "signal", false) => Left(31.0),
        ("process" | "multi_vco" | "multi_lfo", _, false) => Above(21.0),
        ("stock_vcf", _, false) => Above(21.0),
        ("compare" | "process" | "wavetable_vco" | "wavetable_lfo" | "stock_vca", _, true) => {
            Above(24.0)
        }
        ("multi_vco" | "multi_lfo" | "stock_vcf", _, true) => Right(30.0),
        (
            "analog_vco" | "ladder_vcf" | "fade" | "rescale" | "gate" | "attenuverter"
            | "white_noise" | "oscillator",
            _,
            true,
        ) => Right(36.0),
        (
            "analog_vco" | "ladder_vcf" | "fade" | "compare" | "gate" | "white_noise"
            | "wavetable_vco" | "wavetable_lfo" | "stock_vca",
            _,
            false,
        ) => Above(25.0),
        _ => Classic,
    }
}

// The catalog fallback stays a fader until its cell has been checked for a
// rotary, its labels, and its separate exact-value target at reduced zoom.
fn signal_atlas_generic_rotary_kind(kind: &str) -> bool {
    matches!(
        kind,
        "analog_vco"
            | "ladder_vcf"
            | "fade"
            | "compare"
            | "process"
            | "rescale"
            | "gate"
            | "attenuverter"
            | "white_noise"
            | "oscillator"
    )
}

fn oscillator_voice_rotary_parameter(kind: &str, parameter: &str) -> bool {
    match kind {
        "multi_vco" | "multi_lfo" => {
            matches!(
                parameter,
                "frequency" | "pulse_width" | "fm_amount" | "pwm_amount"
            )
        }
        "wavetable_vco" => {
            matches!(
                parameter,
                "tune" | "position" | "fm_amount" | "position_cv_amount"
            )
        }
        "wavetable_lfo" => {
            matches!(
                parameter,
                "rate" | "position" | "fm_amount" | "position_cv_amount"
            )
        }
        _ => false,
    }
}

fn signal_atlas_face_kind(kind: &str) -> bool {
    signal_atlas_generic_rotary_kind(kind)
        || matches!(
            kind,
            "multi_vco"
                | "multi_lfo"
                | "wavetable_vco"
                | "wavetable_lfo"
                | "stock_vcf"
                | "stock_vca"
        )
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum GenericParameterHardware {
    Choice,
    Rotary { body_radius: f32, target_half: f32 },
    Fader,
}

fn generic_parameter_hardware(
    kind: &str,
    parameter: &str,
    layout_radius: f32,
    has_choices: bool,
) -> GenericParameterHardware {
    if has_choices {
        GenericParameterHardware::Choice
    } else if kind == "white_noise" && parameter == "seed" {
        // This 24-bit integer restarts the noise sequence; it is not a
        // continuous setting and retains its existing exact-entry path.
        GenericParameterHardware::Fader
    } else if matches!(kind, "stock_lfo" | "stock_vco") {
        // Preserve the stock oscillators' original 27-point body and 68-point
        // target while other panels adopt a bounded size from their own cells.
        GenericParameterHardware::Rotary {
            body_radius: 27.0,
            target_half: 34.0,
        }
    } else if matches!(kind, "multi_vco" | "multi_lfo")
        && oscillator_voice_rotary_parameter(kind, parameter)
    {
        GenericParameterHardware::Rotary {
            body_radius: 26.0,
            target_half: 29.0,
        }
    } else if matches!(kind, "wavetable_vco" | "wavetable_lfo")
        && oscillator_voice_rotary_parameter(kind, parameter)
    {
        GenericParameterHardware::Rotary {
            body_radius: 19.5,
            target_half: 22.0,
        }
    } else if signal_atlas_generic_rotary_kind(kind) {
        GenericParameterHardware::Rotary {
            body_radius: (layout_radius - 5.0).min(30.0),
            target_half: layout_radius - 2.0,
        }
    } else {
        GenericParameterHardware::Fader
    }
}

pub(crate) const PANEL_HEIGHT: f32 = 396.0;
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
    PaletteConnect {
        arguments: Value,
        selection: crate::cable_palette::Selection,
    },
    ScalePaint {
        arguments: Value,
        gesture: u64,
    },
    ParameterEditAndNavigate {
        arguments: Value,
        forward: bool,
    },
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
    ResizeBlank(BlankResizeProposal),
    ApplyAnnotation(Value),
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

#[derive(Clone)]
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
    ApplyAndNavigate { raw: f32, forward: bool },
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
enum CableEnd {
    Output { node: String, port: String },
    Input { node: String, port: String },
}

impl CableEnd {
    fn node(&self) -> &str {
        match self {
            Self::Output { node, .. } | Self::Input { node, .. } => node,
        }
    }
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
    palette_selection: Option<crate::cable_palette::Selection>,
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
            palette_selection: None,
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
            content: None,
            score: None,
            id: String::new(),
            kind,
            bypassed: false,
            params: module["params"]
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(_, spec)| spec["hidden"] != true)
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
        vec2(module_width(&self.patch.nodes[0]), PANEL_HEIGHT)
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
    blank_resize: Option<BlankResize>,
    annotation_edit: Option<AnnotationEdit>,
    annotation_input_guard: bool,
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
    lighting: crate::lighting::RoomLighting,
    new_cable_selection: Option<crate::cable_palette::Selection>,
    cable_palette_colors: Vec<[u8; 3]>,
    pending: Option<PendingCable>,
    cable_press: Option<PendingCable>,
    cable_secondary_press: Option<(CableEnd, String)>,
    cable_menu: Option<CableMenu>,
    observed_revision: u64,
    dragging_cable: bool,
    editing: Option<ParameterEdit>,
    precision_controls: Vec<PrecisionControl>,
    precision_collect_id: Option<String>,
    precision_anchor: Option<Rect>,
    precision_next: Option<PrecisionNext>,
    precision_error: Option<String>,
    requested_parameter_menu: Option<(String, String)>,
    requested_precision_control: Option<(String, String)>,
    requested_cable_menu: Option<CableEnd>,
    deferred_touch_menu: Option<(CableEnd, String, u64)>,
    shortcuts_enabled: bool,
    suppress_reveal_once: bool,
    gesture: u64,
    layout_revision: u64,
    scope_states: BTreeMap<String, Arc<openpatch_core::ScopeSnapshot>>,
    sequence_states: BTreeMap<String, ModuleTelemetry>,
    stock_seq3_states: BTreeMap<String, Value>,
    stock_quantizer_states: BTreeMap<String, u16>,
    scale_paint: Option<ScalePaint>,
    performance_available: bool,
    module_lights: BTreeMap<String, ModuleLightState>,
    port_lights: Option<Arc<crate::audio::PortLightFrame>>,
    cable_presentation: crate::cable_presentation::CablePresentation,
    module_controls: BTreeMap<String, ModuleControlState>,
    held_manual: Option<HeldManual>,
    random_values_states: BTreeMap<String, Value>,
    shift_register_states: BTreeMap<String, Value>,
    midi_cc_states: BTreeMap<String, Value>,
    midi_gate_states: BTreeMap<String, Value>,
    construction_animation: crate::construction_motion::Tracker,
    construction_anchors: BTreeMap<(String, String), crate::construction_motion::Anchor>,
    construction_clock: Option<std::time::Instant>,
    agent_follow: agent_follow::State,
}

impl Default for Rack {
    fn default() -> Self {
        Self {
            selected: None,
            selection: BTreeSet::new(),
            group_drag: None,
            blank_resize: None,
            annotation_edit: None,
            annotation_input_guard: false,
            reveal_group_once: false,
            reveal_module_once: None,
            positions: BTreeMap::new(),
            pan: vec2(24.0, 12.0),
            zoom: 1.0,
            viewport: None,
            pending_center_zoom_ratio: 1.0,
            positions_locked: false,
            lighting: crate::lighting::RoomLighting::default(),
            new_cable_selection: None,
            cable_palette_colors: Vec::new(),
            pending: None,
            cable_press: None,
            cable_secondary_press: None,
            cable_menu: None,
            observed_revision: 0,
            dragging_cable: false,
            editing: None,
            precision_controls: Vec::new(),
            precision_collect_id: None,
            precision_anchor: None,
            precision_next: None,
            precision_error: None,
            requested_parameter_menu: None,
            requested_precision_control: None,
            requested_cable_menu: None,
            deferred_touch_menu: None,
            shortcuts_enabled: true,
            suppress_reveal_once: false,
            gesture: 0,
            layout_revision: 0,
            scope_states: BTreeMap::new(),
            sequence_states: BTreeMap::new(),
            stock_seq3_states: BTreeMap::new(),
            stock_quantizer_states: BTreeMap::new(),
            scale_paint: None,
            performance_available: false,
            module_lights: BTreeMap::new(),
            port_lights: None,
            cable_presentation: crate::cable_presentation::CablePresentation::default(),
            module_controls: BTreeMap::new(),
            held_manual: None,
            random_values_states: BTreeMap::new(),
            shift_register_states: BTreeMap::new(),
            midi_cc_states: BTreeMap::new(),
            midi_gate_states: BTreeMap::new(),
            construction_animation: crate::construction_motion::Tracker::default(),
            construction_anchors: BTreeMap::new(),
            construction_clock: None,
            agent_follow: agent_follow::State::default(),
        }
    }
}

impl Rack {
    pub(crate) fn set_cable_palette(
        &mut self,
        selection: crate::cable_palette::Selection,
        colors: &[[u8; 3]],
    ) {
        self.new_cable_selection = Some(selection);
        if self.cable_palette_colors != colors {
            self.cable_palette_colors = colors.to_vec();
        }
    }

    /// Suppress only this successful human edit, even if synchronization is delayed.
    pub(crate) fn suppress_cable_move_revision(&mut self, revision: u64) {
        self.suspend_agent_follow();
        self.construction_animation
            .suppress_cable_move_revision(revision);
    }

    pub(crate) fn clear_construction_animation(&mut self) {
        self.suspend_agent_follow();
        self.construction_animation.clear();
    }

    #[cfg(test)]
    pub(crate) fn construction_effect_counts_at(
        &mut self,
        now: std::time::Instant,
    ) -> (usize, usize, usize) {
        self.construction_animation.tick(now);
        (
            self.construction_animation.modules.len(),
            self.construction_animation.cables.len(),
            self.construction_animation.parameters.len(),
        )
    }

    #[cfg(test)]
    pub(crate) fn construction_progress_at(
        &self,
        cable: &Cable,
        now: std::time::Instant,
    ) -> Option<f32> {
        self.construction_animation.progress(cable, now)
    }

    pub(crate) fn set_construction_motion_mode(
        &mut self,
        mode: crate::construction_motion::MotionMode,
    ) {
        if self.construction_animation.set_mode(mode) {
            self.suspend_agent_follow();
            self.construction_anchors.clear();
        }
    }
    /// Deterministic rendering only; never changes the host or audio clock.
    pub(crate) fn set_construction_clock_override(&mut self, now: Option<std::time::Instant>) {
        self.construction_clock = now;
    }
    fn construction_now(&self) -> std::time::Instant {
        self.construction_clock
            .unwrap_or_else(std::time::Instant::now)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe_construction_transition_at(
        &mut self,
        before: &Patch,
        after: &Patch,
        previous_revision: u64,
        revision: u64,
        construction: &Value,
        mode: crate::construction_motion::MotionMode,
        now: std::time::Instant,
    ) {
        static MOTION_CATALOG: OnceLock<Value> = OnceLock::new();
        self.set_construction_motion_mode(mode);
        let observation = self.construction_animation.observe(
            before,
            after,
            previous_revision,
            revision,
            construction,
            MOTION_CATALOG.get_or_init(openpatch_core::catalog),
            now,
        );
        match observation {
            crate::construction_motion::Observation::Commit => {
                self.agent_follow
                    .queue(self.construction_animation.latest_targets(), revision, now)
            }
            crate::construction_motion::Observation::Clear => self.suspend_agent_follow(),
            crate::construction_motion::Observation::Preserve => {}
        }
    }
    pub(crate) fn observe_construction_transition_with_mode(
        &mut self,
        before: &Patch,
        after: &Patch,
        previous_revision: u64,
        revision: u64,
        construction: &Value,
        mode: crate::construction_motion::MotionMode,
    ) {
        self.observe_construction_transition_at(
            before,
            after,
            previous_revision,
            revision,
            construction,
            mode,
            std::time::Instant::now(),
        );
    }
    #[cfg(test)]
    pub(crate) fn observe_construction_transition(
        &mut self,
        before: &Patch,
        after: &Patch,
        previous_revision: u64,
        revision: u64,
        construction: &Value,
        enabled: bool,
    ) {
        self.observe_construction_transition_with_mode(
            before,
            after,
            previous_revision,
            revision,
            construction,
            if enabled {
                crate::construction_motion::MotionMode::Full
            } else {
                crate::construction_motion::MotionMode::Off
            },
        );
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

    pub(crate) fn request_precision_control_for_smoke(&mut self, id: &str, parameter: &str) {
        self.requested_precision_control = Some((id.to_owned(), parameter.to_owned()));
    }

    pub(crate) fn precision_is_open_for_smoke(&self) -> bool {
        self.editing
            .as_ref()
            .is_some_and(|edit| edit.surface == ParameterEditSurface::Window)
            && self.precision_controls.len() > 1
    }

    pub(crate) fn cable_is_dragging_for_smoke(&self) -> bool {
        self.dragging_cable && self.pending.is_some()
    }

    fn take_precision_request(&mut self, id: &str, parameter: &str) -> bool {
        if self
            .requested_precision_control
            .as_ref()
            .is_some_and(|(node, name)| node == id && name == parameter)
        {
            self.requested_precision_control = None;
            true
        } else {
            false
        }
    }

    pub(crate) fn port_center_for_smoke(
        &self,
        canvas: Rect,
        patch: &Patch,
        catalog: &Value,
        id: &str,
        port: &str,
        input: bool,
    ) -> Option<Pos2> {
        let node = patch.nodes.iter().find(|node| node.id == id)?;
        let module = catalog["modules"]
            .as_array()?
            .iter()
            .find(|module| module["kind"] == node.kind)?;
        let ports = module[if input { "inputs" } else { "outputs" }].as_array()?;
        let index = ports.iter().position(|name| name == port)?;
        Some(
            canvas.min
                + self.pan
                + (self.positions.get(id)?.to_vec2()
                    + port_position(&node.kind, input, index, ports.len(), module_width(node))
                        .to_vec2())
                    * self.zoom,
        )
    }

    pub(crate) fn set_shortcuts_enabled(&mut self, enabled: bool) {
        if !enabled {
            self.suspend_agent_follow();
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
                    vec2(module_width(node), PANEL_HEIGHT) * self.zoom,
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
                            .map(|p| p.x + module_width(other))
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
            source.push((new_id.clone(), positions[id], module_width(node)));
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
            .map(|n| (positions[&n.id], module_width(n)))
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
            .map(|node| (positions[&node.id], module_width(node)))
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

    /// Move only the right-hand collision chain reached by a growing mixer.
    /// The caller runs this after saving the outgoing history layout and before
    /// presenting the incoming state, so undo restores the prior geometry.
    pub(crate) fn settle_studio_mixer_growth(
        &mut self,
        patch: &Patch,
        id: &str,
        old_width: f32,
    ) -> Result<(), String> {
        let Some(mixer) = patch
            .nodes
            .iter()
            .find(|node| node.id == id && node.kind == "studio_mixer")
        else {
            return Ok(());
        };
        let delta = module_width(mixer) - old_width;
        if delta <= 0.0 {
            return Ok(());
        }
        let Some(origin) = self.positions.get(id).copied() else {
            return Ok(());
        };
        let old_right = origin.x + old_width;
        let mut neighbors: Vec<_> = patch
            .nodes
            .iter()
            .filter(|node| node.id != id)
            .filter_map(|node| {
                self.positions
                    .get(&node.id)
                    .copied()
                    .map(|position| (node, position))
            })
            .filter(|(_, position)| {
                (position.y - origin.y).abs() < PANEL_HEIGHT && position.x >= old_right - 0.5
            })
            .map(|(node, position)| (node.id.clone(), position, module_width(node)))
            .collect();
        neighbors.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
        let mut right = old_right + delta;
        let mut changes = Vec::new();
        for (neighbor, position, width) in neighbors {
            if position.x >= right - 0.5 {
                break;
            }
            if right + width > 100_000.0 {
                return Err("No room to expand mixer on this rack row".into());
            }
            changes.push((neighbor, pos2(right, position.y)));
            right += width;
        }
        if changes.is_empty() {
            return Ok(());
        }
        for (neighbor, position) in changes {
            self.positions.insert(neighbor, position);
        }
        self.layout_revision = self.layout_revision.wrapping_add(1);
        Ok(())
    }

    /// Append agent-requested modules to the end of their rack row, in graph
    /// order so one batch keeps its add order. Absent IDs are ignored and
    /// every other panel keeps its place.
    pub(crate) fn place_in_rows(&mut self, patch: &Patch, rows: &BTreeMap<String, u32>) {
        // A batch may recreate existing IDs whose layout entries still point
        // far to the right. Clear every requested position first; otherwise a
        // later member sees another member's stale position as an occupant.
        for node in &patch.nodes {
            if rows.contains_key(&node.id) {
                self.positions.remove(&node.id);
            }
        }
        for node in &patch.nodes {
            let Some(row) = rows.get(&node.id) else {
                continue;
            };
            let y = *row as f32 * ROW;
            let x = patch
                .nodes
                .iter()
                .filter(|other| other.id != node.id)
                .filter_map(|other| {
                    self.positions
                        .get(&other.id)
                        .filter(|p| (p.y - y).abs() < 1.0)
                        .map(|p| p.x + module_width(other))
                })
                .fold(0.0, f32::max);
            self.positions.insert(node.id.clone(), pos2(x, y));
            self.layout_revision = self.layout_revision.wrapping_add(1);
        }
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

    pub fn set_scope_states(
        &mut self,
        states: BTreeMap<String, Arc<openpatch_core::ScopeSnapshot>>,
    ) {
        self.scope_states = states;
    }

    pub fn set_sequence_states(
        &mut self,
        states: BTreeMap<String, ModuleTelemetry>,
        available: bool,
    ) {
        self.sequence_states = states;
        self.performance_available = available;
    }

    pub fn set_stock_quantizer_states(&mut self, states: BTreeMap<String, u16>) {
        self.stock_quantizer_states = states
            .into_iter()
            .filter(|(_, mask)| *mask <= 4095)
            .collect();
    }
    pub fn set_stock_seq3_states(&mut self, states: BTreeMap<String, Value>) {
        self.stock_seq3_states = states;
    }

    pub fn set_module_states(
        &mut self,
        lights: BTreeMap<String, ModuleLightState>,
        controls: BTreeMap<String, ModuleControlState>,
    ) {
        self.module_lights = lights;
        self.module_controls = controls;
    }

    /// Actual engine snapshots only: saved seed/held parameters are never meters.
    pub fn set_random_values_states(&mut self, states: BTreeMap<String, Value>) {
        self.random_values_states = states
            .into_iter()
            .filter(|(_, state)| random_values_snapshot_valid(state))
            .collect();
    }

    /// Coherent actual eight-row engine state; saved controls are never meters.
    pub fn set_shift_register_states(&mut self, states: BTreeMap<String, Value>) {
        self.shift_register_states = states
            .into_iter()
            .filter(|(_, state)| shift_register_snapshot_valid(state))
            .collect();
    }

    /// Coherent audio-authoritative controller maps and Learn state only.
    pub fn set_midi_cc_states(&mut self, states: BTreeMap<String, Value>) {
        self.midi_cc_states = states
            .into_iter()
            .filter(|(_, state)| midi_cc_snapshot_valid(state))
            .collect();
    }

    /// Audio-authoritative Gate assignments and transient Learn state only.
    pub fn set_midi_gate_states(&mut self, states: BTreeMap<String, Value>) {
        self.midi_gate_states = states
            .into_iter()
            .filter(|(_, state)| midi_assignment_snapshot_valid(state))
            .collect();
    }

    fn release_manual_if_needed(&mut self, ui: &egui::Ui, patch: &Patch) -> Option<RackAction> {
        let held = self.held_manual.as_ref()?;
        let eligible = ui.is_enabled()
            && self.shortcuts_enabled
            && self.performance_available
            && self.editing.is_none()
            && self.annotation_edit.is_none()
            && self.blank_resize.is_none()
            && ui.input(|i| {
                i.focused
                    && i.pointer.primary_down()
                    && !i.pointer.button_released(PointerButton::Primary)
                    && (!matches!(
                        held.kind.as_str(),
                        "pulses"
                            | "push"
                            | "gates"
                            | "random_values"
                            | "sh_asr"
                            | "adsr_eg"
                            | "stock_seq3"
                    ) || i
                        .pointer
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
        if held.kind == "adsr_eg"
            && !patch
                .nodes
                .iter()
                .any(|n| n.id == held.id && n.kind == held.kind)
        {
            return None;
        }
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
        if self.agent_follow.persisted_pan(self.pan) != vec2(24.0, 12.0) || self.zoom != 1.0 {
            self.layout_revision = self.layout_revision.wrapping_add(1);
        }
        self.set_agent_follow(false);
        self.agent_follow.promote_view();
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
        self.set_agent_follow(false);
        self.agent_follow.promote_view();
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

    /// Host preference, deliberately absent from patch/layout history.
    pub(crate) fn room_brightness(&self) -> f32 {
        self.lighting.amount()
    }

    pub(crate) fn set_room_brightness(&mut self, amount: f32) {
        self.lighting.set(amount);
    }

    pub(crate) fn cable_presentation(&self) -> crate::cable_presentation::CablePresentation {
        self.cable_presentation
    }

    pub(crate) fn set_cable_presentation(
        &mut self,
        presentation: crate::cable_presentation::CablePresentation,
    ) {
        let presentation = presentation.sanitized();
        if (self.cable_presentation.plug_brightness > 0.0) != (presentation.plug_brightness > 0.0) {
            self.port_lights = None;
        }
        self.cable_presentation = presentation;
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
            && self.annotation_edit.is_none()
            && self.blank_resize.is_none()
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
        let pan = self.agent_follow.persisted_pan(self.pan);
        json!({"version":1,"zoom":self.zoom,"pan":[pan.x,pan.y],"positions":positions})
    }

    pub(crate) fn cancel_module_drag(&mut self) {
        self.blank_resize = None;
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
        self.suspend_agent_follow();
        self.agent_follow.promote_view();
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
        self.blank_resize = None;
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
        self.annotation_input_guard = self.annotation_edit.is_some();
        let animation_now = self.construction_now();
        if self.construction_animation.tick(animation_now) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        let navigation_idle_at_frame_start = self.navigation_idle();
        if self.observed_revision != revision {
            self.cancel_module_drag();
            self.pending = None;
            self.cable_press = None;
            self.cable_secondary_press = None;
            self.deferred_touch_menu = None;
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
                && !self
                    .precision_next
                    .as_ref()
                    .is_some_and(|next| next.revision > revision)
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
            && self.annotation_edit.is_none()
            && self.shortcuts_enabled
            && !gesture_interrupted
            && ui.input(|i| i.focused)
            && !egui::Popup::is_any_open(ui.ctx())
            && !ui.input(|i| i.key_pressed(egui::Key::Escape));
        self.validate_scale_paint(ui, patch, revision, drag_interactive);
        if self.positions_locked {
            self.cancel_module_drag();
        }
        if !drag_interactive {
            self.pending = None;
            self.cable_press = None;
            self.cable_secondary_press = None;
            self.deferred_touch_menu = None;
            self.dragging_cable = false;
            self.cancel_module_drag();
        }
        if !interactive {
            self.pending = None;
            self.cable_press = None;
            self.cable_secondary_press = None;
            self.deferred_touch_menu = None;
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
        let blocked = !interactive
            || !self.shortcuts_enabled
            || !ui.input(|i| i.focused)
            || popup_open_at_frame_start
            || !self.navigation_idle();
        let manual = !blocked
            && ui.input(|i| {
                i.key_pressed(egui::Key::Escape)
                    || (canvas.contains(
                        i.pointer
                            .interact_pos()
                            .or(i.pointer.hover_pos())
                            .unwrap_or(Pos2::ZERO),
                    ) && (i.pointer.any_down()
                        || i.pointer.any_pressed()
                        || i.raw_scroll_delta != Vec2::ZERO
                        || i.zoom_delta() != 1.0))
                    || i.events.iter().any(|event| {
                        matches!(
                            event,
                            egui::Event::Key { pressed: true, .. } | egui::Event::Text(_)
                        )
                    })
            });
        if manual {
            self.set_agent_follow(false);
        } else if blocked {
            self.suspend_agent_follow();
        }
        let pan_before_follow = self.pan;
        if self.tick_agent_follow(patch, revision, animation_now) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        let camera_pan = self.pan;
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
        self.cable_edge_autopan(ui, canvas, drag_interactive && !popup_open_at_frame_start);
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
            && self.annotation_edit.is_none()
            && self.blank_resize.is_none()
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
        let background_start = self.lighting.begin(&painter);
        paint_background(&painter, canvas, self.pan, self.zoom);
        self.lighting.finish(&painter, background_start);

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
        if let Some(RackAction::Command("set_parameter", args)) = &module_action {
            if let (Some(id), Some(parameter)) = (args["id"].as_str(), args["parameter"].as_str()) {
                self.construction_animation
                    .interrupt_parameter(id, parameter);
            }
        }
        if module_action.is_some() {
            action = module_action;
        }

        self.paint_construction_overlays(ui, patch, catalog, canvas, animation_now);
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
                        action = palette_commit_action(
                            &pending,
                            cable_release_action(
                                patch,
                                &pending,
                                nearest_jack(position).map(|(target, _)| target),
                                catalog,
                                modifiers.shift,
                            ),
                        );
                    }
                }
            }
            self.dragging_cable = false;
        }
        let cable_start = self.lighting.begin(&painter);
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
                } else if self.dragging_cable
                    && self.pending.as_ref().is_some_and(|pending| {
                        !cable_gesture_related(cable, pending, nearest.map(|(end, _)| end))
                    })
                {
                    0.32
                } else if nearest.is_some() && !traced && self.pending.is_none() {
                    0.48
                } else {
                    1.0
                });
                let fan = {
                    let count = input_groups[&(cable.to.as_str(), cable.input.as_str())].len();
                    let ordinal = cable_ordinals[cable_index];
                    if count > 1 {
                        (ordinal as f32 / (count - 1) as f32 - 0.5) * 40.0
                    } else {
                        0.0
                    }
                };
                let movement = self
                    .construction_animation
                    .cables
                    .movement(cable, animation_now)
                    .and_then(|(old, old_fan, elapsed)| {
                        Some(cable_move_geometry(
                            *outputs.get(&(old.from.clone(), old.output.clone()))?,
                            *inputs.get(&(old.to.clone(), old.input.clone()))?,
                            from,
                            to,
                            old_fan,
                            fan,
                            self.zoom,
                            elapsed,
                            old.from != cable.from || old.output != cable.output,
                            old.to != cable.to || old.input != cable.input,
                        ))
                    });
                // Missing old ports invalidate a decorative route immediately.
                if self
                    .construction_animation
                    .cables
                    .movement(cable, animation_now)
                    .is_some()
                    && movement.is_none()
                {
                    self.construction_animation.cables.interrupt(cable);
                }
                let moving_hover = self.pending.is_none()
                    && movement.is_some_and(|m| {
                        pointer.is_some_and(|p| {
                            cable_curve_near(
                                cable_curve_points(
                                    m.from,
                                    m.to,
                                    self.zoom,
                                    m.fan,
                                    self.cable_presentation.tension,
                                ),
                                p,
                                (4.5 * self.zoom).max(3.0),
                            ) || (m.output_moving && p.distance(m.from) <= 12.0 * self.zoom)
                                || (m.input_moving && p.distance(m.to) <= 12.0 * self.zoom)
                        })
                    });
                let curve_hover = self.pending.is_none()
                    && self
                        .construction_animation
                        .cables
                        .phase(cable, animation_now)
                        .is_some()
                    && pointer.is_some_and(|p| {
                        cable_curve_near(
                            cable_curve_points(
                                from,
                                to,
                                self.zoom,
                                fan,
                                self.cable_presentation.tension,
                            ),
                            p,
                            (4.5 * self.zoom).max(3.0),
                        )
                    });
                if picked_up || traced || curve_hover || moving_hover {
                    self.construction_animation.cables.interrupt(cable);
                }
                let phase = self
                    .construction_animation
                    .cables
                    .phase(cable, animation_now);
                let presentation = crate::cable_presentation::CablePresentation {
                    opacity: if traced {
                        1.0
                    } else {
                        self.cable_presentation.opacity
                    },
                    ..self.cable_presentation
                };
                if let Some(movement) = movement.filter(|_| phase.is_some()) {
                    paint_moving_cable_curves(
                        &painter,
                        from,
                        to,
                        color,
                        self.zoom,
                        configured_bus(patch, catalog, &cable.from),
                        fan,
                        movement,
                        presentation,
                    );
                } else if let Some((phase, ends)) = phase
                    .filter(|p| p.reduced)
                    .zip(self.construction_animation.cables.moved_ends(cable))
                {
                    paint_reduced_cable_move(
                        &painter,
                        from,
                        to,
                        color,
                        self.zoom,
                        configured_bus(patch, catalog, &cable.from),
                        fan,
                        phase.fade,
                        ends,
                        &self.lighting,
                        presentation,
                    );
                } else {
                    paint_motion_cable_curves(
                        &painter,
                        from,
                        to,
                        color,
                        self.zoom,
                        configured_bus(patch, catalog, &cable.from),
                        fan,
                        phase,
                        &self.lighting,
                        presentation,
                    );
                }
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
        let mut lit_anchors = BTreeSet::new();
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
                    } else if self.dragging_cable
                        && self.pending.as_ref().is_some_and(|pending| {
                            !cable_gesture_related(cable, pending, nearest.map(|(end, _)| end))
                        })
                    {
                        0.32
                    } else if nearest.is_some() && !route_traced && self.pending.is_none() {
                        0.48
                    } else {
                        1.0
                    });
                    paint_cable_plug(&painter, point, color, self.zoom);
                    if ordinal + 1 == order.len() && !picked_up {
                        let direction = if output {
                            openpatch_core::PortDirection::Output
                        } else {
                            openpatch_core::PortDirection::Input
                        };
                        self.paint_port_signal(&painter, point, node, direction, port);
                        lit_anchors.insert((node, direction, port));
                    }
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
                        let guide_start = crate::lighting::next(&painter);
                        paint_jack_light(&painter, point, self.zoom, cable_instance_color(cable));
                        self.lighting.preserve(&painter, guide_start);
                    }
                }
            }
        }
        self.lighting.finish(&painter, cable_start);
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
                        paint_cable_curves(
                            &painter,
                            anchor,
                            endpoint,
                            color,
                            self.zoom,
                            bus,
                            0.0,
                            crate::cable_presentation::CablePresentation {
                                opacity: 1.0,
                                ..self.cable_presentation
                            },
                        );
                    } else {
                        paint_cable(
                            &painter,
                            anchor,
                            endpoint,
                            color,
                            self.zoom,
                            false,
                            bus,
                            0.0,
                            crate::cable_presentation::CablePresentation {
                                opacity: 1.0,
                                ..self.cable_presentation
                            },
                        );
                        paint_jack_light(&painter, anchor, self.zoom, color);
                    }
                    // Only the attached end can read an existing physical port.
                    // The free pointer/hover target never acquires activity.
                    let (id, direction, port) = if let Some(old) = &pending.original {
                        if picked_output {
                            (&old.to, openpatch_core::PortDirection::Input, &old.input)
                        } else {
                            (
                                &old.from,
                                openpatch_core::PortDirection::Output,
                                &old.output,
                            )
                        }
                    } else {
                        match &pending.origin {
                            CableEnd::Input { node, port } => {
                                (node, openpatch_core::PortDirection::Input, port)
                            }
                            CableEnd::Output { node, port } => {
                                (node, openpatch_core::PortDirection::Output, port)
                            }
                        }
                    };
                    if lit_anchors.insert((id, direction, port))
                        || pending.operation != CableOperation::Move
                    {
                        self.paint_port_signal(&painter, anchor, id, direction, port);
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
                            crate::cable_presentation::CablePresentation {
                                opacity: 1.0,
                                ..self.cable_presentation
                            },
                        );
                        let (id, direction, port) = match pending.origin {
                            CableEnd::Input { .. } => (
                                &cable.from,
                                openpatch_core::PortDirection::Output,
                                &cable.output,
                            ),
                            CableEnd::Output { .. } => (
                                &cable.to,
                                openpatch_core::PortDirection::Input,
                                &cable.input,
                            ),
                        };
                        if lit_anchors.insert((id, direction, port)) {
                            self.paint_port_signal(&painter, anchor, id, direction, port);
                        }
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
                let guidance = cable_gesture_guidance(
                    patch,
                    pending,
                    nearest.map(|(target, _)| target),
                    catalog,
                    ui.input(|i| i.modifiers.shift),
                    self.dragging_cable,
                );
                paint_cable_gesture_guidance(
                    &painter,
                    canvas,
                    pointer,
                    &guidance,
                    self.dragging_cable,
                );
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
        if pan_before_follow != initial_pan || self.pan != camera_pan {
            self.set_agent_follow(false);
            self.agent_follow.promote_view();
            self.layout_revision = self.layout_revision.wrapping_add(1);
        }
        if !blocked && !manual {
            self.resolve_agent_follow(patch, catalog, canvas, animation_now);
            if self.agent_follow.active() {
                ui.ctx().request_repaint();
            }
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
        if interactive && self.shortcuts_enabled {
            self.annotation_editor(ui.ctx(), patch, revision, &mut action);
        }
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
        if mode.is_interactive() {
            self.construction_anchors.clear();
            self.precision_controls.clear();
            self.precision_anchor = None;
            self.precision_collect_id = self
                .editing
                .as_ref()
                .filter(|edit| {
                    edit.surface == ParameterEditSurface::Window
                        && !(edit.kind == "random" && edit.parameter == "seed")
                })
                .map(|edit| edit.id.clone());
        }
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
        let lighting_start = self.lighting.begin(&painter);
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
        let secondary_events = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::PointerButton {
                        pos,
                        button: PointerButton::Secondary,
                        pressed,
                        ..
                    } => Some((*pos, *pressed)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        let secondary_enabled = drag_interactive
            && interactive
            && !popup_open_at_frame_start
            && self.editing.is_none()
            && self.annotation_edit.is_none()
            && ui.input(|i| i.focused && !i.key_pressed(egui::Key::Escape));
        if !secondary_enabled {
            self.cable_secondary_press = None;
        }
        let deferred_touch_menu = if mode.is_interactive() {
            self.deferred_touch_menu
                .take()
                .and_then(|(end, kind, at_revision)| {
                    let port_valid = match &end {
                        CableEnd::Output { node, port } => {
                            port_exists(patch, catalog, node, port, true)
                        }
                        CableEnd::Input { node, port } => {
                            port_exists(patch, catalog, node, port, false)
                        }
                    };
                    (secondary_enabled
                        && at_revision == revision
                        && port_valid
                        && patch
                            .nodes
                            .iter()
                            .any(|node| node.id == end.node() && node.kind == kind))
                    .then_some(end)
                })
        } else {
            None
        };
        let mut secondary_click_target = None;
        for (pos, pressed) in &secondary_events {
            if !secondary_enabled {
                continue;
            }
            let target = (ui.ctx().layer_id_at(*pos) == Some(ui.layer_id()))
                .then(|| {
                    self.cable_press_target(
                        patch,
                        catalog,
                        canvas,
                        canvas.intersect(ui.clip_rect()),
                        *pos,
                    )
                })
                .flatten();
            if *pressed {
                self.cable_secondary_press = target.and_then(|end| {
                    patch
                        .nodes
                        .iter()
                        .find(|node| node.id == end.node())
                        .map(|node| (end, node.kind.clone()))
                });
            } else {
                let captured = self.cable_secondary_press.take();
                if ui.input(|i| i.pointer.button_clicked(PointerButton::Secondary)) {
                    if let (Some((end, kind)), Some(target)) = (captured, target) {
                        if end == target
                            && patch
                                .nodes
                                .iter()
                                .any(|node| node.id == end.node() && node.kind == kind)
                        {
                            secondary_click_target = Some(end);
                        }
                    }
                }
            }
        }
        if !ui.input(|i| i.pointer.button_down(PointerButton::Secondary)) {
            self.cable_secondary_press = None;
        }
        let hover_target = ui
            .input(|i| {
                (secondary_enabled && !i.pointer.any_down())
                    .then(|| i.pointer.hover_pos())
                    .flatten()
            })
            .filter(|pos| ui.ctx().layer_id_at(*pos) == Some(ui.layer_id()))
            .and_then(|pos| {
                self.cable_press_target(
                    patch,
                    catalog,
                    canvas,
                    canvas.intersect(ui.clip_rect()),
                    pos,
                )
            });
        // egui reports a touch hold on the response, rather than as a raw
        // Secondary pointer event. Route it through the same nearest endpoint
        // after the widget pass, so overlapping touch targets keep their owner.
        let long_touch_target = ui
            .input(|i| secondary_enabled.then(|| i.pointer.hover_pos()).flatten())
            .filter(|pos| ui.ctx().layer_id_at(*pos) == Some(ui.layer_id()))
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
        let mut hovered_jack_response = false;
        let mut long_touched_jack_response = false;
        let mut nearest_jack_tooltip: Option<(egui::Response, String, Pos2, f32)> = None;
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
            let (position, width) = self.annotation_geometry(node);
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
                if !studio_mixer_input_visible(node, port) {
                    continue;
                }
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
            // An active resize must receive release even when its last preview
            // shrank entirely beyond the viewport into another app panel.
            let resizing = self
                .blank_resize
                .as_ref()
                .is_some_and(|d| d.proposal.id == node.id);
            // Resolve a pending follow target through the actual control
            // renderer even offscreen; painters remain clipped to the canvas.
            let follow_parameter = self.agent_follow.pending.iter().any(|target| {
                target.id == node.id
                    && target.kind == node.kind
                    && matches!(
                        target.focus,
                        crate::construction_motion::FollowFocus::Parameter(_)
                    )
            });
            if !rect.intersects(canvas) && !resizing && !follow_parameter {
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
                    &self.lighting,
                );
            }
            if mode.is_interactive() {
                let header_rect = Rect::from_min_max(
                    point(15.0, 19.0),
                    point(
                        if matches!(node.kind.as_str(), "stock_vco" | "stock_lfo") {
                            174.0
                        } else {
                            width - 15.0
                        },
                        69.0,
                    ),
                );
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
                    if matches!(node.kind.as_str(), "notes" | "blank")
                        && ui
                            .button(if node.kind == "notes" {
                                "Edit notes…"
                            } else {
                                "Set width…"
                            })
                            .clicked()
                    {
                        self.open_annotation_editor(node, revision);
                        ui.close();
                    }
                    wavetable_menu(ui, node, patch, revision, &mut action);
                    if node.kind == "random" && ui.button("Edit random seed…").clicked() {
                        self.open_precision_editor(
                            random_seed_edit(node, revision),
                            Rect::from_min_max(point(15.0, 19.0), point(width - 15.0, 69.0)),
                        );
                        ui.close();
                    }
                    utility_menu(ui, node, &mut action);
                    if matches!(node.kind.as_str(), "seq3" | "stock_seq3") {
                        for (direction, label) in [
                            ("left", "Rotate pattern left"),
                            ("right", "Rotate pattern right"),
                        ] {
                            if ui.button(label).clicked() {
                                action = Some(RackAction::Command(
                                    "rotate_sequence",
                                    json!({"id":node.id,"direction":direction,"expected_revision":revision}),
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
                    &self.lighting,
                );
                let header = Rect::from_min_max(
                    point(15.0, 19.0),
                    point(
                        if matches!(node.kind.as_str(), "stock_vco" | "stock_lfo") {
                            174.0
                        } else {
                            width - 15.0
                        },
                        69.0,
                    ),
                );
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
                                    module_width(patch.nodes.iter().find(|n| &n.id == id).unwrap()),
                                )
                            })
                            .collect();
                        let others: Vec<_> = patch
                            .nodes
                            .iter()
                            .filter(|n| !drag.origins.contains_key(&n.id))
                            .map(|n| (self.positions[&n.id], module_width(n)))
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
                    wavetable_menu(ui, node, patch, revision, &mut action);
                    if node.kind == "random" && ui.button("Edit random seed…").clicked() {
                        self.open_precision_editor(
                            random_seed_edit(node, revision),
                            Rect::from_min_max(point(15.0, 19.0), point(width - 15.0, 69.0)),
                        );
                        ui.close();
                    }
                    utility_menu(ui, node, &mut action);
                    if matches!(node.kind.as_str(), "seq3" | "stock_seq3") {
                        for (direction, label) in [
                            ("left", "Rotate pattern left"),
                            ("right", "Rotate pattern right"),
                        ] {
                            if ui.button(label).clicked() {
                                action = Some(RackAction::Command(
                                    "rotate_sequence",
                                    json!({"id":node.id,"direction":direction,"expected_revision":revision}),
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
            if node.kind == "scope" {
                if let Some(edit) = self.scope_panel(ui, node, module, rect) {
                    action = Some(edit);
                }
            }
            if node.kind == "seq3" {
                if let Some(edit) = self.sequence_panel(ui, node, module, rect) {
                    action = Some(edit);
                }
            }
            if node.kind == "stock_quantizer" {
                if let Some(edit) =
                    self.stock_quantizer_panel(ui, node, rect, revision, mode.is_interactive())
                {
                    action = Some(edit);
                }
            }
            if node.kind == "stock_octave" {
                if let Some(edit) =
                    self.stock_octave_panel(ui, node, rect, revision, mode.is_interactive())
                {
                    action = Some(edit);
                }
            }
            if node.kind == "stock_seq3" {
                if let Some(edit) = self.stock_seq3_panel(
                    ui,
                    node,
                    rect,
                    revision,
                    allow_manual_press,
                    mode.is_interactive(),
                ) {
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
                        crate::panel_skin::print_font(painter.ctx(), 9.0 * self.zoom),
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
            if node.kind == "stock_lfo" {
                self.stock_lfo_lights(ui, node, rect);
            }
            if node.kind == "stock_vco" {
                self.stock_vco_lights(ui, node, rect);
            }
            if node.kind == "poly_split" {
                self.split_panel(ui, node, rect);
            }
            if node.kind == "poly_sum" {
                self.sum_panel(ui, node, rect);
            }
            if node.kind == "poly_viz" {
                self.viz_panel(ui, node, rect);
            }
            if node.kind == "modal_resonator" {
                if let Some(edit) = self.resonator_panel(
                    ui,
                    node,
                    module,
                    rect,
                    device_rate,
                    patch.sample_rate.min(device_rate),
                    revision,
                ) {
                    action = Some(edit);
                }
            }
            if matches!(node.kind.as_str(), "notes" | "blank") {
                if let Some(edit) =
                    self.annotation_panel(ui, node, patch, rect, revision, drag_interactive)
                {
                    action = Some(edit);
                }
            }
            if node.kind == "midi_gate" {
                if let Some(edit) = self.midi_gate_panel(ui, node, rect, revision) {
                    action = Some(edit);
                }
            }
            if node.kind == "midi_cc" {
                if let Some(edit) = self.midi_cc_panel(ui, node, rect, revision) {
                    action = Some(edit);
                }
            }
            if node.kind == "stock_vca" {
                if let Some(edit) = self.stock_vca_panel(ui, node, rect, revision) {
                    action = Some(edit);
                }
            }
            if node.kind == "stock_vca_mix" {
                if let Some(edit) =
                    self.stock_vca_mix_panel(ui, node, rect, revision, mode.is_interactive())
                {
                    action = Some(edit);
                }
            }
            if node.kind == "stock_process" {
                if let Some(edit) = self.stock_process_panel(
                    ui,
                    node,
                    rect,
                    revision,
                    allow_manual_press,
                    mode.is_interactive(),
                ) {
                    action = Some(edit);
                }
            }
            if node.kind == "random" {
                if self.editing.as_ref().is_some_and(|edit| {
                    edit.id == node.id
                        && edit.parameter == "seed"
                        && edit.surface == ParameterEditSurface::Window
                }) {
                    self.precision_anchor = Some(Rect::from_min_max(
                        point(15.0, 19.0),
                        point(width - 15.0, 69.0),
                    ));
                }
                paint_random_lights(
                    &painter,
                    rect,
                    self.zoom,
                    self.module_lights.get(&node.id),
                    &self.lighting,
                );
            }
            if is_wavetable(&node.kind) {
                paint_wavetable_display(
                    &painter,
                    node,
                    patch,
                    rect,
                    self.zoom,
                    self.module_lights.get(&node.id),
                    &self.lighting,
                );
            }
            if node.kind == "stock_delay" {
                if let Some(edit) = self.stock_delay_panel(ui, node, rect, revision) {
                    action = Some(edit);
                }
            }
            if node.kind == "stock_vcf" {
                if let Some(edit) = self.stock_vcf_panel(ui, node, rect, revision) {
                    action = Some(edit);
                }
            }
            if node.kind == "adsr_eg" {
                if let Some(edit) = self.adsr_eg_panel(ui, node, rect, revision, allow_manual_press)
                {
                    action = Some(edit);
                }
            }
            if node.kind == "sh_asr" {
                if let Some(edit) =
                    self.shift_register_panel(ui, node, module, rect, revision, allow_manual_press)
                {
                    action = Some(edit);
                }
            }
            if node.kind == "random_values" {
                if let Some(edit) =
                    self.random_values_panel(ui, node, module, rect, revision, allow_manual_press)
                {
                    action = Some(edit);
                }
            }
            if node.kind == "colored_noise" {
                if let Some(edit) = self.noise_panel(ui, node, module, rect, device_rate, revision)
                {
                    action = Some(edit);
                }
            }
            if node.kind == "stock_noise" {
                paint_stock_noise_face(&painter, rect, self.zoom);
            }
            if node.kind == "studio_mixer" {
                if let Some(edit) = self.studio_mixer_panel(
                    ui,
                    node,
                    module,
                    patch,
                    rect,
                    revision,
                    mode.is_interactive(),
                ) {
                    action = Some(edit);
                }
            }
            if matches!(node.kind.as_str(), "mid_side" | "stock_mid_side") {
                for (x, label) in [(width * 0.25, "ENCODER"), (width * 0.75, "DECODER")] {
                    painter.text(
                        point(x, 72.0),
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
                if (node.kind == "random" && *name == "seed")
                    || node.kind == "scope"
                    || node.kind == "modal_resonator"
                    || node.kind == "colored_noise"
                    || node.kind == "random_values"
                    || node.kind == "sh_asr"
                    || node.kind == "adsr_eg"
                    || node.kind == "stock_vcf"
                    || node.kind == "stock_delay"
                    || node.kind == "stock_vca"
                    || node.kind == "stock_octave"
                    || node.kind == "stock_vca_mix"
                    || node.kind == "stock_process"
                    || node.kind == "studio_mixer"
                    || matches!(node.kind.as_str(), "midi_cc" | "midi_gate")
                    || is_voltage_utility(&node.kind)
                    || (node.kind == "vca_mix" && matches!(*name, "channel_exp" | "mix_exp"))
                    || (node.kind == "fade" && *name == "law")
                    || (node.kind == "rescale"
                        && matches!(*name, "multiplier" | "reflect_min" | "reflect_max"))
                    || is_sequential_switch(&node.kind)
                    || (node.kind == "seq3" && sequence_cell(name).is_none())
                    || (node.kind == "stock_seq3" && !stock_seq3_scalar(name))
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
                    if let Some(edit) = self.merge_panel(ui, node, rect) {
                        action = Some(edit);
                    }
                    continue;
                }
                if *name == "frequency" && node.kind == "stock_vco" {
                    // P is V/OCT, F is signed FM input and A is FM depth.
                    // Both formulas remain visible because linear_fm selects
                    // between the exponential and linear-frequency laws.
                    painter.text(
                        point(width * 0.5, 75.0),
                        Align2::CENTER_CENTER,
                        "EXP target: C4·2^(T/12+V+F·A)",
                        crate::panel_skin::print_font(painter.ctx(), 9.0 * self.zoom),
                        INK,
                    );
                    painter.text(
                        point(width * 0.5, 92.0),
                        Align2::CENTER_CENTER,
                        "LIN target: C4·2^(T/12+V)+C4·F·A",
                        crate::panel_skin::print_font(painter.ctx(), 9.0 * self.zoom),
                        INK,
                    );
                    painter.text(
                        point(width * 0.5, 109.0),
                        Align2::CENTER_CENTER,
                        "T=FREQ(st) · V=PITCH(V/OCT) · F=FM(V) · A=DEPTH",
                        crate::panel_skin::print_font(painter.ctx(), 8.0 * self.zoom),
                        MUTED,
                    );
                    painter.text(
                        point(width * 0.5, 126.0),
                        Align2::CENTER_CENTER,
                        "PWM/10×DEPTH→SQUARE WIDTH · SYNC→PHASE",
                        crate::panel_skin::print_font(painter.ctx(), 8.0 * self.zoom),
                        MUTED,
                    );
                }
                if *name == "rate" && node.kind == "stock_lfo" {
                    // Rate is log2; FM depth multiplies its frequency ratio.
                    painter.text(
                        point(width * 0.5, 97.0),
                        Align2::CENTER_CENTER,
                        "fCLK (Hz)/2 × 2^(RATE + FM×DEPTH) → RATE",
                        crate::panel_skin::print_font(painter.ctx(), 9.0 * self.zoom),
                        INK,
                    );
                    painter.text(
                        point(width * 0.5, 115.0),
                        Align2::CENTER_CENTER,
                        "fCLK = measured Hz · starts at 2 Hz until clocked",
                        crate::panel_skin::print_font(painter.ctx(), 8.5 * self.zoom),
                        MUTED,
                    );
                    painter.text(
                        point(width * 0.5, 133.0),
                        Align2::CENTER_CENTER,
                        "PWM/10×DEPTH→SQUARE WIDTH · RESET→PHASE · OFFSET→RANGE",
                        crate::panel_skin::print_font(painter.ctx(), 8.5 * self.zoom),
                        MUTED,
                    );
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
                    let response = crate::panel_skin::toggle_button(
                        ui,
                        rect,
                        if *name == "linear_fm" { "LFM" } else { "SOFT" },
                        enabled,
                        self.zoom,
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
                if node.kind == "stock_vco" && matches!(*name, "linear_fm" | "soft_sync") {
                    let enabled = value_or_default(node, name, spec) >= 0.5;
                    let x = if *name == "linear_fm" { 184.0 } else { 268.0 };
                    let response = crate::panel_skin::toggle_button(
                        ui,
                        Rect::from_min_size(point(x, 39.0), vec2(78.0, 24.0) * self.zoom),
                        if *name == "linear_fm" {
                            "Linear FM"
                        } else {
                            "Soft sync"
                        },
                        enabled,
                        self.zoom,
                    );
                    if mode.is_interactive() {
                        if let Some(cue) = self
                            .construction_animation
                            .parameter(&node.id, &node.kind, name)
                        {
                            let label = |value: f32| match (*name, value >= 0.5) {
                                ("linear_fm", true) => "Linear FM",
                                ("linear_fm", false) => "Exponential FM",
                                (_, true) => "Soft sync",
                                (_, false) => "Hard sync",
                            };
                            self.register_construction_anchor(
                                ui,
                                node,
                                name,
                                crate::construction_motion::Anchor {
                                    rect: response.rect,
                                    old_text: label(cue.old).into(),
                                    new_text: label(cue.new).into(),
                                    marker: None,
                                },
                                &[response.id],
                            );
                        }
                    }
                    if response.clicked() {
                        self.select_primary(node.id.clone());
                        action = Some(RackAction::Command(
                            "set_parameter",
                            json!({"id":node.id,"parameter":name,"value":if enabled {0.0} else {1.0},"expected_revision":revision}),
                        ));
                    }
                    response.on_hover_text(if *name == "linear_fm" {
                        "Linear FM when selected; exponential FM otherwise. Signed FM depth stays independent of the mode."
                    } else {
                        "Soft sync when selected; hard sync otherwise. The SYNC input controls the oscillator, and the adjacent lamp is observed runtime state."
                    });
                    continue;
                }
                if node.kind == "stock_lfo" && matches!(*name, "offset" | "invert") {
                    let enabled = value_or_default(node, name, spec) >= 0.5;
                    let x = if *name == "offset" { 184.0 } else { 268.0 };
                    let response = crate::panel_skin::toggle_button(
                        ui,
                        Rect::from_min_size(point(x, 39.0), vec2(78.0, 24.0) * self.zoom),
                        if *name == "offset" {
                            if enabled {
                                "0–10 V"
                            } else {
                                "±5 V"
                            }
                        } else {
                            "Invert"
                        },
                        enabled,
                        self.zoom,
                    );
                    if mode.is_interactive() {
                        if let Some(cue) = self
                            .construction_animation
                            .parameter(&node.id, &node.kind, name)
                        {
                            let label = |value: f32| {
                                if *name == "offset" {
                                    if value >= 0.5 {
                                        "0–10 V"
                                    } else {
                                        "±5 V"
                                    }
                                } else if value >= 0.5 {
                                    "Invert On"
                                } else {
                                    "Invert Off"
                                }
                            };
                            self.register_construction_anchor(
                                ui,
                                node,
                                name,
                                crate::construction_motion::Anchor {
                                    rect: response.rect,
                                    old_text: label(cue.old).into(),
                                    new_text: label(cue.new).into(),
                                    marker: None,
                                },
                                &[response.id],
                            );
                        }
                    }
                    if response.clicked() {
                        self.select_primary(node.id.clone());
                        action = Some(RackAction::Command(
                            "set_parameter",
                            json!({"id":node.id,"parameter":name,"value":if enabled {0.0} else {1.0},"expected_revision":revision}),
                        ));
                    }
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
                    let button =
                        crate::panel_skin::toggle_button(ui, rect, label, enabled, self.zoom);
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
                if matches!(node.kind.as_str(), "mix6" | "stock_mix")
                    && matches!(*name, "invert" | "average")
                {
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
                    let button = crate::panel_skin::toggle_button(
                        ui,
                        rect,
                        if *name == "average" {
                            "Average"
                        } else {
                            "Invert"
                        },
                        value >= 0.5,
                        self.zoom,
                    );
                    if button.clicked()
                        && (node.kind == "mix6"
                            || (interactive
                                && self.shortcuts_enabled
                                && self.editing.is_none()
                                && self.annotation_edit.is_none()
                                && !popup_open_at_frame_start
                                && !egui::Popup::is_any_open(ui.ctx())))
                    {
                        self.select_primary(node.id.clone());
                        let mut args = json!({"id":node.id,"parameter":name,"value":if value >= 0.5 { 0.0 } else { 1.0 }});
                        if node.kind == "stock_mix" {
                            args["expected_revision"] = json!(revision);
                        }
                        action = Some(RackAction::Command("set_parameter", args));
                    }
                    button.on_hover_text(if *name == "average" && node.kind == "stock_mix" {
                        "Divide by the number of inputs with channels, including connected mono 0 V."
                    } else if *name == "average" {
                        "Divide by the number of active input jacks, including patched 0 V."
                    } else {
                        "Reverse the output polarity."
                    });
                    continue;
                }
                if node.kind == "macro_voice" && *name == "engine" {
                    painter.text(
                        point(25.0, 88.0),
                        Align2::LEFT_CENTER,
                        "ENGINE",
                        crate::panel_skin::print_font(painter.ctx(), 10.0 * self.zoom),
                        INK,
                    );
                    let choices = spec["labels"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|s| s.as_str().unwrap())
                        .collect::<Vec<_>>();
                    if let Some(edit) = self.choice_parameter(
                        ui,
                        node,
                        name,
                        spec,
                        Rect::from_min_size(
                            point(100.0, 75.0),
                            vec2(width - 125.0, 27.0) * self.zoom,
                        ),
                        &choices,
                    ) {
                        action = Some(edit);
                    }
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
                let mut label_gap = match node.kind.as_str() {
                    "stock_lfo" | "stock_vco" => 10.0,
                    // The two rotary rows share a compact 300-point face.
                    // Place captions just above their hit regions so the
                    // preceding row's readout clears the next caption at 25%.
                    "multi_lfo" | "multi_vco" => 9.0,
                    // The wavetable display and its five observed lamps end
                    // above the first row. Seven points keep each caption
                    // clear of its rotary target and the prior row's readout.
                    "wavetable_vco" | "wavetable_lfo" => 7.0,
                    _ => 13.0,
                };
                let (knob_position, radius) = if let Some((row, column)) = sequence_cell(name)
                    .filter(|_| matches!(node.kind.as_str(), "seq3" | "stock_seq3"))
                {
                    (
                        pos2(
                            238.0 + column as f32 * 67.0,
                            123.0
                                + row as f32
                                    * if node.kind == "stock_seq3" {
                                        71.5
                                    } else {
                                        73.0
                                    },
                        ),
                        17.0,
                    )
                } else if node.kind == "stock_seq3" {
                    (stock_seq3_parameter_position(name), 18.0)
                } else if node.kind == "audio_stereo" {
                    (pos2(width * 0.5, 130.0 + index as f32 * 120.0), 30.0)
                } else if node.kind == "score_sequencer" {
                    (pos2(60.0 + index as f32 * 120.0, 130.0), 24.0)
                } else if node.kind == "macro_voice" {
                    (
                        pos2(
                            width * ((index - 1) % 3) as f32 / 3.0 + width / 6.0,
                            154.0 + ((index - 1) / 3) as f32 * 102.0,
                        ),
                        22.0,
                    )
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
                } else if node.kind == "poly_sum" {
                    (pos2(width * 0.5, 145.0), 29.0)
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
                } else if node.kind == "stock_quantizer" {
                    (pos2(width * 0.5, 303.0), 23.0)
                } else if node.kind == "quantizer" {
                    (pos2(width * 0.5, 260.0), 23.0)
                } else if node.kind == "random" {
                    (
                        random_parameter_position(index),
                        if *name == "unipolar" { 10.0 } else { 14.0 },
                    )
                } else if matches!(node.kind.as_str(), "stock_lfo" | "stock_vco") {
                    let column = match *name {
                        "frequency" | "rate" => 0,
                        "pulse_width" => 1,
                        "fm_amount" => 2,
                        "pwm_amount" => 3,
                        _ => index,
                    };
                    (pos2(45.0 + column as f32 * 90.0, 196.0), 30.0)
                } else if is_wavetable(&node.kind) {
                    (wavetable_parameter_position(index), 18.0)
                } else if matches!(node.kind.as_str(), "multi_lfo" | "multi_vco") {
                    (
                        pos2(
                            width * (if index.is_multiple_of(2) { 0.25 } else { 0.75 }),
                            125.0 + (index / 2) as f32 * 100.0,
                        ),
                        23.0,
                    )
                } else if matches!(node.kind.as_str(), "mid_side" | "stock_mid_side") {
                    // Give the section heading and the separate WIDTH caption
                    // distinct lines above the unchanged fader and value well.
                    label_gap = 5.0;
                    (pos2(width * (0.25 + index as f32 * 0.5), 133.0), 27.0)
                } else if matches!(node.kind.as_str(), "mix6" | "stock_mix") {
                    (pos2(width * 0.5, 125.0), 32.0)
                } else if matches!(node.kind.as_str(), "analog_vco" | "ladder_vcf") {
                    voice_parameter_position(index, width)
                } else {
                    // Stacked generic labels share the space between the prior
                    // numeric well and this fader. Keep the larger print below
                    // that well without moving controls or their hit regions.
                    if matches!(count, 2 | 3) && index > 0 {
                        label_gap = 3.0;
                    }
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
                let wave_rate = node.kind == "wavetable_lfo" && *name == "rate";
                let stock_rate = node.kind == "stock_lfo" && *name == "rate";
                let stock_pitch = node.kind == "stock_vco" && *name == "frequency";
                let stock_tempo = node.kind == "stock_seq3" && *name == "tempo";
                let observed_clock = stock_lfo_clock(self.module_lights.get(&node.id));
                let hz_rate = wave_rate || stock_rate || (node.kind == "random" && *name == "rate");
                let clock_ratio = (stock_rate && observed_clock.is_some_and(|clock| clock != 2.0))
                    || (((node.kind == "multi_lfo" && geometric) || wave_rate)
                        && patch
                            .cables
                            .iter()
                            .any(|c| c.to == node.id && c.input == "clock"));
                let gain_power = (node.kind == "vca_mix").then_some(if *name == "mix_level" {
                    1.0
                } else {
                    2.0
                });
                let fade_percent = (node.kind == "stock_seq3"
                    && matches!(*name, "tempo_amount" | "steps_amount"))
                    || node.kind == "fade"
                    || (node.kind == "random" && !matches!(*name, "rate" | "unipolar" | "seed"));
                let vco_percent =
                    (matches!(node.kind.as_str(), "multi_vco" | "stock_lfo" | "stock_vco")
                        && matches!(*name, "pulse_width" | "fm_amount" | "pwm_amount"))
                        || (is_wavetable(&node.kind)
                            && matches!(*name, "position" | "fm_amount" | "position_cv_amount"));
                let slew_min = (node.kind == "process" && *name == "slew").then_some(min);
                let multiplier = node.params.get("multiplier").copied().unwrap_or(1.0);
                let scaled_gain = node.kind == "rescale" && *name == "gain";
                let display_scale = if node.kind == "stock_quantizer" {
                    12.0
                } else if clock_ratio {
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
                let display_value = if node.kind == "stock_quantizer" {
                    stock_quantizer_offset_value(value)
                } else if stock_tempo {
                    stock_seq3_tempo_value(value)
                } else if node.kind == "stock_vco" {
                    stock_vco_parameter_value(name, value)
                } else if node.kind == "stock_lfo" {
                    stock_lfo_parameter_value(name, value, observed_clock)
                } else if node.kind == "random" {
                    random_parameter_value(name, value)
                } else if wave_rate {
                    wavetable_rate_value(value, clock_ratio)
                } else if is_wavetable(&node.kind) {
                    wavetable_parameter_value(name, value)
                } else if let Some(power) = gain_power {
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
                    text: if stock_tempo {
                        (60.0 * value.exp2()).to_string()
                    } else if stock_pitch {
                        stock_vco_frequency_hz(value).to_string()
                    } else if node.kind == "random" && hz_rate {
                        value.exp2().clamp(0.002, 2000.0).to_string()
                    } else if hz_rate {
                        (value.exp2() * display_scale).to_string()
                    } else if let Some(power) = gain_power {
                        gain_db(value, power).to_string()
                    } else {
                        slew_min
                            .map_or(value * display_scale, |min| slew_display(value, min))
                            .to_string()
                    },
                    min: if stock_tempo {
                        60.0 * min.exp2()
                    } else if stock_pitch {
                        stock_vco_frequency_hz(min)
                    } else if node.kind == "random" && hz_rate {
                        0.002
                    } else if hz_rate {
                        min.exp2() * display_scale
                    } else if gain_power.is_some() {
                        f32::NEG_INFINITY
                    } else if slew_min.is_some() {
                        0.0
                    } else {
                        min * display_scale
                    },
                    max: if stock_tempo {
                        60.0 * max.exp2()
                    } else if stock_pitch {
                        stock_vco_frequency_hz(max)
                    } else if node.kind == "random" && hz_rate {
                        2000.0
                    } else if hz_rate {
                        max.exp2() * display_scale
                    } else if let Some(power) = gain_power {
                        gain_db(max, power)
                    } else if slew_min.is_some() {
                        max.exp2() * 1000.0
                    } else {
                        max * display_scale
                    },
                    step: step.map(|step| step * display_scale),
                    display_scale,
                    display_unit: if node.kind == "stock_quantizer" {
                        "st"
                    } else if stock_tempo {
                        "BPM"
                    } else if stock_pitch {
                        "Hz"
                    } else if gain_power.is_some() {
                        "dB"
                    } else if slew_min.is_some() {
                        "ms/V"
                    } else if clock_ratio || (scaled_gain && multiplier != 1.0) {
                        "x"
                    } else if hz_rate {
                        "Hz"
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
                let value_offset = if matches!(node.kind.as_str(), "stock_lfo" | "stock_vco") {
                    16.0
                } else if matches!(node.kind.as_str(), "multi_lfo" | "multi_vco") {
                    18.0
                } else if is_wavetable(&node.kind) {
                    15.0
                } else if node.kind == "vca_mix" {
                    11.0
                } else if node.kind == "fade" {
                    10.0
                } else if count >= 3 {
                    9.0
                } else {
                    15.0
                };
                let value_center = center + vec2(0.0, (radius + value_offset) * self.zoom);
                let readout_width = if node.kind == "random" {
                    if *name == "unipolar" {
                        120.0
                    } else {
                        78.0
                    }
                } else if is_wavetable(&node.kind) {
                    104.0
                } else if node.kind == "score_sequencer" {
                    96.0
                } else if node.kind == "stereo_delay" {
                    width / 4.0 - 12.0
                } else if matches!(
                    node.kind.as_str(),
                    "stereo_reverb" | "sidechain_compressor" | "drum_voice"
                ) {
                    width / 3.0 - 12.0
                } else if matches!(node.kind.as_str(), "stock_lfo" | "stock_vco") {
                    82.0
                } else if node.kind == "vca_mix" {
                    70.0
                } else if matches!(node.kind.as_str(), "seq3" | "stock_seq3") {
                    if node.kind == "stock_seq3" && sequence_cell(name).is_none() {
                        78.0
                    } else {
                        54.0
                    }
                } else if node.kind == "gates" {
                    110.0
                } else if count >= 4
                    || matches!(node.kind.as_str(), "mid_side" | "stock_mid_side")
                    || (index > 0 && matches!(node.kind.as_str(), "analog_vco" | "ladder_vcf"))
                {
                    width * 0.5 - 12.0
                } else {
                    width - 20.0
                };
                let wave_choices = if node.kind == "random" && *name == "unipolar" {
                    Some(
                        json!({"min":0,"max":1,"step":1,"values":[0,1],"labels":["Bipolar","Offset"]}),
                    )
                } else {
                    wavetable_choice_spec(&node.kind, name)
                };
                let choices = wave_choices
                    .as_ref()
                    .and_then(ParameterChoices::from_spec)
                    .or_else(|| ParameterChoices::from_spec(spec));
                let normalized = if geometric {
                    normalize_geometric(value, min, max)
                } else {
                    normalize(value, min, max, logarithmic)
                };
                let fader_track =
                    crate::panel_skin::parameter_fader_track(center, radius, self.zoom);
                let hardware =
                    generic_parameter_hardware(&node.kind, name, radius, choices.is_some());
                let knob_rect = match hardware {
                    GenericParameterHardware::Choice => {
                        Rect::from_center_size(center, vec2(readout_width, 24.0) * self.zoom)
                    }
                    GenericParameterHardware::Rotary { target_half, .. } => {
                        Rect::from_center_size(center, Vec2::splat(2.0 * target_half * self.zoom))
                    }
                    GenericParameterHardware::Fader => Rect::from_center_size(
                        center,
                        vec2(
                            (radius * 1.4).clamp(32.0, 50.0) * self.zoom,
                            fader_track.height() + 7.0 * self.zoom,
                        ),
                    ),
                };
                let mut knob_highlighted = false;
                if mode.is_interactive() {
                    let knob = ui.interact(
                        knob_rect,
                        ui.id().with(("knob", &node.id, name)),
                        if choices.is_some() {
                            Sense::click()
                        } else {
                            Sense::click_and_drag()
                        },
                    );
                    knob.widget_info(|| {
                        if let Some(choices) = choices {
                            egui::WidgetInfo::selected(
                                egui::WidgetType::RadioButton,
                                ui.is_enabled(),
                                true,
                                format!(
                                    "{} {}: {}",
                                    node.id,
                                    name,
                                    choices.label(choices.index(value))
                                ),
                            )
                        } else {
                            egui::WidgetInfo::slider(
                                true,
                                if node.kind == "stock_quantizer" {
                                    f64::from(value * 12.0)
                                } else if stock_pitch {
                                    f64::from(stock_vco_frequency_hz(value))
                                } else if hz_rate {
                                    f64::from(value.exp2() * display_scale)
                                } else if node.kind == "random" {
                                    f64::from(value * display_scale)
                                } else {
                                    f64::from(value)
                                },
                                if stock_pitch {
                                    format!("{} frequency: {}", node.id, display_value)
                                } else if hz_rate {
                                    format!("{} rate: {}", node.id, display_value)
                                } else if node.kind == "random" {
                                    format!("{} {}: {}", node.id, name, display_value)
                                } else {
                                    format!("{} {}", node.id, name)
                                },
                            )
                        }
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
                        ui.strong(if node.kind == "random" {
                            random_parameter_label(name)
                        } else {
                            parameter_label(name)
                        });
                        ui.label(format!("Current: {display_value}"));
                        let outcome = parameter_editor_contents(ui, self.editing.as_mut().unwrap());
                        match outcome {
                            ParameterEditOutcome::Keep => {}
                            ParameterEditOutcome::Apply(raw)
                            | ParameterEditOutcome::ApplyAndNavigate { raw, .. } => {
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
                    let enabled = interactive
                        && self.shortcuts_enabled
                        && self.editing.is_none()
                        && self.annotation_edit.is_none()
                        && !popup_open_at_frame_start
                        && !egui::Popup::is_any_open(ui.ctx());
                    let choice_value = choices.and_then(|choices| {
                        self.parameter_choice_input(ui, &knob, choices, value, enabled)
                    });
                    let input = if choices.is_some() {
                        FaderInput::default()
                    } else if matches!(hardware, GenericParameterHardware::Rotary { .. }) {
                        self.parameter_knob_input(ui, &knob, normalized, enabled)
                    } else {
                        self.parameter_fader_input(ui, &knob, fader_track, normalized, enabled)
                    };
                    if interactive
                        && self.shortcuts_enabled
                        && self.editing.is_none()
                        && self.annotation_edit.is_none()
                        && !egui::Popup::is_any_open(ui.ctx())
                        && ((knob.is_pointer_button_down_on()
                            && ui.input(|i| i.pointer.primary_down()))
                            || knob.clicked_by(PointerButton::Primary)
                            || knob.drag_started_by(PointerButton::Primary))
                    {
                        self.select_primary(node.id.clone());
                    }
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
                    let from_normalized = |n: f32| {
                        if geometric {
                            denormalize_geometric(n.clamp(0.0, 1.0), min, max)
                        } else {
                            denormalize(n.clamp(0.0, 1.0), min, max, logarithmic)
                        }
                    };
                    let new_value = if choices.is_some() {
                        choice_value
                    } else if input.reset {
                        Some(default)
                    } else if let Some(high) = input.boundary {
                        Some(if high { max } else { min })
                    } else if input.nudge != 0 {
                        Some(quantize(
                            if let Some(step) = step {
                                value + step * input.nudge as f32
                            } else {
                                from_normalized(normalized + fine * input.nudge as f32)
                            },
                            min,
                            max,
                            step,
                        ))
                    } else {
                        input
                            .normalized
                            .map(|n| quantize(from_normalized(n), min, max, step))
                    };
                    if let Some(new_value) = new_value.filter(|new_value| *new_value != value) {
                        let mut args = json!({"id":node.id,"parameter":name,"value":new_value,
                            "expected_revision":revision});
                        if let Some(group) = input.history_group {
                            args["history_group"] = json!(group);
                        }
                        action = Some(RackAction::Command("set_parameter", args));
                    }
                    knob_highlighted = knob.hovered() || knob.dragged() || knob.has_focus();
                    if choices.is_some() {
                        knob.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!(
                            "{}: {}\n{}\nClick a labeled choice. Arrow keys choose a neighboring value; Home/End choose the first/last. Clicking the selected choice leaves it unchanged. Click the value or right-click for exact entry.",
                            name, display_value, description
                        ));
                    } else {
                        knob.on_hover_cursor(egui::CursorIcon::ResizeVertical).on_hover_text(format!(
                        "{}: {}\n{}\nClick and drag the {} to change its saved position. Cmd/Ctrl-drag for fine adjustment; add Shift for finer. Up/Down adjusts, Home/End selects limits. Double-click resets; click the number or right-click to type.",
                        name, display_value,
                        if clock_ratio { "Multiplier of measured clock frequency. Free-runs until two rising edges arrive; clock changes speed without resetting phase." } else { description },
                        if matches!(hardware, GenericParameterHardware::Rotary { .. }) { "rotary" } else { "fader" }
                    ));
                    }
                }
                if let Some(choices) = choices {
                    paint_parameter_choices(
                        &painter,
                        knob_rect,
                        choices,
                        value,
                        knob_highlighted,
                        crate::panel_skin::family_accent(&node.kind),
                        self.zoom,
                    );
                } else if let GenericParameterHardware::Rotary { body_radius, .. } = hardware {
                    crate::panel_skin::signal_atlas_rotary(
                        &painter,
                        center,
                        normalized,
                        knob_highlighted,
                        body_radius,
                        self.zoom,
                    );
                } else {
                    crate::panel_skin::parameter_fader(
                        &painter,
                        fader_track,
                        normalized,
                        knob_highlighted,
                        crate::panel_skin::family_accent(&node.kind),
                        self.zoom,
                    );
                }
                crate::panel_skin::control_label(
                    &painter,
                    center - vec2(0.0, (radius + label_gap) * self.zoom),
                    if matches!(node.kind.as_str(), "seq3" | "stock_seq3")
                        && sequence_cell(name).is_some()
                    {
                        ""
                    } else if node.kind == "stock_seq3" {
                        stock_seq3_parameter_label(name)
                    } else if node.kind == "stock_quantizer" {
                        "OFFSET (SEMITONES)"
                    } else if node.kind == "stock_vco" {
                        stock_vco_parameter_label(name)
                    } else if node.kind == "stock_lfo" {
                        stock_lfo_parameter_label(name, clock_ratio)
                    } else if node.kind == "random" {
                        random_parameter_label(name)
                    } else if wave_rate && clock_ratio {
                        "CLOCK RATIO"
                    } else if is_wavetable(&node.kind) {
                        wavetable_parameter_label(name)
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
                    readout_width * self.zoom,
                    self.zoom,
                );
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
                    if let Some(cue) = self
                        .construction_animation
                        .parameter(&node.id, &node.kind, name)
                    {
                        let old = cue.old;
                        let old_text = if node.kind == "stock_quantizer" {
                            stock_quantizer_offset_value(old)
                        } else if stock_tempo {
                            stock_seq3_tempo_value(old)
                        } else if node.kind == "stock_vco" {
                            stock_vco_parameter_value(name, old)
                        } else if node.kind == "stock_lfo" {
                            stock_lfo_parameter_value(name, old, observed_clock)
                        } else if node.kind == "random" {
                            random_parameter_value(name, old)
                        } else if wave_rate {
                            wavetable_rate_value(old, clock_ratio)
                        } else if let Some(power) = gain_power {
                            gain_db_display(old, power)
                        } else if fade_percent || vco_percent {
                            format!("{:.1}%", old * 100.0)
                        } else if clock_ratio {
                            format!("{:.4}x", old * display_scale)
                        } else if let Some(min) = slew_min {
                            format!("{:.2} ms/V", slew_display(old, min))
                        } else if scaled_gain {
                            format!(
                                "{:.2}{}",
                                old * display_scale,
                                if multiplier == 1.0 { "%" } else { "x" }
                            )
                        } else {
                            parameter_value(name, old, spec)
                        };
                        let old_normalized = if geometric {
                            normalize_geometric(old, min, max)
                        } else {
                            normalize(old, min, max, logarithmic)
                        };
                        self.register_construction_anchor(
                            ui,
                            node,
                            name,
                            crate::construction_motion::Anchor {
                                rect: knob_rect.union(value_response.rect),
                                old_text,
                                new_text: display_value.clone(),
                                marker: step.is_none().then_some(
                                    if let GenericParameterHardware::Rotary {
                                        body_radius,
                                        target_half,
                                    } = hardware
                                    {
                                        crate::construction_motion::Marker::Rotary {
                                            center,
                                            radius: (body_radius + 4.0).min(target_half)
                                                * self.zoom,
                                            old: old_normalized,
                                            new: normalized,
                                        }
                                    } else {
                                        crate::construction_motion::Marker::VerticalSlider {
                                            rect: fader_track,
                                            old: old_normalized,
                                            new: normalized,
                                        }
                                    },
                                ),
                            },
                            &[value_response.id, ui.id().with(("knob", &node.id, name))],
                        );
                    }
                    if self.precision_collects(node) {
                        self.register_precision_control(
                            make_edit(ParameterEditSurface::Window),
                            value_response.rect,
                        );
                    }
                    self.precision_affordance(ui, &value_response);
                    if value_response.clicked() || self.take_precision_request(&node.id, name) {
                        consume_parameter_activation(ui.ctx());
                        self.open_precision_editor(
                            make_edit(ParameterEditSurface::Window),
                            value_response.rect,
                        );
                    }
                    value_response.on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Click to type a value or expression. With multiple type-entry controls, Tab applies and moves to the next one.");
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
                if !studio_mixer_input_visible(node, port) {
                    continue;
                }
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
                        &node.kind,
                        port,
                        if node.kind == "stock_noise" {
                            stock_noise_port_label(port)
                        } else if node.kind == "studio_mixer" {
                            studio_mixer_port_label(port)
                        } else if node.kind == "stock_vca_mix" {
                            stock_vca_mix_port_label(port)
                        } else if node.kind == "stock_process" {
                            stock_process_port_label(port)
                        } else if node.kind == "stock_octave" {
                            stock_octave_port_label(port)
                        } else if node.kind == "stock_vca" {
                            stock_vca_port_label(port)
                        } else if node.kind == "stock_vcf" {
                            stock_vcf_port_label(port)
                        } else if node.kind == "random" {
                            random_port_label(port)
                        } else if is_wavetable(&node.kind) {
                            wavetable_port_label(port)
                        } else if node.kind == "stock_delay" {
                            stock_delay_port_label(port)
                        } else {
                            port
                        },
                        false,
                        false,
                        self.zoom,
                        is_voltage_utility(&node.kind)
                            || matches!(
                                node.kind.as_str(),
                                "sh_asr"
                                    | "stock_seq3"
                                    | "stock_quantizer"
                                    | "stock_noise"
                                    | "stock_process"
                            ),
                        port_material_scale(&node.kind),
                    );
                    continue;
                }
                let jack = jack_with_minimum_target(
                    ui,
                    &painter,
                    location,
                    &node.kind,
                    port,
                    ui.id().with(("input", &node.id, port)),
                    if node.kind == "stock_noise" {
                        stock_noise_port_label(port)
                    } else if node.kind == "studio_mixer" {
                        studio_mixer_port_label(port)
                    } else if node.kind == "stock_vca_mix" {
                        stock_vca_mix_port_label(port)
                    } else if node.kind == "stock_process" {
                        stock_process_port_label(port)
                    } else if node.kind == "stock_octave" {
                        stock_octave_port_label(port)
                    } else if node.kind == "stock_vca" {
                        stock_vca_port_label(port)
                    } else if node.kind == "stock_vcf" {
                        stock_vcf_port_label(port)
                    } else if node.kind == "random" {
                        random_port_label(port)
                    } else if is_wavetable(&node.kind) {
                        wavetable_port_label(port)
                    } else if node.kind == "stock_delay" {
                        stock_delay_port_label(port)
                    } else {
                        port
                    },
                    connected,
                    false,
                    self.zoom,
                    is_voltage_utility(&node.kind)
                        || matches!(
                            node.kind.as_str(),
                            "sh_asr"
                                | "stock_seq3"
                                | "stock_quantizer"
                                | "stock_noise"
                                | "stock_process"
                        ),
                    if matches!(node.kind.as_str(), "sh_asr" | "stock_lfo" | "stock_vco") {
                        12.0
                    } else {
                        18.0
                    },
                    port_material_scale(&node.kind),
                    hover_target.as_ref()
                        == Some(&CableEnd::Input {
                            node: node.id.clone(),
                            port: port.to_string(),
                        }),
                );
                hovered_jack_response |= jack.hovered();
                long_touched_jack_response |= jack.long_touched();
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
                let secondary_owner = secondary_click_target.as_ref() == Some(&end);
                if secondary_owner {
                    self.open_cable_menu(end.clone(), patch);
                }
                if self.requested_cable_menu.as_ref() == Some(&end) {
                    self.open_cable_menu(end.clone(), patch);
                    egui::Popup::open_id(ui.ctx(), jack.id.with("popup"));
                    self.requested_cable_menu = None;
                }
                if deferred_touch_menu.as_ref() == Some(&end) {
                    self.open_cable_menu(end.clone(), patch);
                    egui::Popup::open_id(ui.ctx(), jack.id.with("popup"));
                }
                let mut popup = egui::Popup::context_menu(&jack)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .open_memory(if secondary_owner {
                        Some(egui::SetOpenCommand::Bool(true))
                    } else if jack.clicked() {
                        Some(egui::SetOpenCommand::Bool(false))
                    } else {
                        None
                    });
                if !secondary_owner
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
                details.push_str(&self.port_signal_description(
                    &node.id,
                    openpatch_core::PortDirection::Input,
                    port,
                ));
                if hover_target.as_ref() == Some(&end) {
                    nearest_jack_tooltip = Some((
                        jack,
                        format!("{}\n{details}", jack_instruction(false)),
                        location,
                        port_material_scale(&node.kind),
                    ));
                }
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
                        &node.kind,
                        port,
                        if node.kind == "stock_noise" {
                            stock_noise_port_label(port)
                        } else if node.kind == "studio_mixer" {
                            studio_mixer_port_label(port)
                        } else if node.kind == "adsr_eg" {
                            "ENV"
                        } else if node.kind == "stock_vca_mix" {
                            stock_vca_mix_port_label(port)
                        } else if node.kind == "stock_process" {
                            stock_process_port_label(port)
                        } else if node.kind == "stock_octave" {
                            stock_octave_port_label(port)
                        } else if node.kind == "stock_vca" {
                            stock_vca_port_label(port)
                        } else if node.kind == "stock_vcf" {
                            stock_vcf_port_label(port)
                        } else if node.kind == "random" {
                            random_port_label(port)
                        } else if is_wavetable(&node.kind) {
                            wavetable_port_label(port)
                        } else if node.kind == "stock_delay" {
                            stock_delay_port_label(port)
                        } else {
                            port
                        },
                        false,
                        true,
                        self.zoom,
                        is_voltage_utility(&node.kind)
                            || matches!(
                                node.kind.as_str(),
                                "pulses"
                                    | "stock_seq3"
                                    | "stock_quantizer"
                                    | "colored_noise"
                                    | "random_values"
                                    | "sh_asr"
                                    | "midi_cc"
                                    | "midi_gate"
                                    | "stock_noise"
                                    | "stock_process"
                            ),
                        port_material_scale(&node.kind),
                    );
                    if node.kind != "stock_noise" {
                        self.paint_output_signal_light(&painter, node, port_index, output_point);
                    }
                    continue;
                }
                let jack = jack_with_minimum_target(
                    ui,
                    &painter,
                    output_point,
                    &node.kind,
                    port,
                    ui.id().with(("output", &node.id, port)),
                    if node.kind == "stock_noise" {
                        stock_noise_port_label(port)
                    } else if node.kind == "studio_mixer" {
                        studio_mixer_port_label(port)
                    } else if node.kind == "adsr_eg" {
                        "ENV"
                    } else if node.kind == "stock_vca_mix" {
                        stock_vca_mix_port_label(port)
                    } else if node.kind == "stock_process" {
                        stock_process_port_label(port)
                    } else if node.kind == "stock_octave" {
                        stock_octave_port_label(port)
                    } else if node.kind == "stock_vca" {
                        stock_vca_port_label(port)
                    } else if node.kind == "stock_vcf" {
                        stock_vcf_port_label(port)
                    } else if node.kind == "random" {
                        random_port_label(port)
                    } else if is_wavetable(&node.kind) {
                        wavetable_port_label(port)
                    } else if node.kind == "stock_delay" {
                        stock_delay_port_label(port)
                    } else {
                        port
                    },
                    connected,
                    true,
                    self.zoom,
                    is_voltage_utility(&node.kind)
                        || matches!(
                            node.kind.as_str(),
                            "pulses"
                                | "stock_seq3"
                                | "stock_quantizer"
                                | "colored_noise"
                                | "random_values"
                                | "sh_asr"
                                | "midi_cc"
                                | "midi_gate"
                                | "stock_noise"
                                | "stock_process"
                        ),
                    if matches!(
                        node.kind.as_str(),
                        "colored_noise"
                            | "random_values"
                            | "sh_asr"
                            | "midi_cc"
                            | "midi_gate"
                            | "stock_lfo"
                            | "stock_vco"
                            | "stock_noise"
                    ) {
                        12.0
                    } else {
                        18.0
                    },
                    port_material_scale(&node.kind),
                    hover_target.as_ref()
                        == Some(&CableEnd::Output {
                            node: node.id.clone(),
                            port: port.to_string(),
                        }),
                );
                hovered_jack_response |= jack.hovered();
                long_touched_jack_response |= jack.long_touched();
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
                if node.kind != "stock_noise" {
                    self.paint_output_signal_light(&painter, node, port_index, output_point);
                }
                if node.kind != "stock_noise"
                    && patch.output.as_ref() == Some(&node.id)
                    && patch.output_port == *port
                {
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
                let secondary_owner = secondary_click_target.as_ref() == Some(&end);
                if secondary_owner {
                    self.open_cable_menu(end.clone(), patch);
                }
                if self.requested_cable_menu.as_ref() == Some(&end) {
                    self.open_cable_menu(end.clone(), patch);
                    egui::Popup::open_id(ui.ctx(), jack.id.with("popup"));
                    self.requested_cable_menu = None;
                }
                if deferred_touch_menu.as_ref() == Some(&end) {
                    self.open_cable_menu(end.clone(), patch);
                    egui::Popup::open_id(ui.ctx(), jack.id.with("popup"));
                }
                let mut popup = egui::Popup::context_menu(&jack)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .open_memory(if secondary_owner {
                        Some(egui::SetOpenCommand::Bool(true))
                    } else if jack.clicked() {
                        Some(egui::SetOpenCommand::Bool(false))
                    } else {
                        None
                    });
                if !secondary_owner
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
                if hover_target.as_ref() == Some(&end) {
                    nearest_jack_tooltip = Some((
                        jack,
                        format!("{}\n{}.{}\nCapacity: up to {capacity} channels; {units}.{}\nDrag moves the top OUT plug. Cmd/Ctrl-drag adds a cable; Cmd/Ctrl+Shift clones; Shift-click deletes the top cable.", jack_instruction(true), node.id, port, self.port_signal_description(&node.id, openpatch_core::PortDirection::Output, port)),
                        output_point,
                        port_material_scale(&node.kind),
                    ));
                }
            }
        }

        if long_touched_jack_response {
            if let Some(end) = long_touch_target {
                // The owning jack may have been painted before egui reports a
                // long touch on another overlapping response. Defer only
                // this touch, with graph identity, to the next rack frame.
                if let Some(node) = patch.nodes.iter().find(|node| node.id == end.node()) {
                    self.deferred_touch_menu = Some((end, node.kind.clone(), revision));
                    ui.ctx().request_repaint();
                }
            }
        }
        if let Some((jack, text, center, port_scale)) = nearest_jack_tooltip {
            if jack.hovered() {
                jack.on_hover_text(text);
            } else if hovered_jack_response {
                painter.circle_stroke(
                    center,
                    13.0 * self.zoom * port_scale,
                    Stroke::new(1.8 * self.zoom, Color32::from_rgb(221, 157, 64)),
                );
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                if overlapping_jack_tooltip_ready(&jack) {
                    egui::Tooltip::for_widget(&jack).show(|ui| {
                        ui.label(text);
                    });
                }
            }
        }
        self.lighting.finish(&painter, lighting_start);
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
                output_point
                    + vec2(0.0, if node.kind == "compare" { 22.0 } else { 17.0 }) * self.zoom,
                brightness,
                self.zoom,
                &self.lighting,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn register_construction_anchor(
        &mut self,
        ui: &egui::Ui,
        node: &Node,
        name: &str,
        anchor: crate::construction_motion::Anchor,
        ids: &[egui::Id],
    ) {
        if self
            .construction_animation
            .parameter(&node.id, &node.kind, name)
            .is_none()
        {
            return;
        }
        let focused = ids.iter().any(|id| ui.ctx().memory(|m| m.has_focus(*id)));
        let interacting = ui.input(|i| {
            let pointer = i.pointer.interact_pos().or(i.pointer.hover_pos());
            ((i.pointer.any_pressed() || i.pointer.primary_down())
                && pointer.is_some_and(|p| anchor.rect.contains(p)))
                || (focused
                    && i.events.iter().any(|e| {
                        matches!(
                            e,
                            egui::Event::Key { pressed: true, .. } | egui::Event::Text(_)
                        )
                    }))
        });
        if interacting {
            self.construction_animation
                .interrupt_parameter(&node.id, name);
            return;
        }
        self.construction_anchors
            .insert((node.id.clone(), name.into()), anchor);
    }

    fn paint_construction_overlays(
        &mut self,
        ui: &egui::Ui,
        patch: &Patch,
        catalog: &Value,
        canvas: Rect,
        now: std::time::Instant,
    ) {
        use crate::construction_motion::{ease, fraction, Marker, MotionMode};
        if self.construction_animation.mode == MotionMode::Off {
            return;
        }
        let reduced = self.construction_animation.mode == MotionMode::Reduced;
        let painter = ui
            .painter()
            .with_clip_rect(canvas.intersect(ui.clip_rect()));
        let gold = Color32::from_rgb(225, 179, 105);
        for node in &patch.nodes {
            let Some(position) = self.positions.get(&node.id) else {
                continue;
            };
            let rect = Rect::from_min_size(
                canvas.min + self.pan + position.to_vec2() * self.zoom,
                vec2(module_width(node), PANEL_HEIGHT) * self.zoom,
            );
            if !painter.clip_rect().intersects(rect) {
                continue;
            }
            let header_focused = ui
                .ctx()
                .memory(|m| m.has_focus(ui.id().with(("module-header", &node.id))));
            let header_keyboard = header_focused
                && ui.input(|i| {
                    i.events.iter().any(|event| {
                        matches!(
                            event,
                            egui::Event::Key { pressed: true, .. } | egui::Event::Text(_)
                        )
                    })
                });
            if header_keyboard
                || ui.input(|i| {
                    i.pointer.any_pressed()
                        && i.pointer.interact_pos().is_some_and(|p| rect.contains(p))
                })
            {
                self.construction_animation.interrupt_module(&node.id);
            }
            if let Some(cue) = self
                .construction_animation
                .modules
                .iter()
                .find(|m| m.id == node.id && m.kind == node.kind)
            {
                let t = fraction(cue.started, now, if reduced { 300 } else { 320 });
                let fade = 1.0 - ease(t);
                if !reduced {
                    painter.rect_filled(
                        rect.shrink(self.zoom),
                        2.0 * self.zoom,
                        gold.gamma_multiply(0.065 * fade),
                    );
                }
                painter.rect_stroke(
                    rect.shrink(1.5 * self.zoom),
                    2.0 * self.zoom,
                    Stroke::new(1.5 * self.zoom, gold.gamma_multiply((1.0 - t) * 0.8)),
                    egui::StrokeKind::Inside,
                );
                painter.rect_stroke(
                    rect.shrink(4.0 * self.zoom),
                    1.0 * self.zoom,
                    Stroke::new(0.65 * self.zoom, gold.gamma_multiply(fade * 0.35)),
                    egui::StrokeKind::Inside,
                );
            }
            if let Some(cue) = self
                .construction_animation
                .focus
                .iter()
                .find(|m| m.id == node.id && m.kind == node.kind)
            {
                let t = fraction(cue.started, now, if reduced { 300 } else { 420 });
                painter.line_segment(
                    [
                        rect.min + vec2(14.0, 5.0) * self.zoom,
                        rect.right_top() + vec2(-14.0, 5.0) * self.zoom,
                    ],
                    Stroke::new(2.0 * self.zoom, gold.gamma_multiply((1.0 - t) * 0.5)),
                );
            }
            let header_width = if matches!(node.kind.as_str(), "stock_vco" | "stock_lfo") {
                174.0 * self.zoom
            } else {
                rect.width()
            };
            let header = Rect::from_min_size(rect.min, vec2(header_width, 65.0 * self.zoom));
            if header_keyboard
                || ui.input(|i| {
                    i.pointer.any_pressed()
                        && i.pointer.interact_pos().is_some_and(|p| header.contains(p))
                })
            {
                self.construction_animation
                    .parameters
                    .retain(|p| p.id != node.id);
            }
            let cues = self
                .construction_animation
                .parameters
                .iter()
                .filter(|p| p.id == node.id && p.kind == node.kind)
                .cloned()
                .collect::<Vec<_>>();
            let mut fallback = Vec::new();
            let mut fallback_fade = 0.0_f32;
            for cue in cues {
                let t = fraction(cue.started, now, if reduced { 300 } else { 360 });
                let fade = (1.0 - ((t - 0.7) / 0.3).clamp(0.0, 1.0)).clamp(0.0, 1.0);
                if self
                    .editing
                    .as_ref()
                    .is_some_and(|e| e.id == cue.id && e.parameter == cue.parameter)
                {
                    self.construction_animation
                        .interrupt_parameter(&cue.id, &cue.parameter);
                    continue;
                }
                if let Some(anchor) = self
                    .construction_anchors
                    .get(&(cue.id.clone(), cue.parameter.clone()))
                {
                    painter.rect_stroke(
                        anchor.rect.expand(3.0 * self.zoom),
                        4.0 * self.zoom,
                        Stroke::new(1.1 * self.zoom, gold.gamma_multiply(0.75 * fade)),
                        egui::StrokeKind::Outside,
                    );
                    if !reduced {
                        match &anchor.marker {
                            Some(Marker::Rotary {
                                center,
                                radius,
                                old,
                                new,
                            }) => {
                                // A fading cursor and short trail follow the same
                                // bounded angular scale as the actual knob.
                                let previous = ease((t - 0.14).max(0.0));
                                let current = ease(t);
                                for segment in 0..10 {
                                    let a = previous + (current - previous) * segment as f32 / 10.0;
                                    let b = previous
                                        + (current - previous) * (segment + 1) as f32 / 10.0;
                                    let from = crate::construction_motion::rotary_position(
                                        *center,
                                        *radius,
                                        old + (new - old) * a,
                                    );
                                    let to = crate::construction_motion::rotary_position(
                                        *center,
                                        *radius,
                                        old + (new - old) * b,
                                    );
                                    painter.line_segment(
                                        [from, to],
                                        Stroke::new(
                                            2.0 * self.zoom,
                                            gold.gamma_multiply(fade * (segment + 1) as f32 / 10.0),
                                        ),
                                    );
                                }
                                let position = crate::construction_motion::rotary_position(
                                    *center,
                                    *radius,
                                    old + (new - old) * current,
                                );
                                painter.circle_filled(
                                    position,
                                    5.0 * self.zoom,
                                    gold.gamma_multiply(0.12 * fade),
                                );
                                painter.circle_filled(
                                    position,
                                    3.2 * self.zoom,
                                    gold.gamma_multiply(0.3 * fade),
                                );
                                painter.circle_filled(
                                    position,
                                    1.7 * self.zoom,
                                    gold.gamma_multiply(fade),
                                );
                            }
                            Some(Marker::VerticalSlider { rect, old, new }) => {
                                let y = rect.bottom()
                                    - rect.height() * (old + (new - old) * ease(t)).clamp(0.0, 1.0);
                                painter.line_segment(
                                    [
                                        pos2(rect.left() - 5.0 * self.zoom, y),
                                        pos2(rect.right() + 5.0 * self.zoom, y),
                                    ],
                                    Stroke::new(1.3 * self.zoom, gold.gamma_multiply(fade)),
                                );
                            }
                            Some(Marker::Slider { rect, old, new }) => {
                                let x = rect.left()
                                    + rect.width() * (old + (new - old) * ease(t)).clamp(0.0, 1.0);
                                painter.line_segment(
                                    [
                                        pos2(x, rect.top() - 3.0 * self.zoom),
                                        pos2(x, rect.bottom() + 3.0 * self.zoom),
                                    ],
                                    Stroke::new(1.3 * self.zoom, gold.gamma_multiply(fade)),
                                );
                            }
                            None => {}
                        }
                    }
                    let label = format!("{} → {}", anchor.old_text, anchor.new_text);
                    let center = pos2(
                        anchor.rect.center().x,
                        (anchor.rect.top() - 9.0 * self.zoom).max(rect.top() + 74.0 * self.zoom),
                    );
                    paint_motion_badge(&painter, center, &label, rect, self.zoom, fade);
                } else {
                    let spec = catalog["modules"]
                        .as_array()
                        .and_then(|m| m.iter().find(|m| m["kind"] == node.kind))
                        .and_then(|m| m["params"].get(&cue.parameter));
                    let value = spec.map_or_else(
                        || format!("{}", cue.new),
                        |spec| {
                            spec["labels"]
                                .as_array()
                                .and_then(|labels| {
                                    let index = spec["values"]
                                        .as_array()
                                        .and_then(|values| {
                                            values.iter().position(|v| {
                                                v.as_f64().is_some_and(|v| v as f32 == cue.new)
                                            })
                                        })
                                        .or_else(|| {
                                            (cue.new >= 0.0 && cue.new.fract() == 0.0)
                                                .then_some(cue.new as usize)
                                        });
                                    index
                                        .and_then(|index| labels.get(index))
                                        .and_then(Value::as_str)
                                        .map(str::to_owned)
                                })
                                .unwrap_or_else(|| parameter_value(&cue.parameter, cue.new, spec))
                        },
                    );
                    fallback_fade = fallback_fade.max(fade);
                    fallback.push(format!("{}  {}", parameter_label(&cue.parameter), value));
                }
            }
            if !fallback.is_empty() {
                paint_motion_badge(
                    &painter,
                    rect.min + vec2(rect.width() * 0.5, 51.0 * self.zoom),
                    &fallback.join(" · "),
                    rect,
                    self.zoom,
                    fallback_fade * 0.95,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn shift_register_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        rect: Rect,
        revision: u64,
        allow_press: bool,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let screen = |local: Rect| {
            Rect::from_min_max(
                point(local.min.x, local.min.y),
                point(local.max.x, local.max.y),
            )
        };
        let painter = ui.painter().clone();
        let mut action = None;
        let random = node.params.get("random").copied().unwrap_or(0.0) != 0.0;
        let random_rect = screen(shift_register_random_rect());
        let response = ui.interact(
            random_rect,
            ui.id().with((&node.id, "shift-register-random")),
            Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                ui.is_enabled(),
                random,
                "Random voltage normal",
            )
        });
        painter.rect_filled(
            random_rect,
            3.0 * zoom,
            if random {
                Color32::from_rgb(218, 176, 103)
            } else {
                Color32::from_rgb(186, 188, 179)
            },
        );
        painter.rect_stroke(
            random_rect,
            3.0 * zoom,
            Stroke::new(zoom, MUTED),
            egui::StrokeKind::Inside,
        );
        painter.text(
            random_rect.center(),
            Align2::CENTER_CENTER,
            if random { "RND ON" } else { "RND OFF" },
            crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
            INK,
        );
        if response.clicked() {
            consume_parameter_activation(ui.ctx());
            action = Some(RackAction::Command(
                "set_parameter",
                json!({"id":node.id,"parameter":"random","value":if random {0} else {1},"expected_revision":revision}),
            ));
        }
        response.on_hover_text("When IN 1 is unpatched, capture a random voltage instead of 0 V. Each row-1 trigger advances the generator, even with IN 1 patched. RND changes future captures only.");
        painter.text(
            point(104.0, 88.0),
            Align2::LEFT_CENTER,
            "RANGE",
            crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
            INK,
        );
        if let Some(spec) = module.and_then(|module| module["params"].get("range")) {
            if let Some(RackAction::Command(tool, mut args)) = self.choice_parameter(
                ui,
                node,
                "range",
                spec,
                screen(shift_register_range_rect()),
                &RANDOM_VALUES_RANGES,
            ) {
                args["expected_revision"] = json!(revision);
                action = Some(RackAction::Command(tool, args));
            }
        }
        painter.text(
            point(20.0, 118.0),
            Align2::LEFT_CENTER,
            "SEED",
            crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
            INK,
        );
        if let Some(spec) = module.and_then(|module| module["params"].get("seed")) {
            let value = node
                .params
                .get("seed")
                .copied()
                .unwrap_or(spec["default"].as_f64().unwrap_or(1.0) as f32);
            let seed_rect = screen(shift_register_seed_rect());
            crate::panel_skin::readout(
                &painter,
                seed_rect.center(),
                &format!("{value:.0}"),
                seed_rect.width(),
                zoom,
            );
            let response = ui.interact(
                seed_rect,
                ui.id().with((&node.id, "shift-register-seed")),
                Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    format!("Edit {} seed value, {value:.0}", node.id),
                )
            });
            let make_edit = |surface| ParameterEdit {
                id: node.id.clone(),
                kind: node.kind.clone(),
                parameter: "seed".into(),
                revision,
                surface,
                menu_popup: None,
                stale: false,
                text: format!("{value:.0}"),
                min: 0.0,
                max: 16_777_215.0,
                step: Some(1.0),
                display_scale: 1.0,
                display_unit: String::new(),
                slew_min: None,
                gain_power: None,
                raw_max: 16_777_215.0,
                focus: true,
            };
            if self.precision_collects(node) {
                self.register_precision_control(make_edit(ParameterEditSurface::Window), seed_rect);
            }
            self.precision_affordance(ui, &response);
            if response.clicked() {
                consume_parameter_activation(ui.ctx());
                self.open_precision_editor(make_edit(ParameterEditSurface::Window), seed_rect);
            }
            if self
                .requested_parameter_menu
                .as_ref()
                .is_some_and(|(id, name)| id == &node.id && name == "seed")
            {
                egui::Popup::open_id(ui.ctx(), response.id.with("popup"));
                self.requested_parameter_menu = None;
            }
            let mut popup = egui::Popup::context_menu(&response)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
            if !response.secondary_clicked()
                && egui::Popup::position_of_id(ui.ctx(), response.id.with("popup")).is_none()
            {
                popup = popup.anchor(egui::PopupAnchor::Position(response.rect.right_bottom()));
            }
            popup.show(|ui| {
                if !self.editing.as_ref().is_some_and(|edit| {
                    edit.surface == ParameterEditSurface::KnobMenu
                        && edit.id == node.id
                        && edit.parameter == "seed"
                }) {
                    self.editing = Some(make_edit(ParameterEditSurface::KnobMenu));
                }
                self.editing.as_mut().unwrap().menu_popup = Some(response.id.with("popup"));
                ui.set_min_width(244.0);
                ui.strong("SEED · 24 BIT");
                match parameter_editor_contents(ui, self.editing.as_mut().unwrap()) {
                    ParameterEditOutcome::Keep => {}
                    ParameterEditOutcome::Apply(raw)
                    | ParameterEditOutcome::ApplyAndNavigate { raw, .. } => {
                        action = Some(parameter_edit_action(&self.editing.take().unwrap(), raw));
                        ui.close();
                    }
                    ParameterEditOutcome::Cancel => {
                        self.editing = None;
                        ui.close();
                    }
                }
            });
            response.on_hover_text("Click to enter an exact 24-bit seed or expression. Changing the seed resets the random generator while preserving held voltages. Range and RND changes affect future captures only.");
        }
        let enabled = ui.is_enabled()
            && self.shortcuts_enabled
            && self.performance_available
            && !node.bypassed
            && ui.input(|i| i.focused);
        for (index, label, press, release) in [
            (0, "PUSH", "push_press", "push_release"),
            (1, "CLEAR", "reset_press", "reset_release"),
        ] {
            let button_rect = screen(shift_register_button_rect(index));
            let response = ui.interact(
                button_rect,
                ui.id().with((&node.id, "shift-register-button", index)),
                Sense::click_and_drag(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    enabled,
                    format!("Shift Register {} {label} performance button", node.id),
                )
            });
            let pressed = self
                .module_controls
                .get(&node.id)
                .is_some_and(|state| state.pressed_mask & (1 << index) != 0);
            painter.rect_filled(
                button_rect.translate(vec2(1.0, 2.0) * zoom),
                3.0 * zoom,
                Color32::from_black_alpha(70),
            );
            painter.rect_filled(
                button_rect,
                3.0 * zoom,
                if pressed {
                    Color32::from_rgb(226, 197, 143)
                } else if enabled && response.hovered() {
                    Color32::from_rgb(246, 237, 210)
                } else {
                    Color32::from_rgb(190, 186, 175)
                },
            );
            painter.rect_stroke(
                button_rect,
                3.0 * zoom,
                Stroke::new(zoom, MUTED),
                egui::StrokeKind::Inside,
            );
            painter.text(
                button_rect.center(),
                Align2::CENTER_CENTER,
                label,
                crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
                if enabled { INK } else { MUTED },
            );
            if enabled
                && allow_press
                && self.held_manual.is_none()
                && self.editing.is_none()
                && self.annotation_edit.is_none()
                && self.blank_resize.is_none()
                && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
            {
                if response.clicked_by(PointerButton::Primary) {
                    self.select_primary(node.id.clone());
                    action = Some(RackAction::ManualClick {
                        id: node.id.clone(),
                        press,
                        release,
                    });
                } else if response.is_pointer_button_down_on() {
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
            }
            response.on_hover_text(if !enabled { "Performance buttons require a live audio engine and an active module." } else if index == 0 { "Capture row 1 once; its trigger propagates through unpatched TRIG jacks. Holding does not repeat. Each row samples the previous held row before the capture." } else { "Clear all eight held voltages once. Holding does not continually clear. CLEAR wins over simultaneous captures; random draws still advance." });
        }
        for (x, label) in [
            (80.0, "IN"),
            (146.0, "TRIG"),
            (263.0, "HELD VOLTAGE"),
            (360.0, "OUT"),
        ] {
            painter.text(
                point(x, 139.0),
                Align2::CENTER_CENTER,
                label,
                crate::panel_skin::print_font(ui.ctx(), 9.0 * zoom),
                MUTED,
            );
        }
        // Dashed printed traces describe normalled routes, not live cables.
        // Signal rails link the previous OUT to this IN; TRIG inherits above.
        let guide = Stroke::new(0.65 * zoom, Color32::from_rgb(151, 151, 141));
        for index in 1..8 {
            let y = shift_register_row_y(index);
            let previous = shift_register_row_y(index - 1);
            for [from, to] in [
                [point(360.0, previous + 10.0), point(360.0, previous + 15.0)],
                [point(360.0, previous + 15.0), point(43.0, previous + 15.0)],
                [point(43.0, previous + 15.0), point(43.0, y)],
                [point(43.0, y), point(69.0, y)],
                [point(146.0, previous + 11.0), point(146.0, y - 11.0)],
            ] {
                painter.add(egui::Shape::dashed_line(
                    &[from, to],
                    guide,
                    2.5 * zoom,
                    2.0 * zoom,
                ));
            }
            painter.line_segment([point(65.0, y - 2.5), point(69.0, y)], guide);
            painter.line_segment([point(65.0, y + 2.5), point(69.0, y)], guide);
        }
        painter.text(
            point(45.0, shift_register_row_y(0)),
            Align2::CENTER_CENTER,
            "0 / RND",
            crate::panel_skin::print_font(ui.ctx(), 7.0 * zoom),
            MUTED,
        );
        painter.add(egui::Shape::dashed_line(
            &[
                point(61.0, shift_register_row_y(0)),
                point(69.0, shift_register_row_y(0)),
            ],
            guide,
            2.5 * zoom,
            2.0 * zoom,
        ));
        let state = self.shift_register_states.get(&node.id);
        for index in 0..8 {
            let y = shift_register_row_y(index);
            painter.text(
                point(20.0, y),
                Align2::CENTER_CENTER,
                format!("{}", index + 1),
                crate::panel_skin::print_font(ui.ctx(), 10.0 * zoom),
                INK,
            );
            let readout = screen(shift_register_readout_rect(index));
            let display = state.map_or_else(
                || "UNAVAILABLE".to_owned(),
                |state| format!("{:+.3} V", state["values"][index].as_f64().unwrap()),
            );
            crate::panel_skin::readout(&painter, readout.center(), &display, readout.width(), zoom);
            ui.interact(readout, ui.id().with((&node.id, "shift-register-readout", index)), Sense::hover()).on_hover_text(format!("Row {} · mono, input channel 1 only.\n{}\nPUSH clocks row 1. Unpatched TRIG rows 2–8 inherit the preceding row's capture event; patched TRIG breaks inheritance. Dashed lines show unpatched normals.", index + 1, if index == 0 { "Unpatched IN captures 0 V, or random voltage with RND enabled.".to_owned() } else { format!("Unpatched IN captures the OLD held OUT {} value.", index) }));
        }
        painter.text(
            point(195.0, 386.0),
            Align2::CENTER_CENTER,
            if state.is_none() {
                "HELD VALUES UNAVAILABLE · DASHED = NORMALS"
            } else if self.performance_available {
                "LIVE MONO VALUES · DASHED = NORMALS"
            } else {
                "LAST ENGINE STATE · DASHED = NORMALS"
            },
            crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
            MUTED,
        );
        action
    }

    fn random_values_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        rect: Rect,
        revision: u64,
        allow_press: bool,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let screen = |local: Rect| {
            Rect::from_min_max(
                point(local.min.x, local.min.y),
                point(local.max.x, local.max.y),
            )
        };
        let painter = ui.painter().clone();
        let mut action = None;
        for (name, y) in [("RANGE", 88.0), ("SEED", 114.0)] {
            painter.text(
                point(20.0, y),
                Align2::LEFT_CENTER,
                name,
                crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
                INK,
            );
        }
        if let Some(spec) = module.and_then(|module| module["params"].get("range")) {
            if let Some(RackAction::Command(tool, mut args)) = self.choice_parameter(
                ui,
                node,
                "range",
                spec,
                screen(random_values_range_rect()),
                &RANDOM_VALUES_RANGES,
            ) {
                args["expected_revision"] = json!(revision);
                action = Some(RackAction::Command(tool, args));
            }
        }
        if let Some(spec) = module.and_then(|module| module["params"].get("seed")) {
            let value = node
                .params
                .get("seed")
                .copied()
                .unwrap_or(spec["default"].as_f64().unwrap_or(1.0) as f32);
            let seed_rect = screen(random_values_seed_rect());
            crate::panel_skin::readout(
                &painter,
                seed_rect.center(),
                &format!("{value:.0}"),
                seed_rect.width(),
                zoom,
            );
            let response = ui.interact(
                seed_rect,
                ui.id().with((&node.id, "random-values-seed")),
                Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    format!("Edit {} seed value, {value:.0}", node.id),
                )
            });
            let make_edit = |surface| ParameterEdit {
                id: node.id.clone(),
                kind: node.kind.clone(),
                parameter: "seed".into(),
                revision,
                surface,
                menu_popup: None,
                stale: false,
                text: format!("{value:.0}"),
                min: 0.0,
                max: 16_777_215.0,
                step: Some(1.0),
                display_scale: 1.0,
                display_unit: String::new(),
                slew_min: None,
                gain_power: None,
                raw_max: 16_777_215.0,
                focus: true,
            };
            if self.precision_collects(node) {
                self.register_precision_control(make_edit(ParameterEditSurface::Window), seed_rect);
            }
            self.precision_affordance(ui, &response);
            if response.clicked() {
                consume_parameter_activation(ui.ctx());
                self.open_precision_editor(make_edit(ParameterEditSurface::Window), seed_rect);
            }
            if self
                .requested_parameter_menu
                .as_ref()
                .is_some_and(|(id, name)| id == &node.id && name == "seed")
            {
                egui::Popup::open_id(ui.ctx(), response.id.with("popup"));
                self.requested_parameter_menu = None;
            }
            let mut popup = egui::Popup::context_menu(&response)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
            if !response.secondary_clicked()
                && egui::Popup::position_of_id(ui.ctx(), response.id.with("popup")).is_none()
            {
                popup = popup.anchor(egui::PopupAnchor::Position(response.rect.right_bottom()));
            }
            popup.show(|ui| {
                if !self.editing.as_ref().is_some_and(|edit| {
                    edit.surface == ParameterEditSurface::KnobMenu
                        && edit.id == node.id
                        && edit.parameter == "seed"
                }) {
                    self.editing = Some(make_edit(ParameterEditSurface::KnobMenu));
                }
                self.editing.as_mut().unwrap().menu_popup = Some(response.id.with("popup"));
                ui.set_min_width(244.0);
                ui.strong("SEED · 24 BIT");
                match parameter_editor_contents(ui, self.editing.as_mut().unwrap()) {
                    ParameterEditOutcome::Keep => {}
                    ParameterEditOutcome::Apply(raw)
                    | ParameterEditOutcome::ApplyAndNavigate { raw, .. } => {
                        action = Some(parameter_edit_action(&self.editing.take().unwrap(), raw));
                        ui.close();
                    }
                    ParameterEditOutcome::Cancel => {
                        self.editing = None;
                        ui.close();
                    }
                }
            });
            response.on_hover_text("Click to enter an exact 24-bit seed or expression. Changing the seed rerolls every held value immediately. Range changes affect future draws only.");
        }
        let enabled = ui.is_enabled()
            && self.shortcuts_enabled
            && self.performance_available
            && !node.bypassed
            && ui.input(|i| i.focused);
        let pressed = self
            .module_controls
            .get(&node.id)
            .is_some_and(|state| state.pressed_mask & 1 != 0);
        let button_rect = screen(random_values_push_rect());
        let response = ui.interact(
            button_rect,
            ui.id().with((&node.id, "random-values-push")),
            Sense::click_and_drag(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                enabled,
                format!("Random Values {} PUSH performance button", node.id),
            )
        });
        painter.rect_filled(
            button_rect.translate(vec2(1.0, 2.0) * zoom),
            3.0 * zoom,
            Color32::from_black_alpha(70),
        );
        painter.rect_filled(
            button_rect,
            3.0 * zoom,
            if pressed {
                Color32::from_rgb(226, 197, 143)
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
            crate::panel_skin::print_font(ui.ctx(), 12.0 * zoom),
            if enabled { INK } else { MUTED },
        );
        if enabled
            && allow_press
            && self.held_manual.is_none()
            && self.editing.is_none()
            && self.annotation_edit.is_none()
            && self.blank_resize.is_none()
            && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
        {
            if response.clicked_by(PointerButton::Primary) {
                self.select_primary(node.id.clone());
                action = Some(RackAction::ManualClick {
                    id: node.id.clone(),
                    press: "push_press",
                    release: "push_release",
                });
            } else if response.is_pointer_button_down_on() {
                self.held_manual = Some(HeldManual {
                    id: node.id.clone(),
                    kind: node.kind.clone(),
                    release: "push_release",
                    button_rect,
                });
                self.select_primary(node.id.clone());
                action = Some(RackAction::Command(
                    "trigger_module",
                    json!({"id":node.id,"action":"push_press"}),
                ));
            }
        }
        response.on_hover_text(if enabled {
            "Draw all seven outputs once. Holding does not redraw. TRIG draws a fresh value for each channel on its rising edge. Release or move outside to end the press."
        } else { "PUSH requires a live audio engine and an active module." });
        painter.text(
            point(228.0, 151.0),
            Align2::CENTER_CENTER,
            "7 × 16",
            crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
            MUTED,
        );
        let state = self.random_values_states.get(&node.id);
        let channels = state.and_then(|s| s["active_channels"].as_u64());
        for index in 0..RANDOM_VALUES_OUTPUTS.len() {
            let y = random_values_row_y(index);
            painter.line_segment(
                [point(18.0, y - 14.0), point(282.0, y - 14.0)],
                Stroke::new(0.5 * zoom, Color32::from_rgb(173, 169, 157)),
            );
            painter.text(
                point(20.0, y),
                Align2::LEFT_CENTER,
                format!("OUT {}", index + 1),
                crate::panel_skin::print_font(ui.ctx(), 10.0 * zoom),
                INK,
            );
            let readout = screen(random_values_readout_rect(index));
            let display = random_values_voltage_text(state, index);
            crate::panel_skin::readout(&painter, readout.center(), &display, readout.width(), zoom);
            let detail = state.map_or_else(
                || "Held values unavailable. No engine snapshot has been published.".to_owned(),
                |s| {
                    let mut text = format!(
                        "OUT {} · {} active channel{}\nActual held engine voltages",
                        index + 1,
                        channels.unwrap(),
                        if channels == Some(1) { "" } else { "s" }
                    );
                    for channel in 0..16 {
                        text.push_str(&format!(
                            "\nCH {:02}: {:+.5} V{}",
                            channel + 1,
                            s["values"][index][channel].as_f64().unwrap(),
                            if channel < channels.unwrap() as usize {
                                ""
                            } else {
                                " · held, inactive"
                            }
                        ));
                    }
                    text
                },
            );
            ui.interact(
                readout,
                ui.id().with((&node.id, "random-values-readout", index)),
                Sense::hover(),
            )
            .on_hover_text(detail);
        }
        painter.text(
            point(150.0, 382.0),
            Align2::CENTER_CENTER,
            state.map_or_else(
                || "HELD VALUES UNAVAILABLE".to_owned(),
                |_| {
                    format!(
                        "{} · {} CHANNEL{}",
                        if self.performance_available {
                            "LIVE"
                        } else {
                            "LAST ENGINE STATE"
                        },
                        channels.unwrap(),
                        if channels == Some(1) { "" } else { "S" }
                    )
                },
            ),
            crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
            MUTED,
        );
        action
    }

    #[allow(clippy::too_many_arguments)]
    fn noise_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        rect: Rect,
        rate: u32,
        revision: u64,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let painter = ui.painter().clone();
        let mut action = None;
        for (index, (name, spec)) in ordered_parameters(node, module).into_iter().enumerate() {
            let (slider_local, value_local) = noise_control_rects(index);
            let screen = |local: Rect| {
                Rect::from_min_max(
                    point(local.min.x, local.min.y),
                    point(local.max.x, local.max.y),
                )
            };
            let slider_rect = screen(slider_local);
            let value_rect = screen(value_local);
            let min = spec["min"].as_f64().unwrap_or(0.0) as f32;
            let mut max = spec["max"].as_f64().unwrap_or(1.0) as f32;
            if let Some(ratio) = spec["sample_rate_max_ratio"].as_f64() {
                max = max.min(rate as f32 * ratio as f32);
            }
            let default = (spec["default"].as_f64().unwrap_or(0.0) as f32).clamp(min, max);
            let value = node
                .params
                .get(name)
                .copied()
                .unwrap_or(default)
                .clamp(min, max);
            let step = spec["step"].as_f64().filter(|s| *s > 0.0).map(|s| s as f32);
            let scale = 1.0;
            let display = if name == "level" {
                format!("{value:.2}×")
            } else {
                parameter_value(name, value, spec)
            };
            let make_edit = |surface| ParameterEdit {
                id: node.id.clone(),
                kind: node.kind.clone(),
                parameter: name.into(),
                revision,
                surface,
                menu_popup: None,
                stale: false,
                text: (value * scale).to_string(),
                min: min * scale,
                max: max * scale,
                step: step.map(|s| s * scale),
                display_scale: scale,
                display_unit: spec["unit"].as_str().unwrap_or("").into(),
                slew_min: None,
                gain_power: None,
                raw_max: max,
                focus: true,
            };
            if self.precision_collects(node) {
                self.register_precision_control(
                    make_edit(ParameterEditSurface::Window),
                    value_rect,
                );
            }
            painter.text(
                point(20.0, slider_local.center().y),
                Align2::LEFT_CENTER,
                parameter_label(name),
                crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
                INK,
            );
            let mut next = value;
            let slider = if name == "level" {
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(slider_rect).id_salt((
                        "noise-slider",
                        &node.id,
                        name,
                    )),
                    |ui| {
                        crate::panel_skin::control_style(ui, zoom);
                        // Slider's thickness also follows Body font height;
                        // scale it so miniature previews retain exact geometry.
                        ui.style_mut()
                            .text_styles
                            .insert(egui::TextStyle::Body, FontId::proportional(11.0 * zoom));
                        ui.spacing_mut().slider_width = slider_rect.width();
                        ui.spacing_mut().interact_size.y = slider_rect.height();
                        ui.spacing_mut().slider_rail_height = 4.0 * zoom;
                        ui.style_mut().visuals.handle_shape =
                            egui::style::HandleShape::Rect { aspect_ratio: 0.5 };
                        ui.add(
                            egui::Slider::new(&mut next, min..=max)
                                .show_value(false)
                                .logarithmic(matches!(name, "frequency" | "decay"))
                                .smart_aim(false),
                        )
                    },
                )
                .inner
            } else {
                painter.text(
                    slider_rect.center(),
                    Align2::CENTER_CENTER,
                    "24-BIT",
                    crate::panel_skin::print_font(ui.ctx(), 9.0 * zoom),
                    MUTED,
                );
                ui.interact(
                    slider_rect,
                    ui.id().with(("noise-slider", &node.id, name)),
                    Sense::click(),
                )
            };
            // Preserve Slider's changed flag while adding click/reset/menu
            // semantics to its native drag and keyboard response.
            let slider = slider.union(slider.interact(Sense::click_and_drag()));
            slider.widget_info(|| {
                egui::WidgetInfo::slider(
                    ui.is_enabled(),
                    value as f64,
                    format!("{} {}", node.id, name),
                )
            });
            if slider.drag_started()
                || (slider.is_pointer_button_down_on()
                    && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary)))
            {
                self.select_primary(node.id.clone());
                slider.request_focus();
                if ui.data(|d| d.get_temp::<String>(slider.id)).is_none() {
                    self.gesture = self.gesture.wrapping_add(1);
                    ui.data_mut(|d| {
                        d.insert_temp(slider.id, format!("rack-slider-{}", self.gesture))
                    });
                }
            }
            if slider.double_clicked() {
                next = default;
            }
            if (slider.changed() || slider.double_clicked()) && next != value {
                self.select_primary(node.id.clone());
                let mut args =
                    json!({"id":node.id,"parameter":name,"value":quantize(next,min,max,step)});
                if (slider.dragged() || slider.is_pointer_button_down_on())
                    && !slider.double_clicked()
                {
                    if let Some(group) = ui.data(|d| d.get_temp::<String>(slider.id)) {
                        args["history_group"] = json!(group);
                    }
                }
                action = Some(RackAction::Command("set_parameter", args));
            }
            if slider.drag_stopped()
                || ui.input(|i| i.pointer.button_released(PointerButton::Primary))
            {
                ui.data_mut(|d| d.remove::<String>(slider.id));
            }
            if self
                .requested_parameter_menu
                .as_ref()
                .is_some_and(|(id, parameter)| id == &node.id && parameter == name)
            {
                egui::Popup::open_id(ui.ctx(), slider.id.with("popup"));
                self.requested_parameter_menu = None;
            }
            let mut popup = egui::Popup::context_menu(&slider)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
            if !slider.secondary_clicked()
                && egui::Popup::position_of_id(ui.ctx(), slider.id.with("popup")).is_none()
            {
                popup = popup.anchor(egui::PopupAnchor::Position(slider.rect.right_bottom()));
            }
            popup.show(|ui| {
                if !self.editing.as_ref().is_some_and(|edit| {
                    edit.surface == ParameterEditSurface::KnobMenu
                        && edit.id == node.id
                        && edit.parameter == name
                }) {
                    self.editing = Some(make_edit(ParameterEditSurface::KnobMenu));
                }
                self.editing.as_mut().unwrap().menu_popup = Some(slider.id.with("popup"));
                ui.set_min_width(244.0);
                ui.strong(parameter_label(name));
                ui.label(format!("Current: {display}"));
                match parameter_editor_contents(ui, self.editing.as_mut().unwrap()) {
                    ParameterEditOutcome::Keep => {}
                    ParameterEditOutcome::Apply(raw)
                    | ParameterEditOutcome::ApplyAndNavigate { raw, .. } => {
                        action = Some(parameter_edit_action(&self.editing.take().unwrap(), raw));
                        ui.close();
                    }
                    ParameterEditOutcome::Cancel => {
                        self.editing = None;
                        ui.close();
                    }
                }
            });
            if name == "seed" && slider.clicked() {
                self.open_precision_editor(make_edit(ParameterEditSurface::Window), value_rect);
            }
            let motion_slider_id = slider.id;
            let motion_slider_rect = slider.rect.shrink2(vec2(slider.rect.height() * 0.2, 0.0));
            slider.on_hover_text(format!(
                "{}\nLevel: drag or use arrow keys; double-click to reset. Click the value to type.\n{}",
                display,
                noise_parameter_help(name)
            ));
            crate::panel_skin::readout(
                &painter,
                value_rect.center(),
                &display,
                value_rect.width(),
                zoom,
            );
            let readout = ui.interact(
                value_rect,
                ui.id().with(("noise-value", &node.id, name)),
                Sense::click(),
            );
            readout.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    format!("Edit {} {} value, {}", node.id, name, display),
                )
            });
            self.precision_affordance(ui, &readout);
            if readout.clicked() {
                consume_parameter_activation(ui.ctx());
                self.open_precision_editor(make_edit(ParameterEditSurface::Window), value_rect);
            }
            readout
                .clone()
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if let Some(cue) = self
                .construction_animation
                .parameter(&node.id, &node.kind, name)
            {
                self.register_construction_anchor(
                    ui,
                    node,
                    name,
                    crate::construction_motion::Anchor {
                        rect: slider_rect.union(value_rect),
                        old_text: if name == "seed" {
                            format!("{:.0}", cue.old)
                        } else {
                            parameter_value(name, cue.old, spec)
                        },
                        new_text: display.clone(),
                        marker: (name == "level").then_some(
                            crate::construction_motion::Marker::Slider {
                                rect: motion_slider_rect,
                                old: normalize(cue.old, min, max, false),
                                new: normalize(cue.new, min, max, false),
                            },
                        ),
                    },
                    &[motion_slider_id, readout.id],
                );
            }
            readout.on_hover_text("Click to type a value or expression");
        }
        paint_noise_spectra(&painter, rect.min, zoom, rate);
        let spectra = Rect::from_min_max(point(18.0, 146.0), point(194.0, 389.0));
        ui.interact(spectra, ui.id().with(("noise-spectra", &node.id)), Sense::hover())
            .on_hover_text(format!("Static spectral/filter shapes at {} Hz; no live signal is displayed.\nSlopes describe power density per octave. Gray uses a fixed inverse A-weighting filter; black is uniform random voltage. All outputs are mono.", rate));
        action
    }

    /// VIZ keeps channel activity separate from signed indicator release tails.
    /// Computer-native parameter controls beside an explicitly configured
    /// modal response. The core helper supplies the same modal coefficients
    /// used by DSP; no telemetry or simulated signal is drawn here.
    #[allow(clippy::too_many_arguments)]
    fn resonator_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        rect: Rect,
        rate: u32,
        parameter_rate: u32,
        revision: u64,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let painter = ui.painter().clone();
        let mut action = None;
        painter.line_segment(
            [point(296.0, 79.0), point(296.0, 316.0)],
            Stroke::new(0.6 * zoom, Color32::from_rgb(166, 164, 157)),
        );
        for (index, (name, spec)) in ordered_parameters(node, module).into_iter().enumerate() {
            let (slider_local, value_local) = resonator_control_rects(index);
            let screen = |local: Rect| {
                Rect::from_min_max(
                    point(local.min.x, local.min.y),
                    point(local.max.x, local.max.y),
                )
            };
            let slider_rect = screen(slider_local);
            let value_rect = screen(value_local);
            let min = spec["min"].as_f64().unwrap_or(0.0) as f32;
            let mut max = spec["max"].as_f64().unwrap_or(1.0) as f32;
            if let Some(ratio) = spec["sample_rate_max_ratio"].as_f64() {
                max = max.min(parameter_rate as f32 * ratio as f32);
            }
            let default = (spec["default"].as_f64().unwrap_or(0.0) as f32).clamp(min, max);
            let value = node
                .params
                .get(name)
                .copied()
                .unwrap_or(default)
                .clamp(min, max);
            let step = spec["step"].as_f64().filter(|s| *s > 0.0).map(|s| s as f32);
            let percent = matches!(
                name,
                "stiffness" | "damping" | "strike_position" | "pickup_position"
            );
            let scale = if percent { 100.0 } else { 1.0 };
            let display = if percent {
                format!("{:.1}%", value * scale)
            } else if matches!(name, "strike_strength" | "level") {
                format!("{value:.2}×")
            } else {
                parameter_value(name, value, spec)
            };
            let make_edit = |surface| ParameterEdit {
                id: node.id.clone(),
                kind: node.kind.clone(),
                parameter: name.into(),
                revision,
                surface,
                menu_popup: None,
                stale: false,
                text: (value * scale).to_string(),
                min: min * scale,
                max: max * scale,
                step: step.map(|s| s * scale),
                display_scale: scale,
                display_unit: if percent {
                    "%"
                } else {
                    spec["unit"].as_str().unwrap_or("")
                }
                .into(),
                slew_min: None,
                gain_power: None,
                raw_max: max,
                focus: true,
            };
            if self.precision_collects(node) {
                self.register_precision_control(
                    make_edit(ParameterEditSurface::Window),
                    value_rect,
                );
            }
            painter.text(
                point(20.0, slider_local.center().y),
                Align2::LEFT_CENTER,
                parameter_label(name),
                crate::panel_skin::print_font(ui.ctx(), 10.0 * zoom),
                INK,
            );
            let mut next = value;
            let slider = ui
                .scope_builder(
                    egui::UiBuilder::new().max_rect(slider_rect).id_salt((
                        "resonator-slider",
                        &node.id,
                        name,
                    )),
                    |ui| {
                        crate::panel_skin::control_style(ui, zoom);
                        // Slider's thickness also follows Body font height;
                        // scale it so miniature previews retain exact geometry.
                        ui.style_mut()
                            .text_styles
                            .insert(egui::TextStyle::Body, FontId::proportional(10.0 * zoom));
                        ui.spacing_mut().slider_width = slider_rect.width();
                        ui.spacing_mut().interact_size.y = slider_rect.height();
                        ui.spacing_mut().slider_rail_height = 4.0 * zoom;
                        ui.style_mut().visuals.handle_shape =
                            egui::style::HandleShape::Rect { aspect_ratio: 0.5 };
                        ui.add(
                            egui::Slider::new(&mut next, min..=max)
                                .show_value(false)
                                .logarithmic(matches!(name, "frequency" | "decay"))
                                .smart_aim(false),
                        )
                    },
                )
                .inner;
            // Preserve Slider's changed flag while adding click/reset/menu
            // semantics to its native drag and keyboard response.
            let slider = slider.union(slider.interact(Sense::click_and_drag()));
            slider.widget_info(|| {
                egui::WidgetInfo::slider(
                    ui.is_enabled(),
                    value as f64,
                    format!("{} {}", node.id, name),
                )
            });
            if slider.drag_started()
                || (slider.is_pointer_button_down_on()
                    && ui.input(|i| i.pointer.button_pressed(PointerButton::Primary)))
            {
                self.select_primary(node.id.clone());
                slider.request_focus();
                if ui.data(|d| d.get_temp::<String>(slider.id)).is_none() {
                    self.gesture = self.gesture.wrapping_add(1);
                    ui.data_mut(|d| {
                        d.insert_temp(slider.id, format!("rack-slider-{}", self.gesture))
                    });
                }
            }
            if slider.double_clicked() {
                next = default;
            }
            if (slider.changed() || slider.double_clicked()) && next != value {
                self.select_primary(node.id.clone());
                let mut args =
                    json!({"id":node.id,"parameter":name,"value":quantize(next,min,max,step)});
                if (slider.dragged() || slider.is_pointer_button_down_on())
                    && !slider.double_clicked()
                {
                    if let Some(group) = ui.data(|d| d.get_temp::<String>(slider.id)) {
                        args["history_group"] = json!(group);
                    }
                }
                action = Some(RackAction::Command("set_parameter", args));
            }
            if slider.drag_stopped()
                || ui.input(|i| i.pointer.button_released(PointerButton::Primary))
            {
                ui.data_mut(|d| d.remove::<String>(slider.id));
            }
            if self
                .requested_parameter_menu
                .as_ref()
                .is_some_and(|(id, parameter)| id == &node.id && parameter == name)
            {
                egui::Popup::open_id(ui.ctx(), slider.id.with("popup"));
                self.requested_parameter_menu = None;
            }
            let mut popup = egui::Popup::context_menu(&slider)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
            if !slider.secondary_clicked()
                && egui::Popup::position_of_id(ui.ctx(), slider.id.with("popup")).is_none()
            {
                popup = popup.anchor(egui::PopupAnchor::Position(slider.rect.right_bottom()));
            }
            popup.show(|ui| {
                if !self.editing.as_ref().is_some_and(|edit| {
                    edit.surface == ParameterEditSurface::KnobMenu
                        && edit.id == node.id
                        && edit.parameter == name
                }) {
                    self.editing = Some(make_edit(ParameterEditSurface::KnobMenu));
                }
                self.editing.as_mut().unwrap().menu_popup = Some(slider.id.with("popup"));
                ui.set_min_width(244.0);
                ui.strong(parameter_label(name));
                ui.label(format!("Current: {display}"));
                match parameter_editor_contents(ui, self.editing.as_mut().unwrap()) {
                    ParameterEditOutcome::Keep => {}
                    ParameterEditOutcome::Apply(raw)
                    | ParameterEditOutcome::ApplyAndNavigate { raw, .. } => {
                        action = Some(parameter_edit_action(&self.editing.take().unwrap(), raw));
                        ui.close();
                    }
                    ParameterEditOutcome::Cancel => {
                        self.editing = None;
                        ui.close();
                    }
                }
            });
            let motion_slider_id = slider.id;
            let motion_slider_rect = slider.rect.shrink2(vec2(slider.rect.height() * 0.2, 0.0));
            slider.on_hover_text(format!(
                "{}\nDrag or use arrow keys; double-click to reset. Click the value to type.\n{}",
                display,
                resonator_parameter_help(name)
            ));
            crate::panel_skin::readout(
                &painter,
                value_rect.center(),
                &display,
                value_rect.width(),
                zoom,
            );
            let readout = ui.interact(
                value_rect,
                ui.id().with(("resonator-value", &node.id, name)),
                Sense::click(),
            );
            readout.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    format!("Edit {} {} value, {}", node.id, name, display),
                )
            });
            self.precision_affordance(ui, &readout);
            if readout.clicked() {
                consume_parameter_activation(ui.ctx());
                self.open_precision_editor(make_edit(ParameterEditSurface::Window), value_rect);
            }
            readout
                .clone()
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if let Some(cue) = self
                .construction_animation
                .parameter(&node.id, &node.kind, name)
            {
                self.register_construction_anchor(
                    ui,
                    node,
                    name,
                    crate::construction_motion::Anchor {
                        rect: slider_rect.union(value_rect),
                        old_text: if percent {
                            format!("{:.1}%", cue.old * scale)
                        } else if matches!(name, "strike_strength" | "level") {
                            format!("{:.2}×", cue.old)
                        } else {
                            parameter_value(name, cue.old, spec)
                        },
                        new_text: display.clone(),
                        marker: step.is_none().then_some(
                            crate::construction_motion::Marker::Slider {
                                rect: motion_slider_rect,
                                old: motion_slider_normalize(
                                    cue.old,
                                    min,
                                    max,
                                    matches!(name, "frequency" | "decay"),
                                ),
                                new: motion_slider_normalize(
                                    cue.new,
                                    min,
                                    max,
                                    matches!(name, "frequency" | "decay"),
                                ),
                            },
                        ),
                    },
                    &[motion_slider_id, readout.id],
                );
            }
            readout.on_hover_text("Click to type a value or expression");
        }
        paint_resonator_configuration(&painter, node, rect.min, zoom, rate);
        painter.text(
            point(20.0, 316.0),
            Align2::LEFT_CENTER,
            "STRIKE / PICKUP",
            crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
            MUTED,
        );
        painter.line_segment(
            [point(105.0, 315.0), point(282.0, 315.0)],
            Stroke::new(1.0 * zoom, MUTED),
        );
        let strike = node.params.get("strike_position").copied().unwrap_or(0.23);
        let pickup = node.params.get("pickup_position").copied().unwrap_or(0.61);
        painter.rect_filled(
            Rect::from_center_size(point(105.0 + strike * 177.0, 315.0), vec2(6.0, 6.0) * zoom),
            0.0,
            Color32::from_rgb(151, 93, 57),
        );
        painter.circle_filled(
            point(105.0 + pickup * 177.0, 315.0),
            3.0 * zoom,
            Color32::from_rgb(68, 113, 121),
        );
        painter.line_segment(
            [point(20.0, 325.0), point(460.0, 325.0)],
            Stroke::new(0.6 * zoom, Color32::from_rgb(166, 164, 157)),
        );
        painter.text(
            point(155.0, 387.0),
            Align2::CENTER_CENTER,
            "PITCH · GATE STRIKE · AUDIO FORCE",
            crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
            MUTED,
        );
        painter.text(
            point(410.0, 387.0),
            Align2::CENTER_CENTER,
            "VELOCITY PICKUP",
            crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
            MUTED,
        );
        action
    }

    fn viz_panel(&self, ui: &mut egui::Ui, node: &Node, rect: Rect) {
        let zoom = self.zoom;
        let painter = ui.painter();
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let state = self
            .module_lights
            .get(&node.id)
            .filter(|s| s.count == 16 && s.input_channels.is_some_and(|c| c <= 16));
        painter.text(
            point(90.0, 91.0),
            Align2::CENTER_CENTER,
            state.map_or_else(
                || "TELEMETRY UNAVAILABLE".to_owned(),
                |s| format!("{} / 16 CHANNELS", s.input_channels.unwrap()),
            ),
            crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
            MUTED,
        );
        for channel in 0..16 {
            let x = if channel < 8 { 32.0 } else { 112.0 };
            let y = 120.0 + (channel % 8) as f32 * 24.0;
            let active = state.is_some_and(|s| channel < s.input_channels.unwrap() as usize);
            painter.text(
                point(x, y),
                Align2::CENTER_CENTER,
                format!("{:02}", channel + 1),
                crate::panel_skin::print_font(painter.ctx(), 12.0 * zoom),
                if active { INK } else { Color32::from_gray(155) },
            );
            for (polarity, color) in [
                Color32::from_rgb(65, 174, 102),
                Color32::from_rgb(216, 75, 65),
            ]
            .into_iter()
            .enumerate()
            {
                let center = point(x + 28.0 + if polarity == 0 { -5.0 } else { 5.0 }, y);
                // Retired voices can retain a fading light after their number dims.
                let value = state.map_or(0.0, |s| s.brightness[channel][polarity].clamp(0.0, 1.0));
                painter.circle_filled(center, 4.0 * zoom, Color32::from_gray(70));
                self.lighting.circle(
                    painter,
                    center,
                    3.0 * zoom,
                    if state.is_none() {
                        Color32::from_gray(100)
                    } else {
                        color.gamma_multiply(0.15 + value * 0.85)
                    },
                    value,
                    0.15,
                );
            }
        }
        painter.text(
            point(90.0, 313.0),
            Align2::CENTER_CENTER,
            "+ GREEN   − RED",
            crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
            MUTED,
        );
        ui.interact(Rect::from_min_max(point(20.0, 106.0), point(158.0, 302.0)),
            ui.id().with((&node.id, "viz-meter")), Sense::hover()).on_hover_text(
                "Sixteen input voices. Bright numbers identify active channels, including silent voices. Green shows positive voltage; red shows negative voltage. Gray indicators mean telemetry is unavailable.");
    }

    /// SUM's telemetry is post-attenuator; absent publication stays visibly unavailable.
    fn sum_panel(&self, ui: &mut egui::Ui, node: &Node, rect: Rect) {
        let zoom = self.zoom;
        let painter = ui.painter();
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let state = self
            .module_lights
            .get(&node.id)
            .filter(|state| state.count >= 6 && state.input_channels.is_some());
        painter.text(
            point(75.0, 211.0),
            Align2::CENTER_CENTER,
            "INPUT CHANNELS",
            crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
            MUTED,
        );
        painter.text(
            point(75.0, 233.0),
            Align2::CENTER_CENTER,
            state.map_or_else(
                || "— / 16".to_owned(),
                |s| format!("{} / 16", s.input_channels.unwrap().min(16)),
            ),
            crate::panel_skin::print_font(painter.ctx(), 18.0 * zoom),
            INK,
        );
        painter.text(
            point(75.0, 254.0),
            Align2::CENTER_CENTER,
            "OUT · 10 V = 0 dB",
            crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
            MUTED,
        );
        for (band, db) in [0, -3, -6, -12, -24, -36].into_iter().enumerate() {
            let y = 268.0 + band as f32 * 10.0;
            let brightness = state.map_or(0.0, |s| s.brightness[band][0]).clamp(0.0, 1.0);
            let color = if band == 0 {
                Color32::from_rgb(218, 141, 53)
            } else {
                Color32::from_rgb(76, 163, 112)
            };
            self.lighting.rect(
                painter,
                Rect::from_min_size(point(41.0, y - 4.0), vec2(55.0, 8.0) * zoom),
                1.0,
                color.gamma_multiply(0.14 + brightness * 0.86),
                brightness,
                0.14,
            );
            painter.text(
                point(105.0, y),
                Align2::LEFT_CENTER,
                format!("{db}"),
                crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
                MUTED,
            );
        }
        painter.text(
            point(75.0, 333.0),
            Align2::CENTER_CENTER,
            if state.is_some() {
                "POST-LEVEL"
            } else {
                "METER UNAVAILABLE"
            },
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            MUTED,
        );
        ui.interact(Rect::from_min_max(point(24.0, 204.0), point(130.0, 338.0)),
            ui.id().with((&node.id, "sum-meter")), Sense::hover()).on_hover_text(
                "Active input channels: 0–16. Output meter measures the signed channel sum after LEVEL, relative to 10 V. Opposite voices can cancel. No published telemetry is shown as unavailable.");
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
                    self.lighting.rect(
                        painter,
                        Rect::from_min_size(
                            center + vec2(-25.0 + segment as f32 * 4.2, -3.0) * zoom,
                            vec2(3.1, 6.0) * zoom,
                        ),
                        0.5,
                        color.gamma_multiply(if lit { 1.0 } else { 0.16 }),
                        if lit { 1.0 } else { 0.0 },
                        0.16,
                    );
                }
                ui.interact(Rect::from_center_size(center,vec2(54.0,13.0)*zoom),ui.id().with((&node.id,"strip-vu",channel)),Sense::hover()).on_hover_text("DSP strip VU: signed channel sum / 5 V, peak smoothing, -24 to 0 dB brightness. Polyphonic lanes can cancel. This is the signal meter; the fader shows configured gain.");
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
                &self.lighting,
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
                paint_signal_light(&painter, center, brightness, zoom, &self.lighting);
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
                crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
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
            self.lighting.circle(
                &painter,
                point(132.0, y),
                2.8 * zoom,
                Color32::from_rgb(238, 177, 65).gamma_multiply(0.12 + 0.88 * brightness),
                brightness,
                0.12,
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
            self.lighting.circle(
                &painter,
                point(x, 265.0),
                4.4 * zoom,
                led_color.gamma_multiply(0.12 + 0.88 * brightness),
                brightness,
                0.12,
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
                crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
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
                    crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
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
            crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
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
            // These route indicators already have two soft rings. Preserve
            // only the engine-selected route; static disabled markers dim.
            let lamp_start = if self.lighting.amount() < 1.0 {
                crate::lighting::next(&painter)
            } else {
                0
            };
            paint_jack_light(&painter, point(many_x - 22.0, y), zoom * 0.65, color);
            if active {
                self.lighting.preserve(&painter, lamp_start);
            }
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
            crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
            MUTED,
        );
        for row in 0..3 {
            painter.text(
                point(203.0, 123.0 + row as f32 * 73.0),
                Align2::RIGHT_CENTER,
                format!("{}", row + 1),
                crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
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
                self.lighting.circle(
                    &painter,
                    point(x + 23.0, 85.0),
                    3.0 * zoom,
                    Color32::from_rgb(51, 137, 80),
                    1.0,
                    0.0,
                );
            }
        }
        action
    }

    /// A passive measured count, shared verbatim by the rack and library.
    /// Saved topology and sixteen available sockets never stand in for telemetry.
    fn split_panel(&self, ui: &mut egui::Ui, node: &Node, rect: Rect) {
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let painter = ui.painter();
        let channels = self
            .module_lights
            .get(&node.id)
            .and_then(|state| state.input_channels)
            .filter(|count| *count <= 16);
        painter.text(
            point(20.0, 83.0),
            Align2::LEFT_CENTER,
            "INPUT CHANNELS",
            crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
            INK,
        );
        // Below the title and above the signal label/socket; the complete
        // 14-point Rajdhani line box fits this 22-point display at every zoom.
        let screen = Rect::from_min_size(point(200.0, 72.0), vec2(75.0, 22.0) * zoom);
        let screen_start = crate::lighting::next(painter);
        painter.rect_filled(screen, 2.0 * zoom, Color32::from_rgb(20, 31, 31));
        painter.text(
            screen.center(),
            Align2::CENTER_CENTER,
            channels.map_or_else(|| "—".to_owned(), |count| count.to_string()),
            crate::panel_skin::print_font(ui.ctx(), 14.0 * zoom),
            if channels.is_some() {
                Color32::from_rgb(155, 220, 197)
            } else {
                Color32::from_rgb(135, 145, 138)
            },
        );
        if channels.is_some() {
            self.lighting.preserve(painter, screen_start);
        }
        ui.interact(screen, ui.id().with((&node.id, "split-input-count")), Sense::hover())
            .on_hover_text(if channels.is_some() {
                "Measured input channels. Silent active channels count; zero means a valid zero-channel input. All sixteen named outputs remain available."
            } else {
                "Input channel telemetry unavailable. Sixteen output sockets do not imply sixteen live input channels."
            });
    }

    /// MERGE uses the same panel in the rack and library; channel counts are
    /// runtime measurements and are never inferred from saved cable topology.
    fn merge_panel(&mut self, ui: &mut egui::Ui, node: &Node, rect: Rect) -> Option<RackAction> {
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let painter = ui.painter().clone();
        let measured = self.module_lights.get(&node.id);
        let output = measured
            .and_then(|state| state.output_channels)
            .filter(|count| *count <= 16);
        let automatic = measured
            .and_then(|state| state.automatic_channels)
            .filter(|count| *count <= 16);
        // The output socket/label occupy the center of this lower row. Its
        // left and right gutters keep the measured readout clear of all sixteen
        // inputs and the fixed Channels control, including full font line boxes.
        painter.text(
            point(20.0, 356.0),
            Align2::LEFT_CENTER,
            "OUTPUT CHANNELS",
            crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
            INK,
        );
        let screen = Rect::from_min_size(point(200.0, 345.0), vec2(75.0, 22.0) * zoom);
        let screen_start = crate::lighting::next(&painter);
        painter.rect_filled(screen, 2.0 * zoom, Color32::from_rgb(20, 31, 31));
        painter.text(
            screen.center(),
            Align2::CENTER_CENTER,
            output.map_or_else(|| "—".to_owned(), |count| count.to_string()),
            crate::panel_skin::print_font(ui.ctx(), 14.0 * zoom),
            if output.is_some() {
                Color32::from_rgb(155, 220, 197)
            } else {
                Color32::from_rgb(135, 145, 138)
            },
        );
        if output.is_some() {
            self.lighting.preserve(&painter, screen_start);
        }
        ui.interact(screen, ui.id().with((&node.id, "merge-output-count")), Sense::hover())
            .on_hover_text(if output.is_some() {
                "Measured output channels. Zero means the running engine publishes a zero-channel output."
            } else {
                "Output channel telemetry unavailable. The saved patch does not determine the live count."
            });
        painter.text(
            point(20.0, 330.0),
            Align2::LEFT_CENTER,
            "CHANNELS",
            crate::panel_skin::print_font(ui.ctx(), 12.0 * zoom),
            INK,
        );
        let control = Rect::from_min_size(point(200.0, 318.0), vec2(75.0, 24.0) * zoom);
        let old = node.params.get("channels").copied().unwrap_or(0.0);
        let mut value = old;
        let label = merge_channels_label(old);
        let response = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(control)
                    .layout(egui::Layout::left_to_right(egui::Align::Center))
                    .id_salt((&node.id, "merge-channels")),
                |ui| {
                    *ui.visuals_mut() = egui::Visuals::light();
                    crate::panel_skin::control_style(ui, zoom);
                    ui.style_mut().override_font_id =
                        Some(crate::panel_skin::print_font(ui.ctx(), 12.0 * zoom));
                    let spacing = ui.spacing_mut();
                    spacing.button_padding = vec2(6.0, 3.0) * zoom;
                    spacing.icon_width = 14.0 * zoom;
                    spacing.icon_width_inner = 8.0 * zoom;
                    spacing.icon_spacing = 4.0 * zoom;
                    spacing.interact_size = vec2(40.0, 18.0) * zoom;
                    spacing.item_spacing = vec2(8.0, 3.0) * zoom;
                    egui::ComboBox::from_id_salt("choice")
                        .width(control.width())
                        .height(420.0 * zoom)
                        .wrap_mode(egui::TextWrapMode::Truncate)
                        .selected_text(&label)
                        .show_ui(ui, |ui| {
                            ui.set_min_width(130.0 * zoom);
                            let auto = automatic.map_or_else(
                                || "Auto".to_owned(),
                                |count| format!("Auto · {count} channels"),
                            );
                            ui.selectable_value(&mut value, 0.0, auto);
                            ui.selectable_value(&mut value, -1.0, "0");
                            for count in 1..=16 {
                                ui.selectable_value(&mut value, count as f32, count.to_string());
                            }
                        })
                        .response
                },
            )
            .inner;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::ComboBox,
                ui.is_enabled(),
                format!("{} channels, {}", node.id, label),
            )
        });
        let revision_id = response.id.with("opening-revision");
        if response.clicked() {
            let revision = self.observed_revision;
            ui.ctx()
                .data_mut(|data| data.insert_temp(revision_id, revision));
        }
        if let Some(cue) = self
            .construction_animation
            .parameter(&node.id, &node.kind, "channels")
        {
            self.register_construction_anchor(
                ui,
                node,
                "channels",
                crate::construction_motion::Anchor {
                    rect: control,
                    old_text: merge_channels_label(cue.old),
                    new_text: merge_channels_label(cue.new),
                    marker: None,
                },
                &[response.id],
            );
        }
        if value == old {
            return None;
        }
        let revision = ui
            .ctx()
            .data(|data| data.get_temp::<u64>(revision_id))
            .unwrap_or(self.observed_revision);
        self.select_primary(node.id.clone());
        Some(RackAction::Command(
            "set_parameter",
            json!({
                "id":node.id, "parameter":"channels", "value":value, "expected_revision":revision,
            }),
        ))
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
                .layout(egui::Layout::left_to_right(egui::Align::Center))
                .id_salt((&node.id, parameter)),
            |ui| {
                *ui.visuals_mut() = egui::Visuals::light();
                crate::panel_skin::control_style(ui, self.zoom);
                let font = crate::panel_skin::print_font(ui.ctx(), 12.0 * self.zoom);
                ui.style_mut().override_font_id = Some(font);
                let spacing = ui.spacing_mut();
                spacing.button_padding = vec2(6.0, 3.0) * self.zoom;
                spacing.icon_width = 14.0 * self.zoom;
                spacing.icon_width_inner = 8.0 * self.zoom;
                spacing.icon_spacing = 4.0 * self.zoom;
                spacing.interact_size = vec2(40.0, 18.0) * self.zoom;
                spacing.item_spacing = vec2(8.0, 3.0) * self.zoom;
                egui::ComboBox::from_id_salt("choice")
                    .width(rect.width())
                    .wrap_mode(egui::TextWrapMode::Truncate)
                    .selected_text(choices[value])
                    .show_ui(ui, |ui| {
                        for (index, label) in choices.iter().enumerate() {
                            ui.selectable_value(&mut value, index, *label);
                        }
                    });
            },
        );
        if let Some(cue) = self
            .construction_animation
            .parameter(&node.id, &node.kind, parameter)
        {
            self.register_construction_anchor(
                ui,
                node,
                parameter,
                crate::construction_motion::Anchor {
                    rect,
                    old_text: choices.get(cue.old as usize).unwrap_or(&"?").to_string(),
                    new_text: choices.get(cue.new as usize).unwrap_or(&"?").to_string(),
                    marker: None,
                },
                &[],
            );
        }
        (value != old).then(|| {
            RackAction::Command(
                "set_parameter",
                json!({"id":node.id,"parameter":parameter,"value":value}),
            )
        })
    }

    fn scope_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        module: Option<&Value>,
        rect: Rect,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let painter = ui.painter().clone();
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let display = Rect::from_min_max(point(30.0, 70.0), point(600.0, 250.0));
        painter.rect_filled(display.expand(3.0 * zoom), 3.0 * zoom, INK);
        let screen_start = crate::lighting::next(&painter);
        painter.rect_filled(display, 1.0, Color32::from_rgb(14, 24, 27));
        let plot = Rect::from_min_max(point(40.0, 101.0), point(590.0, 213.0));
        for i in 1..10 {
            let x = plot.left() + plot.width() * i as f32 / 10.0;
            painter.line_segment(
                [pos2(x, plot.top()), pos2(x, plot.bottom())],
                Stroke::new(0.5 * zoom, Color32::from_rgb(36, 50, 53)),
            );
        }
        for i in 1..6 {
            let y = plot.top() + plot.height() * i as f32 / 6.0;
            painter.line_segment(
                [pos2(plot.left(), y), pos2(plot.right(), y)],
                Stroke::new(0.5 * zoom, Color32::from_rgb(36, 50, 53)),
            );
        }
        let trace = painter.with_clip_rect(plot.shrink(1.0));
        if let Some(state) = self.scope_states.get(&node.id) {
            let auto_range = scope_auto_range_enabled(node);
            let range_id = ui.id().with(("scope-auto-range", &node.id));
            let previous = ui.data(|data| data.get_temp::<[ScopeDisplayRange; 2]>(range_id));
            let ranges = scope_display_ranges(state, auto_range, previous);
            ui.data_mut(|data| {
                if auto_range {
                    data.insert_temp(range_id, ranges);
                } else {
                    data.remove::<[ScopeDisplayRange; 2]>(range_id);
                }
            });
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
                        FontId::monospace(8.0 * zoom),
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
                        FontId::monospace(8.0 * zoom),
                        color,
                    );
                }
            }
            if scope_x_trigger_guide(state) {
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
                        FontId::monospace(8.0 * zoom),
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
                        FontId::monospace(8.0 * zoom),
                        SCOPE_TRIGGER_COLOR,
                    );
                }
            }
            painter.text(
                point(40.0, 89.0),
                Align2::LEFT_TOP,
                scope_range_label(auto_range, ranges),
                FontId::monospace(9.5 * zoom),
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
                    FontId::monospace(9.5 * zoom),
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
                    scope_trigger_status(state)
                ),
                FontId::monospace(9.5 * zoom),
                Color32::from_gray(216),
            );
        } else {
            painter.text(
                plot.center(),
                Align2::CENTER_CENTER,
                "NO ENGINE CAPTURE · X / Y VOLTS",
                FontId::monospace(12.0 * zoom),
                Color32::from_gray(155),
            );
        }
        if self.scope_states.contains_key(&node.id) {
            self.lighting.preserve(&painter, screen_start);
        }
        let mut action = None;
        for (index, (name, label, suffix)) in SCOPE_CONTROLS.iter().enumerate() {
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
            if let Some(spec) = module.and_then(|m| m["params"].get(*name)) {
                let edit = if matches!(*name, "x_scale" | "y_scale") {
                    self.choice_parameter(ui, node, name, spec, control, &SCOPE_SCALE_CHOICES)
                } else if matches!(*name, "xy" | "trigger_enabled" | "auto_range") {
                    self.choice_parameter(ui, node, name, spec, control, &["OFF", "ON"])
                } else {
                    self.compact_parameter(ui, node, name, spec, control, suffix)
                };
                if let Some(edit) = edit {
                    action = Some(scope_parameter_action(node, edit, self.observed_revision));
                }
            }
        }
        action
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
        let editor_start = if self.lighting.amount() < 1.0 {
            crate::lighting::next(ui.painter())
        } else {
            0
        };
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(rect).id_salt(id), |ui| {
                // Retain the established active gold cue while matching the
                // inactive and text-edit surfaces to the panel readouts.
                ui.visuals_mut().widgets.active.bg_fill = Color32::from_rgb(226, 197, 143);
                crate::panel_skin::control_style(ui, self.zoom);
                ui.style_mut().override_font_id =
                    Some(crate::panel_skin::print_font(ui.ctx(), 10.0 * self.zoom));
                ui.spacing_mut().button_padding = vec2(2.0, 1.0) * self.zoom;
                ui.spacing_mut().interact_size = vec2(8.0, 18.0) * self.zoom;
                ui.add_sized(
                    rect.size(),
                    egui::DragValue::new(&mut value)
                        .range(min * scale..=max * scale)
                        .speed(if step.is_some() {
                            0.1
                        } else if node.kind == "scope" && parameter == "time" {
                            0.001
                        } else {
                            0.02 * scale
                        })
                        .max_decimals(if step.is_some() {
                            0
                        } else if node.kind == "scope" && parameter == "time" {
                            3
                        } else {
                            2
                        })
                        .suffix(suffix),
                )
            })
            .inner;
        // DragValue becomes an inline TextEdit while focused, on the same
        // rack layer. Preserve that active editor, including its release frame,
        // while ordinary ink-on-metal readouts still receive ambient lighting.
        if response.has_focus() || response.lost_focus() {
            self.lighting.preserve(ui.painter(), editor_start);
        }
        response.widget_info(|| {
            egui::WidgetInfo::slider(true, value as f64, format!("{} {}", node.id, parameter))
        });
        if let Some(cue) = self
            .construction_animation
            .parameter(&node.id, &node.kind, parameter)
        {
            let display = |raw: f32| {
                if step.is_some() {
                    format!("{:.0}{suffix}", raw * scale)
                } else {
                    format!("{:.2}{suffix}", raw * scale)
                }
            };
            self.register_construction_anchor(
                ui,
                node,
                parameter,
                crate::construction_motion::Anchor {
                    rect,
                    old_text: display(cue.old),
                    new_text: display(cue.new),
                    marker: None,
                },
                &[response.id],
            );
        }
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
        if pending.operation == CableOperation::Create {
            pending.palette_selection = self.new_cable_selection;
            pending.color = pending.palette_selection.map(|selection| selection.color);
        }
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
                    if input && !studio_mixer_input_visible(node, port) {
                        continue;
                    }
                    let center = canvas.min
                        + self.pan
                        + (self.positions[&node.id].to_vec2()
                            + port_position(
                                &node.kind,
                                input,
                                index,
                                ports.len(),
                                module_width(node),
                            )
                            .to_vec2())
                            * self.zoom;
                    let hit = jack_target_rect(
                        &node.kind,
                        center,
                        self.zoom,
                        if matches!(
                            node.kind.as_str(),
                            "colored_noise" | "random_values" | "sh_asr" | "stock_noise"
                        ) {
                            12.0
                        } else {
                            18.0
                        },
                        port_material_scale(&node.kind),
                    )
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
                        *action = palette_commit_action(
                            &pending,
                            cable_action_mode(
                                patch,
                                &pending,
                                &end,
                                catalog,
                                response.ctx.input(|i| i.modifiers.shift),
                            ),
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
        if self
            .cable_secondary_press
            .as_ref()
            .is_some_and(|(end, kind)| {
                !patch
                    .nodes
                    .iter()
                    .any(|node| node.id == end.node() && node.kind == kind.as_str())
                    || match end {
                        CableEnd::Output { node, port } => {
                            !port_exists(patch, catalog, node, port, true)
                        }
                        CableEnd::Input { node, port } => {
                            !port_exists(patch, catalog, node, port, false)
                        }
                    }
            })
        {
            self.cable_secondary_press = None;
        }
        if self
            .deferred_touch_menu
            .as_ref()
            .is_some_and(|(end, kind, _)| {
                !patch
                    .nodes
                    .iter()
                    .any(|node| node.id == end.node() && node.kind == kind.as_str())
                    || match end {
                        CableEnd::Output { node, port } => {
                            !port_exists(patch, catalog, node, port, true)
                        }
                        CableEnd::Input { node, port } => {
                            !port_exists(patch, catalog, node, port, false)
                        }
                    }
            })
        {
            self.deferred_touch_menu = None;
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
                        x = x.max(position.x + module_width(other));
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
                .map(|n| self.positions[&n.id].x + module_width(n))
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
        let size = (vec2(module_width(node), PANEL_HEIGHT) * self.zoom).min(viewport.size());
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
}

// Original stock-family Random panel. Rack and browser use this same geometry.
fn random_parameter_position(index: usize) -> Pos2 {
    if index == 8 {
        pos2(180.0, 276.0)
    } else {
        pos2(
            45.0 + (index % 4) as f32 * 90.0,
            128.0 + (index / 4) as f32 * 76.0,
        )
    }
}
fn random_parameter_label(name: &str) -> &str {
    match name {
        "rate" => "RATE",
        "probability" => "PROBABILITY",
        "spread" => "SPREAD",
        "shape" => "SHAPE",
        "rate_cv" => "RATE CV",
        "probability_cv" => "PROB CV",
        "spread_cv" => "SPREAD CV",
        "shape_cv" => "SHAPE CV",
        "unipolar" => "RANGE",
        other => other,
    }
}
fn random_parameter_value(name: &str, value: f32) -> String {
    match name {
        "rate" => {
            let hz = value.exp2();
            if hz < 1.0 {
                format!("{hz:.4} Hz")
            } else {
                format!("{hz:.2} Hz")
            }
        }
        "unipolar" => if value < 0.5 {
            "−5 to +5 V"
        } else {
            "0 to 10 V"
        }
        .into(),
        "seed" => format!("{value:.0}"),
        "rate_cv" | "probability_cv" | "spread_cv" | "shape_cv" => {
            format!("{:+.1}%", value * 100.0)
        }
        _ => format!("{:.1}%", value * 100.0),
    }
}
fn random_port_label(port: &str) -> &str {
    match port {
        "rate_cv" => "RATE CV",
        "shape_cv" => "SHAPE CV",
        "trigger" => "TRIG",
        "external" => "EXT",
        "probability_cv" => "PROB CV",
        "spread_cv" => "SPREAD CV",
        "stepped" => "STEP",
        "linear" => "LIN",
        "smooth" => "SMOOTH",
        "exponential" => "EXP",
        other => other,
    }
}
fn random_seed_edit(node: &Node, revision: u64) -> ParameterEdit {
    ParameterEdit {
        id: node.id.clone(),
        kind: node.kind.clone(),
        parameter: "seed".into(),
        revision,
        surface: ParameterEditSurface::Window,
        menu_popup: None,
        stale: false,
        text: node.params.get("seed").copied().unwrap_or(1.0).to_string(),
        min: 1.0,
        max: 16_777_215.0,
        step: Some(1.0),
        display_scale: 1.0,
        display_unit: String::new(),
        slew_min: None,
        gain_power: None,
        raw_max: 16_777_215.0,
        focus: true,
    }
}
fn random_light_strengths(lights: Option<&ModuleLightState>) -> [f32; 5] {
    std::array::from_fn(|index| {
        let raw = lights
            .filter(|state| index < usize::from(state.count))
            .map_or(0.0, |state| state.brightness[index][0]);
        if !raw.is_finite() {
            return 0.0;
        }
        // Rate/probability are the engine's 0–3 trigger flashes. The other
        // lights report modulation or offset state directly from the engine.
        (raw / if matches!(index, 0 | 2) { 3.0 } else { 1.0 }).clamp(0.0, 1.0)
    })
}
fn paint_random_lights(
    painter: &egui::Painter,
    rect: Rect,
    zoom: f32,
    lights: Option<&ModuleLightState>,
    lighting: &crate::lighting::RoomLighting,
) {
    let strengths = random_light_strengths(lights);
    for (index, label) in ["RATE", "SHAPE", "PROB", "SPREAD", "OFFSET"]
        .iter()
        .enumerate()
    {
        let center = rect.min + vec2(20.0 + index as f32 * 68.0, 78.0) * zoom;
        let strength = strengths[index];
        lighting.circle(
            painter,
            center,
            2.5 * zoom,
            Color32::from_rgb(207, 176, 83).gamma_multiply(0.12 + strength * 0.88),
            strength,
            0.12,
        );
        painter.text(
            center + vec2(6.0, 0.0) * zoom,
            Align2::LEFT_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
            MUTED,
        );
    }
}

// Original 24 HP wavetable faceplate, shared by the rack and browser renderer.
fn is_wavetable(kind: &str) -> bool {
    matches!(kind, "wavetable_vco" | "wavetable_lfo")
}
fn wavetable_parameter_position(index: usize) -> Pos2 {
    pos2(
        60.0 + (index % 3) as f32 * 120.0,
        205.0 + (index / 3) as f32 * 85.0,
    )
}
fn wavetable_parameter_label(name: &str) -> &str {
    match name {
        "tune" => "TUNE",
        "rate" => "FREE RATE",
        "position" => "POSITION",
        "fm_amount" => "FM DEPTH",
        "position_cv_amount" => "POSITION CV",
        "sync_mode" => "SYNC",
        "fm_mode" => "FM MODE",
        "offset" => "RANGE",
        "invert" => "POLARITY",
        other => other,
    }
}
fn wavetable_rate_value(raw: f32, clock_ratio: bool) -> String {
    let rate = raw.exp2() * if clock_ratio { 0.5 } else { 1.0 };
    if clock_ratio {
        format!("{rate:.4}x")
    } else if rate < 1.0 {
        format!("{rate:.8} Hz")
    } else {
        format!("{rate:.3} Hz")
    }
}
fn wavetable_parameter_value(name: &str, value: f32) -> String {
    match name {
        "tune" => format!("{value:+.2} st"),
        "rate" => wavetable_rate_value(value, false),
        "position" => format!("{:.1}%", value * 100.0),
        "fm_amount" | "position_cv_amount" => format!("{:+.1}%", value * 100.0),
        "sync_mode" => if value < 0.5 { "Hard" } else { "Soft" }.into(),
        "fm_mode" => if value < 0.5 {
            "Exponential"
        } else {
            "Linear TZ"
        }
        .into(),
        "offset" => if value < 0.5 { "Bipolar" } else { "Offset" }.into(),
        "invert" => if value < 0.5 { "Normal" } else { "Invert" }.into(),
        _ => format!("{value:.2}"),
    }
}
fn wavetable_choice_spec(kind: &str, name: &str) -> Option<Value> {
    if !is_wavetable(kind) {
        return None;
    }
    let labels = match name {
        "sync_mode" => ["Hard", "Soft"],
        "fm_mode" => ["Exp", "Linear"],
        "offset" => ["±5 V", "0–10 V"],
        "invert" => ["Normal", "Invert"],
        _ => return None,
    };
    Some(json!({"min":0,"max":1,"step":1,"values":[0,1],"labels":labels}))
}
fn wavetable_port_label(port: &str) -> &str {
    match port {
        "fm" => "FM",
        "sync" => "SYNC",
        "reset" => "RESET",
        "position_cv" => "POS CV",
        "pitch" => "V/OCT",
        "clock" => "CLOCK",
        "out" => "OUT",
        other => other,
    }
}
fn wavetable_menu(
    ui: &mut egui::Ui,
    node: &Node,
    patch: &Patch,
    revision: u64,
    action: &mut Option<RackAction>,
) {
    if !is_wavetable(&node.kind) {
        return;
    }
    ui.separator();
    for (label, command) in [
        ("Load wavetable…", "open_wavetable_import"),
        ("Save wavetable…", "open_wavetable_export"),
    ] {
        if ui.button(label).clicked() {
            *action = Some(RackAction::Command(
                command,
                json!({"id":node.id,"expected_revision":revision}),
            ));
            ui.close();
        }
    }
    ui.menu_button("Wave points", |ui| {
        let (current, sample_count) = match &node.content {
            Some(openpatch_core::ModuleContent::Wavetable {
                asset_id,
                wave_points,
                ..
            }) => (
                *wave_points as usize,
                patch
                    .assets
                    .get(asset_id)
                    .map_or(0, |asset| asset.sample_count()),
            ),
            _ => (1024, 4096),
        };
        for points in [
            2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384,
        ] {
            if node.kind == "wavetable_vco" && points < 32 {
                continue;
            }
            if ui
                .add_enabled(
                    points <= sample_count,
                    egui::Button::new(format!("{points}")).selected(current == points),
                )
                .clicked()
            {
                let (asset_id, name) = match &node.content {
                    Some(openpatch_core::ModuleContent::Wavetable { asset_id, name, .. }) => {
                        (asset_id.clone(), name.clone())
                    }
                    _ => (
                        openpatch_core::default_wavetable_asset().id().to_owned(),
                        "Basic".into(),
                    ),
                };
                *action = Some(RackAction::Command(
                    "set_wavetable",
                    json!({"id":node.id,"asset_id":asset_id,
                    "wave_points":points,"name":name,"expected_revision":revision}),
                ));
                ui.close();
            }
        }
        ui.label("Rebuilds the table before the graph changes.");
    });
    ui.separator();
}

fn wavetable_display_position(node: &Node, lights: Option<&ModuleLightState>) -> f32 {
    lights
        .and_then(|state| state.waveform_position)
        .filter(|position| position.is_finite())
        .unwrap_or_else(|| node.params.get("position").copied().unwrap_or(0.0))
        .clamp(0.0, 1.0)
}
// Shows the actual source samples, interpolating the selected complete waves.
// Basic is the same four mathematical waves used by the control-thread factory.
fn wavetable_display_samples(node: &Node, patch: &Patch, position: f32) -> Option<[f32; 161]> {
    let (asset, points) = match &node.content {
        Some(openpatch_core::ModuleContent::Wavetable {
            asset_id,
            wave_points,
            ..
        }) => (Some(patch.assets.get(asset_id)?), *wave_points as usize),
        None => (None, 1024),
        _ => return None,
    };
    let sample_count = asset.map_or(4096, |asset| asset.sample_count());
    if points < 2 || points > sample_count {
        return None;
    }
    let waves = sample_count / points;
    let wave = position.clamp(0.0, 1.0) * (waves - 1) as f32;
    let first = wave.floor() as usize;
    let second = (first + 1).min(waves - 1);
    let blend = f64::from(wave.fract());
    let sample = |index: usize| -> f64 {
        if let Some(asset) = asset {
            return f64::from(asset.sample(index).unwrap_or(0.0));
        }
        let phase = (index % 1024) as f32 / 1024.0;
        f64::from(match index / 1024 {
            0 => (std::f32::consts::TAU * phase).sin(),
            1 => {
                if phase < 0.25 {
                    phase * 4.0
                } else if phase < 0.75 {
                    2.0 - phase * 4.0
                } else {
                    phase * 4.0 - 4.0
                }
            }
            2 => {
                if phase < 0.5 {
                    2.0 * phase
                } else {
                    2.0 * phase - 2.0
                }
            }
            _ => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
        })
    };
    Some(std::array::from_fn(|index| {
        let phase = index as f64 / 160.0 * points as f64;
        let point = phase.floor() as usize % points;
        let next = (point + 1) % points;
        let fraction = phase.fract();
        let a = sample(first * points + point) * (1.0 - fraction)
            + sample(first * points + next) * fraction;
        let b = sample(second * points + point) * (1.0 - fraction)
            + sample(second * points + next) * fraction;
        (a * (1.0 - blend) + b * blend) as f32
    }))
}
fn paint_wavetable_display(
    painter: &egui::Painter,
    node: &Node,
    patch: &Patch,
    rect: Rect,
    zoom: f32,
    lights: Option<&ModuleLightState>,
    lighting: &crate::lighting::RoomLighting,
) {
    let point = |x, y| rect.min + vec2(x, y) * zoom;
    let outer_clip = painter.clip_rect();
    let screen = Rect::from_min_max(point(18.0, 78.0), point(342.0, 144.0));
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(screen));
    painter.rect_filled(screen, 3.0 * zoom, Color32::from_rgb(32, 44, 43));
    let position = wavetable_display_position(node, lights);
    let (name, points) = match &node.content {
        Some(openpatch_core::ModuleContent::Wavetable {
            name, wave_points, ..
        }) => (name.as_str(), *wave_points),
        _ => ("Basic", 1024),
    };
    painter.text(
        point(25.0, 87.0),
        Align2::LEFT_CENTER,
        format!("{name} · {points} points"),
        crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
        Color32::from_rgb(201, 224, 207),
    );
    if let Some(samples) = wavetable_display_samples(node, patch, position) {
        let peak = samples
            .iter()
            .map(|v| f64::from(*v).abs())
            .fold(1.0_f64, f64::max);
        let line: Vec<_> = samples
            .iter()
            .enumerate()
            .map(|(i, sample)| {
                point(
                    25.0 + i as f32 / 160.0 * 310.0,
                    115.0 - (f64::from(*sample) / peak) as f32 * 19.0,
                )
            })
            .collect();
        painter.add(egui::Shape::line(
            line,
            Stroke::new(1.2 * zoom, Color32::from_rgb(167, 224, 187)),
        ));
    } else {
        painter.text(
            point(180.0, 116.0),
            Align2::CENTER_CENTER,
            "Table unavailable",
            crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
            Color32::from_gray(190),
        );
    }
    let track = Rect::from_min_max(point(25.0, 138.0), point(335.0, 141.0));
    painter.rect_filled(track, 0.0, Color32::from_gray(85));
    painter.circle_filled(
        pos2(egui::lerp(track.x_range(), position), track.center().y),
        2.5 * zoom,
        Color32::from_rgb(235, 198, 94),
    );
    let painter = painter.with_clip_rect(rect.intersect(outer_clip));
    let labels = if node.kind == "wavetable_vco" {
        ["−", "+", "POLY", "SOFT", "LIN"]
    } else {
        ["A", "B", "POLY", "OFFSET", "INV"]
    };
    for (index, label) in labels.iter().enumerate() {
        let center = point(54.0 + index as f32 * 63.0, 154.0);
        let strength = lights
            .filter(|state| index < usize::from(state.count))
            .map_or(0.0, |state| state.brightness[index][0])
            .clamp(0.0, 1.0);
        lighting.circle(
            &painter,
            center,
            2.5 * zoom,
            Color32::from_rgb(207, 176, 83).gamma_multiply(0.12 + strength * 0.88),
            strength,
            0.12,
        );
        painter.text(
            center + vec2(6.0, 0.0) * zoom,
            Align2::LEFT_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
            MUTED,
        );
    }
}

include!("annotation_panel.rs");
include!("precision_editor.rs");
include!("adsr_eg_panel.rs");
include!("stock_vcf_panel.rs");
include!("stock_delay_panel.rs");
include!("stock_vca_panel.rs");
include!("stock_octave_panel.rs");
include!("midi_cc_panel.rs");
include!("midi_gate_panel.rs");
include!("midi_assignment_panel.rs");

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
    if matches!(edit.kind.as_str(), "wavetable_lfo" | "random" | "stock_lfo")
        && edit.parameter == "rate"
    {
        (parsed / edit.display_scale).log2()
    } else if edit.kind == "stock_seq3" && edit.parameter == "tempo" {
        (parsed / 60.0).log2()
    } else if edit.kind == "stock_vco" && edit.parameter == "frequency" {
        stock_vco_frequency_raw(parsed)
    } else if edit.kind == "stock_vcf" && edit.parameter == "frequency" {
        stock_vcf_frequency_raw(parsed)
    } else if edit.kind == "stock_vcf" && edit.parameter == "drive" {
        parsed / 100.0 - 1.0
    } else if adsr_eg_time_edit(edit) || stock_delay_time_edit(edit) {
        parsed.log10() / 4.0
    } else if edit.kind == "stock_delay" && edit.parameter == "tone" {
        parsed / 200.0 + 0.5
    } else if let Some(power) = edit.gain_power {
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
    parameter_editor_contents_with_navigation(ui, edit, false)
}

fn parameter_editor_contents_with_navigation(
    ui: &mut egui::Ui,
    edit: &mut ParameterEdit,
    navigation: bool,
) -> ParameterEditOutcome {
    ui.label(format!("Range: {} to {}", edit.min, edit.max));
    let mut field = egui::TextEdit::singleline(&mut edit.text)
        .id(ui.id().with("parameter-draft"))
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
    if adsr_eg_time_edit(edit) {
        ui.label("Enter a time constant in milliseconds (1–10,000 ms). Attack time to peak is longer than its time constant.");
    } else if edit.kind == "adsr_eg" && edit.parameter == "sustain" {
        ui.label("Enter sustain from 0% to 100% of the nominal 10 V peak.");
    } else if edit.kind == "adsr_eg" {
        ui.label("Enter CV depth from −100% to +100%. Negative depth inverts the CV contribution.");
    }
    if edit.kind == "stock_vcf" {
        ui.label(match edit.parameter.as_str() {
            "frequency" => "Enter cutoff from 8 to 22,000 Hz, or a note such as A4.",
            "drive" => "Enter drive from 0% to 200%; 100% is nominal drive.",
            "resonance" => "Enter resonance from 0% to 100%.",
            _ => "Enter CV depth from −100% to +100%. Negative depth inverts modulation.",
        });
    }
    if edit.kind == "stock_delay" {
        ui.label(if stock_delay_time_edit(edit) { "Enter delay time in milliseconds (1–10,000 ms), or add s for seconds. The clock input sets time from incoming edges." } else if edit.parameter == "tone" { "Tone −100% to +100%, dark to bright; center is neutral." } else if edit.parameter.ends_with("_cv_amount") { "CV depth −100% to +100%. Negative depth inverts modulation." } else { "Enter a value from 0% to 100%." });
    }
    if edit.kind == "random" {
        ui.label(match edit.parameter.as_str() {
            "rate" => "Enter the free-running rate in Hz (0.002–2,000 Hz). An external trigger replaces the internal clock.",
            "unipolar" => "0 = bipolar (−5 to +5 V), 1 = offset (0 to 10 V).",
            "seed" => "Enter an integer seed from 1 to 16,777,215.",
            "rate_cv" | "probability_cv" | "spread_cv" | "shape_cv" => "Enter CV depth from −100% to +100%. Negative depth inverts modulation.",
            _ => "Enter a value from 0% to 100%.",
        });
    }
    if edit.kind == "stock_vca" {
        ui.label(if edit.parameter == "level" { "Level 0–100%. The meter shows effective gain, including silent voices." } else { "Response: 0 = EXP (fourth power), 1 = LIN. Bypass freezes gain telemetry and passes raw signal." });
    }
    if edit.gain_power.is_some() {
        ui.label("Enter dB. -inf sets the gain to zero.");
    }
    if edit.slew_min.is_some() {
        ui.label(if edit.kind == "stock_process" {
            "Enter 0 for instant (slew CV ignored), or more than 1 ms/V for slew."
        } else {
            "Enter milliseconds per volt. 0 = instant (slew CV ignored)."
        });
    }
    if edit.display_scale == 0.5 {
        ui.label("Enter a clock multiplier (1 = same rate as clock).\nFree-runs until two rising edges arrive.");
    }
    if edit.kind == "stock_lfo" && edit.parameter == "rate" {
        ui.label("Rate stores log2 Hz. A measured 2 Hz reference uses Hz; another measured clock uses a multiplier. Clock telemetry unavailable: enter reference Hz (2^rate). Runtime rate also follows the measured clock and FM.");
    }
    if edit.kind == "wavetable_lfo" && edit.parameter == "rate" && edit.display_scale != 0.5 {
        ui.label("Enter the free-running rate in Hz. Connecting a clock changes this control to a clock multiplier.");
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
    } else if edit.kind != "adsr_eg"
        && matches!(edit.parameter.as_str(), "pulse_width" | "duty" | "sustain")
    {
        ui.label("Enter a ratio from 0 to 1.");
    }
    if edit.kind != "adsr_eg" && matches!(edit.parameter.as_str(), "attack" | "decay" | "release") {
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
    let tab = navigation
        && (input.has_focus() || input.lost_focus())
        && ui.input(|i| {
            i.focused
                && !i.modifiers.alt
                && !i.modifiers.ctrl
                && !i.modifiers.command
                && i.key_pressed(egui::Key::Tab)
        });
    let mut navigate = None;
    if tab {
        let backwards = ui.input(|i| i.modifiers.shift);
        ui.input_mut(|i| {
            let modifiers = i.modifiers;
            i.consume_key(modifiers, egui::Key::Tab);
        });
        input.request_focus();
        if parsed.is_some() && !edit.stale {
            navigate = Some(!backwards);
        }
    }
    if navigation {
        ui.horizontal(|ui| {
            let ready = parsed.is_some() && !edit.stale;
            if ui
                .add_enabled(ready, egui::Button::new("Previous"))
                .on_hover_text(
                    "Apply this value and edit the previous type-entry control (Shift+Tab)",
                )
                .clicked()
            {
                navigate = Some(false);
            }
            if ui
                .add_enabled(ready, egui::Button::new("Apply & Next"))
                .on_hover_text("Apply this value and edit the next type-entry control (Tab)")
                .clicked()
            {
                navigate = Some(true);
            }
        });
        ui.small("Tab applies and moves · Shift+Tab goes back");
    }
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
    } else if let Some(forward) = navigate {
        ParameterEditOutcome::ApplyAndNavigate {
            raw: parameter_edit_raw_value(edit, parsed.unwrap()),
            forward,
        }
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

fn paint_signal_light(
    painter: &egui::Painter,
    center: Pos2,
    brightness: [f32; 2],
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
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
        lighting.circle(
            painter,
            point,
            2.2 * zoom,
            base.gamma_multiply(0.12 + strength * 0.88),
            strength,
            0.12,
        );
    }
}

fn is_signal_utility(kind: &str) -> bool {
    matches!(kind, "logic" | "compare" | "process" | "rescale")
}

fn parsed_parameter_result(edit: &ParameterEdit) -> Result<f32, String> {
    let mut text = edit.text.trim();
    let mut unit_scale = 1.0;
    if edit.kind == "stock_process"
        && edit.parameter == "slew"
        && text
            .get(text.len().saturating_sub(4)..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case("ms/V"))
    {
        text = text[..text.len() - 4].trim_end();
    }
    if stock_delay_time_edit(edit) {
        if text
            .get(text.len().saturating_sub(2)..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case("ms"))
        {
            text = text[..text.len() - 2].trim_end();
        } else if text
            .get(text.len().saturating_sub(1)..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case("s"))
        {
            text = text[..text.len() - 1].trim_end();
            unit_scale = 1000.0;
        }
    }
    if edit.gain_power.is_some()
        && text.len() > 2
        && text
            .get(text.len().saturating_sub(2)..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case("db"))
    {
        text = text[..text.len() - 2].trim_end();
    }
    if matches!(edit.kind.as_str(), "adsr_eg" | "stock_vcf" | "stock_delay")
        || (edit.kind == "stock_vca" && edit.parameter == "level")
    {
        let suffix = if adsr_eg_time_edit(edit) || stock_delay_time_edit(edit) {
            "ms"
        } else if edit.kind == "stock_vcf" && edit.parameter == "frequency" {
            "Hz"
        } else {
            "%"
        };
        if text
            .get(text.len().saturating_sub(suffix.len())..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix))
        {
            text = text[..text.len() - suffix.len()].trim_end();
        }
    }
    if edit.kind == "random" {
        let suffix = match edit.parameter.as_str() {
            "rate" => "Hz",
            "unipolar" | "seed" => "",
            _ => "%",
        };
        if !suffix.is_empty()
            && text
                .get(text.len().saturating_sub(suffix.len())..)
                .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix))
        {
            text = text[..text.len() - suffix.len()].trim_end();
        }
    }
    if edit.kind == "stock_quantizer" {
        for suffix in ["semitones", "semitone", "st"] {
            if text
                .get(text.len().saturating_sub(suffix.len())..)
                .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix))
            {
                text = text[..text.len() - suffix.len()].trim_end();
                break;
            }
        }
    }
    if edit.kind == "stock_seq3" {
        let suffix = if edit.parameter == "tempo" {
            "BPM"
        } else if matches!(edit.parameter.as_str(), "tempo_amount" | "steps_amount") {
            "%"
        } else {
            "V"
        };
        if text
            .get(text.len().saturating_sub(suffix.len())..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix))
        {
            text = text[..text.len() - suffix.len()].trim_end();
        }
    }
    if edit.kind == "stock_vco" {
        let suffix = if edit.parameter == "frequency" {
            "Hz"
        } else {
            "%"
        };
        if text
            .get(text.len().saturating_sub(suffix.len())..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix))
        {
            text = text[..text.len() - suffix.len()].trim_end();
        }
    }
    if edit.kind == "stock_lfo" {
        let suffix = if edit.parameter == "rate" {
            edit.display_unit.as_str()
        } else {
            "%"
        };
        if text
            .get(text.len().saturating_sub(suffix.len())..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix))
        {
            text = text[..text.len() - suffix.len()].trim_end();
        }
    }
    if is_wavetable(&edit.kind) {
        let suffix = match edit.parameter.as_str() {
            "position" | "fm_amount" | "position_cv_amount" => "%",
            "tune" => "st",
            "rate" => edit.display_unit.as_str(),
            _ => "",
        };
        if !suffix.is_empty()
            && text
                .get(text.len().saturating_sub(suffix.len())..)
                .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix))
        {
            text = text[..text.len() - suffix.len()].trim_end();
        }
    }
    let evaluated =
        crate::expression::evaluate(text).map_err(|error| error.to_string())? * unit_scale;
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
    if edit.kind == "stock_process"
        && edit.parameter == "slew"
        && value > 0.0
        && parameter_edit_raw_value(edit, value) <= STOCK_PROCESS_MIN_SLEW
    {
        return Err("Enter 0 for instant, or more than 1 ms/V for slew.".into());
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
    if node.kind == "stock_vca" {
        ui.separator();
        ui.label("CV response");
        let response = node.params.get("response").copied().unwrap_or(1.0);
        for (value, label) in [(0, "Exponential (fourth power)"), (1, "Linear")] {
            if ui
                .selectable_label(response == value as f32, label)
                .clicked()
            {
                *action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":"response","value":value}),
                ));
                ui.close();
            }
        }
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
        "studio_mixer" => return studio_mixer_port(input, index, width),
        "stock_quantizer" => return pos2(if input { 85.0 } else { 215.0 }, 376.0),
        "stock_vca_mix" if input => {
            return match index {
                0 => pos2(stock_vca_mix_column(4), 336.0),
                1..=4 => pos2(stock_vca_mix_column(index - 1), 292.0),
                5..=8 => pos2(stock_vca_mix_column(index - 5), 336.0),
                _ => pos2(width * 0.5, 292.0),
            }
        }
        "stock_vca_mix" => {
            return if index == 0 {
                pos2(stock_vca_mix_column(4), 380.0)
            } else {
                pos2(stock_vca_mix_column(index - 1), 380.0)
            }
        }
        "stock_process" => return stock_process_port_position(input, index),
        "stock_octave" => {
            return pos2(
                if input {
                    45.0 + index as f32 * 90.0
                } else {
                    90.0
                },
                376.0,
            )
        }
        "stock_seq3" => {
            return if input {
                pos2(28.0 + index as f32 * 37.5, 382.0)
            } else {
                pos2(
                    238.0
                        + (if (4..12).contains(&index) {
                            index - 4
                        } else if index < 4 {
                            index
                        } else {
                            index - 8
                        }) as f32
                            * 67.0,
                    if (4..12).contains(&index) {
                        382.0
                    } else {
                        341.0
                    },
                )
            }
        }
        "stock_noise" if !input => return pos2(width - 30.0, stock_noise_row_y(index)),
        "stock_lfo" | "stock_vco" => {
            return pos2(
                width * (index + 1) as f32 / 5.0,
                if input { 334.0 } else { 376.0 },
            )
        }
        "random" => {
            return pos2(
                if input {
                    30.0 + index as f32 * 60.0
                } else {
                    36.0 + index as f32 * 72.0
                },
                if input { 341.0 } else { 382.0 },
            );
        }
        "macro_voice" => {
            return pos2(
                width * (index + 1) as f32 / (count + 1) as f32,
                if input { 337.0 } else { 378.0 },
            )
        }
        "midi_cc" if !input => return midi_cc_output_position(index),
        "midi_gate" if !input => return midi_assignment_output_position(index),
        "stock_vca" => {
            return pos2(
                if input {
                    32.0 + index as f32 * 55.0
                } else {
                    146.0
                },
                365.0,
            )
        }
        "wavetable_vco" | "wavetable_lfo" => {
            return pos2(36.0 + if input { index as f32 } else { 4.0 } * 72.0, 367.0);
        }
        "stock_delay" => {
            return if input && index < 4 {
                pos2(stock_delay_column(index), 344.0)
            } else if input {
                pos2(stock_delay_column(index - 4), 380.0)
            } else {
                pos2(stock_delay_column(index + 2), 380.0)
            };
        }
        "stock_vcf" => {
            return if input {
                pos2(37.5 + index as f32 * 65.0, 337.0)
            } else {
                pos2(85.0 + index as f32 * 100.0, 374.0)
            };
        }
        "adsr_eg" => {
            return if input {
                if index < 4 {
                    pos2(58.0 + index as f32 * 80.0, 314.0)
                } else {
                    pos2(46.0 + (index - 4) as f32 * 82.0, 374.0)
                }
            } else {
                pos2(314.0, 374.0)
            };
        }
        "sh_asr" => {
            return if input {
                pos2(
                    if index < 8 { 80.0 } else { 146.0 },
                    shift_register_row_y(index % 8),
                )
            } else {
                pos2(width - 30.0, shift_register_row_y(index))
            }
        }
        "random_values" => {
            return if input {
                pos2(34.0, 150.0)
            } else {
                pos2(width - 25.0, random_values_row_y(index))
            }
        }
        "colored_noise" if !input => return pos2(width - 25.0, noise_row_y(index)),
        "modal_resonator" => {
            return pos2(
                if input {
                    60.0 + index as f32 * 95.0
                } else {
                    410.0
                },
                363.0,
            )
        }
        "scope" => {
            return pos2(
                if input {
                    [70.0, 170.0, 285.0][index]
                } else {
                    [460.0, 560.0][index]
                },
                360.0,
            )
        }
        "poly_viz" => return pos2(width * 0.5, 367.0),
        "poly_sum" => return pos2(if input { 45.0 } else { 105.0 }, 367.0),
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
    if matches!(kind, "mid_side" | "stock_mid_side") {
        let index = if input && kind == "stock_mid_side" {
            [4, 0, 1, 5, 2, 3][index]
        } else {
            index
        };
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
    if matches!(kind, "mix6" | "stock_mix") && input {
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

fn palette_commit_action(pending: &PendingCable, action: Option<RackAction>) -> Option<RackAction> {
    match (pending.operation, pending.palette_selection, action) {
        (
            CableOperation::Create,
            Some(selection),
            Some(RackAction::Command("connect", arguments)),
        ) => Some(RackAction::PaletteConnect {
            arguments,
            selection,
        }),
        (_, _, action) => action,
    }
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

// These helpers describe the same command used on release; they never stage an
// edit or modify endpoint order. Press-captured clone/fan-out intent stays fixed,
// while Shift's duplicate policy is read again on every frame and on release.
fn cable_gesture_related(cable: &Cable, pending: &PendingCable, target: Option<&CableEnd>) -> bool {
    let touches = |end: &CableEnd| match end {
        CableEnd::Output { node, port } => cable.from == *node && cable.output == *port,
        CableEnd::Input { node, port } => cable.to == *node && cable.input == *port,
    };
    touches(&pending.origin)
        || target.is_some_and(touches)
        || pending
            .original
            .iter()
            .chain(&pending.companions)
            .any(|old| {
                // Both fixed anchors of a picked-up bundle remain legible.
                (cable.from == old.from && cable.output == old.output)
                    || (cable.to == old.to && cable.input == old.input)
            })
}

#[derive(Debug, PartialEq)]
struct CableGestureGuidance {
    route: String,
    action: String,
}

fn cable_gesture_guidance(
    patch: &Patch,
    pending: &PendingCable,
    target: Option<&CableEnd>,
    catalog: &Value,
    shift: bool,
    dragging: bool,
) -> CableGestureGuidance {
    let command = if dragging {
        cable_release_action(patch, pending, target, catalog, shift)
    } else {
        target.and_then(|target| cable_action_mode(patch, pending, target, catalog, shift))
    };
    let plan = target.and_then(|target| cable_plan(pending, target));
    let endpoint = |node: &str, port: &str| format!("{node} · {}", port_label(port).to_uppercase());
    let route = if let Some((cable, _)) = &plan {
        format!(
            "{} -> {}",
            endpoint(&cable.from, &cable.output),
            endpoint(&cable.to, &cable.input)
        )
    } else if let Some(old) = &pending.original {
        match pending.origin {
            CableEnd::Input { .. } => format!("{} -> IN ?", endpoint(&old.from, &old.output)),
            CableEnd::Output { .. } => format!("OUT ? -> {}", endpoint(&old.to, &old.input)),
        }
    } else {
        match &pending.origin {
            CableEnd::Input { node, port } => format!("OUT ? -> {}", endpoint(node, port)),
            CableEnd::Output { node, port } => format!("{} -> IN ?", endpoint(node, port)),
        }
    };
    let duplicate_deleted = !pending.allow_duplicate(shift)
        && pending.operation == CableOperation::Move
        && plan.as_ref().is_some_and(|(planned, old)| {
            patch.cables.iter().any(|cable| {
                !old.is_some_and(|old| same_cable(old, cable)) && same_route(cable, planned)
            })
        });
    let operation = match &command {
        Some(RackAction::Command("disconnect" | "disconnect_cables", _)) => "Delete",
        Some(RackAction::Command("promote_cables", _)) => "Keep route · raise plug",
        Some(RackAction::Command("move_cables", _))
            if duplicate_deleted && pending.companions.is_empty() =>
        {
            "Delete duplicate"
        }
        Some(RackAction::Command("move_cables" | "apply_edits", _)) => "Move",
        Some(RackAction::Command("connect", _)) if pending.operation == CableOperation::Clone => {
            "Clone"
        }
        Some(RackAction::Command("connect", _)) => {
            if plan.as_ref().is_some_and(|(planned, _)| {
                patch
                    .cables
                    .iter()
                    .any(|cable| cable.from == planned.from && cable.output == planned.output)
            }) {
                "Fan out"
            } else {
                "Create"
            }
        }
        _ if !dragging && target.is_none() => "Choose a jack",
        _ if dragging && pending.operation != CableOperation::Move => "Discard",
        _ if pending.operation == CableOperation::Move => "Keep original",
        _ => "No connection",
    };
    let mut action = format!(
        "{}: {operation}",
        if dragging { "Release" } else { "Click" }
    );
    if command.is_none() && patch.version < 2 && plan.is_some() {
        // Explain legacy routing/stacking limits only when the identical edit
        // would actually succeed in v2. Capacity and invalid-port failures must
        // not promise that an engine change would accept the gesture.
        let mut version_two = patch.clone();
        version_two.version = 2;
        if target.is_some_and(|target| {
            cable_action_mode(&version_two, pending, target, catalog, shift).is_some()
        }) {
            action.push_str(" · requires engine v2");
        }
    }
    if command.is_some() && !pending.companions.is_empty() {
        action.push_str(&format!(" · {} cables", pending.companions.len() + 1));
    }
    if matches!(
        &command,
        Some(RackAction::Command(
            "connect" | "move_cables" | "apply_edits",
            _
        ))
    ) {
        if !duplicate_deleted
            && plan.as_ref().is_some_and(|(planned, old)| {
                patch.cables.iter().any(|cable| {
                    !old.is_some_and(|old| same_cable(old, cable))
                        && cable.to == planned.to
                        && cable.input == planned.input
                })
            })
        {
            action.push_str(" · stack");
        }
        if !pending.companions.is_empty() && !pending.allow_duplicate(shift) {
            action.push_str(" · duplicate conflicts deleted");
        }
    }
    CableGestureGuidance { route, action }
}

fn cable_guidance_rect(canvas: Rect, pointer: Pos2, size: Vec2) -> Rect {
    let bounds = canvas.shrink(6.0_f32.min(canvas.width().min(canvas.height()) * 0.25));
    let size = size.min(bounds.size());
    let preferred = pointer + vec2(18.0, 22.0);
    Rect::from_min_size(
        pos2(
            preferred.x.clamp(bounds.left(), bounds.right() - size.x),
            preferred.y.clamp(bounds.top(), bounds.bottom() - size.y),
        ),
        size,
    )
}

fn paint_cable_gesture_guidance(
    painter: &egui::Painter,
    canvas: Rect,
    pointer: Pos2,
    guidance: &CableGestureGuidance,
    dragging: bool,
) {
    if canvas.width() < 24.0 || canvas.height() < 24.0 {
        return;
    }
    // Canvas-clipped paint only: no response, focus, hit target or overlay layer.
    // Painted after room lighting, like the compatible-target ring and draft.
    let painter = painter.with_clip_rect(canvas.intersect(painter.clip_rect()));
    let text = format!("{}\n{} · Esc cancels", guidance.route, guidance.action);
    let galley = painter.layout(
        text,
        FontId::proportional(12.0),
        Color32::from_gray(238),
        (canvas.width() - 32.0).clamp(1.0, 320.0),
    );
    let anchor = if dragging {
        pointer
    } else {
        canvas.left_bottom() + vec2(0.0, -galley.size().y - 50.0)
    };
    let rect = cable_guidance_rect(canvas, anchor, galley.size() + vec2(16.0, 12.0));
    let painter = painter.with_clip_rect(rect.intersect(canvas));
    painter.rect_filled(rect, 4.0, Color32::from_black_alpha(236));
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0_f32, Color32::from_gray(100)),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.min + vec2(8.0, 6.0), galley, Color32::from_gray(238));
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

fn merge_channels_label(value: f32) -> String {
    if value == 0.0 {
        "Auto".to_owned()
    } else if value == -1.0 {
        "0".to_owned()
    } else {
        format!("{value:.0}")
    }
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

pub(crate) fn module_width(node: &Node) -> f32 {
    if node.kind == "studio_mixer" {
        return studio_mixer_channels(node) as f32 * 72.0 + 228.0;
    }
    node.effective_blank_width_hp()
        .map_or_else(|| panel_width(&node.kind), |hp| f32::from(hp) * HP)
}

pub(crate) fn panel_width(kind: &str) -> f32 {
    match kind {
        "notes" => HP * 16.0,
        "blank" => HP * 10.0,
        "random" => HP * 24.0,
        "random_values" => HP * 20.0,
        "sh_asr" => HP * 26.0,
        "colored_noise" => HP * 16.0,
        "stock_noise" => HP * 20.0,
        "studio_mixer" => 804.0,
        "modal_resonator" => HP * 32.0,
        "macro_voice" => HP * 20.0,
        "scope" => HP * 42.0,
        "score_sequencer" => HP * 40.0,
        "stereo_delay" | "sidechain_compressor" => HP * 28.0,
        "stereo_reverb" | "drum_voice" => HP * 24.0,
        "svf_filter" => HP * 18.0,
        "vca_mix" => HP * 26.0,
        "stock_vca_mix" => HP * 18.0,
        "stock_process" => HP * 20.0,
        "fade" => HP * 16.0,
        "logic" | "compare" | "process" | "rescale" => HP * 16.0,
        "attenuverter8" | "mutes10" | "cv_mix" | "pulses" | "gates" => HP * 16.0,
        "seq3" | "stock_seq3" => HP * 52.0,
        "switch_1to4" | "switch_4to1" => HP * 12.0,
        "poly_constant" => HP * 24.0,
        "midi_cv" => HP * 24.0,
        "midi_cc" | "midi_gate" => HP * 32.0,
        "audio_stereo" | "octave" | "push" => HP * 12.0,
        "mid_side" | "stock_mid_side" | "mult" | "multi_lfo" | "multi_vco" => HP * 20.0,
        "mix6" | "stock_mix" => HP * 16.0,
        "poly_split" | "poly_merge" => HP * 20.0,
        "poly_sum" => HP * 10.0,
        "poly_viz" => HP * 12.0,
        "stock_quantizer" => HP * 20.0,
        "quantizer" => HP * 16.0,
        "white_noise" | "attenuverter" | "crossfade" => HP * 12.0,
        "sample_hold" => HP * 10.0,
        "adsr" => HP * 16.0,
        "adsr_eg" => HP * 24.0,
        "stock_vcf" => HP * 18.0,
        "stock_delay" | "stock_lfo" | "stock_vco" | "wavetable_vco" | "wavetable_lfo" => HP * 24.0,
        "stock_vca" => HP * 12.0,
        "stock_octave" => HP * 12.0,
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
        "notes" => "NOTES",
        "blank" => "BLANK",
        "modal_resonator" => "MODAL RESONATOR",
        "macro_voice" => "MACRO VOICE",
        "scope" => "SCOPE",
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
        "midi_cc" => "MIDI CC",
        "midi_gate" => "MIDI GATE",
        "audio_stereo" => "STEREO AUDIO",
        "mid_side" => "MID / SIDE",
        "stock_mid_side" => "STOCK MID/SIDE",
        "mix6" => "MIX 6",
        "stock_mix" => "STOCK MIX",
        "stock_vca_mix" => "STOCK VCA MIX",
        "stock_process" => "STOCK PROCESS",
        "stock_noise" => "STOCK NOISE",
        "studio_mixer" => "STUDIO MIXER",
        "octave" => "OCTAVE",
        "mult" => "MULT",
        "poly_split" => "SPLIT",
        "poly_merge" => "MERGE",
        "poly_sum" => "SUM",
        "poly_viz" => "VIZ",
        "quantizer" => "QUANTIZER",
        "white_noise" => "WHITE NOISE",
        "colored_noise" => "NOISE",
        "random" => "RANDOM",
        "random_values" => "RANDOM VALUES",
        "sh_asr" => "SHIFT REGISTER",
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
        "stock_vcf" => "VCF",
        "wavetable_vco" => "WAVETABLE VCO",
        "wavetable_lfo" => "WAVETABLE LFO",
        "stock_delay" => "DELAY",
        "stock_vca" => "VCA",
        "stock_lfo" => "STOCK LFO",
        "stock_vco" => "STOCK VCO",
        "stock_seq3" => "STOCK SEQ3",
        "stock_quantizer" => "STOCK QUANTIZER",
        "stock_octave" => "STOCK OCTAVE",
        "analog_vca" => "OTA VCA",
        "adsr" => "ADSR",
        "adsr_eg" => "ADSR EG",
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
        "engine" => "ENGINE",
        "harmonics" => "HARMONICS",
        "timbre" => "TIMBRE",
        "morph" => "MORPH",
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
        "stiffness" => "STIFFNESS",
        "strike_position" => "STRIKE POS",
        "pickup_position" => "PICKUP POS",
        "strike_strength" => "STRENGTH",
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
        "range" => "RANGE",
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
        "harmonics" => "HARM",
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
        "attack_cv" => "A CV",
        "decay_cv" => "D CV",
        "sustain_cv" => "S CV",
        "release_cv" => "R CV",
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
        "macro_voice" => &[
            "engine",
            "frequency",
            "harmonics",
            "timbre",
            "morph",
            "decay",
            "level",
        ],
        "modal_resonator" => &[
            "frequency",
            "stiffness",
            "decay",
            "damping",
            "strike_position",
            "pickup_position",
            "strike_strength",
            "level",
        ],
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
        "stock_vca_mix" => &STOCK_VCA_MIX_ALL_PARAMS,
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
        "midi_gate" => &[
            "note1", "note2", "note3", "note4", "note5", "note6", "note7", "note8", "note9",
            "note10", "note11", "note12", "note13", "note14", "note15", "note16", "channel",
            "velocity", "mpe",
        ],
        "midi_cc" => &[
            "cc1",
            "cc2",
            "cc3",
            "cc4",
            "cc5",
            "cc6",
            "cc7",
            "cc8",
            "cc9",
            "cc10",
            "cc11",
            "cc12",
            "cc13",
            "cc14",
            "cc15",
            "cc16",
            "channel",
            "smooth",
            "mpe",
            "high_resolution",
        ],
        "mid_side" | "stock_mid_side" => &["encode_width", "decode_width"],
        "mix6" | "stock_mix" => &["level", "average", "invert"],
        "octave" | "stock_octave" => &["octaves"],
        "white_noise" | "colored_noise" => &["level", "seed"],
        "stock_noise" => &[],
        "random" => &[
            "rate",
            "probability",
            "spread",
            "shape",
            "rate_cv",
            "probability_cv",
            "spread_cv",
            "shape_cv",
            "unipolar",
            "seed",
        ],
        "random_values" => &["range", "seed"],
        "sh_asr" => &["random", "range", "seed"],
        "attenuverter" => &["gain", "offset"],
        "crossfade" => &["mix", "law"],
        "analog_vco" => &["frequency", "waveform", "pulse_width"],
        "ladder_vcf" => &["cutoff", "resonance", "drive"],
        "stock_vcf" => &STOCK_VCF_PARAMS,
        "wavetable_vco" => &[
            "tune",
            "position",
            "fm_amount",
            "position_cv_amount",
            "sync_mode",
            "fm_mode",
        ],
        "wavetable_lfo" => &[
            "rate",
            "position",
            "fm_amount",
            "position_cv_amount",
            "offset",
            "invert",
        ],
        "stock_delay" => &STOCK_DELAY_PARAMS,
        "stock_vca" => &STOCK_VCA_PARAMS,
        "adsr" => &["attack", "decay", "sustain", "release"],
        "adsr_eg" => &[
            "attack",
            "decay",
            "sustain",
            "release",
            "attack_amount",
            "decay_amount",
            "sustain_amount",
            "release_amount",
        ],
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
        "stock_vco" => &[
            "frequency",
            "pulse_width",
            "fm_amount",
            "pwm_amount",
            "linear_fm",
            "soft_sync",
        ],
        "stock_lfo" => &[
            "rate",
            "pulse_width",
            "fm_amount",
            "pwm_amount",
            "offset",
            "invert",
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
                .filter(|(_, spec)| spec["hidden"] != true)
                .map(|(name, spec)| (name.as_str(), spec))
        })
        .collect();
    // Newly introduced catalog controls remain accessible without a custom order.
    ordered.extend(
        params
            .iter()
            .filter(|(name, spec)| !order.contains(&name.as_str()) && spec["hidden"] != true)
            .map(|(name, spec)| (name.as_str(), spec)),
    );
    ordered
}
fn consume_parameter_activation(ctx: &egui::Context) {
    ctx.input_mut(|input| {
        let modifiers = input.modifiers;
        input.consume_key(modifiers, egui::Key::Enter);
        input.consume_key(modifiers, egui::Key::Space);
    });
}
fn shift_register_row_y(index: usize) -> f32 {
    157.0 + index as f32 * 30.0
}
fn shift_register_random_rect() -> Rect {
    Rect::from_min_size(pos2(20.0, 77.0), vec2(74.0, 22.0))
}
fn shift_register_range_rect() -> Rect {
    Rect::from_min_size(pos2(150.0, 78.0), vec2(220.0, 20.0))
}
fn shift_register_seed_rect() -> Rect {
    Rect::from_min_size(pos2(60.0, 108.0), vec2(130.0, 20.0))
}
fn shift_register_button_rect(index: usize) -> Rect {
    Rect::from_min_size(pos2(202.0 + index as f32 * 84.0, 106.0), vec2(74.0, 25.0))
}
fn shift_register_readout_rect(index: usize) -> Rect {
    Rect::from_center_size(pos2(263.0, shift_register_row_y(index)), vec2(154.0, 19.0))
}
fn shift_register_snapshot_valid(state: &Value) -> bool {
    state["rng_state"]
        .as_u64()
        .is_some_and(|rng| (1..=u32::MAX as u64).contains(&rng))
        && state["values"].as_array().is_some_and(|values| {
            values.len() == 8
                && values.iter().all(|v| {
                    v.as_f64()
                        .is_some_and(|v| v.is_finite() && v.abs() <= 1_000_000.0)
                })
        })
}

const RANDOM_VALUES_RANGES: [&str; 6] = [
    "0..10 V",
    "0..5 V",
    "0..1 V",
    "−10..10 V",
    "−5..5 V",
    "−1..1 V",
];
const RANDOM_VALUES_OUTPUTS: [&str; 7] = ["out1", "out2", "out3", "out4", "out5", "out6", "out7"];
fn random_values_row_y(index: usize) -> f32 {
    183.0 + index as f32 * 30.0
}
fn random_values_range_rect() -> Rect {
    Rect::from_min_size(pos2(80.0, 78.0), vec2(202.0, 20.0))
}
fn random_values_seed_rect() -> Rect {
    Rect::from_min_size(pos2(80.0, 104.0), vec2(202.0, 20.0))
}
fn random_values_push_rect() -> Rect {
    Rect::from_min_size(pos2(91.0, 137.0), vec2(104.0, 27.0))
}
fn random_values_readout_rect(index: usize) -> Rect {
    Rect::from_center_size(pos2(165.0, random_values_row_y(index)), vec2(166.0, 20.0))
}
fn random_values_snapshot_valid(state: &Value) -> bool {
    state["active_channels"]
        .as_u64()
        .is_some_and(|count| count <= 16)
        && state["rng_state"]
            .as_u64()
            .is_some_and(|rng| rng <= u32::MAX as u64)
        && state["values"].as_array().is_some_and(|outputs| {
            outputs.len() == 7
                && outputs.iter().all(|output| {
                    output.as_array().is_some_and(|channels| {
                        channels.len() == 16
                            && channels
                                .iter()
                                .all(|voltage| voltage.as_f64().is_some_and(f64::is_finite))
                    })
                })
        })
}
fn random_values_voltage_text(state: Option<&Value>, output: usize) -> String {
    state.map_or_else(
        || "UNAVAILABLE".to_owned(),
        |s| format!("CH 1  {:+.3} V", s["values"][output][0].as_f64().unwrap()),
    )
}

// The row geometry is shared by real sockets, previews, labels, and picking.
const NOISE_OUTPUTS: [&str; 7] = ["white", "pink", "red", "violet", "blue", "gray", "black"];
fn noise_row_y(index: usize) -> f32 {
    175.0 + index as f32 * 31.0
}
fn noise_control_rects(index: usize) -> (Rect, Rect) {
    let y = 79.0 + index as f32 * 30.0;
    (
        Rect::from_min_size(pos2(73.0, y), vec2(61.0, 20.0)),
        Rect::from_min_size(pos2(142.0, y), vec2(80.0, 20.0)),
    )
}
fn noise_parameter_help(name: &str) -> &'static str {
    match name {
        "seed" => "Integer seed, 0–16,777,215. The same seed and sample rate reproduce the same sequence when the graph restarts. Changing the seed restarts this generator; level changes preserve its sequence.",
        "level" => "Shared output gain from 0× to 2×. The six Gaussian colors have nominal 3.536 V RMS at 1×; black is uniform ±5 V, with 2.887 V RMS.",
        _ => "",
    }
}
fn paint_noise_spectra(painter: &egui::Painter, origin: Pos2, zoom: f32, rate: u32) {
    let point = |x, y| origin + vec2(x, y) * zoom;
    painter.text(
        point(20.0, 147.0),
        Align2::LEFT_CENTER,
        "STATIC SPECTRA · LOG Hz",
        crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
        MUTED,
    );
    let colors = [
        Color32::from_rgb(111, 113, 109),
        Color32::from_rgb(178, 86, 124),
        Color32::from_rgb(170, 75, 61),
        Color32::from_rgb(123, 89, 171),
        Color32::from_rgb(65, 116, 173),
        Color32::from_rgb(115, 124, 129),
        Color32::from_rgb(54, 61, 57),
    ];
    let descriptions = [
        "0 dB/oct",
        "−3 dB/oct",
        "−6 dB/oct",
        "+6 dB/oct",
        "+3 dB/oct",
        "Inverse A",
        "Uniform ±5 V",
    ];
    let ceiling = (f64::from(rate) * 0.45).clamp(21.0, 20_000.0);
    for (index, port) in NOISE_OUTPUTS.iter().enumerate() {
        let y = noise_row_y(index);
        let color = colors[index];
        painter.line_segment(
            [point(18.0, y - 9.0), point(18.0, y + 11.0)],
            Stroke::new(3.0 * zoom, color),
        );
        painter.text(
            point(26.0, y - 5.5),
            Align2::LEFT_CENTER,
            port.to_uppercase(),
            crate::panel_skin::print_font(painter.ctx(), 13.0 * zoom),
            INK,
        );
        painter.text(
            point(26.0, y + 10.0),
            Align2::LEFT_CENTER,
            descriptions[index],
            crate::panel_skin::print_font(painter.ctx(), 9.5 * zoom),
            MUTED,
        );
        let plot = Rect::from_min_max(point(124.0, y - 8.0), point(186.0, y + 8.0));
        painter.rect_filled(
            plot.expand(2.0 * zoom),
            1.0 * zoom,
            Color32::from_rgb(216, 218, 210),
        );
        painter.line_segment(
            [
                pos2(plot.left(), plot.center().y),
                pos2(plot.right(), plot.center().y),
            ],
            Stroke::new(0.5 * zoom, Color32::from_rgb(172, 177, 167)),
        );
        // This is a fixed filter response, normalized at 1 kHz. It never reads
        // runtime sample history, level, seed, or wall-clock animation time.
        let reference =
            openpatch_core::colored_noise_response_db(port, 1000.0, rate).unwrap_or(0.0);
        let points = (0..33)
            .filter_map(|i| {
                let t = i as f64 / 32.0;
                let hz = 20.0 * (ceiling / 20.0).powf(t);
                openpatch_core::colored_noise_response_db(port, hz, rate).map(|db| {
                    pos2(
                        plot.left() + t as f32 * plot.width(),
                        plot.center().y
                            - ((db - reference) / 36.0).clamp(-1.0, 1.0) as f32
                                * plot.height()
                                * 0.5,
                    )
                })
            })
            .collect::<Vec<_>>();
        if points.len() >= 2 {
            painter.add(egui::Shape::line(points, Stroke::new(1.5 * zoom, color)));
        }
    }
}

// A large tuning control above a matched pair of secondary controls. Keep this
// geometry shared by rack rendering, previews, and the parameter hit regions.
fn resonator_control_rects(index: usize) -> (Rect, Rect) {
    let y = 85.0 + index as f32 * 29.0;
    (
        Rect::from_min_size(pos2(102.0, y), vec2(103.0, 20.0)),
        Rect::from_min_size(pos2(213.0, y), vec2(71.0, 20.0)),
    )
}

fn resonator_parameter_help(name: &str) -> &'static str {
    match name {
        "frequency" => "Undamped natural fundamental at zero pitch CV; damping can lower the ringing frequency. PITCH transposes at 1 V/octave.",
        "stiffness" => "Moves natural modes from harmonic string spacing toward a stiff beam. No material presets.",
        "decay" => "Fundamental RT60: time to lose 60 dB. Higher modes also follow DAMPING.",
        "damping" => "Adds loss to higher modes, shortening their RT60.",
        "strike_position" => "Force position along the string/beam, from one fixed end to the other. Shapes mode excitation.",
        "pickup_position" => "Velocity pickup position along the string/beam. Mode nodes can cancel individual partials.",
        "strike_strength" => "Impulse = 0.01 × strength N·s on each rising STRIKE edge. EXCITE applies continuous audio force.",
        "level" => "Output level after the velocity pickup.",
        _ => "",
    }
}

fn paint_resonator_configuration(
    painter: &egui::Painter,
    node: &Node,
    origin: Pos2,
    zoom: f32,
    rate: u32,
) {
    let point = |x, y| origin + vec2(x, y) * zoom;
    let print = |x, y, text: String, size, color| {
        painter.text(
            point(x, y),
            Align2::LEFT_CENTER,
            text,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    };
    let copper = Color32::from_rgb(151, 93, 57);
    let teal = Color32::from_rgb(68, 113, 121);
    print(311.0, 83.0, "NATURAL MODES".into(), 11.0, INK);
    print(
        311.0,
        99.0,
        format!("CONFIG · {} kHz · ZERO CV", rate as f32 / 1000.0),
        8.0,
        MUTED,
    );
    let Ok(modes) = openpatch_core::modal_resonator_modes(node, rate) else {
        print(311.0, 164.0, "Configuration unavailable".into(), 9.0, MUTED);
        return;
    };
    let plot = Rect::from_min_max(point(313.0, 118.0), point(456.0, 210.0));
    painter.rect_filled(
        plot.expand(7.0 * zoom),
        2.0 * zoom,
        Color32::from_rgb(207, 207, 195),
    );
    let ceiling = f64::from(rate) * 0.45;
    let x = |frequency: f64| {
        plot.left()
            + ((frequency.max(20.0) / 20.0).ln() / (ceiling / 20.0).ln()).clamp(0.0, 1.0) as f32
                * plot.width()
    };
    for (hz, label) in [
        (20.0, "20"),
        (200.0, "200"),
        (2000.0, "2k"),
        (20000.0, "20k"),
    ] {
        if hz > ceiling {
            continue;
        }
        let axis_x = x(hz);
        painter.line_segment(
            [pos2(axis_x, plot.top()), pos2(axis_x, plot.bottom())],
            Stroke::new(0.5 * zoom, Color32::from_rgb(182, 183, 170)),
        );
        painter.text(
            pos2(axis_x, plot.bottom() + 11.0 * zoom),
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            MUTED,
        );
    }
    let baseline = plot.center().y;
    painter.line_segment(
        [pos2(plot.left(), baseline), pos2(plot.right(), baseline)],
        Stroke::new(0.8 * zoom, Color32::from_rgb(128, 132, 120)),
    );
    for mode in &modes {
        if !mode.active {
            continue;
        }
        let foot = pos2(x(mode.frequency_hz), baseline);
        let tip = foot - vec2(0.0, mode.coupling as f32 * plot.height() * 0.45);
        let color = if mode.coupling >= 0.0 { copper } else { teal };
        painter.line_segment([foot, tip], Stroke::new(1.6 * zoom, color));
        painter.circle_filled(tip, 1.8 * zoom, color);
    }
    print(
        311.0,
        238.0,
        format!(
            "{} / {} modes below {:.1} kHz",
            modes.iter().filter(|mode| mode.active).count(),
            modes.len(),
            ceiling / 1000.0
        ),
        8.0,
        MUTED,
    );
    print(311.0, 253.0, "± FORCE × PICKUP · LOG Hz".into(), 8.0, MUTED);
    print(
        311.0,
        270.0,
        format!(
            "RT60  {:.2} → {:.2} s",
            modes[0].decay_seconds,
            modes[modes.len() - 1].decay_seconds
        ),
        9.0,
        INK,
    );
    // A static decay profile indexed by mode, independent of audio energy.
    for (index, mode) in modes.iter().enumerate() {
        let left = 313.0 + index as f32 * 9.0;
        let height = (mode.decay_seconds / modes[0].decay_seconds).clamp(0.0, 1.0) as f32 * 17.0;
        painter.rect_filled(
            Rect::from_min_max(point(left, 299.0 - height), point(left + 5.0, 299.0)),
            0.5 * zoom,
            if mode.active {
                copper
            } else {
                Color32::from_rgb(178, 178, 162)
            },
        );
    }
    print(
        311.0,
        312.0,
        format!("MODE 1 → {} · DECAY PROFILE", modes.len()),
        8.0,
        MUTED,
    );
}

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
    if kind == "stock_vco" && name == "frequency" {
        // The stored pitch is already logarithmic in Hz.
        return false;
    }
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
        let row_y = canvas.top() + pan.y + row as f32 * ROW * zoom;
        // Each faceplate attaches to its own lower rail. The next row's top
        // rail remains at ROW, leaving the existing layout pitch unchanged.
        for y in [row_y, row_y + PANEL_HEIGHT * zoom] {
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
}

fn panel_heading(kind: &str, width: f32) -> &str {
    if kind == "blank" && width < 75.0 {
        "BLK"
    } else {
        title(kind)
    }
}

fn paint_panel(
    painter: &egui::Painter,
    rect: Rect,
    node: &Node,
    selected: bool,
    master: bool,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    if matches!(node.kind.as_str(), "stock_vca_mix" | "stock_process") {
        crate::panel_skin::signal_atlas_face(painter, rect, zoom);
    } else if node.kind == "stock_noise" || signal_atlas_face_kind(&node.kind) {
        crate::panel_skin::signal_atlas_face(painter, rect, zoom);
        crate::panel_skin::identity_header(painter, rect, &node.kind, zoom);
    } else {
        crate::panel_skin::metal(painter, rect, zoom);
        crate::panel_skin::identity_header(painter, rect, &node.kind, zoom);
    }
    // Reflective mounting hardware belongs to the panel paint, including
    // browser thumbnails, and dims with the room like the rails behind it.
    for x in [rect.left() + 7.5 * zoom, rect.right() - 7.5 * zoom] {
        for y in [rect.top() + 5.0 * zoom, rect.bottom() - 5.0 * zoom] {
            let center = pos2(x, y);
            painter.circle_filled(
                center + vec2(0.0, 0.6) * zoom,
                3.7 * zoom,
                Color32::from_black_alpha(104),
            );
            painter.circle_filled(center, 3.2 * zoom, Color32::from_rgb(100, 103, 100));
            painter.circle_stroke(
                center,
                3.2 * zoom,
                Stroke::new(0.65 * zoom, Color32::from_rgb(143, 146, 140)),
            );
            painter.line_segment(
                [
                    center + vec2(-1.7, 1.0) * zoom,
                    center + vec2(1.7, -1.0) * zoom,
                ],
                Stroke::new(1.0 * zoom, Color32::from_rgb(39, 42, 40)),
            );
        }
    }
    if node.bypassed {
        painter.rect_stroke(
            rect.shrink(3.0),
            0,
            Stroke::new(1.8 * zoom, Color32::from_rgb(146, 102, 61)),
            egui::StrokeKind::Inside,
        );
    }
    if selected {
        let selection_start = crate::lighting::next(painter);
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
        lighting.preserve(painter, selection_start);
    }
    let point = |x, y| rect.min + vec2(x, y) * zoom;
    let width = rect.width() / zoom;
    let inset = if width < 90.0 { 8.0 } else { 14.0 };
    let label = panel_heading(&node.kind, width);
    let header_ink = if matches!(node.kind.as_str(), "stock_vca_mix" | "stock_process") {
        crate::panel_skin::INK
    } else {
        crate::panel_skin::HEADER_INK
    };
    let mut font = crate::panel_skin::heading_font(
        painter.ctx(),
        if width < 140.0 { 24.0 } else { 28.0 } * zoom,
    );
    let measured = painter.layout_no_wrap(label.to_owned(), font.clone(), header_ink);
    let available = (width - inset * 2.0 - if master { 16.0 } else { 0.0 }).max(1.0) * zoom;
    font.size *= (available / measured.size().x.max(1.0)).min(1.0);
    let heading = painter.layout_no_wrap(label.to_owned(), font, header_ink);
    painter.galley(point(inset, 19.0), heading, header_ink);
    // Bypass is current state, rather than a permanent descriptive subtitle.
    if node.bypassed {
        painter.text(
            point(inset, 55.0),
            Align2::LEFT_CENTER,
            if width < 90.0 { "BYP" } else { "BYPASSED" },
            crate::panel_skin::print_font(painter.ctx(), 8.0 * zoom),
            header_ink,
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
            crate::panel_skin::print_font(painter.ctx(), 11.0 * zoom),
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

fn jack_target_rect(
    kind: &str,
    center: Pos2,
    zoom: f32,
    minimum_target: f32,
    port_scale: f32,
) -> Rect {
    let side = (29.0 * zoom * port_scale).max(minimum_target);
    // VCF's staggered rows have a 37-point vertical center gap and closely
    // spaced diagonal ports. Keep an 18-screen-point minimum at small zoom,
    // but avoid overlapping the two rows at native scale.
    let height = if kind == "stock_vcf" {
        (36.0 * zoom).max(minimum_target)
    } else {
        side
    };
    Rect::from_center_size(center, vec2(side, height))
}

#[allow(clippy::too_many_arguments)]
fn jack_with_minimum_target(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    center: Pos2,
    kind: &str,
    port: &str,
    id: egui::Id,
    label: &str,
    connected: bool,
    output: bool,
    zoom: f32,
    compact: bool,
    minimum_target: f32,
    port_scale: f32,
    hover_owner: bool,
) -> egui::Response {
    let response = ui.interact(
        jack_target_rect(kind, center, zoom, minimum_target, port_scale),
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
    paint_jack(
        painter, center, kind, port, label, connected, output, zoom, compact, port_scale,
    );
    if hover_owner && response.hovered() {
        painter.circle_stroke(
            center,
            13.0 * zoom * port_scale,
            Stroke::new(1.8 * zoom, Color32::from_rgb(221, 157, 64)),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

fn jack_instruction(output: bool) -> &'static str {
    if output {
        "Occupied OUT: drag or click another OUT to move the top cable source. Cmd/Ctrl starts a new cable to IN. Empty OUT starts a new cable. Cmd/Ctrl+Shift clones the top cable; Shift-click removes it."
    } else {
        "Drag to OUT to connect. Drag a connected input to another input to move its cable. Right-click to unplug; Esc cancels."
    }
}

// A low-zoom jack may own the nearest center while egui hovers an overlapping
// jack response. Keep its standard delayed text tooltip without changing any
// response flags or the focusable click-and-drag widgets.
fn overlapping_jack_tooltip_ready(response: &egui::Response) -> bool {
    if egui::Popup::is_any_open(&response.ctx) {
        return false;
    }
    let delay = response.ctx.style().interaction.tooltip_delay;
    response.ctx.input(|i| {
        i.focused
            && i.pointer.has_pointer()
            && !i.pointer.any_down()
            && i.pointer.is_still()
            && i.smooth_scroll_delta == Vec2::ZERO
            && i.time_since_last_scroll() >= delay
            && i.pointer.time_since_last_movement() >= delay
            && i.pointer.time_since_last_click() >= delay
            && i.pointer.time_since_last_click() >= i.pointer.time_since_last_movement() + 0.1
    })
}

// Material scale is separate from logical cable endpoints and zoom.
#[allow(clippy::too_many_arguments)]
fn paint_jack(
    painter: &egui::Painter,
    center: Pos2,
    kind: &str,
    port: &str,
    label: &str,
    connected: bool,
    output: bool,
    zoom: f32,
    compact: bool,
    port_scale: f32,
) {
    if output {
        painter.circle_filled(center, 13.0 * zoom * port_scale, crate::panel_skin::INK);
        painter.circle_stroke(
            center,
            13.0 * zoom * port_scale,
            Stroke::new(0.6 * zoom, Color32::from_rgb(106, 109, 106)),
        );
    }
    if !compact {
        let caption = jack_caption_placement(kind, port, output);
        let offset = match caption {
            JackCaptionPlacement::Classic => None,
            JackCaptionPlacement::ExistingEnlarged => Some(if output {
                vec2(36.0, 0.0)
            } else {
                vec2(0.0, -25.0)
            }),
            JackCaptionPlacement::Above(distance) => Some(vec2(0.0, -distance)),
            JackCaptionPlacement::Right(distance) => Some(vec2(distance, 0.0)),
            JackCaptionPlacement::Left(distance) => Some(vec2(-distance, 0.0)),
        };
        if let Some(offset) = offset {
            paint_enlarged_port_label(painter, center + offset * zoom, label, output, zoom);
        } else {
            crate::panel_skin::port_label(
                painter,
                center - vec2(0.0, if port_scale > 1.0 { 21.5 } else { 19.0 }) * zoom,
                &port_label(label).to_uppercase(),
                output,
                zoom,
            );
        }
    }
    crate::panel_skin::socket(painter, center, connected, zoom, port_scale);
}

/// Kept at the readable 10.5/9.5-point type used by existing enlarged Stock
/// jacks; placement is selected separately for each panel's port layout.
fn paint_enlarged_port_label(
    painter: &egui::Painter,
    center: Pos2,
    label: &str,
    output: bool,
    zoom: f32,
) {
    let color = if output {
        Color32::from_rgb(245, 242, 233)
    } else {
        INK
    };
    let size = (if output { 9.5 } else { 10.5 }) * zoom;
    let galley = painter.layout_no_wrap(
        port_label(label).to_uppercase(),
        crate::panel_skin::print_font(painter.ctx(), size),
        color,
    );
    let text_rect = Rect::from_center_size(center, galley.size());
    if output {
        let plate = Rect::from_center_size(center, galley.size() + vec2(8.0, 0.0) * zoom);
        painter.rect_filled(plate, 1.5 * zoom, INK);
    }
    painter.galley(text_rect.min, galley, color);
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
    presentation: crate::cable_presentation::CablePresentation,
) {
    paint_cable_curves(painter, from, to, color, zoom, bus, fan, presentation);
    paint_cable_plug(painter, from, color, zoom);
    if plugged {
        paint_cable_plug(painter, to, color, zoom);
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_cable_curves(
    painter: &egui::Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    zoom: f32,
    bus: bool,
    fan: f32,
    presentation: crate::cable_presentation::CablePresentation,
) {
    paint_cable_segment(
        painter,
        cable_curve_points(from, to, zoom, fan, presentation.tension),
        color,
        zoom,
        bus,
        112,
        presentation.opacity,
    );
}

fn cable_curve_points(from: Pos2, to: Pos2, zoom: f32, fan: f32, tension: f32) -> [Pos2; 4] {
    let sag = (from.distance(to) * 0.29 + 38.0 * zoom).clamp(45.0 * zoom, 195.0 * zoom);
    let original = [
        from,
        from + vec2(fan * zoom, sag),
        to + vec2(fan * zoom, sag),
        to,
    ];
    let tension = if tension.is_finite() {
        tension.clamp(0.0, 1.0)
    } else {
        0.5
    };
    // Keep the established default and lower-half looseness unchanged. Taut
    // cables retain a restrained gravity bow instead of becoming rigid rulers.
    if tension == 0.5 {
        return original;
    }
    let straight = [from, from.lerp(to, 1.0 / 3.0), from.lerp(to, 2.0 / 3.0), to];
    if tension < 0.5 {
        let looseness = 2.0 * (1.0 - tension);
        return [
            from,
            straight[1] + (original[1] - straight[1]) * looseness,
            straight[2] + (original[2] - straight[2]) * looseness,
            to,
        ];
    }
    let distance = from.distance(to);
    // Distance-relative bounds prevent tiny or vertical cables folding back.
    // These are presentation controls, not a physical cable simulation.
    let bow = vec2(
        (fan * zoom * 0.12).clamp(-distance * 0.03, distance * 0.03),
        (distance * 0.055).min(16.0 * zoom),
    );
    let amount = (tension - 0.5) * 2.0;
    [
        from,
        original[1].lerp(straight[1] + bow, amount),
        original[2].lerp(straight[2] + bow, amount),
        to,
    ]
}

// Decoration-only hover sampling: no widget, pointer capture or route hit target.
fn cable_curve_near(points: [Pos2; 4], pointer: Pos2, radius: f32) -> bool {
    let mut previous = points[0];
    for step in 1..=32 {
        let t = step as f32 / 32.0;
        let next = cable_curve_prefix(points, t)[3];
        let segment = next - previous;
        let u = if segment.length_sq() > 0.0 {
            ((pointer - previous).dot(segment) / segment.length_sq()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if pointer.distance(previous + segment * u) <= radius {
            return true;
        }
        previous = next;
    }
    false
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

// Native egui log sliders use geometric mapping for these positive ranges.
fn motion_slider_normalize(value: f32, min: f32, max: f32, logarithmic: bool) -> f32 {
    if logarithmic && min > 0.0 && max > min {
        normalize_geometric(value, min, max)
    } else {
        normalize(value, min, max, false)
    }
}

fn paint_motion_badge(
    painter: &egui::Painter,
    center: Pos2,
    label: &str,
    panel: Rect,
    zoom: f32,
    fade: f32,
) {
    let mut job = egui::text::LayoutJob::simple(
        label.to_owned(),
        crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
        Color32::from_rgb(242, 228, 201).gamma_multiply(fade),
        (panel.width() - 22.0 * zoom).max(1.0),
    );
    job.wrap.max_rows = 1;
    let galley = painter.layout_job(job);
    let size = galley.size() + vec2(12.0, 6.0) * zoom;
    let min_x = panel.left() + size.x * 0.5 + 5.0 * zoom;
    let max_x = panel.right() - size.x * 0.5 - 5.0 * zoom;
    let x = if min_x <= max_x {
        center.x.clamp(min_x, max_x)
    } else {
        panel.center().x
    };
    let bounds = Rect::from_center_size(pos2(x, center.y), size);
    painter.rect_filled(
        bounds.translate(vec2(0.0, 1.5) * zoom),
        3.0 * zoom,
        Color32::from_black_alpha((60.0 * fade) as u8),
    );
    painter.rect_filled(
        bounds,
        3.0 * zoom,
        Color32::from_rgb(42, 43, 40).gamma_multiply(fade),
    );
    painter.rect_stroke(
        bounds,
        3.0 * zoom,
        Stroke::new(
            0.65 * zoom,
            Color32::from_rgb(191, 154, 99).gamma_multiply(fade),
        ),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        bounds.center() - galley.size() * 0.5,
        galley,
        Color32::WHITE,
    );
}

/// Screen geometry is resolved anew from port identities every frame. These
/// positions never enter jack hit testing or pending cable operations.
#[derive(Clone, Copy, Debug)]
struct CableMoveGeometry {
    from: Pos2,
    to: Pos2,
    fan: f32,
    output_moving: bool,
    input_moving: bool,
    settle: f32,
}
fn smooth_motion(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}
#[allow(clippy::too_many_arguments)]
fn cable_move_geometry(
    old_from: Pos2,
    old_to: Pos2,
    from: Pos2,
    to: Pos2,
    old_fan: f32,
    fan: f32,
    zoom: f32,
    elapsed: f32,
    output_moving: bool,
    input_moving: bool,
) -> CableMoveGeometry {
    let ms = elapsed.clamp(0.0, 1.0) * 420.0;
    let travel = smooth_motion((ms - 55.0) / 290.0);
    let settle = smooth_motion((ms - 345.0) / 75.0);
    let lift = 9.0 * zoom * smooth_motion(ms / 55.0) * (1.0 - settle);
    let endpoint = |old: Pos2, new: Pos2, moving| {
        if moving {
            old.lerp(new, travel) - vec2(0.0, lift)
        } else {
            new
        }
    };
    CableMoveGeometry {
        from: endpoint(old_from, from, output_moving),
        to: endpoint(old_to, to, input_moving),
        fan: egui::lerp(old_fan..=fan, travel),
        output_moving,
        input_moving,
        settle,
    }
}
#[allow(clippy::too_many_arguments)]
fn paint_moving_cable_curves(
    painter: &egui::Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    zoom: f32,
    bus: bool,
    fan: f32,
    movement: CableMoveGeometry,
    presentation: crate::cable_presentation::CablePresentation,
) {
    // True committed route is always visible. Real caps/lights are painted later
    // in independent endpoint-stack order; decorative plugs carry no telemetry.
    paint_cable_curves(
        painter,
        from,
        to,
        color.gamma_multiply(0.28 + 0.72 * movement.settle),
        zoom,
        bus,
        fan,
        presentation,
    );
    let opacity = presentation.opacity * (1.0 - movement.settle);
    if opacity <= 0.0 {
        return;
    }
    paint_cable_curves(
        painter,
        movement.from,
        movement.to,
        color,
        zoom,
        bus,
        movement.fan,
        crate::cable_presentation::CablePresentation {
            opacity,
            ..presentation
        },
    );
    for (point, moving) in [
        (movement.from, movement.output_moving),
        (movement.to, movement.input_moving),
    ] {
        if moving {
            let mut plug_painter = painter.clone();
            plug_painter.multiply_opacity(opacity);
            paint_cable_plug(&plug_painter, point, color, zoom);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_reduced_cable_move(
    painter: &egui::Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    zoom: f32,
    bus: bool,
    fan: f32,
    fade: f32,
    ends: (bool, bool),
    lighting: &crate::lighting::RoomLighting,
    presentation: crate::cable_presentation::CablePresentation,
) {
    paint_cable_curves(painter, from, to, color, zoom, bus, fan, presentation);
    let start = crate::lighting::next(painter);
    for (point, changed) in [(from, ends.0), (to, ends.1)] {
        if changed {
            painter.circle_stroke(
                point,
                11.5 * zoom,
                Stroke::new(
                    1.4 * zoom,
                    color.gamma_multiply(fade * presentation.opacity),
                ),
            );
        }
    }
    lighting.preserve(painter, start);
}

#[allow(clippy::too_many_arguments)]
fn paint_motion_cable_curves(
    painter: &egui::Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    zoom: f32,
    bus: bool,
    fan: f32,
    phase: Option<crate::cable_animation::Phase>,
    lighting: &crate::lighting::RoomLighting,
    presentation: crate::cable_presentation::CablePresentation,
) {
    let Some(phase) = phase else {
        paint_cable_curves(painter, from, to, color, zoom, bus, fan, presentation);
        return;
    };
    if phase.reduced {
        paint_cable_curves(painter, from, to, color, zoom, bus, fan, presentation);
        let accent_start = crate::lighting::next(painter);
        painter.circle_stroke(
            to,
            11.5 * zoom,
            Stroke::new(1.4 * zoom, color.gamma_multiply(phase.fade)),
        );
        lighting.preserve(painter, accent_start);
        return;
    }
    let prefix_accent = paint_committed_cable_curves(
        painter,
        from,
        to,
        color,
        zoom,
        bus,
        fan,
        Some(phase.progress),
        presentation,
    );
    lighting.preserve(painter, prefix_accent);
    let points = cable_curve_points(from, to, zoom, fan, presentation.tension);
    let tip = cable_curve_prefix(points, phase.progress)[3];
    let ring = if phase.progress >= 1.0 {
        9.0 + 8.0 * crate::construction_motion::ease(phase.settle)
    } else {
        4.0
    };
    let accent_start = crate::lighting::next(painter);
    painter.circle_stroke(
        tip,
        ring * zoom,
        Stroke::new(1.35 * zoom, color.gamma_multiply(phase.fade)),
    );
    if phase.progress < 1.0 {
        painter.circle_filled(tip, 1.8 * zoom, Color32::from_rgb(247, 233, 202));
    }
    lighting.preserve(painter, accent_start);
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
    presentation: crate::cable_presentation::CablePresentation,
) -> usize {
    let Some(progress) = progress else {
        paint_cable_curves(painter, from, to, color, zoom, bus, fan, presentation);
        return crate::lighting::next(painter);
    };
    let points = cable_curve_points(from, to, zoom, fan, presentation.tension);
    // The entire committed route remains visible; this flourish never represents
    // a pending connection or delays its electrical/hit-test state.
    paint_cable_segment(
        painter,
        points,
        color.gamma_multiply(0.28),
        zoom,
        bus,
        32,
        presentation.opacity,
    );
    if progress > 0.0 {
        let prefix = cable_curve_prefix(points, progress);
        paint_cable_segment(painter, prefix, color, zoom, bus, 112, presentation.opacity);
        let accent_start = crate::lighting::next(painter);
        painter.circle_stroke(prefix[3], 4.2 * zoom, Stroke::new(1.2 * zoom, color));
        accent_start
    } else {
        crate::lighting::next(painter)
    }
}

fn paint_cable_segment(
    painter: &egui::Painter,
    points: [Pos2; 4],
    color: Color32,
    zoom: f32,
    bus: bool,
    shadow_alpha: u8,
    opacity: f32,
) {
    if opacity <= 0.0 {
        return;
    }
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
        Color32::from_black_alpha((f32::from(shadow_alpha) * opacity).round() as u8),
    ));
    painter.add(curve(
        points,
        (if bus { 7.8 } else { 6.0 }) * zoom,
        color.gamma_multiply(0.64).gamma_multiply(opacity),
    ));
    painter.add(curve(
        points.map(|point| point - vec2(0.5, 0.8) * zoom),
        (if bus { 3.6 } else { 2.6 }) * zoom,
        color.gamma_multiply(1.13).gamma_multiply(opacity),
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

const SCOPE_TRIGGER_COLOR: Color32 = Color32::from_rgb(205, 125, 190);
#[derive(Clone, Copy, Debug, PartialEq)]
struct ScopeDisplayRange {
    span: f64,
    offset: f64,
}
fn scope_auto_range_enabled(node: &Node) -> bool {
    node.params.get("auto_range").copied().unwrap_or(1.0) >= 0.5
}
fn scope_display_fraction(volts: f32, range: ScopeDisplayRange) -> f64 {
    (f64::from(volts) + range.offset) / range.span
}
fn scope_nice_span(required: f64) -> f64 {
    let decade = 10.0_f64.powf(required.log10().floor());
    for step in [1.0, 2.0, 5.0, 10.0] {
        let span = step * decade;
        if span >= required {
            return span;
        }
    }
    required
}
fn scope_auto_axis_range(
    samples: &[[openpatch_core::ScopePoint; openpatch_core::SCOPE_BINS]; 16],
    channels: u8,
    paired: Option<&[[openpatch_core::ScopePoint; openpatch_core::SCOPE_BINS]; 16]>,
    previous: Option<ScopeDisplayRange>,
) -> ScopeDisplayRange {
    // Zero remains an absolute voltage reference, including for DC. Invalid
    // bins and inactive voices cannot set the display range.
    let mut low = 0.0_f64;
    let mut high = 0.0_f64;
    for (voice, lane) in samples
        .iter()
        .enumerate()
        .take(usize::from(channels.min(16)))
    {
        for (bin, point) in lane.iter().enumerate() {
            if scope_point_valid(*point)
                && paired.is_none_or(|axis| scope_point_valid(axis[voice][bin]))
            {
                low = low.min(f64::from(point.min));
                high = high.max(f64::from(point.max));
            }
        }
    }
    if high == low {
        return ScopeDisplayRange {
            span: 20.0,
            offset: 0.0,
        };
    }
    let candidate = ScopeDisplayRange {
        span: scope_nice_span((high - low) * 1.25),
        offset: -(low + high) * 0.5,
    };
    if let Some(old) = previous
        .filter(|range| range.span.is_finite() && range.span > 0.0 && range.offset.is_finite())
    {
        // Keep a useful calibrated range through small amplitude/DC changes.
        // Expand immediately before clipping; shrink only a full range step.
        let inner_low = -old.offset - old.span * 0.42;
        let inner_high = -old.offset + old.span * 0.42;
        if low >= inner_low && high <= inner_high && candidate.span >= old.span * 0.5 {
            return old;
        }
    }
    candidate
}
fn scope_display_ranges(
    state: &openpatch_core::ScopeSnapshot,
    automatic: bool,
    previous: Option<[ScopeDisplayRange; 2]>,
) -> [ScopeDisplayRange; 2] {
    if automatic {
        let channels = if state.xy {
            [state.x_channels.min(state.y_channels); 2]
        } else {
            [state.x_channels, state.y_channels]
        };
        [
            scope_auto_axis_range(
                &state.x,
                channels[0],
                state.xy.then_some(&state.y),
                previous.map(|ranges| ranges[0]),
            ),
            scope_auto_axis_range(
                &state.y,
                channels[1],
                state.xy.then_some(&state.x),
                previous.map(|ranges| ranges[1]),
            ),
        ]
    } else {
        [
            (state.x_scale, state.x_offset),
            (state.y_scale, state.y_offset),
        ]
        .map(|(scale, offset)| ScopeDisplayRange {
            span: f64::from(scope_volts_per_screen(scale.min(8))),
            offset: if offset.is_finite() {
                f64::from(offset)
            } else {
                0.0
            },
        })
    }
}
fn scope_calibrated_voltage(volts: f64) -> String {
    let (value, unit) = if volts == 0.0 || volts.abs() >= 1.0 {
        (volts, "V")
    } else if volts.abs() >= 0.001 {
        (volts * 1000.0, "mV")
    } else {
        (volts * 1_000_000.0, "µV")
    };
    format!("{} {unit}", scope_measurement_number(value))
}
fn scope_range_label(automatic: bool, ranges: [ScopeDisplayRange; 2]) -> String {
    format!(
        "{} X {}/screen offset {}  |  Y {}/screen offset {}",
        if automatic { "AUTO" } else { "MANUAL" },
        scope_calibrated_voltage(ranges[0].span),
        scope_calibrated_voltage(ranges[0].offset),
        scope_calibrated_voltage(ranges[1].span),
        scope_calibrated_voltage(ranges[1].offset),
    )
}
fn scope_parameter_action(node: &Node, action: RackAction, revision: u64) -> RackAction {
    match action {
        RackAction::Command("set_parameter", mut args) => {
            let manual = args["parameter"].as_str().is_some_and(|name| {
                matches!(name, "x_scale" | "y_scale" | "x_offset" | "y_offset")
            });
            if scope_auto_range_enabled(node) && manual {
                let args_object = args.as_object_mut().unwrap();
                args_object.remove("expected_revision");
                let history_group = args_object.remove("history_group");
                let primary = args["parameter"].clone();
                let mut transaction = json!({
                    "expected_revision":revision,
                    "edits":[
                        {"tool":"set_parameter","arguments":args},
                        {"tool":"set_parameter","arguments":{"id":node.id,"parameter":"auto_range","value":0}}
                    ]
                });
                if let Some(group) = history_group {
                    transaction["history_group"] = group;
                    transaction["history_parameter"] = json!({"id":node.id,"parameter":primary});
                }
                RackAction::Command("apply_edits", transaction)
            } else {
                args["expected_revision"] = json!(revision);
                RackAction::Command("set_parameter", args)
            }
        }
        other => other,
    }
}
fn scope_x_trigger_guide(state: &openpatch_core::ScopeSnapshot) -> bool {
    state.trigger_enabled && !state.xy && state.trigger_input_channels == 0 && state.x_channels > 0
}
fn scope_trigger_status(state: &openpatch_core::ScopeSnapshot) -> String {
    if state.xy {
        "XY · continuous capture · trigger ignored".into()
    } else if !state.trigger_enabled {
        "FREE RUN · trigger off".into()
    } else {
        format!(
            "{} trigger · level {} V{}",
            if state.trigger_input_channels > 0 {
                "EXT"
            } else {
                "X"
            },
            scope_measurement_number(f64::from(state.threshold)),
            if state.waiting {
                " · WAITING FOR TRIGGER"
            } else {
                ""
            }
        )
    }
}
const SCOPE_SCALE_CHOICES: [&str; 9] = [
    "20 V/screen",
    "10 V/screen",
    "5 V/screen",
    "2.5 V/screen",
    "1.25 V/screen",
    "0.625 V/screen",
    "0.3125 V/screen",
    "0.15625 V/screen",
    "0.078125 V/screen",
];
fn scope_volts_per_screen(scale: u8) -> f32 {
    20.0 / 2.0_f32.powi(i32::from(scale))
}
fn scope_extrema_label(axis: &str, extrema: Option<openpatch_core::ScopeExtrema>) -> String {
    match extrema {
        Some(e) => format!(
            "{axis}  MIN {} V   MAX {} V   P-P {} V",
            scope_measurement_number(e.min),
            scope_measurement_number(e.max),
            scope_measurement_number(e.peak_to_peak)
        ),
        None => format!("{axis}  MIN —   MAX —   P-P —   NO CAPTURE"),
    }
}
fn scope_measurement_number(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    if value.abs() >= 100_000.0 || value.abs() < 0.000001 {
        format!("{value:.3e}")
    } else {
        let text = format!("{value:.6}");
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}
const SCOPE_CONTROLS: [(&str, &str, &str); 9] = [
    ("x_scale", "X RANGE", ""),
    ("y_scale", "Y RANGE", ""),
    ("time", "WINDOW", " s"),
    ("xy", "XY MODE", ""),
    ("x_offset", "X OFFSET", " V"),
    ("y_offset", "Y OFFSET", " V"),
    ("threshold", "TRIGGER LEVEL", " V"),
    ("trigger_enabled", "TRIGGER", ""),
    ("auto_range", "AUTO RANGE", ""),
];
fn scope_control_rect(index: usize) -> Rect {
    if index == 8 {
        return Rect::from_min_size(pos2(320.0, 353.0), vec2(105.0, 22.0));
    }
    Rect::from_min_size(
        pos2(
            30.0 + (index % 4) as f32 * 145.0,
            274.0 + (index / 4) as f32 * 40.0,
        ),
        vec2(130.0, 22.0),
    )
}
fn scope_voice_color(base: Color32, voice: usize) -> Color32 {
    let factor = 0.55 + 0.45 * (voice % 4) as f32 / 3.0;
    let mix = (voice / 4) as u8 * 18;
    Color32::from_rgb(
        ((base.r() as f32 * factor) as u8).saturating_add(mix),
        ((base.g() as f32 * factor) as u8).saturating_add(mix),
        ((base.b() as f32 * factor) as u8).saturating_add(mix),
    )
}

fn scope_midpoint(point: openpatch_core::ScopePoint) -> f32 {
    ((f64::from(point.min) + f64::from(point.max)) * 0.5) as f32
}
fn scope_position_finite(point: Pos2) -> bool {
    point.x.is_finite() && point.y.is_finite()
}
fn scope_point_valid(point: openpatch_core::ScopePoint) -> bool {
    point.min.is_finite() && point.max.is_finite() && point.min <= point.max
}
fn scope_xy_order(buffer_index: usize) -> impl Iterator<Item = usize> {
    (0..openpatch_core::SCOPE_BINS).map(move |index| {
        (buffer_index % openpatch_core::SCOPE_BINS + index) % openpatch_core::SCOPE_BINS
    })
}

#[cfg(test)]
mod tests {
    mod scope_ui_tests {
        include!("newscope_ui_tests.rs");
    }
    mod scope_visibility_tests {
        include!("scope_visibility_tests.rs");
    }
    mod viz_ui_tests {
        include!("viz_ui_tests.rs");
    }
    mod sum_ui_tests {
        include!("sum_ui_tests.rs");
    }
    mod resonator_ui_tests {
        include!("resonator_ui_tests.rs");
    }
    mod shift_register_ui_tests {
        include!("shift_register_ui_tests.rs");
    }
    mod numeric_activation_ui_tests {
        include!("numeric_activation_ui_tests.rs");
    }
    mod random_ui_tests {
        include!("random_ui_tests.rs");
    }
    mod random_values_ui_tests {
        include!("random_values_ui_tests.rs");
    }
    mod noise_ui_tests {
        include!("noise_ui_tests.rs");
    }
    mod lighting_ui_tests {
        include!("lighting_ui_tests.rs");
    }
    mod port_signal_ui_tests {
        include!("port_signal_ui_tests.rs");
    }
    mod cable_presentation_ui_tests {
        include!("cable_presentation_ui_tests.rs");
    }
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
            80,
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
            // Header typography must fit the panel at each real library/rack
            // scale; clipping would conceal a long or narrow module identity.
            let heading = thumbnail
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text)
                        if text.galley.text() == panel_heading(kind, panel_width(kind))
                            && text.pos.y < rect.top() + 65.0 * scale =>
                    {
                        Some(Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                })
                .expect("shared preview draws a module heading");
            assert!(
                rect.expand(0.1).contains_rect(heading),
                "{kind} at {scale}: heading {heading:?} clips outside {rect:?}"
            );
            assert!(
                heading.bottom() <= rect.top() + 65.0 * scale,
                "{kind} at {scale}: heading overlaps the controls below the header"
            );
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
            "macro_voice",
            "mix6",
            "stock_mix",
            "stock_vca_mix",
            "multi_lfo",
            "stock_lfo",
            "stock_vco",
            "wavetable_vco",
            "wavetable_lfo",
            "poly_viz",
            "score_sequencer",
            "drum_voice",
            "svf_filter",
            "stereo_delay",
            "stereo_reverb",
            "sidechain_compressor",
            "audio_stereo",
            "modal_resonator",
            "colored_noise",
            "stock_noise",
            "random_values",
            "sh_asr",
        ] {
            let module = catalog["modules"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["kind"] == kind)
                .unwrap();
            for scale in [0.25, 0.42, 0.85, 1.0] {
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
                        if matches!(
                            (kind, name),
                            ("mix6" | "stock_mix", "average" | "invert")
                                | ("multi_lfo", "unipolar" | "invert")
                                | ("stock_lfo", "offset" | "invert")
                                | ("stock_vco", "linear_fm" | "soft_sync")
                        ) {
                            continue;
                        }
                        assert!(
                            texts.iter().any(|(text, _)| text
                                == if kind == "stock_vco" {
                                    stock_vco_parameter_label(name)
                                } else if kind == "stock_lfo" {
                                    stock_lfo_parameter_label(name, false)
                                } else if is_wavetable(kind) {
                                    wavetable_parameter_label(name)
                                } else {
                                    parameter_label(name)
                                }
                                || (kind == "sh_asr"
                                    && name == "random"
                                    && text.starts_with("RND "))
                                || (kind == "stock_vca_mix"
                                    && match name {
                                        "level1" | "level2" | "level3" | "level4" =>
                                            text == "LEVEL",
                                        "mix_level" => text == "MIX LEVEL",
                                        "channel_exp" => text.starts_with("CHANNEL CV  "),
                                        "mix_exp" => text.starts_with("MIX CV  "),
                                        _ => false,
                                    })),
                            "{kind}: missing parameter label {name}"
                        );
                    }
                    assert!(
                        texts
                            .iter()
                            .all(|(text, _)| !text.eq_ignore_ascii_case("openpatch")),
                        "{kind} at {scale}: module branding must be absent"
                    );
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
            let pressed = position;
            frame(
                &ctx,
                &mut rack,
                &patch,
                time,
                vec![egui::Event::PointerMoved(pressed), button(pressed, true)],
            );
            // A rotary uses 150 logical points of relative vertical travel.
            // One ninth of that range moves the nine-octave frequency one octave.
            let moved = position
                - vec2(
                    0.0,
                    if name == "frequency" {
                        150.0 / 9.0
                    } else {
                        15.0
                    },
                );
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
                    "one ninth of relative rotary travel must double Hz"
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
            for (time, pressed) in [(1.0, true), (1.04, false)] {
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
                + port_position(&node.kind, input, index, ports.len(), module_width(node))
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
                        || p.x >= q.x + module_width(node)
                );
            }
        }
        rack.apply_duplicate_placement(placement, true);
        assert_eq!(rack.selected_ids(), vec!["small-copy", "wide-copy"]);
        let mut updated = patch;
        updated.nodes.push(Node {
            content: None,
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
            for (time, pressed) in [(1.0, true), (1.04, false)] {
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
        for (index, channel) in (0..6).flat_map(|index| (0..2).map(move |channel| (index, channel)))
        {
            let jack = port_position("gates", false, index, 6, 240.0);
            let point = pos2(
                24.0 + jack.x + if channel == 0 { -3.5 } else { 3.5 },
                12.0 + jack.y + 17.0,
            );
            assert_eq!(
                output.shapes.iter().any(|shape| {
                    match &shape.shape {
                        egui::epaint::Shape::Circle(circle) if circle.center == point => {
                            circle.fill.a() == 0
                                && if channel == 0 {
                                    circle.fill.r() > circle.fill.g()
                                        && circle.fill.g() > circle.fill.b()
                                } else {
                                    circle.fill.b() > circle.fill.g()
                                        && circle.fill.g() > circle.fill.r()
                                }
                        }
                        _ => false,
                    }
                }),
                matches!((index, channel), (1, 1) | (5, 0)),
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
            for (time, pressed) in [(1.0, true), (1.04, false)] {
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
                content: None,
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
                    content: None,
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
                    content: None,
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
                content: None,
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
            for (time, pressed) in [(1.0, true), (1.04, false)] {
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
                    content: None,
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
                content: None,
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
                content: None,
                score: None,
                bypassed: false,
                id: "test".into(),
                kind: module["kind"].as_str().unwrap().into(),
                params: Default::default(),
            };
            assert_eq!(
                ordered_parameters(&node, Some(module)).len(),
                module["params"]
                    .as_object()
                    .unwrap()
                    .values()
                    .filter(|spec| spec["hidden"] != true)
                    .count(),
                "{} public controls were hidden",
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
            content: None,
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
    fn legacy_vco_waveform_uses_exact_idempotent_choices() {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        // Version-one patches retain the same raw Saw/Pulse domain.
        let mut patch: Patch = serde_json::from_value(json!({"version":1,
            "nodes":[{"id":"vco","kind":"analog_vco"}]}))
        .unwrap();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let pulse = pos2(99.75, 264.0);
        frame(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(pulse), button(pulse, true)],
        );
        let edit = frame(&ctx, &mut rack, &patch, 1.05, vec![button(pulse, false)]);
        assert!(
            matches!(edit, Some(RackAction::Command("set_parameter", ref args))
            if args["id"] == "vco" && args["parameter"] == "waveform" && args["value"] == 1.0)
        );
        patch.nodes[0].params.insert("waveform".into(), 1.0);
        frame(&ctx, &mut rack, &patch, 1.1, vec![button(pulse, true)]);
        assert!(
            frame(&ctx, &mut rack, &patch, 1.15, vec![button(pulse, false)]).is_none(),
            "a second click must keep Pulse rather than resetting to Saw"
        );
        let edit = frame(
            &ctx,
            &mut rack,
            &patch,
            1.2,
            vec![egui::Event::Key {
                key: egui::Key::ArrowLeft,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            matches!(edit, Some(RackAction::Command("set_parameter", ref args))
            if args["parameter"] == "waveform" && args["value"] == 0.0)
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
    fn agent_rows_append_in_graph_order_without_moving_other_panels() {
        let mut patch = Patch::new_v2();
        for (id, kind) in [
            ("old", "constant"),
            ("clock", "gate"),
            ("lfo", "multi_lfo"),
            ("voice", "macro_voice"),
            ("skip", "constant"),
        ] {
            patch.nodes.push(Node {
                id: id.into(),
                kind: kind.into(),
                params: BTreeMap::new(),
                bypassed: false,
                content: None,
                score: None,
            });
        }
        let mut rack = Rack::default();
        rack.positions.insert("old".into(), pos2(0.0, ROW));
        let revision = rack.layout_revision();
        rack.place_in_rows(
            &patch,
            &BTreeMap::from([
                ("lfo".into(), 1),
                ("clock".into(), 1),
                ("voice".into(), 2),
                ("absent".into(), 0),
            ]),
        );
        let width = |id: &str| module_width(patch.nodes.iter().find(|n| n.id == id).unwrap());
        assert_eq!(rack.positions["old"], pos2(0.0, ROW));
        assert_eq!(rack.positions["clock"], pos2(width("old"), ROW));
        assert_eq!(
            rack.positions["lfo"],
            pos2(width("old") + width("clock"), ROW)
        );
        assert_eq!(rack.positions["voice"], pos2(0.0, 2.0 * ROW));
        assert!(!rack.positions.contains_key("skip"));
        assert!(!rack.positions.contains_key("absent"));
        assert_ne!(rack.layout_revision(), revision);
    }

    #[test]
    fn agent_rows_ignore_stale_positions_for_recreated_ids_and_append_default_and_explicit_row_zero_in_graph_order(
    ) {
        let mut patch = Patch::new_v2();
        for (id, kind) in [
            ("survivor", "constant"),
            ("recreated-b", "gate"),
            ("default-zero", "constant"),
            ("recreated-a", "multi_lfo"),
            ("explicit-zero", "macro_voice"),
        ] {
            patch.nodes.push(Node {
                id: id.into(),
                kind: kind.into(),
                params: BTreeMap::new(),
                bypassed: false,
                content: None,
                score: None,
            });
        }
        let mut rack = Rack::default();
        rack.positions.insert("survivor".into(), pos2(45.0, 0.0));
        rack.positions
            .insert("recreated-b".into(), pos2(1800.0, 0.0));
        rack.positions
            .insert("recreated-a".into(), pos2(2700.0, 0.0));
        let rows = BTreeMap::from([
            ("recreated-b".into(), 0),
            ("default-zero".into(), 0),
            ("recreated-a".into(), 0),
            ("explicit-zero".into(), 0),
        ]);
        rack.place_in_rows(&patch, &rows);
        let width = |id: &str| module_width(patch.nodes.iter().find(|node| node.id == id).unwrap());
        assert_eq!(rack.positions["survivor"], pos2(45.0, 0.0));
        let mut next_x = 45.0 + width("survivor");
        for id in [
            "recreated-b",
            "default-zero",
            "recreated-a",
            "explicit-zero",
        ] {
            assert_eq!(rack.positions[id], pos2(next_x, 0.0), "{id}");
            next_x += width(id);
        }
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
                    vec2(module_width(node), PANEL_HEIGHT),
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
    fn expression_legacy_value_click_preserves_display_unit_conversions() {
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
            for (time, pressed) in [(1.0, true), (1.04, false)] {
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
                    .expect("single click opens value editor")
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
            palette_selection: None,
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
    mod palette_gesture_tests {
        include!("palette_gesture_tests.rs");
    }
    mod cable_guidance_tests {
        include!("cable_guidance_tests.rs");
    }
    mod annotation_ui_tests {
        include!("annotation_ui_tests.rs");
    }
    mod midi_gate_ui_tests {
        include!("midi_gate_ui_tests.rs");
    }
    mod midi_cc_ui_tests {
        include!("midi_cc_ui_tests.rs");
    }
    mod stock_mix_ui_tests {
        include!("stock_mix_ui_tests.rs");
    }
    mod stock_mid_side_ui_tests {
        include!("stock_mid_side_ui_tests.rs");
    }
    mod stock_quantizer_ui_tests {
        include!("stock_quantizer_ui_tests.rs");
    }
    mod stock_octave_ui_tests {
        include!("stock_octave_ui_tests.rs");
    }
    mod stock_seq3_ui_tests {
        include!("stock_seq3_ui_tests.rs");
    }
    mod stock_vco_ui_tests {
        include!("stock_vco_ui_tests.rs");
    }
    mod stock_lfo_ui_tests {
        include!("stock_lfo_ui_tests.rs");
    }
    mod split_ui_tests {
        include!("split_ui_tests.rs");
    }
    mod merge_ui_tests {
        include!("merge_ui_tests.rs");
    }
    mod stock_vca_ui_tests {
        include!("stock_vca_ui_tests.rs");
    }
    mod stock_vca_mix_ui_tests {
        include!("stock_vca_mix_ui_tests.rs");
    }
    mod stock_process_ui_tests {
        include!("stock_process_ui_tests.rs");
    }
    mod stock_noise_ui_tests {
        include!("stock_noise_ui_tests.rs");
    }
    mod studio_mixer_ui_tests {
        include!("studio_mixer_ui_tests.rs");
    }
    mod cable_autopan_tests {
        include!("cable_autopan_tests.rs");
    }
    mod generic_choice_ui_tests {
        include!("generic_choice_ui_tests.rs");
    }
    mod generic_fader_ui_tests {
        include!("generic_fader_ui_tests.rs");
    }
    mod generic_fader_geometry_tests {
        include!("generic_fader_geometry_tests.rs");
    }
    mod large_socket_ui_tests {
        include!("large_socket_ui_tests.rs");
    }

    mod stock_delay_ui_tests {
        include!("stock_delay_ui_tests.rs");
    }
    mod wavetable_ui_tests {
        include!("wavetable_ui_tests.rs");
    }
    mod stock_vcf_ui_tests {
        include!("stock_vcf_ui_tests.rs");
    }
    mod adsr_eg_ui_tests {
        include!("adsr_eg_ui_tests.rs");
    }
    mod precision_editor_tests {
        include!("precision_editor_tests.rs");
    }
    include!("cable_menu_tests.rs");
    include!("cable_animation_render_tests.rs");
    include!("cable_move_render_tests.rs");
    mod construction_motion_render_tests {
        include!("construction_motion_render_tests.rs");
    }
    mod agent_follow_tests {
        include!("agent_follow_tests.rs");
    }
}
