use crate::{
    audio::{Audio, AudioTimingSnapshot},
    cable_presentation::CablePresentation,
    chrome::{icon_button, Icon},
    document_state::{apply_document_guarded, new_document_guarded, DocumentState},
    keyboard::Keyboard,
    library::{LibraryAction, LibraryDrag, ModuleLibrary},
    midi::{MidiInputManager, MidiPortInfo},
    project::{Document, FileToken, Store},
    rack::{
        duplicate_all_supported, parameter_label, parameter_value, title as module_title,
        DuplicateCableMode, ModuleMove, PresetMode, Rack, RackAction,
    },
    scope::Scope,
    visuals::{Visualizer, DEFAULT_SHADER},
};
use eframe::{
    egui::{self, Color32, RichText, Vec2},
    glow,
};
use openpatch::control::{execute_shared, SharedSession};
use openpatch_core::{catalog, MidiEvent, Patch};
include!("toolbar.rs");
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    time::{Duration, Instant},
};

const CHROME: Color32 = Color32::from_rgb(32, 35, 35);
const TEXT: Color32 = Color32::from_rgb(235, 233, 226);
const MUTED: Color32 = Color32::from_rgb(164, 167, 162);
const ACCENT: Color32 = Color32::from_rgb(229, 186, 106);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnalysisTab {
    Scope,
    Visuals,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TimingStatus {
    Unavailable,
    Stopped,
    ResetPending,
    Waiting,
    Contended,
    Ready,
}

fn timing_status(snapshot: Option<&AudioTimingSnapshot>) -> TimingStatus {
    let Some(snapshot) = snapshot else {
        return TimingStatus::Unavailable;
    };
    if snapshot.closed {
        TimingStatus::Stopped
    } else if snapshot.reset_pending {
        TimingStatus::ResetPending
    } else if !snapshot.coherent {
        TimingStatus::Contended
    } else if !snapshot.available
        || snapshot.callbacks == 0
        || snapshot.sample_rate == 0
        || snapshot.frames == 0
        || snapshot.utilization.is_none()
    {
        TimingStatus::Waiting
    } else {
        TimingStatus::Ready
    }
}

fn timing_footer(snapshot: Option<&AudioTimingSnapshot>) -> String {
    match timing_status(snapshot) {
        TimingStatus::Ready => {
            format!("DSP {:.1}%", snapshot.unwrap().utilization.unwrap() * 100.0)
        }
        TimingStatus::ResetPending => "DSP reset…".into(),
        _ => "DSP —".into(),
    }
}

fn timing_explanation(status: TimingStatus) -> &'static str {
    match status {
        TimingStatus::Unavailable => "Audio is unavailable. No callback timing has been measured.",
        TimingStatus::Stopped => {
            "Audio stream stopped. Start or reconnect audio for fresh measurements."
        }
        TimingStatus::ResetPending => {
            "Reset requested. Waiting for the next audio callback to start a fresh interval."
        }
        TimingStatus::Waiting => "Waiting for a measured audio callback with a valid frame budget.",
        TimingStatus::Contended => {
            "A coherent timing snapshot is unavailable. Retrying on the next frame."
        }
        TimingStatus::Ready => {
            "Measured audio callback processing time relative to its frame budget."
        }
    }
}

fn weighted_mean_utilization(snapshot: &AudioTimingSnapshot) -> Option<f64> {
    (snapshot.sample_rate > 0 && snapshot.total_frames > 0).then(|| {
        snapshot.total_duration_ns as f64 * snapshot.sample_rate as f64
            / (snapshot.total_frames as f64 * 1_000_000_000.0)
    })
}

#[derive(Clone, Copy, PartialEq)]
enum FileMode {
    Open,
    Save,
    WavetableImport,
    WavetableExport,
}

struct WavetableFileTarget {
    id: String,
    revision: u64,
    current_points: u32,
    audio: bool,
    // Empty means automatic WAV hint or the module's current geometry.
    wave_points: String,
}

struct WavetableImportJob {
    result: Receiver<Result<Value, String>>,
    allowed: Arc<AtomicBool>,
    filename: String,
}

impl Drop for WavetableImportJob {
    fn drop(&mut self) {
        // App teardown also revokes an import still preparing off thread.
        self.allowed.store(false, Ordering::Release);
    }
}

fn wavetable_file_extension(filename: &str) -> String {
    Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn wavetable_filename_with_extension(filename: &str, extension: &str) -> String {
    let mut filename = PathBuf::from(if filename.is_empty() {
        "wavetable"
    } else {
        filename
    });
    filename.set_extension(extension);
    filename.to_string_lossy().into_owned()
}

enum PendingAction {
    NewTemplate,
    Replace(Patch, String),
    Load(String),
    Recover(String),
    Close,
    // Retain explicit Discard permission across a busy session lock.
    DiscardClose,
}

struct TemplateSave {
    document: Document,
    token: Option<FileToken>,
}

struct PresetDialog {
    id: String,
    kind: String,
    revision: u64,
    mode: PresetMode,
    name: String,
    search: String,
    query: String,
    capture: Option<Value>,
    source: String,
    frame: Option<u64>,
    entries: Vec<Value>,
    offset: usize,
    total: usize,
    preview: Option<Value>,
    preview_filename: Option<String>,
    status: String,
    pending: bool,
    await_release: bool,
    focus_name: bool,
}

impl PresetDialog {
    fn new(id: String, kind: String, revision: u64, mode: PresetMode) -> Self {
        Self {
            id,
            kind,
            revision,
            mode,
            name: String::new(),
            search: String::new(),
            query: String::new(),
            capture: None,
            source: String::new(),
            frame: None,
            entries: Vec::new(),
            offset: 0,
            total: 0,
            preview: None,
            preview_filename: None,
            status: String::new(),
            pending: true,
            await_release: true,
            focus_name: true,
        }
    }

    fn target_status(&self, patch: &Patch, revision: u64) -> Result<(), String> {
        let Some(node) = patch.nodes.iter().find(|node| node.id == self.id) else {
            return Err(
                "This module was removed. Close the preset window and choose another module."
                    .into(),
            );
        };
        if node.kind != self.kind {
            return Err(
                "This module changed type. Close the preset window and choose it again.".into(),
            );
        }
        if revision != self.revision {
            return Err("The patch changed since this preset window opened. Refresh the target before continuing.".into());
        }
        Ok(())
    }
}

enum PresetIntent {
    None,
    Cancel,
    Retry,
    Search,
    RefreshTarget,
    Previous,
    Next,
    Read(String),
    Save,
    Apply,
}

fn preset_controls(ui: &mut egui::Ui, preset: &Value, catalog: &Value) {
    ui.label("Controls in this preset");
    let module = catalog["modules"].as_array().and_then(|modules| {
        modules
            .iter()
            .find(|module| module["kind"] == preset["kind"])
    });
    egui::ScrollArea::vertical()
        .id_salt("preset-controls-scroll")
        .max_height(170.0)
        .show(ui, |ui| {
            egui::Grid::new("preset-controls")
                .striped(true)
                .show(ui, |ui| {
                    if let Some(params) = preset["params"].as_object() {
                        for (name, value) in params {
                            let spec = module
                                .and_then(|module| module["params"].get(name))
                                .unwrap_or(&Value::Null);
                            if spec["hidden"] == true {
                                continue;
                            }
                            ui.label(if name == "running" {
                                "RUN"
                            } else {
                                parameter_label(name)
                            });
                            ui.label(preset_control_value(name, value, spec));
                            ui.end_row();
                        }
                    }
                    if let Some(bypassed) = preset["bypassed"].as_bool() {
                        ui.label("Bypass");
                        ui.label(if bypassed { "On" } else { "Off" });
                        ui.end_row();
                    }
                });
        });
    if preset["content"]["type"] == "wavetable" {
        // Review only the table identity and geometry. The portable preset
        // keeps encoded sample data for Save/Apply without laying it out.
        ui.label(format!(
            "Embedded wavetable: {}",
            preset["content"]["name"].as_str().unwrap_or("Unnamed")
        ));
        if let Some(points) = preset["content"]["wave_points"].as_u64() {
            ui.label(format!("{points} points per wave"));
        }
    }
}

fn preset_control_value(name: &str, value: &Value, spec: &Value) -> String {
    if name == "running" {
        return if value.as_f64().unwrap_or(0.0) >= 0.5 {
            "On"
        } else {
            "Off"
        }
        .into();
    }
    value.as_f64().map_or_else(
        || value.to_string(),
        |raw| parameter_value(name, raw as f32, spec),
    )
}

fn preset_filename(name: &str) -> Option<String> {
    let stem = name.trim().trim_end_matches(".opreset");
    let filename = format!("{stem}.opreset");
    (!stem.is_empty()
        && !stem.starts_with('.')
        && filename.len() <= 120
        && stem
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte)))
    .then_some(filename)
}

pub struct OpenPatch {
    shared: SharedSession,
    patch: Patch,
    revision: u64,
    state_id: u64,
    history_layouts: BTreeMap<u64, Value>,
    audio_revision: Option<u64>,
    live_events_attach_pending: bool,
    audio: Option<Audio>,
    device: String,
    output_device: Option<String>,
    output_devices: Vec<String>,
    midi: Option<MidiInputManager>,
    midi_ports: Vec<MidiPortInfo>,
    midi_device: Option<String>,
    midi_epoch: u64,
    settings_open: bool,
    settings_status: String,
    audio_performance_open: bool,
    smoke_diagnostics: Option<AudioTimingSnapshot>,
    keyboard: Keyboard,
    keyboard_open: bool,
    rack: Rack,
    cable_palette: crate::cable_palette::Palette,
    cable_palette_ui: crate::cable_palette_ui::Panel,
    visualizer: Option<Visualizer>,
    gl: Option<std::sync::Arc<glow::Context>>,
    shader: String,
    shader_open: bool,
    shader_status: String,
    analysis_tab: Option<AnalysisTab>,
    visual_mode: i32,
    scope: Scope,
    volume: f32,
    status: String,
    filename: String,
    title: String,
    files: Vec<String>,
    workspace: String,
    file_mode: Option<FileMode>,
    file_focus: bool,
    wavetable_file_target: Option<WavetableFileTarget>,
    wavetable_import: Option<WavetableImportJob>,
    browser_open: bool,
    library: ModuleLibrary,
    preset_dialog: Option<PresetDialog>,
    template_save: Option<TemplateSave>,
    catalog: Value,
    start: Instant,
    mcp_session: crate::mcp_session_ui::Panel,
    construction: crate::construction_ui::Panel,
    smoke_motion_origin: Option<Instant>,
    documents: DocumentState,
    projects: Option<Store>,
    active_file: Option<String>,
    active_token: Option<FileToken>,
    after_save: Option<PendingAction>,
    deferred_action: Option<PendingAction>,
    deferred_manual_release: Option<(u64, crate::audio::ManualReleaseTicket)>,
    autosave_needed: bool,
    recovery_names: Vec<String>,
    recovery_details: BTreeMap<String, String>,
    recovery_open: bool,
    restored_recovery: Option<String>,
    autosaved_document: Option<Document>,
    last_document_change: Instant,
    last_autosave_attempt: Instant,
    can_undo: bool,
    can_redo: bool,
    confirm_engine_upgrade: bool,
    confirm_discard: Option<PendingAction>,
    allow_close: bool,
    smoke_test: Option<(PathBuf, u32, usize)>,
}
impl OpenPatch {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn set_local_mcp(&mut self, server: Result<openpatch::local_mcp::LocalServer, String>) {
        self.mcp_session.set_server(server);
    }

    fn mcp_status(&mut self, ui: &mut egui::Ui) {
        self.mcp_session.status(ui);
    }

    pub fn new(
        cc: &eframe::CreationContext<'_>,
        shared: SharedSession,
        mcp: bool,
        smoke_test: Option<PathBuf>,
        startup: crate::startup::Startup,
    ) -> Self {
        crate::panel_skin::install_fonts(&cc.egui_ctx);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = CHROME;
        style.visuals.window_fill = Color32::from_rgb(42, 45, 44);
        style.visuals.extreme_bg_color = Color32::from_rgb(21, 24, 24);
        style.visuals.override_text_color = Some(TEXT);
        style.visuals.selection.bg_fill = Color32::from_rgb(106, 82, 43);
        style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, TEXT);
        style.visuals.widgets.noninteractive.bg_stroke =
            egui::Stroke::new(1.0_f32, Color32::from_rgb(65, 69, 66));
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(48, 52, 51);
        style.visuals.widgets.inactive.bg_stroke =
            egui::Stroke::new(1.0_f32, Color32::from_rgb(76, 80, 76));
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(64, 68, 65);
        style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(94, 74, 43);
        style.visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
        style.visuals.widgets.inactive.weak_bg_fill = style.visuals.widgets.inactive.bg_fill;
        style.visuals.widgets.hovered.weak_bg_fill = style.visuals.widgets.hovered.bg_fill;
        style.visuals.widgets.active.weak_bg_fill = style.visuals.widgets.active.bg_fill;
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(10.0, 5.0);
        cc.egui_ctx.set_style(style);
        let (patch, wave_banks, revision, state_id, workspace) = {
            let mut s = shared.lock().unwrap();
            // A fresh engine starts from this exact document, with no old
            // signal history to which earlier host edges could apply.
            s.discard_audio_lifecycle_through_current();
            (
                s.patch().clone(),
                s.prepared_wave_banks(),
                s.revision(),
                s.state_id(),
                s.workspace().display().to_string(),
            )
        };
        let preferences = std::fs::read(PathBuf::from(&workspace).join(".host-settings.json"))
            .ok()
            .filter(|bytes| bytes.len() <= 16_384)
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .unwrap_or(Value::Null);
        let output_device = preferences["output_device"].as_str().map(str::to_owned);
        let midi_device = preferences["midi_device"].as_str().map(str::to_owned);
        let cable_presentation = CablePresentation::from_settings(&preferences);
        let (mut audio, device) = Self::open_audio(
            &patch,
            wave_banks,
            output_device.as_deref(),
            cable_presentation.plug_brightness > 0.0,
        );
        let midi_epoch = audio.as_ref().map_or(0, Audio::midi_epoch);
        let midi = audio
            .as_mut()
            .and_then(Audio::take_midi_sender)
            .map(MidiInputManager::new);
        let (visualizer, shader_status) = match cc.gl.as_ref().map(|gl| Visualizer::new(gl)) {
            Some(Ok(v)) => (Some(v), "GLSL 330 ready".into()),
            Some(Err(e)) => (None, e),
            None => (None, "OpenGL unavailable".into()),
        };
        let audio_revision = None;
        let volume = audio.as_ref().map_or(0.65, Audio::volume);
        let (rack, documents) = Self::startup_view(&startup, &patch, revision);
        let history_layouts = BTreeMap::from([
            (startup.state_id, startup.document.layout.clone()),
            (state_id, rack.layout()),
        ]);
        let recovery_names = startup
            .store
            .as_ref()
            .map_or_else(Vec::new, Store::recoveries);
        let recovery_open = startup.offer_recovery;
        let filename = startup.active_file.clone().unwrap_or_else(|| {
            if startup.imported {
                format!("{}.openpatch", startup.title)
            } else {
                "my-patch.openpatch".into()
            }
        });
        let mut app = Self {
            shared,
            patch,
            revision,
            state_id,
            history_layouts,
            audio_revision,
            live_events_attach_pending: false,
            audio,
            device,
            output_device,
            output_devices: Vec::new(),
            midi,
            midi_ports: Vec::new(),
            midi_device,
            midi_epoch,
            settings_open: false,
            settings_status: String::new(),
            audio_performance_open: false,
            smoke_diagnostics: None,
            keyboard: Keyboard::default(),
            keyboard_open: false,
            rack,
            visualizer,
            gl: cc.gl.clone(),
            shader: DEFAULT_SHADER.into(),
            shader_open: false,
            shader_status,
            analysis_tab: None,
            visual_mode: 0,
            scope: Scope::default(),
            volume,
            status: startup
                .status
                .unwrap_or_else(|| "Ready — press Play to hear the patch".into()),
            filename,
            title: startup.title,
            files: vec![],
            workspace,
            file_mode: None,
            file_focus: false,
            wavetable_file_target: None,
            wavetable_import: None,
            browser_open: false,
            library: ModuleLibrary::default(),
            preset_dialog: None,
            template_save: None,
            catalog: catalog(),
            start: Instant::now(),
            mcp_session: crate::mcp_session_ui::Panel::new(mcp),
            construction: crate::construction_ui::Panel::default(),
            cable_palette: crate::cable_palette::Palette::default(),
            cable_palette_ui: crate::cable_palette_ui::Panel::default(),
            smoke_motion_origin: None,
            documents,
            projects: startup.store,
            active_file: startup.active_file,
            active_token: startup.active_token,
            after_save: None,
            deferred_action: None,
            deferred_manual_release: None,
            autosave_needed: false,
            recovery_names,
            recovery_details: BTreeMap::new(),
            recovery_open,
            restored_recovery: None,
            autosaved_document: None,
            last_document_change: Instant::now(),
            last_autosave_attempt: Instant::now(),
            can_undo: false,
            can_redo: false,
            confirm_engine_upgrade: false,
            confirm_discard: None,
            allow_close: false,
            smoke_test: smoke_test.map(|path| (path, 0, 0)),
        };
        app.refresh_recovery_list();
        app.apply_host_preferences(&preferences);
        app.refresh_files();

        app.refresh_devices();
        app.connect_selected_midi();
        app.attach_live_events();
        app
    }
    fn startup_view(
        startup: &crate::startup::Startup,
        patch: &Patch,
        revision: u64,
    ) -> (Rack, DocumentState) {
        let mut rack = Rack::default();
        rack.restore_layout(&startup.document.layout)
            .expect("validated startup layout");
        rack.reconcile(patch, &catalog());
        let mut documents = DocumentState::new(patch, revision, &rack);
        // MCP may already have changed the live graph since startup preparation.
        // Its edits remain current; the original document is the saved baseline.
        documents.mark_saved(&startup.document);
        debug_assert!(revision >= startup.revision);
        (rack, documents)
    }

    fn attach_live_events(&mut self) {
        // A retiring callback may still own a Learn decision. Retain the old
        // Session backend until its completion can be reconciled, then retry
        // against the current audio instance rather than a captured sender.
        self.live_events_attach_pending = true;
        let sender = self.audio.as_ref().map(Audio::module_event_sender);
        let result = match self.shared.try_lock() {
            Ok(mut session) => session.set_live_module_events(sender),
            Err(std::sync::TryLockError::WouldBlock) => return,
            Err(std::sync::TryLockError::Poisoned(_)) => Err("Session lock poisoned".into()),
        };
        match result {
            Ok(()) => {
                self.live_events_attach_pending = false;
                if self
                    .settings_status
                    .starts_with("Audio controls reconnecting:")
                {
                    self.settings_status = if self.audio.is_some() {
                        "Audio controls ready"
                    } else {
                        "Audio stopped · controls detached"
                    }
                    .into();
                }
            }
            Err(error) => {
                self.settings_status = format!("Audio controls reconnecting: {error}");
            }
        }
    }

    fn sync_midi_cc_states(&mut self) {
        // Offline smoke scenes retain their actual, explicitly captured DSP
        // snapshots, just like the other stateful module panels.
        if self.smoke_test.is_some() && self.audio.is_none() {
            return;
        }
        let states = self
            .audio
            .as_ref()
            .filter(|_| self.patch.nodes.iter().any(|node| node.kind == "midi_cc"))
            .map(Audio::midi_cc_states)
            .unwrap_or_default();
        self.rack.set_midi_cc_states(states);
    }
    fn sync_midi_gate_states(&mut self) {
        if self.smoke_test.is_some() && self.audio.is_none() {
            return;
        }
        let states = self
            .audio
            .as_ref()
            .filter(|_| self.patch.nodes.iter().any(|node| node.kind == "midi_gate"))
            .map(Audio::midi_gate_states)
            .unwrap_or_default();
        self.rack.set_midi_gate_states(states);
    }
    fn open_audio(
        patch: &Patch,
        wave_banks: std::sync::Arc<openpatch_core::PreparedWaveBanks>,
        device: Option<&str>,
        port_lights_enabled: bool,
    ) -> (Option<Audio>, String) {
        let opened =
            Audio::new_with_device_and_wave_banks(patch, wave_banks, device, port_lights_enabled);
        match opened {
            Ok(audio) => {
                let label = format!(
                    "{} · {} Hz · {} output channels",
                    audio.device_name, audio.sample_rate, audio.device_channels
                );
                (Some(audio), label)
            }
            Err(e) => (None, e),
        }
    }
    // Rack replacement is a document operation; appearance and follow opt-in
    // belong to the window, including when examples replace the entire rack.
    fn replace_rack(&mut self, rack: Rack) {
        // Smoke scenes may replace Session directly before the next sync.
        // Production document operations pass their accepted revision below.
        let revision = self
            .shared
            .lock()
            .map(|live| live.revision())
            .unwrap_or(self.revision);
        self.replace_rack_for_document(rack, revision);
    }
    fn replace_rack_for_document(&mut self, mut rack: Rack, document_revision: u64) {
        self.construction.begin_document(document_revision);
        rack.set_cable_presentation(self.rack.cable_presentation());
        rack.set_agent_follow(self.rack.agent_follow_enabled());
        self.rack = rack;
    }
    fn set_cable_presentation(&mut self, presentation: CablePresentation) {
        let old_enabled = self.rack.cable_presentation().plug_brightness > 0.0;
        let presentation = presentation.sanitized();
        self.rack.set_cable_presentation(presentation);
        let enabled = presentation.plug_brightness > 0.0;
        if old_enabled != enabled {
            // Cached measurements cannot reappear on a rapid off/on gesture.
            // Audio separately requires a new real acquisition before display.
            self.rack.set_port_lights(None);
            if let Some(audio) = &self.audio {
                audio.set_port_lights_enabled(enabled);
            }
        }
    }
    fn apply_host_preferences(&mut self, preferences: &Value) {
        // Clamp JSON's finite f64 before narrowing; huge valid numbers must
        // clamp to the zoom bound rather than overflow and select the fallback.
        self.library.set_preview_zoom(
            preferences["library_zoom"]
                .as_f64()
                .filter(|value| value.is_finite())
                .map_or(0.5, |value| value.clamp(0.25, 2.0) as f32),
        );
        self.set_cable_presentation(CablePresentation::from_settings(preferences));
        if let Some(audio) = &self.audio {
            audio.set_port_lights_enabled(self.rack.cable_presentation().plug_brightness > 0.0);
        }
        self.cable_palette =
            crate::cable_palette::Palette::from_settings(&preferences["cable_palette"]);
        self.rack.set_room_brightness(
            preferences["room_brightness"]
                .as_f64()
                .map_or(1.0, |value| value.clamp(0.0, 1.0) as f32),
        );
        self.rack.set_positions_locked(
            preferences["lock_module_positions"]
                .as_bool()
                .unwrap_or(false),
        );
        let favorites: Vec<String> = preferences["module_favorites"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect();
        self.library.set_favorites(&favorites, &self.catalog);
    }

    fn save_host_settings(&mut self) {
        let path = PathBuf::from(&self.workspace).join(".host-settings.json");
        let temporary = path.with_extension(format!(
            "{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let result = (|| -> Result<(), String> {
            let data = serde_json::to_vec_pretty(&json!({
                "output_device": self.output_device, "midi_device": self.midi_device,
                "module_favorites": self.library.favorites(),
                "library_zoom": self.library.preview_zoom(),
                "lock_module_positions": self.rack.positions_locked(),
                "room_brightness": self.rack.room_brightness(),
                "cable_opacity": self.rack.cable_presentation().opacity,
                "cable_tension": self.rack.cable_presentation().tension,
                "plug_light_brightness": self.rack.cable_presentation().plug_brightness,
                "cable_palette": self.cable_palette.settings()
            }))
            .map_err(|e| e.to_string())?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            file.write_all(&data).map_err(|e| e.to_string())?;
            drop(file);
            std::fs::rename(&temporary, &path).map_err(|e| e.to_string())
        })();
        if let Err(error) = result {
            self.settings_status = format!("Preferences could not be saved: {error}");
        }
    }
    fn refresh_devices(&mut self) {
        match Audio::output_devices() {
            Ok(devices) => self.output_devices = devices,
            Err(error) => self.settings_status = error,
        }
        if let Some(midi) = &mut self.midi {
            match midi.refresh() {
                Ok(ports) => self.midi_ports = ports,
                Err(error) => self.settings_status = error,
            }
        }
    }
    fn connect_selected_midi(&mut self) {
        if let Some(midi) = &mut self.midi {
            midi.disconnect();
            if let Some(id) = &self.midi_device {
                match midi.connect(id) {
                    Ok(()) => {
                        self.settings_status = "MIDI connected — press Play to perform".into()
                    }
                    Err(error) => self.settings_status = error,
                }
            }
        }
    }
    fn reopen_audio(&mut self, selected: Option<String>) {
        self.scope.reset_connection();
        let ticket = match self.shared.lock() {
            Ok(mut session) => match session.audio_snapshot() {
                Ok(ticket) => ticket,
                Err(error) => {
                    self.settings_status = error;
                    return;
                }
            },
            Err(_) => {
                self.settings_status = "Session lock poisoned".into();
                return;
            }
        };
        let (audio, label) = Self::open_audio(
            &ticket.patch,
            ticket.banks.clone(),
            selected.as_deref(),
            self.rack.cable_presentation().plug_brightness > 0.0,
        );
        if let Some(mut audio) = audio {
            // The new engine was compiled from this immutable ticket. Its
            // predecessor's signal history does not cross a device restart.
            if let Ok(mut session) = self.shared.lock() {
                session.acknowledge_audio_snapshot(ticket.id);
            }
            self.midi = None;
            self.keyboard.release();
            audio.set_volume(self.volume);
            self.midi_epoch = audio.midi_epoch();
            self.midi = audio.take_midi_sender().map(MidiInputManager::new);
            self.audio = Some(audio);
            self.device = label;
            self.output_device = selected;
            self.audio_revision = None;
            self.attach_live_events();
            self.status = "Audio ready — press Play".into();
            self.refresh_devices();
            self.connect_selected_midi();
            self.save_host_settings();
        } else {
            self.settings_status = label;
        }
    }
    fn send_notes(&mut self, events: Vec<MidiEvent>) {
        if events.contains(&MidiEvent::Panic) {
            self.panic_notes();
            self.status = "All notes released".into();
            return;
        }
        if let Some(audio) = &mut self.audio {
            if !audio.is_playing() {
                return;
            }
            for event in events {
                if !audio.send_midi_in_epoch(event, self.midi_epoch) {
                    self.keyboard.release();
                    self.status = "MIDI queue reset — play the notes again".into();
                    break;
                }
            }
        }
    }
    fn panic_notes(&mut self) {
        self.keyboard.release();
        if let Some(audio) = &self.audio {
            audio.panic_midi();
        }
    }
    fn performance_keyboard_enabled(&self, ctx: &egui::Context, playing: bool) -> bool {
        self.keyboard_open
            && !egui::Popup::is_any_open(ctx)
            && playing
            && !ctx.wants_keyboard_input()
            && !self.rack.annotation_keyboard_active()
            && !self.cable_palette_ui.keyboard_active()
            && ctx.input(|i| i.focused)
            && !egui::DragAndDrop::has_payload_of_type::<LibraryDrag>(ctx)
            && self.preset_dialog.is_none()
            && self.template_save.is_none()
            && self.file_mode.is_none()
            && !self.shader_open
            && !self.mcp_session.is_open()
            && !self.settings_open
            && !self.audio_performance_open
            && !self.recovery_open
            && !self.confirm_engine_upgrade
            && self.confirm_discard.is_none()
            && !self.allow_close
    }
    fn update_performance_keyboard(
        &mut self,
        ctx: &egui::Context,
        playing: bool,
    ) -> Vec<openpatch_core::MidiEvent> {
        let enabled = self.performance_keyboard_enabled(ctx, playing);
        self.keyboard.update(ctx, enabled)
    }
    fn service_performance_keyboard(&mut self, ctx: &egui::Context) {
        let playing = self.audio.as_ref().is_some_and(Audio::is_playing);
        let events = self.update_performance_keyboard(ctx, playing);
        self.send_notes(events);
    }
    fn performance_keyboard(&mut self, ctx: &egui::Context) {
        let enabled = self
            .performance_keyboard_enabled(ctx, self.audio.as_ref().is_some_and(Audio::is_playing));
        if self.keyboard_open {
            let mut events = Vec::new();
            egui::TopBottomPanel::bottom("performance-keyboard").show(ctx, |ui| {
                events = self.keyboard.show(ui, enabled);
            });
            self.send_notes(events);
        }
    }
    fn audio_settings(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let (anchor, layer) = ctx
            .data(|data| {
                data.get_temp::<(egui::Rect, egui::LayerId)>(egui::Id::new("audio-settings-anchor"))
            })
            .unwrap_or_else(|| {
                let right = ctx.content_rect().right() - 12.0;
                (
                    egui::Rect::from_min_size(
                        egui::pos2(right - 30.0, 7.0),
                        egui::vec2(30.0, 30.0),
                    ),
                    egui::LayerId::background(),
                )
            });
        let id = egui::Id::new("audio-settings-dropdown");
        // ComboBox popups use their own foreground layer. A selection in that
        // child belongs to settings even when its list extends past this frame.
        let child_under_pointer = egui::Popup::is_any_open(ctx)
            && ctx
                .pointer_interact_pos()
                .and_then(|pos| ctx.layer_id_at(pos))
                .is_some_and(|layer| layer.order == egui::Order::Foreground && layer.id != id);
        let width = 430.0_f32.min(ctx.content_rect().width() - 24.0).max(240.0);
        let max_height = (ctx.content_rect().bottom() - anchor.bottom() - 24.0).max(40.0);
        let mut open = true;
        let response = egui::Popup::new(id, ctx.clone(), anchor, layer)
            .kind(egui::PopupKind::Menu)
            .open_bool(&mut open)
            .align(egui::emath::RectAlign::BOTTOM_END)
            .align_alternatives(&[])
            .gap(6.0)
            .width(width)
            .close_behavior(if child_under_pointer {
                egui::PopupCloseBehavior::IgnoreClicks
            } else {
                egui::PopupCloseBehavior::CloseOnClickOutside
            })
            .show(|ui| {
                ui.set_width(width);
                egui::ScrollArea::vertical()
                    .id_salt("audio-settings-scroll")
                    .max_height(max_height)
                    .show(ui, |ui| {
                        ui.heading("Audio & MIDI");
                        ui.add_space(4.0);
                        ui.heading("Audio output");
                        let mut selected = self.output_device.clone();
                        let audio_device = egui::ComboBox::from_id_salt("audio-device")
                            .width(ui.available_width())
                            .selected_text(selected.as_deref().unwrap_or("System default"))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut selected, None, "System default");
                                for device in &self.output_devices {
                                    ui.selectable_value(
                                        &mut selected,
                                        Some(device.clone()),
                                        device,
                                    );
                                }
                            });
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(
                                egui::Id::new("settings-audio-device-rect"),
                                audio_device.response.rect,
                            );
                            data.insert_temp(
                                egui::Id::new("settings-audio-device-id"),
                                audio_device.response.id,
                            );
                        });
                        if selected != self.output_device {
                            self.reopen_audio(selected);
                        }
                        ui.label(&self.device);
                        if self
                            .audio
                            .as_ref()
                            .is_some_and(|audio| audio.device_channels == 1)
                        {
                            ui.label("Mono device: stereo patches are mixed to (L + R) / 2.");
                        }
                        ui.label(
                            "Changing the output pauses playback. Sample rate follows the device.",
                        );
                        ui.separator();
                        ui.heading("Global MIDI input");
                        let mut midi = self.midi_device.clone();
                        let midi_label = midi
                            .as_ref()
                            .map(|id| {
                                self.midi_ports
                                    .iter()
                                    .find(|port| &port.id == id)
                                    .map_or("Unavailable device", |port| port.name.as_str())
                            })
                            .unwrap_or("None — use the on-screen keyboard");
                        egui::ComboBox::from_id_salt("midi-device")
                            .width(ui.available_width())
                            .selected_text(midi_label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut midi,
                                    None,
                                    "None — use the on-screen keyboard",
                                );
                                for port in &self.midi_ports {
                                    ui.selectable_value(
                                        &mut midi,
                                        Some(port.id.clone()),
                                        &port.name,
                                    );
                                }
                            });
                        if midi != self.midi_device {
                            self.midi_device = midi;
                            self.connect_selected_midi();
                            self.save_host_settings();
                        }
                        let connected = self
                            .midi
                            .as_ref()
                            .and_then(MidiInputManager::connected_id)
                            .is_some();
                        ui.label(if connected {
                            "MIDI connected"
                        } else {
                            "No hardware MIDI input connected"
                        });
                        ui.label("Add MIDI → CV and connect pitch, gate and retrigger to a voice.");
                        ui.label(
                            "Pause, reconnect, and voice allocation changes release held notes.",
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Refresh devices").clicked() {
                                self.refresh_devices();
                            }
                            if ui.button("Reconnect MIDI").clicked() {
                                self.connect_selected_midi();
                            }
                            if ui.button("All notes off").clicked() {
                                self.panic_notes();
                            }
                        });
                        ui.checkbox(&mut self.keyboard_open, "Show performance keyboard");
                        if let Some(audio) = &self.audio {
                            if audio.midi_overflows() > 0 {
                                ui.label(format!("MIDI queue resets: {}", audio.midi_overflows()));
                            }
                        }
                        ui.label(&self.settings_status);
                    });
            });
        if let Some(response) = response {
            // Native outside-click dismissal excludes drags. An outside
            // release also ends this dropdown's ownership, after the rack has
            // already ignored the entire gesture for this frame.
            let released_outside = !child_under_pointer
                && ctx.input(|input| input.pointer.any_released())
                && ctx.pointer_interact_pos().is_some_and(|position| {
                    !response.response.rect.contains(position) && !anchor.contains(position)
                });
            open &= !released_outside;
            ctx.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("audio-settings-dropdown-rect"),
                    response.response.rect,
                )
            });
        }
        if !open {
            egui::Popup::close_all(ctx);
        }
        self.settings_open = open;
    }

    fn diagnostic_snapshot(&self) -> Option<AudioTimingSnapshot> {
        if self.smoke_test.is_some() {
            if let Some(snapshot) = &self.smoke_diagnostics {
                return Some(snapshot.clone());
            }
        }
        self.audio.as_ref().map(Audio::diagnostics)
    }

    fn audio_performance(&mut self, ctx: &egui::Context) {
        if !self.audio_performance_open {
            return;
        }
        let snapshot = self.diagnostic_snapshot();
        let status = timing_status(snapshot.as_ref());
        let simulated = self.smoke_test.is_some() && self.smoke_diagnostics.is_some();
        let mut open = true;
        let mut reset = false;
        egui::Window::new("Audio performance")
            .default_pos(ctx.content_rect().center() - Vec2::new(210.0, 205.0))
            .open(&mut open)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                if simulated {
                    ui.colored_label(ACCENT, "SIMULATED DATA · SCREENSHOT FIXTURE");
                } else if self.audio.as_ref().is_some_and(|audio| !audio.is_playing())
                    && status == TimingStatus::Ready
                {
                    ui.label("Transport paused · audio callbacks continue");
                }
                ui.label(timing_explanation(status));
                if status == TimingStatus::Ready {
                    let snapshot = snapshot.as_ref().unwrap();
                    let percent = |value: f64| format!("{:.1}%", value * 100.0);
                    let current = snapshot.utilization.map(percent).unwrap_or_else(|| "—".into());
                    let peak = snapshot.peak_utilization.map(percent).unwrap_or_else(|| "—".into());
                    let mean = weighted_mean_utilization(snapshot)
                        .map(percent)
                        .unwrap_or_else(|| "—".into());
                    ui.separator();
                    egui::Grid::new("audio-performance-metrics")
                        .num_columns(2)
                        .spacing([18.0, 5.0])
                        .show(ui, |ui| {
                            ui.label("Current budget use");
                            ui.strong(current);
                            ui.end_row();
                            ui.label("Peak budget use");
                            ui.strong(peak);
                            ui.end_row();
                            ui.label("Weighted mean");
                            ui.strong(mean);
                            ui.end_row();
                            ui.label("Last processing / budget");
                            ui.label(format!(
                                "{:.3} / {:.3} ms",
                                snapshot.duration_ns as f64 / 1_000_000.0,
                                snapshot.frames as f64 * 1000.0 / snapshot.sample_rate as f64
                            ));
                            ui.end_row();
                            ui.label("Measured callbacks");
                            ui.label(snapshot.callbacks.to_string());
                            ui.end_row();
                            ui.label("Invalid or empty callbacks");
                            ui.label(snapshot.invalid_callbacks.to_string());
                            ui.end_row();
                            ui.label("Processing budget overruns");
                            ui.colored_label(ACCENT, snapshot.overruns.to_string());
                            ui.end_row();
                            ui.label("Graph transitions in interval");
                            ui.label(snapshot.transition_callbacks.to_string());
                            ui.end_row();
                        });
                    ui.separator();
                    ui.label(format!(
                        "Last callback · {} frames × {} channels at {} Hz",
                        snapshot.frames, snapshot.channels, snapshot.sample_rate
                    ));
                    ui.label(format!(
                        "Graph revision {} · callback sequence {}",
                        snapshot.revision, snapshot.callback_sequence
                    ));
                    ui.label(format!(
                        "Graph adopted: {} · crossfade: {}",
                        if snapshot.adopted_graph { "yes" } else { "no" },
                        if snapshot.crossfade { "yes" } else { "no" }
                    ));
                }
                ui.separator();
                ui.label(RichText::new("Processing overruns mean measured callback time exceeded its frame budget. Elapsed time includes preemption during rendering; it does not measure late callback delivery or hardware xruns.").size(10.0).color(MUTED));
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.audio.is_some() && !simulated && status != TimingStatus::Stopped,
                            egui::Button::new("Reset statistics"),
                        )
                        .on_hover_text("Start a new measurement interval on the next audio callback")
                        .clicked()
                    {
                        reset = true;
                    }
                    if status == TimingStatus::ResetPending {
                        ui.label("Waiting for callback…");
                    }
                });
            });
        self.audio_performance_open = open;
        if reset {
            if let Some(audio) = &self.audio {
                audio.reset_diagnostics();
                ctx.request_repaint();
            }
        }
    }
    fn refresh_files(&mut self) {
        self.files = std::fs::read_dir(&self.workspace)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| {
                (name.ends_with(".openpatch") || name.ends_with(".json"))
                    && !name.starts_with('.')
                    && !name.ends_with(".layout.json")
            })
            .collect();
        self.files.sort();
    }
    fn command(&mut self, name: &str, mut args: Value) -> bool {
        if matches!(name, "open_wavetable_import" | "open_wavetable_export") {
            return self.wavetable_file_dialog(name, &args);
        }
        if !matches!(name, "save_patch" | "render_audio") && args.get("expected_revision").is_none()
        {
            args["expected_revision"] = json!(self.revision);
        }
        match execute_shared(&self.shared, name, args) {
            Ok(receipt) => {
                if name == "replace_patch" {
                    if let Some(revision) = receipt["revision"].as_u64() {
                        self.construction.begin_document(revision);
                    }
                }
                // The hand has already moved this cable. Consume only this
                // accepted local revision before sync, including a busy retry;
                // a following MCP revision still gets its own motion cue.
                if let Some(revision) = receipt["revision"]
                    .as_u64()
                    .filter(|_| matches!(name, "move_cables" | "apply_edits"))
                {
                    self.rack.suppress_cable_move_revision(revision);
                }
                self.status = match name {
                    "trigger_module" => "Performance action queued",
                    "learn_midi_cc" => "MIDI Learn requested · move a controller",
                    "learn_midi_gate" => "MIDI Learn requested · play a note",
                    "cancel_midi_cc_learn" | "cancel_midi_gate_learn" => {
                        "MIDI Learn cancellation requested"
                    }
                    _ => "Patch updated",
                }
                .into();
                self.sync();
                true
            }
            Err(e) => {
                self.status = e;
                self.sync();
                false
            }
        }
    }
    fn connect_from_palette(
        &mut self,
        arguments: Value,
        selection: crate::cable_palette::Selection,
    ) {
        // `connect` always adds exactly one cable when its transaction succeeds.
        // Preference failure must never roll back that accepted graph change.
        let result = execute_shared(&self.shared, "connect", arguments);
        match result {
            Ok(_) => {
                self.status = "Cable connected".into();
                if self.cable_palette.advance_on_commit(selection) {
                    self.save_host_settings();
                }
            }
            Err(error) => self.status = error,
        }
        self.sync();
    }

    fn edit_parameter_and_navigate(&mut self, arguments: Value, forward: bool) {
        // Capture the accepted revision at the transaction boundary. A following
        // MCP edit or a busy GUI sync must never rebase the editor silently.
        let result = self
            .shared
            .lock()
            .map_err(|_| "Session lock poisoned".to_owned())
            .and_then(|mut session| {
                session.execute("set_parameter", arguments)?;
                Ok(session.revision())
            });
        self.status = match &result {
            Ok(_) => "Value applied".into(),
            Err(error) => error.clone(),
        };
        self.rack.complete_parameter_navigation(result, forward);
        self.sync();
    }
    fn paint_scale(&mut self, arguments: Value, gesture: u64) {
        let result = self
            .shared
            .lock()
            .map_err(|_| "Session lock poisoned".to_owned())
            .and_then(|mut session| {
                session.execute("set_parameter", arguments)?;
                Ok(session.revision())
            });
        self.status = match &result {
            Ok(_) => "Scale painted".into(),
            Err(error) => error.clone(),
        };
        self.rack.complete_scale_paint(gesture, result);
        self.sync();
    }
    fn manual_click(&mut self, id: &str, press: &str, release: &str) -> bool {
        // A fast pointer click can arrive entirely within one GUI frame.
        // Keep both actions on the same module incarnation while enqueueing;
        // the ordinary release path also handles queue-full safety.
        let result = self
            .shared
            .lock()
            .map_err(|_| "Session lock poisoned".to_owned())
            .and_then(|mut session| {
                session.execute(
                    "trigger_module",
                    json!({"id":id,"action":press,"expected_revision":self.revision}),
                )?;
                session.execute(
                    "trigger_module",
                    json!({"id":id,"action":release,"expected_revision":self.revision}),
                )
            });
        let succeeded = result.is_ok();
        self.status = match result {
            Ok(receipt) if receipt["receipt"]["emergency_release"] == true => {
                "Button release requested through the safety path".into()
            }
            Ok(_) => "Button click queued".into(),
            Err(error) => error,
        };
        self.sync();
        succeeded
    }
    fn finish_previous_manual_click(&mut self) -> bool {
        if let Some((_, ticket)) = self.deferred_manual_release.as_mut() {
            if ticket.ready_to_release() {
                ticket.begin_release();
            }
            if !ticket.is_finished() {
                self.status="Previous button click is waiting for the audio engine to sample press and release; try again afterward".into();
                return false;
            }
        }
        self.deferred_manual_release.take();
        true
    }
    fn manual_gate_click(&mut self, ctx: &egui::Context, id: &str) {
        self.manual_button_click(ctx, id, "push_press", "push_release");
    }
    fn manual_button_click(&mut self, ctx: &egui::Context, id: &str, press: &str, release: &str) {
        // Resolve the old ticket before accepting another press. The ticket
        // retains its backend and incarnation; it never retargets a reused ID.
        if !self.finish_previous_manual_click() {
            return;
        }
        let result = self
            .shared
            .lock()
            .map_err(|_| "Session lock poisoned".to_owned())
            .and_then(|session| {
                if session.revision() != self.revision
                    || !session.patch().nodes.iter().any(|n| {
                        n.id == id
                            && matches!(n.kind.as_str(), "adsr_eg" | "stock_seq3")
                            && !n.bypassed
                    })
                {
                    return Err("Module changed before its button could be queued".into());
                }
                let press = serde_json::from_value::<openpatch_core::ModuleAction>(json!(press))
                    .map_err(|e| e.to_string())?;
                let release =
                    serde_json::from_value::<openpatch_core::ModuleAction>(json!(release))
                        .map_err(|e| e.to_string())?;
                let audio = self
                    .audio
                    .as_ref()
                    .ok_or_else(|| "Audio is unavailable".to_owned())?;
                if press == openpatch_core::ModuleAction::PushPress
                    && release == openpatch_core::ModuleAction::PushRelease
                {
                    audio.press_gate_with_ticket(id)
                } else {
                    audio.press_manual_with_ticket(id, press, release)
                }
            });
        match result {
            Ok(ticket) => {
                self.deferred_manual_release = Some((ctx.cumulative_frame_nr(), ticket));
                self.status = "Button queued · release after the audio engine samples it".into();
                ctx.request_repaint();
            }
            Err(error) => self.status = error,
        }
    }
    fn service_manual_release(&mut self, ctx: &egui::Context) {
        if !ctx.input(|input| input.focused) || !self.library_enabled() {
            self.deferred_manual_release.take();
            return;
        }
        if let Some((frame, ticket)) = self.deferred_manual_release.as_mut() {
            if *frame != ctx.cumulative_frame_nr() && ticket.ready_to_release() {
                ticket.begin_release();
            }
            if ticket.is_finished() {
                self.deferred_manual_release.take();
            }
        }
        if self.deferred_manual_release.is_some() {
            ctx.request_repaint();
        }
    }
    fn sync(&mut self) {
        let mut audio_ticket = None;
        if self.live_events_attach_pending {
            self.attach_live_events();
        }
        if let Ok(mut s) = self.shared.try_lock() {
            let construction = s.construction_snapshot();
            self.can_undo = s.can_undo();
            self.can_redo = s.can_redo();
            let mut restored_positions = BTreeSet::new();
            if s.revision() != self.revision {
                // Preview positions never belong to the outgoing history state.
                // Cancel even when a grouped edit retains the same state ID.
                self.rack.cancel_module_drag();
                self.history_layouts
                    .insert(self.state_id, self.rack.layout());
                if self.state_id != s.state_id() {
                    self.state_id = s.state_id();
                    if let Some(layout) = self.history_layouts.get(&self.state_id) {
                        if self.rack.restore_layout(layout).is_ok() {
                            if let Some(positions) = layout["positions"].as_object() {
                                restored_positions.extend(positions.keys().cloned());
                            }
                        }
                    }
                }
                let retained: BTreeSet<_> = s.history_state_ids().collect();
                self.history_layouts.retain(|id, _| retained.contains(id));
            }
            // Row hints belong to the incoming committed state. Save the
            // outgoing geometry and cancel preview drags before applying them,
            // then let construction motion observe the final destination.
            // Local add receipts may already have installed an explicit drop
            // position at this exact state/revision before sync drains hints.
            if let Some(layout) = self.history_layouts.get(&s.state_id()) {
                if let Some(positions) = layout["positions"].as_object() {
                    restored_positions.extend(positions.keys().cloned());
                }
            }
            let mut placements = s.take_layout_placements();
            placements.retain(|id, _| !restored_positions.contains(id));
            // A console grows only after the outgoing geometry is saved.
            // History layouts and freshly added incarnations already carry
            // authoritative placement; ordinary GUI and MCP channel edits
            // push only the overlapping right-hand chain in this new state.
            if s.revision() != self.revision {
                for node in &s.patch().nodes {
                    if node.kind != "studio_mixer"
                        || restored_positions.contains(&node.id)
                        || placements.contains_key(&node.id)
                    {
                        continue;
                    }
                    if let Some(previous) = self
                        .patch
                        .nodes
                        .iter()
                        .find(|old| old.id == node.id && old.kind == node.kind)
                    {
                        if let Err(error) = self.rack.settle_studio_mixer_growth(
                            s.patch(),
                            &node.id,
                            crate::rack::module_width(previous),
                        ) {
                            self.status = error;
                        }
                    }
                }
            }
            if !placements.is_empty() {
                self.rack.place_in_rows(s.patch(), &placements);
            }
            if let Some(now) = self.smoke_motion_origin {
                // The smoke fixture captures controlled presentation phases of
                // a real commit. Normalize only its cloned presentation age;
                // scheduler delays must not erase the phase being inspected.
                // The authoritative host receipt and production clock are untouched.
                let mut presentation = construction.clone();
                if presentation["last_commit"].is_object() {
                    presentation["last_commit"]["age_ms"] = json!(0);
                }
                if presentation["last_move"].is_object() {
                    presentation["last_move"]["age_ms"] = json!(0);
                }
                self.rack.observe_construction_transition_at(
                    &self.patch,
                    s.patch(),
                    self.revision,
                    s.revision(),
                    &presentation,
                    self.construction.motion_mode(),
                    now,
                );
            } else {
                self.rack.observe_construction_transition_with_mode(
                    &self.patch,
                    s.patch(),
                    self.revision,
                    s.revision(),
                    &construction,
                    self.construction.motion_mode(),
                );
            }
            if s.revision() != self.revision {
                self.patch = s.patch().clone();
                self.rack.reconcile_selection(&self.patch);
                self.revision = s.revision();
            }
            // Seal patch, banks, revision and ordered host effects together.
            // Later commits append a separate segment while this one retries.
            if self.audio.is_some() && self.audio_revision != Some(s.revision()) {
                match s.audio_snapshot() {
                    Ok(ticket) => audio_ticket = Some(ticket),
                    Err(error) => self.status = error,
                }
            }
            self.construction.observe(construction);
        }
        if let Some(midi) = &mut self.midi {
            if let Some(error) = midi.poll() {
                self.settings_status = error;
            }
        }
        let mut failure = None;
        if let Some(audio) = &mut self.audio {
            audio.service();
            if self.midi_epoch != audio.midi_epoch() {
                self.midi_epoch = audio.midi_epoch();
                self.keyboard.release();
            }
            if let Some(ticket) = audio_ticket {
                match audio.submit_patch_with_host_lifecycle(
                    &ticket.patch,
                    ticket.revision,
                    ticket.banks.clone(),
                    &ticket.delta,
                ) {
                    Ok(()) => {
                        self.audio_revision = Some(ticket.revision);
                        if let Ok(mut session) = self.shared.lock() {
                            session.acknowledge_audio_snapshot(ticket.id);
                        }
                    }
                    Err(e) => {
                        audio.set_playing(false);
                        failure = Some(e);
                    }
                }
            }
            if audio.take_stream_error() {
                audio.set_playing(false);
                failure = Some(
                    "The audio device disconnected. Reconnect it and click Retry audio.".into(),
                );
            }
        }
        if let Some(error) = failure {
            self.midi = None;
            self.keyboard.release();
            self.audio = None;
            self.attach_live_events();
            self.audio_revision = None;
            self.device = error;
            self.status = "Audio stopped".into();
        }
    }
    fn history(&mut self, redo: bool) {
        let tool = if redo { "redo" } else { "undo" };
        if self.command(tool, json!({})) {
            self.status = if redo {
                "Patch edit redone"
            } else {
                "Patch edit undone"
            }
            .into();
        }
    }
    fn toggle_play(&mut self) {
        if let Some(audio) = &self.audio {
            audio.set_playing(!audio.is_playing());
            self.status = if audio.is_playing() {
                "Playing"
            } else {
                "Paused"
            }
            .into();
        }
    }
    fn browse(&mut self) {
        self.browser_open = true;
        self.library.request_focus();
    }
    fn file_dialog(&mut self, mode: FileMode) {
        self.cancel_wavetable_import();
        self.wavetable_file_target = None;
        self.refresh_files();
        if mode == FileMode::Save {
            self.filename = self.active_file.as_ref().map_or_else(
                || "my-patch.openpatch".into(),
                |name| format!("{}-copy.openpatch", name.trim_end_matches(".openpatch")),
            );
        }
        self.file_mode = Some(mode);
        self.file_focus = true;
    }

    fn wavetable_file_dialog(&mut self, command: &str, args: &Value) -> bool {
        let target = (|| {
            let id = args["id"].as_str().ok_or("Wavetable target is missing")?;
            let revision = args["expected_revision"]
                .as_u64()
                .ok_or("Wavetable target revision is missing")?;
            if revision != self.revision {
                return Err("Patch changed; reopen the wavetable file dialog");
            }
            let node = self
                .patch
                .nodes
                .iter()
                .find(|node| node.id == id)
                .ok_or("Wavetable module was removed")?;
            if !matches!(node.kind.as_str(), "wavetable_vco" | "wavetable_lfo") {
                return Err("Choose a wavetable module");
            }
            let current_points = match &node.content {
                Some(openpatch_core::ModuleContent::Wavetable { wave_points, .. }) => *wave_points,
                _ => 1024,
            };
            Ok(WavetableFileTarget {
                id: id.into(),
                revision,
                current_points,
                audio: node.kind == "wavetable_vco",
                wave_points: String::new(),
            })
        })();
        let target = match target {
            Ok(target) => target,
            Err(error) => {
                self.status = error.into();
                return false;
            }
        };
        let importing = command == "open_wavetable_import";
        self.cancel_wavetable_import();
        self.files = std::fs::read_dir(&self.workspace)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| {
                !name.starts_with('.')
                    && matches!(
                        wavetable_file_extension(name).as_str(),
                        "wav" | "f32" | "s8" | "i8" | "s16" | "i16" | "s24" | "i24" | "s32" | "i32"
                    )
            })
            .collect();
        self.files.sort();
        self.filename = if importing {
            self.files.first().cloned().unwrap_or_default()
        } else {
            "wavetable.f32".into()
        };
        self.status = format!(
            "{} wavetable for {}",
            if importing { "Load" } else { "Save" },
            target.id
        );
        self.wavetable_file_target = Some(target);
        self.file_mode = Some(if importing {
            FileMode::WavetableImport
        } else {
            FileMode::WavetableExport
        });
        self.file_focus = true;
        true
    }

    fn apply_wavetable_file(&mut self, mode: FileMode) {
        if mode == FileMode::WavetableImport && self.wavetable_import.is_some() {
            return;
        }
        let Some(target) = &self.wavetable_file_target else {
            self.status = "Wavetable target is unavailable; reopen the file dialog".into();
            return;
        };
        let importing = mode == FileMode::WavetableImport;
        let mut args = json!({
            "id":target.id,"filename":self.filename,"expected_revision":target.revision
        });
        if importing && !target.wave_points.trim().is_empty() {
            let points = target
                .wave_points
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|value| (2..=524288).contains(value));
            let Some(points) = points else {
                self.status =
                    "Wave points must be an integer from 2 to 524288, or empty for Auto".into();
                return;
            };
            args["wave_points"] = json!(points);
        }
        if importing {
            let shared = self.shared.clone();
            let allowed = Arc::new(AtomicBool::new(true));
            let worker_allowed = allowed.clone();
            let (sender, result) = mpsc::channel();
            let worker = std::thread::Builder::new()
                .name("wavetable-import".into())
                .spawn(move || {
                    // Decoding, preparation and release of replaced banks remain
                    // on this control worker. The core helper rechecks authorization
                    // and the opening revision immediately before publication.
                    let result = openpatch::control::import_wavetable_cancellable(
                        &shared,
                        args,
                        &worker_allowed,
                    );
                    let _ = sender.send(result);
                });
            match worker {
                Ok(_) => {
                    self.wavetable_import = Some(WavetableImportJob {
                        result,
                        allowed,
                        filename: self.filename.clone(),
                    });
                    self.status = "Preparing wavetable…".into();
                }
                Err(error) => self.status = format!("Could not start wavetable import: {error}"),
            }
        } else if self.command("export_wavetable", args) {
            self.status = format!("Wavetable saved: {}", self.filename);
            self.file_mode = None;
            self.wavetable_file_target = None;
        }
    }

    fn cancel_wavetable_import(&mut self) {
        if let Some(job) = &self.wavetable_import {
            if job.allowed.swap(false, Ordering::AcqRel) {
                self.status = "Wavetable import cancelled".into();
            }
        }
    }

    fn poll_wavetable_import(&mut self) {
        let Some(job) = &self.wavetable_import else {
            return;
        };
        let result = match job.result.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("Wavetable preparation worker stopped unexpectedly".into())
            }
        };
        let job = self.wavetable_import.take().unwrap();
        if !job.allowed.load(Ordering::Acquire) {
            // A cancelled result belongs to the dismissed dialog. Never change
            // a newer dialog or overwrite status from subsequent user actions.
            return;
        }
        self.sync();
        match result {
            Ok(_) => {
                self.status = format!("Wavetable loaded: {}", job.filename);
                self.file_mode = None;
                self.wavetable_file_target = None;
            }
            Err(error) => self.status = format!("Could not load wavetable: {error}"),
        }
    }
    fn observe_document(&mut self) {
        let running = self
            .shared
            .try_lock()
            .ok()
            .and_then(|live| live.capture_project_running(Some(self.revision)).ok())
            .map(|snapshot| snapshot.saved_states);
        if self
            .documents
            .observe_saved_runtime(&self.patch, self.revision, &self.rack, running)
        {
            self.last_document_change = Instant::now();
            self.autosave_needed = true;
        }
    }
    // Persist one coherent graph/Run/HOLD/layout snapshot. Raw capture_document below
    // remains the graph/layout receipt used by history and UI installers.
    fn capture_persisted_document(&mut self) -> Result<Document, String> {
        self.rack.reconcile(&self.patch, &self.catalog);
        let captured = self
            .shared
            .try_lock()
            .map_err(|_| "Project is busy; try again".to_string())?
            .capture_project_patch(Some(self.revision))?;
        if self.documents.observe_saved_runtime(
            &self.patch,
            self.revision,
            &self.rack,
            Some(captured.saved_states),
        ) {
            self.last_document_change = Instant::now();
            self.autosave_needed = true;
        }
        Ok(Document {
            patch: captured.patch,
            layout: self.rack.layout(),
        })
    }
    fn close_if_saved(&mut self) {
        let result = match self.shared.try_lock() {
            Ok(mut live) => live
                .capture_project_patch(Some(self.revision))
                .and_then(|snapshot| {
                    let document = Document {
                        patch: snapshot.patch,
                        layout: self.rack.layout(),
                    };
                    if self.documents.matches_saved(&document) {
                        live.seal_for_close(self.revision).map(|()| true)
                    } else {
                        Ok(false)
                    }
                }),
            Err(_) => {
                self.deferred_action = Some(PendingAction::Close);
                return;
            }
        };
        match result {
            Ok(true) => {
                self.clear_recovery();
                self.allow_close = true;
            }
            other => {
                if let Err(error) = other {
                    self.status = format!("Could not verify saved live state: {error}");
                }
                self.observe_document();
                self.confirm_discard = Some(PendingAction::Close);
            }
        }
    }
    fn dirty(&self) -> bool {
        self.documents.dirty()
    }
    fn capture_document(&mut self) -> Document {
        self.rack.reconcile(&self.patch, &self.catalog);
        self.observe_document();
        Document {
            patch: self.patch.clone(),
            layout: self.rack.layout(),
        }
    }
    fn request_action(&mut self, action: PendingAction) {
        self.cancel_wavetable_import();
        if matches!(action, PendingAction::DiscardClose) {
            self.perform_action(action);
            return;
        }
        // A document decision should describe a stable construction prefix.
        // Cancelling the dialog leaves the plan paused for an explicit Resume.
        let construction = self
            .shared
            .try_lock()
            .ok()
            .map(|live| live.construction_snapshot());
        let Some(construction) = construction else {
            self.deferred_action = Some(action);
            return;
        };
        if construction["status"] == "running" {
            if let Err(error) = execute_shared(
                &self.shared,
                "pause_construction",
                json!({"plan_id":construction["plan_id"]}),
            ) {
                self.status = error;
                self.deferred_action = Some(action);
                return;
            }
        }
        self.sync();
        let current = self
            .shared
            .try_lock()
            .is_ok_and(|live| live.revision() == self.revision);
        if !current {
            self.deferred_action = Some(action);
            return;
        }
        self.capture_document();
        self.file_mode = None;
        self.recovery_open = false;
        if self.dirty() {
            self.confirm_discard = Some(action);
        } else if matches!(action, PendingAction::Close) {
            self.close_if_saved();
        } else {
            match action {
                PendingAction::NewTemplate => self.new_from_template(true),
                PendingAction::Replace(patch, title) => {
                    self.replace_now_guarded(patch, &title, true)
                }
                PendingAction::Load(name) => self.load_document_guarded(&name, false, true),
                PendingAction::Recover(name) => self.load_document_guarded(&name, true, true),
                PendingAction::Close | PendingAction::DiscardClose => unreachable!(),
            }
        }
    }
    fn new_from_template(&mut self, require_saved: bool) {
        let (document, warning) = match self.projects.as_ref().map(Store::load_template) {
            Some(Ok(Some(document))) => (document, None),
            Some(Ok(None)) => (crate::startup::factory_document(), None),
            Some(Err(error)) => (
                crate::startup::factory_document(),
                Some(format!(
                    "Template unavailable: {error}. Using the factory patch."
                )),
            ),
            None => (
                crate::startup::factory_document(),
                Some("Project storage unavailable; using the factory patch.".into()),
            ),
        };
        let mut changed = false;
        let result = new_document_guarded(&self.shared, self.revision, &document, |live| {
            if require_saved {
                if let Err(error) = self.check_saved_content(live) {
                    changed = true;
                    return Err(error);
                }
            }
            Ok(())
        });
        match result {
            Ok(applied) => {
                let locked = self.rack.positions_locked();
                let room_brightness = self.rack.room_brightness();
                self.patch = applied.patch;
                self.replace_rack_for_document(applied.rack, applied.revision);
                self.rack.set_positions_locked(locked);
                self.rack.set_room_brightness(room_brightness);
                self.revision = applied.revision;
                self.state_id = applied.state_id;
                self.history_layouts.clear();
                self.history_layouts
                    .insert(self.state_id, self.rack.layout());
                self.panic_notes();
                if let Some(audio) = &self.audio {
                    audio.set_playing(false);
                }
                self.clear_recovery();
                self.documents = DocumentState::new(&self.patch, self.revision, &self.rack);
                self.active_file = None;
                self.active_token = None;
                self.title = "Untitled".into();
                self.filename = "my-patch.openpatch".into();
                self.file_mode = None;
                self.recovery_open = false;
                self.autosave_needed = false;
                self.status = warning
                    .unwrap_or_else(|| "New patch from template — playback is paused".into());
                self.sync();
                self.capture_document();
            }
            Err(error) => {
                self.status = format!("New patch failed: {error}");
                if changed {
                    self.confirm_discard = Some(PendingAction::NewTemplate);
                }
                self.sync();
                self.capture_document();
            }
        }
    }

    fn begin_save_template(&mut self) {
        self.sync();
        let result = self.capture_persisted_document().and_then(|document| {
            let token = self
                .projects
                .as_ref()
                .ok_or("Project storage is unavailable")?
                .template_token()?;
            Ok(TemplateSave { document, token })
        });
        match result {
            Ok(snapshot) => self.template_save = Some(snapshot),
            Err(error) => self.status = format!("Template could not be captured: {error}"),
        }
    }

    fn save_confirmed_template(&mut self) {
        let Some(snapshot) = self.template_save.take() else {
            return;
        };
        let result = self
            .projects
            .as_ref()
            .ok_or_else(|| "Project storage is unavailable".to_owned())
            .and_then(|store| store.save_template(&snapshot.document, snapshot.token.as_ref()));
        self.status = match result {
            Ok(token) => token.durability_warning.unwrap_or_else(|| "Template saved. New patches will start from this snapshot.".into()),
            Err(error) => format!("Template was not saved: {error}. Choose Save template again to review a fresh snapshot."),
        };
    }

    fn template_window(&mut self, ctx: &egui::Context) {
        let Some(snapshot) = &self.template_save else {
            return;
        };
        let mut save = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("save-template")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("Use this patch as your template?");
            ui.label(format!("Captured {} modules and {} cables, including layout and saved Run/HOLD settings.", snapshot.document.patch.nodes.len(), snapshot.document.patch.cables.len()));
            ui.label("This snapshot will replace the template used for New and future launches. Your current project and its saved file stay unchanged.");
            ui.horizontal(|ui| {
                cancel = ui.button("Cancel").clicked();
                save = ui.button("Save template").clicked();
            });
        });
        if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.template_save = None;
        } else if save {
            self.save_confirmed_template();
        }
    }

    fn replace_from_menu(&mut self, patch: Patch, title: &str) {
        self.request_action(PendingAction::Replace(patch, title.into()));
    }
    fn replace_now(&mut self, patch: Patch, title: &str) {
        self.replace_now_guarded(patch, title, false);
    }
    fn check_saved_content(&self, live: &openpatch::control::Session) -> Result<(), String> {
        let snapshot = live.capture_project_patch(Some(self.revision))?;
        let document = Document {
            patch: snapshot.patch,
            layout: self.rack.layout(),
        };
        if self.documents.matches_saved(&document) {
            Ok(())
        } else {
            Err("New unsaved changes arrived; choose Save, Discard, or Cancel".into())
        }
    }
    fn replace_now_guarded(&mut self, patch: Patch, title: &str, require_saved: bool) {
        let result = self
            .shared
            .lock()
            .map_err(|_| "Session lock poisoned".to_string())
            .and_then(|mut live| {
                if require_saved {
                    self.check_saved_content(&live)?;
                }
                live.replace_patch(patch.clone(), Some(self.revision))
            });
        let document_revision = match result {
            Ok(revision) => revision,
            Err(error) => {
                self.status = error;
                if require_saved {
                    self.confirm_discard = Some(PendingAction::Replace(patch, title.into()));
                }
                self.sync();
                self.observe_document();
                return;
            }
        };
        self.sync();
        self.panic_notes();
        self.clear_recovery();
        self.title = title.into();
        self.active_file = None;
        self.active_token = None;
        let positions_locked = self.rack.positions_locked();
        let room_brightness = self.rack.room_brightness();
        self.replace_rack_for_document(Rack::default(), document_revision);
        self.rack.set_positions_locked(positions_locked);
        self.rack.set_room_brightness(room_brightness);
        if title == "Sequenced analog voice" {
            self.sequence_layout();
        } else if title == "Delayed gate voice" {
            self.rack
                .restore_layout(&Self::gates_layout())
                .expect("bundled Gates layout");
        } else if title == "Manual push voice" {
            self.rack
                .restore_layout(&Self::push_layout())
                .expect("bundled Push layout");
        } else if title == "Manual pulses voice" {
            self.rack
                .restore_layout(&Self::pulses_layout())
                .expect("bundled Pulses layout");
        } else if title == "Switched stereo voice" {
            self.switched_stereo_layout();
        } else if title == "Synced waveform voice" {
            self.rack
                .restore_layout(&Self::waveform_layout())
                .expect("bundled waveform layout");
        } else if title == "Stacked pitch voice" {
            self.rack
                .restore_layout(&Self::stacked_layout())
                .expect("bundled stacked layout");
        } else if title == "Shift register canon" {
            self.rack
                .restore_layout(&Self::shift_register_layout())
                .expect("bundled shift register layout");
        } else if title == "Annotated drum rack" {
            self.rack
                .restore_layout(&Self::annotated_drums_layout())
                .expect("bundled annotated drum rack layout");
        } else if title == "MIDI drum pads" {
            self.rack
                .restore_layout(&Self::midi_gate_drums_layout())
                .expect("bundled MIDI drum pads layout");
        } else if title == "MIDI controller voice" {
            self.rack
                .restore_layout(&Self::midi_cc_voice_layout())
                .expect("bundled MIDI controller layout");
        } else if title == "Random values voice" {
            self.rack
                .restore_layout(&Self::random_values_layout())
                .expect("bundled Random Values layout");
        } else if title == "Layered stereo morph" {
            self.rack
                .restore_layout(&Self::mixing_layout())
                .expect("bundled mixer layout");
        } else if title == "Random motion voice" {
            self.rack
                .restore_layout(&Self::random_motion_layout())
                .expect("bundled Random motion layout");
        } else if title == "Stock Mix layered voice" {
            self.rack
                .restore_layout(&Self::stock_mix_voice_layout())
                .expect("bundled Stock Mix voice layout");
        } else if title == "Studio Mixer FX" {
            self.rack
                .restore_layout(&Self::studio_mixer_layout())
                .expect("bundled Studio Mixer layout");
        } else if title == "Noise into matter" {
            self.rack
                .restore_layout(&Self::stock_noise_layout())
                .expect("bundled Stock Noise resonator layout");
        } else if title == "Stock Process voice" {
            self.rack
                .restore_layout(&Self::stock_process_layout())
                .expect("bundled Stock Process voice layout");
        } else if title == "Stock VCA Mix voice" {
            self.rack
                .restore_layout(&Self::stock_vca_mix_voice_layout())
                .expect("bundled Stock VCA Mix voice layout");
        } else if title == "Stock Octave voice" {
            self.rack
                .restore_layout(&Self::stock_octave_voice_layout())
                .expect("bundled Stock Octave voice layout");
        } else if title == "Mid/Side stereo voice" {
            self.rack
                .restore_layout(&Self::stock_mid_side_voice_layout())
                .expect("bundled Mid/Side stereo voice layout");
        } else if title == "Quantized stock voice" {
            self.rack
                .restore_layout(&Self::stock_quantized_voice_layout())
                .expect("bundled Quantized stock voice layout");
        } else if title == "Stock sequenced voice" {
            self.rack
                .restore_layout(&Self::stock_sequenced_voice_layout())
                .expect("bundled Stock sequenced voice layout");
        } else if title == "Stock modular voice" {
            self.rack
                .restore_layout(&Self::stock_voice_layout())
                .expect("bundled Stock modular voice layout");
        } else if title == "Wavetable voice" {
            self.rack
                .restore_layout(&Self::wavetable_voice_layout())
                .expect("bundled wavetable voice layout");
        }
        self.documents.mark_unsaved();
        self.autosave_needed = true;
        self.capture_document();
    }
    fn stock_mix_voice_layout() -> Value {
        json!({"version":1,"zoom":0.75,"pan":[24,12],"positions":{
            "bass":[0,0],"fifth":[390,0],"upper":[780,0],
            "mix":[0,420],"scope":[270,420],"motion":[930,420],"out":[1320,420]
        }})
    }
    fn studio_mixer_layout() -> Value {
        let voice = crate::rack::panel_width("stock_vco");
        let noise = crate::rack::panel_width("stock_noise");
        let console: openpatch_core::Node = serde_json::from_value(json!({
            "id":"console","kind":"studio_mixer","params":{"channels":4}
        }))
        .expect("factory console");
        let console = crate::rack::module_width(&console);
        let scope = crate::rack::panel_width("scope");
        let out = crate::rack::panel_width("audio_stereo");
        let delay = crate::rack::panel_width("stereo_delay");
        let reverb = crate::rack::panel_width("stereo_reverb");
        let merge = crate::rack::panel_width("poly_merge");
        json!({"version":1,"zoom":0.6,"pan":[24,12],"positions":{
            "bass":[0,0],"lead":[voice,0],"air":[2.0*voice,0],"motion":[2.0*voice+noise,0],
            "console":[0,420],"scope":[console,420],"out":[console+scope,420],"guide":[console+scope+out,420],
            "delay":[0,840],"reverb":[delay,840],"return-a":[delay+reverb,840],"return-b":[delay+reverb+merge,840]
        }})
    }
    fn stock_noise_layout() -> Value {
        let noise = crate::rack::panel_width("stock_noise");
        let clock = crate::rack::panel_width("gate");
        let envelope = crate::rack::panel_width("adsr_eg");
        let body = crate::rack::panel_width("modal_resonator");
        let scope = crate::rack::panel_width("scope");
        let output = crate::rack::panel_width("audio_stereo");
        json!({"version":1,"zoom":0.7,"pan":[24,12],"positions":{
            "noise":[0,0],"clock":[noise,0],"envelope":[noise+clock,0],
            "amplifier":[noise+clock+envelope,0],
            "body":[0,420],"scope":[body,420],"out":[body+scope,420],
            "guide":[body+scope+output,420]
        }})
    }
    fn stock_process_layout() -> Value {
        let motion = crate::rack::panel_width("stock_lfo");
        let range = crate::rack::panel_width("gain");
        let clock = crate::rack::panel_width("gate");
        let voice = crate::rack::panel_width("stock_vco");
        let scope = crate::rack::panel_width("scope");
        let output = crate::rack::panel_width("audio_stereo");
        json!({"version":1,"zoom":0.75,"pan":[24,12],"positions":{
            "pitch-motion":[0,0],"pitch-range":[motion,0],
            "gate-clock":[motion+range,0],"process":[motion+range+clock,0],
            "voice":[0,420],"scope":[voice,420],
            "out":[voice+scope,420],"guide":[voice+scope+output,420]
        }})
    }
    fn stock_vca_mix_voice_layout() -> Value {
        let voice = crate::rack::panel_width("stock_vco");
        let mixer = crate::rack::panel_width("stock_vca_mix");
        let motion = crate::rack::panel_width("stock_lfo");
        let scope = crate::rack::panel_width("scope");
        let output = crate::rack::panel_width("audio_stereo");
        json!({"version":1,"zoom":0.65,"pan":[24,12],"positions":{
            "bass":[0,0],"low":[voice,0],"fifth":[2.0*voice,0],"air":[3.0*voice,0],
            "mix":[0,420],"motion":[mixer,420],"scope":[mixer+motion,420],
            "out":[mixer+motion+scope,420],"guide":[mixer+motion+scope+output,420]
        }})
    }
    fn stock_octave_voice_layout() -> Value {
        let chord = crate::rack::panel_width("poly_constant");
        let octave = crate::rack::panel_width("stock_octave");
        let motion = crate::rack::panel_width("stock_lfo");
        let scope = crate::rack::panel_width("scope");
        let voice = crate::rack::panel_width("stock_vco");
        let clock = crate::rack::panel_width("gate");
        let envelope = crate::rack::panel_width("adsr_eg");
        let amplifier = crate::rack::panel_width("stock_vca");
        json!({"version":1,"zoom":0.75,"pan":[24,12],"positions":{
            "chord":[0,0],"octave":[chord,0],"motion":[chord+octave,0],
            "scope":[chord+octave+motion,0],"guide":[chord+octave+motion+scope,0],
            "voice":[0,420],"clock":[voice,420],"envelope":[voice+clock,420],
            "amplifier":[voice+clock+envelope,420],"out":[voice+clock+envelope+amplifier,420]
        }})
    }
    fn stock_mid_side_voice_layout() -> Value {
        json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
            "left":[0,0],"right":[390,0],"width":[780,0],
            "motion":[0,420],"scope":[390,420],"out":[1050,420]
        }})
    }
    fn stock_quantized_voice_layout() -> Value {
        json!({"version":1,"zoom":0.75,"pan":[24,12],"positions":{
            "scale":[0,0],"scope":[300,0],"guide":[900,0],"voice":[1140,0],"filter":[1530,0],
            "sequence":[0,420],"amplifier":[780,420],"envelope":[990,420],"out":[1380,420]
        }})
    }
    fn stock_sequenced_voice_layout() -> Value {
        json!({"version":1,"zoom":0.75,"pan":[24,12],"positions":{
            "sequence":[0,0],"scope":[780,0],"guide":[1380,0],
            "voice":[0,420],"filter":[390,420],"amplifier":[690,420],"envelope":[900,420],"out":[1290,420]
        }})
    }
    fn stock_voice_layout() -> Value {
        json!({"version":1,"zoom":0.75,"pan":[24,12],"positions":{
            "voice":[0,0],"filter":[390,0],"amplifier":[690,0],"out":[900,0],
            "motion":[0,420],"envelope":[390,420],"clock":[780,420],"scope":[960,420],"guide":[1110,0]
        }})
    }
    fn annotated_drums_layout() -> Value {
        json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
            "guide":[0,0],"pads":[270,0],"spacer":[750,0],"mix":[825,0],"out":[1095,0],
            "kick":[0,420],"clap":[390,420],"hat":[780,420]
        }})
    }
    fn midi_gate_drums_layout() -> Value {
        json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
            "pads":[0,0],"mix":[510,0],"out":[900,0],
            "kick":[0,420],"clap":[390,420],"hat":[780,420]
        }})
    }
    fn midi_cc_voice_layout() -> Value {
        json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
            "controller":[0,0],"cutoff":[510,0],"out":[810,0],
            "vco":[0,420],"filter":[210,420],"vca":[450,420]
        }})
    }
    fn shift_register_layout() -> Value {
        json!({"version":1,"zoom":0.6,"pan":[24,12],"positions":{
            "register":[0,0],"scope":[420,0],"clock":[1080,0],"out":[1410,0],
            "notes1":[0,420],"voice1":[270,420],"notes2":[600,420],"voice2":[870,420],
            "notes3":[1200,420],"voice3":[1470,420],"mix":[1800,420]
        }})
    }
    fn random_values_layout() -> Value {
        json!({"version":1,"zoom":0.75,"pan":[24,12],"positions":{
            "values":[0,0],"scope":[330,0],"clock":[0,420],
            "notes":[330,420],"vco":[600,420],"out":[930,420]
        }})
    }

    fn random_motion_layout() -> Value {
        json!({"version":1,"zoom":0.65,"pan":[24,12],"positions":{
            "motion":[0,0],"range":[390,0],"notes":[570,0],"voice":[840,0],"out":[1170,0],
            "filter":[0,420],"envelope":[330,420],"amplifier":[720,420],"scope":[930,420],"guide":[1590,420]
        }})
    }

    fn wavetable_voice_layout() -> Value {
        json!({"version":1,"zoom":0.72,"pan":[24,12],"positions":{
            "voice":[0,0],"morph":[390,0],"filter":[780,0],"amplifier":[1080,0],"out":[1290,0],
            "clock":[0,420],"envelope":[180,420],"scope":[570,420],"guide":[1230,420]
        }})
    }
    fn switched_stereo_layout(&mut self) {
        let _ = self.rack.restore_layout(
            &json!({"version":1,"zoom":1.0,"pan":[24,12],"positions":{
                "pitches":[0,0],"split":[360,0],"select":[660,0],"clock":[840,0],
                "vco":[0,420],"route":[210,420],"left":[390,420],"right":[540,420],"out":[690,420]
            }}),
        );
    }
    fn gates_layout() -> Value {
        json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
            "clock":[0,0],"gates":[150,0],"vco":[390,0],"out":[600,0],
            "dry_envelope":[0,420],"dry_vca":[240,420],"echo_envelope":[390,420],"echo_vca":[630,420]
        }})
    }
    fn push_layout() -> Value {
        json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
            "push":[0,0],"buttons":[180,0],"vco":[420,0],"filter":[630,0],
            "envelope":[0,420],"vca":[240,420],"out":[390,420]
        }})
    }
    fn pulses_layout() -> Value {
        json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
            "buttons":[0,0],"sequence":[240,0],
            "vco":[0,420],"envelope":[210,420],"vca":[450,420],"out":[600,420]
        }})
    }
    fn waveform_layout() -> Value {
        json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
            "notes":[0,0],"master":[360,0],"voice":[660,0],
            "motion":[0,420],"mix":[300,420],"out":[690,420]
        }})
    }

    fn stacked_layout() -> Value {
        json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
            "notes":[0,0],"vco":[360,0],"out":[570,0],
            "vibrato":[0,420],"depth":[300,420],"transpose":[480,420]
        }})
    }

    fn mixing_layout() -> Value {
        json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
            "mix":[0,0],"morph":[390,0],"motion":[630,0],"out":[930,0],
            "voice1":[0,420],"voice2":[210,420],"voice3":[420,420],"voice4":[630,420]
        }})
    }

    fn sequence_layout(&mut self) {
        let _ = self.rack.restore_layout(&json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
            "sequence":[0,0],"vco":[780,0],"filter":[990,0],"envelope":[0,420],"vca":[240,420],"accent":[390,420],"out":[540,420]
        }}));
    }
    fn perform_action(&mut self, action: PendingAction) {
        match action {
            PendingAction::NewTemplate => self.new_from_template(false),
            PendingAction::Replace(patch, title) => self.replace_now(patch, &title),
            PendingAction::Load(name) => self.load_document(&name, false),
            PendingAction::Recover(name) => self.load_document(&name, true),
            PendingAction::Close | PendingAction::DiscardClose => {
                let sealed = self.shared.try_lock().ok().map(|mut live| {
                    let revision = live.revision();
                    live.seal_for_close(revision)
                });
                if matches!(sealed, Some(Ok(()))) {
                    self.clear_recovery();
                    self.allow_close = true;
                } else {
                    self.deferred_action = Some(PendingAction::DiscardClose);
                }
            }
        }
    }
    fn refresh_recovery_list(&mut self) {
        if let Some(store) = &self.projects {
            self.recovery_names = store.recoveries();
            self.recovery_details = self
                .recovery_names
                .iter()
                .map(|name| {
                    let detail = match store.load_recovery(name) {
                        Ok(loaded) => format!(
                            "{} modules · {} cables\n{}",
                            loaded.document.patch.nodes.len(),
                            loaded.document.patch.cables.len(),
                            loaded
                                .document
                                .patch
                                .nodes
                                .iter()
                                .take(6)
                                .map(|node| node.id.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        Err(error) => format!("This copy could not be read: {error}"),
                    };
                    (name.clone(), detail)
                })
                .collect();
        }
    }
    fn clear_recovery(&mut self) {
        if let Some(store) = &self.projects {
            if let Err(error) = store.clear_recovery() {
                self.status = format!("Could not clear recovery: {error}");
            }
            if let Some(name) = &self.restored_recovery {
                if let Err(error) = store.discard_recovery(name) {
                    self.status = format!("Could not clear recovered copy: {error}");
                }
            }
        }
        self.restored_recovery = None;
        self.autosaved_document = None;
        self.refresh_recovery_list();
    }
    fn save_current(&mut self) {
        if let Some(name) = self.active_file.clone() {
            self.save_document(&name, false);
        } else {
            self.file_dialog(FileMode::Save);
        }
    }
    fn save_document(&mut self, name: &str, new_file: bool) {
        // This pair belongs to one observed UI revision. Never re-sync between
        // capturing its graph and layout, or while doing disk I/O.
        self.sync();
        let document = match self.capture_persisted_document() {
            Ok(document) => document,
            Err(error) => {
                self.observe_document();
                self.status = format!(
                    "Save needs current live state: {error}. Try again when the engine is ready."
                );
                self.restore_save_continuation();
                return;
            }
        };
        let result = self
            .projects
            .as_ref()
            .ok_or_else(|| "Project storage is unavailable".to_string())
            .and_then(|store| {
                if new_file {
                    store.save_new(name, &document)
                } else {
                    store.save_existing(
                        name,
                        &document,
                        self.active_token
                            .as_ref()
                            .ok_or("No saved project selected")?,
                    )
                }
            });
        match result {
            Ok(token) => {
                let warning = token.durability_warning.clone();
                self.active_file = Some(name.into());
                self.active_token = Some(token);
                self.documents.mark_saved(&document);
                self.title = name.trim_end_matches(".openpatch").into();
                self.filename = name.into();
                self.file_mode = None;
                self.refresh_files();
                self.status = warning.unwrap_or_else(|| format!("Saved {name}"));
                self.clear_recovery();
                self.sync();
                self.capture_document();
                if let Some(action) = self.after_save.take() {
                    // A concurrent MCP edit after the snapshot must remain
                    // dirty and get another Save/Discard/Cancel decision.
                    self.request_action(action);
                }
            }
            Err(error) => {
                self.status = format!("Save failed: {error}. Use Save As to keep a separate copy.");
                self.restore_save_continuation();
            }
        }
    }
    fn restore_save_continuation(&mut self) {
        if let Some(action) = self.after_save.take() {
            self.file_mode = None;
            self.confirm_discard = Some(action);
        }
    }
    fn load_document(&mut self, name: &str, recovery: bool) {
        self.load_document_guarded(name, recovery, false);
    }
    fn load_document_guarded(&mut self, name: &str, recovery: bool, require_saved: bool) {
        let mut changed = false;
        let expected_revision = self.revision;
        let result = self
            .projects
            .as_ref()
            .ok_or_else(|| "Project storage is unavailable".to_string())
            .and_then(|store| {
                if recovery {
                    store.load_recovery(name)
                } else {
                    store.load(name)
                }
            })
            .and_then(|loaded| {
                let applied = apply_document_guarded(
                    &self.shared,
                    expected_revision,
                    &loaded.document,
                    |live| {
                        if require_saved {
                            if let Err(error) = self.check_saved_content(live) {
                                changed = true;
                                return Err(error);
                            }
                        }
                        Ok(())
                    },
                )?;
                Ok((loaded, applied))
            });
        match result {
            Ok((loaded, applied)) => {
                self.history_layouts
                    .insert(self.state_id, self.rack.layout());
                self.patch = applied.patch;
                let positions_locked = self.rack.positions_locked();
                let room_brightness = self.rack.room_brightness();
                self.replace_rack_for_document(applied.rack, applied.revision);
                self.rack.set_positions_locked(positions_locked);
                self.rack.set_room_brightness(room_brightness);
                self.revision = applied.revision;
                self.state_id = applied.state_id;
                self.history_layouts
                    .insert(self.state_id, self.rack.layout());
                self.panic_notes();
                if let Some(audio) = &self.audio {
                    audio.set_playing(false);
                }
                self.clear_recovery();
                self.documents.invalidate();
                let document = self.capture_document();
                self.documents.mark_saved(&document);
                self.active_file = if loaded.imported || recovery {
                    None
                } else {
                    Some(name.into())
                };
                self.active_token = loaded.token;
                if recovery {
                    self.documents.mark_unsaved();
                    self.autosave_needed = true;
                    self.restored_recovery = Some(name.into());
                    self.title = "Recovered project".into();
                    self.status =
                        "Recovered unsaved work — Save As to keep it. Playback is paused.".into();
                } else {
                    self.title = name
                        .trim_end_matches(".openpatch")
                        .trim_end_matches(".json")
                        .into();
                    self.status = if loaded.imported {
                        format!("Imported {name} — Save As creates a native project; original JSON is preserved.")
                    } else {
                        format!("Opened {name}")
                    };
                }
                self.filename = name.into();
                self.file_mode = None;
                self.recovery_open = false;
                self.sync();
                self.capture_document();
            }
            Err(error) => {
                self.status = format!("Open failed: {error}");
                if changed {
                    self.confirm_discard = Some(if recovery {
                        PendingAction::Recover(name.into())
                    } else {
                        PendingAction::Load(name.into())
                    });
                }
                self.sync();
                self.capture_document();
            }
        }
    }
    fn autosave(&mut self) {
        if self.smoke_test.is_some() || self.allow_close {
            return;
        }
        self.observe_document();
        if !self.dirty() {
            self.autosave_needed = false;
            if self.autosaved_document.is_some() {
                self.clear_recovery();
            }
            return;
        }
        if !self.autosave_needed {
            return;
        }
        let now = Instant::now();
        if now.duration_since(self.last_autosave_attempt) < Duration::from_secs(2)
            || (now.duration_since(self.last_document_change) < Duration::from_secs(2)
                && now.duration_since(self.last_autosave_attempt) < Duration::from_secs(15))
        {
            return;
        }
        self.last_autosave_attempt = now;
        let document = match self.capture_persisted_document() {
            Ok(document) => document,
            Err(error) => {
                self.status = format!("Recovery is waiting for current live state: {error}");
                return;
            }
        };
        if self
            .autosaved_document
            .as_ref()
            .is_some_and(|old| old.equivalent(&document))
        {
            self.autosave_needed = false;
            return;
        }
        self.last_autosave_attempt = now;
        if let Some(store) = &self.projects {
            match store.save_recovery(&document) {
                Ok(()) => {
                    self.autosaved_document = Some(document);
                    self.autosave_needed = false;
                }
                Err(error) => self.status = format!("Recovery copy could not be saved: {error}"),
            }
        }
    }
    fn add_module(&mut self, kind: &str, placement: Option<(egui::Pos2, bool)>) -> bool {
        let id = (1..)
            .map(|i| format!("{kind}-{i}"))
            .find(|id| !self.patch.nodes.iter().any(|node| node.id == *id))
            .unwrap();
        let mut edits = vec![json!({"tool":"add_module","arguments":{"id":id,"kind":kind}})];
        if matches!(kind, "output" | "audio_out" | "audio_stereo") && self.patch.output.is_none() {
            edits.push(json!({"tool":"set_output","arguments":{"id":id,"port":"out"}}));
        }
        let base_state_id = self.state_id;
        let base_layout = self.rack.layout();
        let receipt = match execute_shared(
            &self.shared,
            "apply_edits",
            json!({"edits":edits,"expected_revision":self.revision}),
        ) {
            Ok(receipt) => receipt,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        self.install_add_receipt(&receipt, id, placement, base_state_id, base_layout)
            .expect("shared add command returns a committed graph snapshot");
        self.sync();
        self.status = format!("Added {}", module_title(kind));
        true
    }

    fn install_add_receipt(
        &mut self,
        receipt: &Value,
        id: String,
        placement: Option<(egui::Pos2, bool)>,
        base_state_id: u64,
        base_layout: Value,
    ) -> Result<(), String> {
        let patch: Patch =
            serde_json::from_value(receipt["patch"].clone()).map_err(|error| error.to_string())?;
        let revision = receipt["revision"]
            .as_u64()
            .ok_or("add receipt has no revision")?;
        let state_id = receipt["state_id"]
            .as_u64()
            .ok_or("add receipt has no state identity")?;
        if !patch.nodes.iter().any(|node| node.id == id) {
            return Err("add receipt is missing the new module".into());
        }
        self.rack.clear_construction_animation();
        self.history_layouts.insert(base_state_id, base_layout);
        self.patch = patch;
        self.revision = revision;
        self.state_id = state_id;
        self.can_undo = receipt["history"]["undo_steps"]
            .as_u64()
            .is_some_and(|n| n > 0);
        self.can_redo = receipt["history"]["redo_steps"]
            .as_u64()
            .is_some_and(|n| n > 0);
        match placement {
            Some((position, reveal)) => self.rack.place_added_module(id, position, reveal),
            None => self.rack.select_only(id),
        }
        self.capture_document();
        self.history_layouts.insert(state_id, self.rack.layout());
        Ok(())
    }

    fn resize_blank(&mut self, proposal: crate::rack::BlankResizeProposal) -> bool {
        self.rack.cancel_module_drag();
        if proposal.movement.revision != self.revision {
            self.status = "Rack changed; retry the resize".into();
            self.sync();
            return false;
        }
        if let Err(error) = self.rack.validate_blank_resize(&self.patch, &proposal) {
            self.status = error;
            return false;
        }
        if proposal.before_hp == proposal.width_hp
            && proposal.movement.before == proposal.movement.after
        {
            return true;
        }
        let base_state_id = self.state_id;
        let base_layout = self.rack.layout();
        let result = execute_shared(
            &self.shared,
            "set_module_content",
            json!({
                "id":proposal.id,"content":{"type":"blank","width_hp":proposal.width_hp},
                "expected_revision":proposal.movement.revision
            }),
        );
        let receipt = match result {
            Ok(receipt) => receipt,
            Err(error) => {
                self.status = error;
                self.sync();
                return false;
            }
        };
        // This exact receipt owns both content and its local position. A newer
        // MCP revision may already exist; never label geometry with that revision.
        self.install_move_receipt(&receipt, &proposal.movement, base_state_id, base_layout)
            .expect("validated resize returns an exact committed graph receipt");
        self.sync();
        self.observe_document();
        self.status = format!("Blank resized to {} HP", proposal.width_hp);
        true
    }

    fn move_modules(&mut self, movement: ModuleMove) -> bool {
        self.rack.cancel_module_drag();
        if movement.revision != self.revision {
            self.status = "Rack changed; retry the move".into();
            self.sync();
            return false;
        }
        if movement.before == movement.after {
            return false;
        }
        let base_state_id = self.state_id;
        let base_layout = self.rack.layout();
        // Prepare and validate geometry before recording the shared history step.
        let prepared = (|| {
            if movement
                .before
                .keys()
                .any(|id| !self.patch.nodes.iter().any(|node| &node.id == id))
            {
                return Err("Moved module is no longer in the patch".to_string());
            }
            let mut candidate = Rack::default();
            candidate.restore_layout(&base_layout)?;
            candidate.apply_module_move(&movement)?;
            Ok(())
        })();
        if let Err(error) = prepared {
            self.status = error;
            return false;
        }
        let result = self
            .shared
            .lock()
            .map_err(|_| "Session lock poisoned".to_string())
            .and_then(|mut live| live.record_layout_checkpoint(movement.revision));
        let receipt = match result {
            Ok(receipt) => receipt,
            Err(error) => {
                self.status = error;
                self.sync();
                return false;
            }
        };
        self.install_move_receipt(&receipt, &movement, base_state_id, base_layout)
            .expect("validated move and exact checkpoint receipt");
        self.sync();
        if let Ok(live) = self.shared.try_lock() {
            let retained: BTreeSet<_> = live.history_state_ids().collect();
            self.history_layouts.retain(|id, _| retained.contains(id));
        }
        self.observe_document();
        self.status = if movement.after.len() == 1 {
            "Module moved".into()
        } else {
            format!("Moved {} modules", movement.after.len())
        };
        true
    }

    fn install_move_receipt(
        &mut self,
        receipt: &Value,
        movement: &ModuleMove,
        base_state_id: u64,
        base_layout: Value,
    ) -> Result<(), String> {
        let patch: Patch =
            serde_json::from_value(receipt["patch"].clone()).map_err(|error| error.to_string())?;
        let revision = receipt["revision"]
            .as_u64()
            .ok_or("move receipt has no revision")?;
        let state_id = receipt["state_id"]
            .as_u64()
            .ok_or("move receipt has no state identity")?;
        // No Session read here: newer MCP commits must not acquire this move's geometry.
        self.rack.apply_module_move(movement)?;
        self.rack.clear_construction_animation();
        self.history_layouts.insert(base_state_id, base_layout);
        self.patch = patch;
        self.revision = revision;
        self.state_id = state_id;
        self.can_undo = receipt["history"]["undo_steps"]
            .as_u64()
            .is_some_and(|n| n > 0);
        self.can_redo = receipt["history"]["redo_steps"]
            .as_u64()
            .is_some_and(|n| n > 0);
        self.history_layouts.insert(state_id, self.rack.layout());
        Ok(())
    }

    fn duplicate_modules(
        &mut self,
        ids: &[String],
        cable_mode: DuplicateCableMode,
        expected_revision: u64,
        select_copies: bool,
    ) -> bool {
        if expected_revision != self.revision {
            self.status = "Rack changed; retry duplication from the current patch".into();
            return false;
        }
        if cable_mode == DuplicateCableMode::All && !duplicate_all_supported(&self.patch, ids) {
            self.status = "Copying outgoing cables requires engine v2. Duplicate without cables remains available.".into();
            return false;
        }
        let mut reserved: BTreeSet<String> = self
            .patch
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect();
        let mut mappings = Vec::new();
        for id in ids {
            if !self.patch.nodes.iter().any(|node| &node.id == id) {
                self.status = "Selected module no longer exists".into();
                return false;
            }
            let new_id = (1..)
                .map(|index| {
                    let suffix = format!("-copy-{index}");
                    format!("{}{}", &id[..id.len().min(64 - suffix.len())], suffix)
                })
                .find(|candidate| !reserved.contains(candidate))
                .unwrap();
            reserved.insert(new_id.clone());
            mappings.push((id.clone(), new_id));
        }
        let placement = match self.rack.duplicate_placement(&self.patch, &mappings) {
            Ok(placement) => placement,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        let modules: Vec<_> = mappings
            .iter()
            .map(|(id, new_id)| json!({"id":id,"new_id":new_id}))
            .collect();
        let base_state_id = self.state_id;
        let base_layout = self.rack.layout();
        let receipt = match execute_shared(
            &self.shared,
            "duplicate_modules",
            json!({"modules":modules,"cable_mode":cable_mode.as_str(),"expected_revision":expected_revision}),
        ) {
            Ok(receipt) => receipt,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        self.install_duplicate_receipt(
            &receipt,
            placement,
            base_state_id,
            base_layout,
            select_copies,
        )
        .expect("shared duplicate command returns a committed graph snapshot");
        self.sync();
        self.status = format!(
            "Duplicated {} module{}{}",
            ids.len(),
            if ids.len() == 1 { "" } else { "s" },
            match cable_mode {
                DuplicateCableMode::None => "",
                DuplicateCableMode::Inputs => " with inputs",
                DuplicateCableMode::All => " with cables",
            }
        );
        true
    }

    /// Install the exact commit before polling newer MCP revisions. This path
    /// must not acquire the shared mutex: another request may already own it.
    fn install_duplicate_receipt(
        &mut self,
        receipt: &Value,
        placement: BTreeMap<String, egui::Pos2>,
        base_state_id: u64,
        base_layout: Value,
        select_copies: bool,
    ) -> Result<(), String> {
        let patch: Patch =
            serde_json::from_value(receipt["patch"].clone()).map_err(|error| error.to_string())?;
        let revision = receipt["revision"]
            .as_u64()
            .ok_or("duplicate receipt has no revision")?;
        let state_id = receipt["state_id"]
            .as_u64()
            .ok_or("duplicate receipt has no state identity")?;
        self.rack.clear_construction_animation();
        self.history_layouts.insert(base_state_id, base_layout);
        self.patch = patch;
        self.revision = revision;
        self.state_id = state_id;
        self.can_undo = receipt["history"]["undo_steps"]
            .as_u64()
            .is_some_and(|count| count > 0);
        self.can_redo = receipt["history"]["redo_steps"]
            .as_u64()
            .is_some_and(|count| count > 0);
        self.rack
            .apply_duplicate_placement(placement, select_copies);
        self.capture_document();
        self.history_layouts.insert(state_id, self.rack.layout());
        Ok(())
    }

    fn save_or_load(&mut self, mode: FileMode) {
        if matches!(mode, FileMode::WavetableImport | FileMode::WavetableExport) {
            self.apply_wavetable_file(mode);
            return;
        }
        let name = self.filename.clone();
        if mode == FileMode::Save {
            self.save_document(&name, true);
        } else {
            self.file_mode = None;
            self.request_action(PendingAction::Load(name));
        }
    }
    fn parameter_history_shortcut(&mut self, ctx: &egui::Context) {
        let modifiers = ctx.input(|i| i.modifiers);
        if (modifiers.command || modifiers.ctrl)
            && !modifiers.alt
            && !ctx.input(|i| i.pointer.any_down())
            && ctx.input_mut(|i| i.consume_key(modifiers, egui::Key::Z))
        {
            self.rack.prepare_parameter_history(ctx);
            self.history(modifiers.shift);
        }
    }

    fn global_shortcuts(&mut self, ctx: &egui::Context) {
        if !self.library_enabled()
            || !ctx.input(|i| i.focused)
            || self.rack.annotation_keyboard_active()
            || self.cable_palette_ui.keyboard_active()
            || (self.browser_open && self.library.keyboard_active())
            || egui::Popup::is_any_open(ctx)
            || egui::DragAndDrop::has_payload_of_type::<LibraryDrag>(ctx)
        {
            return;
        }
        if ctx.wants_keyboard_input() {
            if self.rack.parameter_keyboard_active(ctx) {
                self.parameter_history_shortcut(ctx);
            }
            return;
        }
        if !(self.browser_open && self.library.keyboard_active())
            && !self.cable_palette_ui.keyboard_active()
            && self.rack.navigation_idle()
            && ctx.input(|i| i.modifiers == egui::Modifiers::NONE && !i.pointer.any_down())
        {
            let direction = ctx.input_mut(|input| {
                if input.consume_key(egui::Modifiers::NONE, egui::Key::OpenBracket) {
                    Some(false)
                } else if input.consume_key(egui::Modifiers::NONE, egui::Key::CloseBracket) {
                    Some(true)
                } else {
                    None
                }
            });
            if direction.is_some_and(|forward| self.cable_palette.cycle(forward)) {
                self.save_host_settings();
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.toggle_play();
        }
        if ctx.input(|i| {
            i.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        pressed: true,
                        repeat: false,
                        ..
                    }
                )
            })
        }) {
            self.browse();
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            self.after_save = None;
            if ctx.input(|i| i.modifiers.shift) {
                self.file_dialog(FileMode::Save);
            } else {
                self.save_current();
            }
        }
        if ctx.input(|i| {
            i.modifiers.command
                && !i.modifiers.shift
                && !i.modifiers.alt
                && i.key_pressed(egui::Key::N)
        }) {
            self.request_action(PendingAction::NewTemplate);
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O)) {
            self.file_dialog(FileMode::Open);
        }
        self.parameter_history_shortcut(ctx);
        if self.rack.navigation_idle()
            && ctx.input(|i| {
                (i.modifiers.command || i.modifiers.ctrl)
                    && !i.modifiers.alt
                    && !i.pointer.any_down()
            })
        {
            if ctx.input(|i| i.key_pressed(egui::Key::Num0)) {
                self.rack.set_zoom(1.0);
            } else if ctx
                .input(|i| i.key_pressed(egui::Key::Equals) || i.key_pressed(egui::Key::Plus))
            {
                self.rack.zoom_step(true);
            } else if ctx.input(|i| i.key_pressed(egui::Key::Minus)) {
                self.rack.zoom_step(false);
            }
        }
    }

    fn cable_presentation_controls(&mut self, ui: &mut egui::Ui) {
        ui.label("Cables and plugs");
        for (key, label, reset, help) in [
            ("cable_opacity", "Cable opacity", 1.0, "Fade cable bodies to reveal the panels. Hover a connected port to reveal its cables. Double-click to reset to 100%."),
            ("cable_tension", "Cable tension", 0.5, "Choose loose or taut cable curves without moving their plugs. Double-click to reset to 50%."),
            ("plug_light_brightness", "Plug light brightness", 1.0, "Adjust live signal light emission at cable plugs. At 0%, signal measurement is disabled; module lights and Scope stay available. Double-click to reset to 100%."),
        ] {
            let mut presentation = self.rack.cable_presentation();
            let amount = match key {
                "cable_opacity" => presentation.opacity,
                "cable_tension" => presentation.tension,
                _ => presentation.plug_brightness,
            };
            let mut percent = amount * 100.0;
            let mut response = ui.push_id(key, |ui| {
                ui.add(egui::Slider::new(&mut percent, 0.0..=100.0)
                    .suffix("%") .text(label).step_by(0.1))
            }).inner.on_hover_text(help);
            // egui's rail senses dragging rather than clicks. Read its real
            // pointer double-click while hovered so reset works on the rail as
            // well as on the percentage readout.
            let reset_requested = response.hovered()
                && ui.input(|input| input.pointer.button_double_clicked(egui::PointerButton::Primary));
            if reset_requested {
                percent = reset * 100.0;
                response.mark_changed();
            }
            if response.changed() {
                match key {
                    "cable_opacity" => presentation.opacity = percent / 100.0,
                    "cable_tension" => presentation.tension = percent / 100.0,
                    _ => presentation.plug_brightness = percent / 100.0,
                }
                self.set_cable_presentation(presentation);
            }
            // Dragging previews every frame. A release, keyboard edit, direct
            // entry or double-click reset publishes only the host preference.
            if response.drag_stopped() || reset_requested
                || (response.changed() && !response.dragged()) {
                self.save_host_settings();
            }
            #[cfg(test)]
            ui.ctx().data_mut(|data| data.insert_temp(
                egui::Id::new(("cable-presentation-control", key)), response,
            ));
        }
    }

    fn footer(&mut self, ctx: &egui::Context) {
        let timing = self.diagnostic_snapshot();
        let timing_label = timing_footer(timing.as_ref());
        let timing_help = timing_explanation(timing_status(timing.as_ref()));
        egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::new().fill(CHROME).inner_margin(egui::Margin::symmetric(12,5)))
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Audio health stays visible even when a status message is long.
                    if ui
                        .button(RichText::new(&timing_label).size(10.0).color(ACCENT))
                        .on_hover_text(format!("{timing_help}\nOpen Audio performance for details."))
                        .clicked()
                    {
                        self.audio_performance_open = true;
                    }
                    if self.audio.is_none() {
                        if ui.button("Retry audio").on_hover_text(&self.device).clicked() {
                            self.reopen_audio(self.output_device.clone());
                        }
                    } else if let Some(audio)=&self.audio {
                        ui.label(RichText::new(format!("{} Hz",audio.sample_rate)).size(10.0).color(MUTED)).on_hover_text(&self.device);
                        ui.add(egui::ProgressBar::new(audio.peak().min(1.0)).desired_width(48.0).desired_height(5.0).fill(ACCENT))
                            .on_hover_text("Combined output peak · after master volume and graph crossfade");
                        ui.label(RichText::new(if audio.is_playing(){"Playing"}else{"Paused"}).size(10.0).color(MUTED));
                        if audio.solver_failures()!=0 {
                            ui.colored_label(ACCENT,"Solver holds").on_hover_text(format!("{} circuit solves reached the bounded work limit. The last valid capacitor state was retained. Reduce drive, resonance, or modulation.",audio.solver_failures()));
                        }
                    }
                    self.mcp_status(ui);
                    ui.separator();
                    ui.allocate_ui_with_layout(ui.available_size(),egui::Layout::left_to_right(egui::Align::Center),|ui| {
                        ui.label(RichText::new(format!("{} modules · {} cables",self.patch.nodes.len(),self.patch.cables.len())).size(10.0).color(MUTED))
                            .on_hover_text(if self.patch.version==2 {"Polyphonic engine v2"}else{"Legacy mono engine"});
                        if self.rack.selection_count()>1 {
                            ui.label(RichText::new(format!("{} selected",self.rack.selection_count())).size(10.0).color(ACCENT))
                                .on_hover_text("Shift-click headers to select modules. Cmd/Ctrl+D duplicates the selection; add Shift to include incoming cables.");
                        }
                        ui.separator();
                        ui.add(egui::Label::new(RichText::new(&self.status).size(10.0).color(ACCENT)).truncate()).on_hover_text(&self.status);
                    });
                });
            });
    }
    fn select_analysis(&mut self, tab: Option<AnalysisTab>) {
        self.analysis_tab = tab;
        if tab == Some(AnalysisTab::Scope) {
            self.scope.open = true;
        } else if tab.is_none() {
            self.scope.open = false;
        }
        // Visuals keeps an already-active scope armed, including frozen captures.
    }
    fn toggle_analysis(&mut self, tab: AnalysisTab) {
        self.select_analysis((self.analysis_tab != Some(tab)).then_some(tab));
    }
    fn analysis_dock(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.analysis_tab else {
            return;
        };
        let available = ctx.available_rect().height();
        let max_height = (available - 220.0).clamp(220.0, 500.0);
        egui::TopBottomPanel::bottom("analysis-dock")
            .default_height(260.0_f32.min(max_height))
            .height_range(220.0..=max_height)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(27, 31, 31))
                    .inner_margin(8),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (value, label) in [
                        (AnalysisTab::Scope, "Scope"),
                        (AnalysisTab::Visuals, "Visuals"),
                    ] {
                        if ui.selectable_label(tab == value, label).clicked() {
                            self.select_analysis(Some(value));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button("×")
                            .on_hover_text("Close analysis dock")
                            .clicked()
                        {
                            self.select_analysis(None);
                        }
                        if tab == AnalysisTab::Visuals && ui.button("Edit GLSL").clicked() {
                            self.shader_open = true;
                        }
                    });
                });
                ui.separator();
                match self.analysis_tab {
                    Some(AnalysisTab::Scope) => self.scope.contents(ui, &self.patch, &self.catalog),
                    Some(AnalysisTab::Visuals) => {
                        ui.horizontal(|ui| {
                            for (mode, name) in crate::visuals::MODES {
                                let response =
                                    ui.selectable_value(&mut self.visual_mode, mode, name);
                                #[cfg(test)]
                                ui.ctx().data_mut(|d| {
                                    d.insert_temp(
                                        egui::Id::new(("visual-mode", mode)),
                                        response.rect,
                                    )
                                });
                                let _ = response;
                            }
                        });
                        let (wave, peak) = self
                            .audio
                            .as_ref()
                            .map(|audio| (audio.waveform(), audio.peak()))
                            .or_else(|| {
                                self.smoke_test.as_ref().and_then(|_| {
                                    ctx.data(|d| {
                                        d.get_temp::<([f32; 256], f32)>(egui::Id::new(
                                            "ferrofluid-smoke-audio",
                                        ))
                                    })
                                })
                            })
                            .unwrap_or(([0.0; 256], 0.0));
                        if let Some(v) = &self.visualizer {
                            v.paint(
                                ui,
                                ui.available_size().max(Vec2::new(1.0, 1.0)),
                                self.start.elapsed().as_secs_f32(),
                                peak,
                                wave,
                                self.visual_mode,
                            );
                        } else {
                            ui.label(&self.shader_status);
                        }
                    }
                    None => {}
                }
            });
    }

    fn agent_motion_controls(&mut self, ui: &mut egui::Ui) {
        use crate::construction_motion::MotionMode;
        let allowed = self.library_enabled()
            && ui.ctx().input(|input| input.focused)
            && !egui::DragAndDrop::has_payload_of_type::<LibraryDrag>(ui.ctx());
        ui.add_enabled_ui(allowed, |ui| {
            ui.label("Menu & agent animations");
            ui.horizontal(|ui| {
                for mode in [MotionMode::Full, MotionMode::Reduced, MotionMode::Off] {
                    if ui.selectable_label(self.construction.motion_mode() == mode, mode.label()).clicked() {
                        self.construction.set_motion_mode(mode);
                        self.rack.set_construction_motion_mode(mode);
                    }
                }
            });
            // Read Rack every pass: manual navigation turns follow off immediately.
            let mut follow = self.rack.agent_follow_enabled();
            if ui.add_enabled(self.construction.motion_mode() != MotionMode::Off,
                egui::Checkbox::new(&mut follow, "Follow agent edits"))
                .on_hover_text("Reveal future agent edits with a small camera pan. Full moves smoothly; Reduced reveals immediately. Manual rack navigation turns follow off. Remembered for this window.")
                .on_disabled_hover_text("Choose Full or Reduced motion to follow agent edits.")
                .changed()
            {
                self.rack.set_agent_follow(follow);
            }
        });
    }

    fn service_agent_follow(&mut self, ctx: &egui::Context) {
        if !self.library_enabled()
            || !ctx.input(|input| input.focused)
            || ctx.wants_keyboard_input()
            || self.rack.annotation_keyboard_active()
            || self.cable_palette_ui.keyboard_active()
            || (self.browser_open && self.library.keyboard_active())
            || egui::Popup::is_any_open(ctx)
            || egui::DragAndDrop::has_payload_of_type::<LibraryDrag>(ctx)
        {
            self.rack.suspend_agent_follow();
        }
    }

    fn rack_workspace(&mut self, ctx: &egui::Context, library_action: Option<LibraryAction>) {
        self.service_agent_follow(ctx);
        let mut action = None;
        let mut pending_add = None;
        self.rack.set_cable_palette(
            self.cable_palette.selection(),
            &self.cable_palette.presets()[self.cable_palette.active_index()].colors,
        );
        if !self.library_enabled() || !ctx.input(|i| i.focused) {
            egui::DragAndDrop::clear_payload(ctx);
        }
        let drag = egui::DragAndDrop::payload::<LibraryDrag>(ctx);
        self.rack.set_shortcuts_enabled(
            drag.is_none()
                && !self.settings_open
                && !self.audio_performance_open
                && !self.shader_open
                && !self.mcp_session.is_open()
                && !self.cable_palette_ui.keyboard_active()
                && !(self.browser_open && self.library.keyboard_active())
                && self.preset_dialog.is_none()
                && self.template_save.is_none(),
        );
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(0))
            .show(ctx, |ui| {
                let rate = self
                    .audio
                    .as_ref()
                    .map_or(self.patch.sample_rate, |a| a.sample_rate);
                let interactive = self.file_mode.is_none()
                    && !self.settings_open
                    && self.confirm_discard.is_none()
                    && !self.recovery_open
                    && !self.confirm_engine_upgrade
                    && self.preset_dialog.is_none()
                    && self.template_save.is_none()
                    && !self.audio_performance_open
                    && !self.mcp_session.is_open()
                    && !self.allow_close;
                let canvas = ui.available_rect_before_wrap();
                if self.smoke_test.is_some() {
                    ctx.data_mut(|data| {
                        data.insert_temp(egui::Id::new("smoke-rack-canvas"), canvas)
                    });
                }
                ui.add_enabled_ui(interactive && drag.is_none(), |ui| {
                    action = self
                        .rack
                        .show(ui, &self.patch, &self.catalog, rate, self.revision);
                });
                if self.library_enabled() {
                    if let Some(payload) = &drag {
                        if let Some(pointer) = ctx.input(|i| i.pointer.hover_pos()) {
                            let over_rack = canvas.contains(pointer)
                                && ctx.layer_id_at(pointer) == Some(ui.layer_id());
                            if over_rack {
                                match self.rack.library_placement(
                                    &self.patch,
                                    &payload.kind,
                                    canvas,
                                    pointer,
                                ) {
                                    Ok(position) => {
                                        self.rack.paint_library_preview(
                                            ui.painter(),
                                            &payload.kind,
                                            canvas,
                                            position,
                                        );
                                        if ctx.input(|i| {
                                            i.pointer.button_released(egui::PointerButton::Primary)
                                        }) {
                                            egui::DragAndDrop::take_payload::<LibraryDrag>(ctx);
                                            pending_add =
                                                Some((payload.kind.clone(), position, false));
                                        }
                                    }
                                    Err(error) => self.status = error,
                                }
                            }
                        }
                    } else if let Some(LibraryAction::Add(kind)) = &library_action {
                        match self.rack.library_placement(
                            &self.patch,
                            kind,
                            canvas,
                            canvas.center(),
                        ) {
                            Ok(position) => pending_add = Some((kind.clone(), position, true)),
                            Err(error) => self.status = error,
                        }
                    }
                }
            });
        match action {
            Some(RackAction::Command(name, args)) => {
                let manual_takeover = name == "trigger_module"
                    && (args["action"] == "push_press"
                        || args["id"].as_str().is_some_and(|id| {
                            self.patch
                                .nodes
                                .iter()
                                .any(|node| node.id == id && node.kind == "stock_seq3")
                        }));
                if !manual_takeover || self.finish_previous_manual_click() {
                    self.command(name, args);
                }
            }
            Some(RackAction::PaletteConnect {
                arguments,
                selection,
            }) => {
                self.connect_from_palette(arguments, selection);
            }
            Some(RackAction::ScalePaint { arguments, gesture }) => {
                self.paint_scale(arguments, gesture)
            }
            Some(RackAction::ParameterEditAndNavigate { arguments, forward }) => {
                self.edit_parameter_and_navigate(arguments, forward);
            }
            Some(RackAction::ManualClick { id, press, release }) => {
                if self
                    .patch
                    .nodes
                    .iter()
                    .any(|n| n.id == id && n.kind == "adsr_eg")
                {
                    self.manual_gate_click(ctx, &id);
                } else if self
                    .patch
                    .nodes
                    .iter()
                    .any(|node| node.id == id && node.kind == "stock_seq3")
                {
                    self.manual_button_click(ctx, &id, press, release);
                } else {
                    self.manual_click(&id, press, release);
                }
            }
            Some(RackAction::Duplicate {
                ids,
                cable_mode,
                revision,
                select_copies,
            }) => {
                self.duplicate_modules(&ids, cable_mode, revision, select_copies);
            }
            Some(RackAction::OpenBrowser) => self.browse(),
            Some(RackAction::Preset {
                id,
                kind,
                revision,
                mode,
            }) => {
                self.preset_dialog = Some(PresetDialog::new(id, kind, revision, mode));
            }
            Some(RackAction::Inspect { lane, id, port }) => {
                self.scope.inspect(lane, id, port);
                self.select_analysis(Some(AnalysisTab::Scope));
            }
            Some(RackAction::ApplyAnnotation(arguments)) => {
                let accepted = self.command("set_module_content", arguments);
                self.rack.finish_annotation_edit(accepted);
            }
            Some(RackAction::ResizeBlank(proposal)) => {
                let accepted = self.resize_blank(proposal);
                self.rack.finish_annotation_edit(accepted);
            }
            Some(RackAction::MoveModules(movement)) => {
                self.move_modules(movement);
            }
            None => {}
        }
        if self.library_enabled() {
            if let Some((kind, position, reveal)) = pending_add {
                self.add_module(&kind, Some((position, reveal)));
            } else if matches!(library_action, Some(LibraryAction::EnablePolyphony)) {
                self.confirm_engine_upgrade = true;
            }
        }
    }

    fn library_enabled(&self) -> bool {
        self.file_mode.is_none()
            && self.confirm_discard.is_none()
            && !self.recovery_open
            && !self.confirm_engine_upgrade
            && self.preset_dialog.is_none()
            && self.template_save.is_none()
            && !self.settings_open
            && !self.audio_performance_open
            && !self.shader_open
            && !self.mcp_session.is_open()
            && !self.allow_close
    }

    fn browser(&mut self, ctx: &egui::Context) -> Option<LibraryAction> {
        let enabled = self.library_enabled();
        let action = self.library.show(
            ctx,
            &mut self.browser_open,
            &self.catalog,
            self.patch.version,
            enabled,
        );
        // Drain both independent flags even when the first is true, then
        // persist one coherent host preference snapshot for this UI pass.
        let favorites_changed = self.library.take_favorites_changed();
        let zoom_changed = self.library.take_preview_zoom_changed();
        if favorites_changed || zoom_changed {
            self.save_host_settings();
        }
        action
    }

    fn load_preset_dialog(&self, dialog: &mut PresetDialog) {
        dialog.pending = false;
        if let Err(error) = dialog.target_status(&self.patch, self.revision) {
            dialog.status = error;
            return;
        }
        match dialog.mode {
            PresetMode::Save => {
                match execute_shared(
                    &self.shared,
                    "capture_module_preset",
                    json!({
                        "id":dialog.id,"expected_revision":dialog.revision,"include_asset_data":true
                    }),
                ) {
                    Ok(result) => {
                        let preset = result.get("preset").cloned();
                        if result["id"].as_str() != Some(dialog.id.as_str())
                            || result["revision"].as_u64() != Some(dialog.revision)
                            || preset.as_ref().is_none_or(|value| {
                                value["kind"].as_str() != Some(dialog.kind.as_str())
                            })
                        {
                            dialog.status =
                                "The capture did not match this module. Retry capture.".into();
                            return;
                        }
                        dialog.capture = preset;
                        dialog.source = result["state_source"]
                            .as_str()
                            .unwrap_or("saved_parameters")
                            .into();
                        dialog.frame = result["frame"].as_u64();
                        dialog.status =
                            "Review the captured controls, then save under a new name.".into();
                    }
                    Err(error) => {
                        dialog.capture = None;
                        dialog.status = format!("Current module state is unavailable: {error}. Retry capture when ready.");
                    }
                }
            }
            PresetMode::Browse => {
                match execute_shared(
                    &self.shared,
                    "list_module_presets",
                    json!({
                        "kind":dialog.kind,"query":dialog.query,"offset":dialog.offset,"limit":30
                    }),
                ) {
                    Ok(result) => {
                        dialog.entries = result["presets"].as_array().cloned().unwrap_or_default();
                        dialog.offset = result["offset"].as_u64().unwrap_or(0) as usize;
                        dialog.total = result["total"].as_u64().unwrap_or(0) as usize;
                        dialog.status = if dialog.total == 0 {
                            if dialog.query.is_empty() {
                                "No presets for this module yet.".into()
                            } else {
                                "No matching presets.".into()
                            }
                        } else {
                            format!("{} compatible library entries", dialog.total)
                        };
                    }
                    Err(error) => {
                        dialog.entries.clear();
                        dialog.status = format!("Could not list presets: {error}");
                    }
                }
            }
        }
    }

    fn preset_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.preset_dialog.take() else {
            return;
        };
        if dialog.await_release {
            dialog.await_release = false;
        } else if dialog.pending {
            self.load_preset_dialog(&mut dialog);
        }
        let target_error = dialog.target_status(&self.patch, self.revision).err();
        let mut intent = PresetIntent::None;
        egui::Modal::new(egui::Id::new("module-preset-dialog")).show(ctx, |ui| {
            ui.set_width(540.0);
            ui.heading(match dialog.mode {
                PresetMode::Save => "Save module preset",
                PresetMode::Browse => "Browse module presets",
            });
            ui.label(format!("{} · {}", module_title(&dialog.kind), dialog.id));
            ui.label(RichText::new(if matches!(dialog.kind.as_str(), "wavetable_vco" | "wavetable_lfo") {
                "Wavetable presets store controls, bypass and an embedded table. Rack cables and position stay in the project."
            } else {
                "Presets store module controls and bypass only. Rack cables and position stay in the project."
            }).color(MUTED).size(11.0));
            if let Some(error) = &target_error {
                ui.colored_label(Color32::LIGHT_RED, error);
                if dialog.mode == PresetMode::Save && dialog.capture.is_some() {
                    ui.label("The captured snapshot remains saveable. Refresh only if you want a new capture.");
                }
                if self.patch.nodes.iter().any(|node| node.id == dialog.id && node.kind == dialog.kind)
                    && ui.button("Refresh target").clicked()
                {
                    intent = PresetIntent::RefreshTarget;
                }
            }
            ui.separator();
            match dialog.mode {
                PresetMode::Save => {
                    if let Some(preset) = &dialog.capture {
                        let source = match dialog.source.as_str() {
                            "live" => "Current live module state",
                            "offline_reset" => "Offline module state",
                            _ => "Saved module controls",
                        };
                        ui.label(match dialog.frame {
                            Some(frame) => format!("Captured from {source} at audio frame {frame}"),
                            None => format!("Captured from {source}"),
                        });
                        preset_controls(ui, preset, &self.catalog);
                    } else {
                        ui.label("Waiting for a current module snapshot.");
                        if ui.button("Retry capture").clicked() {
                            intent = PresetIntent::Retry;
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label("New preset name");
                        let response = ui.add(egui::TextEdit::singleline(&mut dialog.name)
                            .hint_text("warm-vco").desired_width(240.0));
                        if dialog.focus_name {
                            response.request_focus();
                            dialog.focus_name = false;
                        }
                        ui.label(".opreset");
                        if (response.has_focus() || response.lost_focus())
                            && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            intent = PresetIntent::Save;
                        }
                    });
                    ui.label(RichText::new("Use letters, numbers, dashes, underscores or periods. Existing presets are never overwritten.").color(MUTED).size(11.0));
                    if !dialog.name.is_empty() && preset_filename(&dialog.name).is_none() {
                        ui.colored_label(Color32::LIGHT_RED, "Enter a simple name without spaces or paths.");
                    }
                }
                PresetMode::Browse => {
                    ui.horizontal(|ui| {
                        ui.label("Search presets");
                        let response = ui.add(egui::TextEdit::singleline(&mut dialog.search)
                            .hint_text("Search the full library by name").desired_width(270.0));
                        if ui.button("Search").clicked()
                            || (response.has_focus() || response.lost_focus())
                                && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            intent = PresetIntent::Search;
                        }
                    });
                    egui::ScrollArea::vertical().id_salt("preset-library-scroll").max_height(190.0).show(ui, |ui| {
                        for entry in &dialog.entries {
                            let filename = entry["filename"].as_str().unwrap_or("");
                            if entry["valid"] == false {
                                ui.colored_label(Color32::LIGHT_RED,
                                    format!("{} — {}", filename, entry["error"].as_str().unwrap_or("Invalid preset")));
                                continue;
                            }
                            let response = ui.selectable_label(dialog.preview_filename.as_deref() == Some(filename), filename);
                            if response.clicked() {
                                response.request_focus();
                                intent = PresetIntent::Read(filename.to_owned());
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.add_enabled(dialog.offset > 0, egui::Button::new("Previous")).clicked() {
                            intent = PresetIntent::Previous;
                        }
                        ui.label(format!("{}–{} of {}", if dialog.total == 0 { 0 } else { dialog.offset + 1 },
                            (dialog.offset + dialog.entries.len()).min(dialog.total), dialog.total));
                        if ui.add_enabled(dialog.offset + dialog.entries.len() < dialog.total,
                            egui::Button::new("Next")).clicked() {
                            intent = PresetIntent::Next;
                        }
                        if ui.button("Refresh library").clicked() {
                            intent = PresetIntent::Retry;
                        }
                    });
                    if let Some(preset) = &dialog.preview {
                        ui.separator();
                        ui.strong(dialog.preview_filename.as_deref().unwrap_or("Preset preview"));
                        preset_controls(ui, preset, &self.catalog);
                        ui.label(RichText::new("Apply uses these reviewed settings. It leaves the module's cables and rack position in place.").color(MUTED).size(11.0));
                    } else {
                        ui.label("Select a preset to preview its controls before applying.");
                    }
                }
            }
            ui.separator();
            if !dialog.status.is_empty() {
                ui.label(&dialog.status);
            }
            ui.horizontal(|ui| {
                let enabled = match dialog.mode {
                    PresetMode::Save => dialog.capture.is_some() && preset_filename(&dialog.name).is_some(),
                    PresetMode::Browse => target_error.is_none() && dialog.preview.is_some(),
                };
                let label = if dialog.mode == PresetMode::Save { "Save preset" } else { "Apply preset" };
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    intent = if dialog.mode == PresetMode::Save { PresetIntent::Save } else { PresetIntent::Apply };
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    intent = PresetIntent::Cancel;
                }
            });
        });
        if matches!(&intent, PresetIntent::None)
            && dialog.mode == PresetMode::Browse
            && dialog.preview.is_some()
            && target_error.is_none()
            && ctx.input(|i| i.key_pressed(egui::Key::Enter))
            && !ctx.wants_keyboard_input()
        {
            intent = PresetIntent::Apply;
        }
        let mut close = false;
        match intent {
            PresetIntent::None => {}
            PresetIntent::Cancel => close = true,
            PresetIntent::Retry => dialog.pending = true,
            PresetIntent::Search => {
                dialog.query = dialog.search.trim().to_owned();
                dialog.offset = 0;
                dialog.preview = None;
                dialog.preview_filename = None;
                dialog.pending = true;
            }
            PresetIntent::RefreshTarget => {
                dialog.revision = self.revision;
                dialog.capture = None;
                dialog.preview = None;
                dialog.preview_filename = None;
                dialog.status = "Refreshing this module…".into();
                dialog.pending = true;
            }
            PresetIntent::Previous => {
                dialog.offset = dialog.offset.saturating_sub(30);
                dialog.pending = true;
            }
            PresetIntent::Next => {
                dialog.offset += 30;
                dialog.pending = true;
            }
            PresetIntent::Read(filename) => {
                dialog.preview = None;
                dialog.preview_filename = None;
                match execute_shared(
                    &self.shared,
                    "read_module_preset",
                    json!({"filename":filename,"include_asset_data":true}),
                ) {
                    Ok(result)
                        if result["filename"].as_str() == Some(filename.as_str())
                            && result["preset"]["kind"].as_str() == Some(dialog.kind.as_str()) =>
                    {
                        dialog.preview = result.get("preset").cloned();
                        dialog.preview_filename = Some(filename);
                        dialog.status = "Review the settings, then choose Apply preset.".into();
                    }
                    Ok(_) => {
                        dialog.status =
                            "This preset is not compatible with the selected module.".into()
                    }
                    Err(error) => dialog.status = format!("Could not read preset: {error}"),
                }
            }
            PresetIntent::Save => {
                if let Some(preset) = &dialog.capture {
                    if let Some(filename) = preset_filename(&dialog.name) {
                        match execute_shared(
                            &self.shared,
                            "save_module_preset",
                            json!({
                                "filename":filename,"preset":preset
                            }),
                        ) {
                            Ok(_) => {
                                self.status = format!("Preset saved: {filename}");
                                close = true;
                            }
                            Err(error) => dialog.status = format!("Could not save preset: {error}"),
                        }
                    } else {
                        dialog.status =
                            "Enter a simple preset name without spaces or paths.".into();
                    }
                }
            }
            PresetIntent::Apply => {
                if let Err(error) = dialog.target_status(&self.patch, self.revision) {
                    dialog.status = error;
                } else if let Some(preset) = &dialog.preview {
                    match execute_shared(
                        &self.shared,
                        "apply_module_preset",
                        json!({
                            "id":dialog.id,"preset":preset,"expected_revision":dialog.revision
                        }),
                    ) {
                        Ok(_) => {
                            self.sync();
                            self.status = format!(
                                "Preset applied: {}",
                                dialog.preview_filename.as_deref().unwrap_or("preset")
                            );
                            close = true;
                        }
                        Err(error) => dialog.status = format!("Could not apply preset: {error}"),
                    }
                }
            }
        }
        if !close {
            self.preset_dialog = Some(dialog);
        }
    }
    fn files_window(&mut self, ctx: &egui::Context) {
        self.poll_wavetable_import();
        let Some(mode) = self.file_mode else {
            return;
        };
        let preparing = mode == FileMode::WavetableImport && self.wavetable_import.is_some();
        let mut run = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("project-file-dialog")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(match mode {
                FileMode::Save => "Save project as",
                FileMode::Open => "Open project",
                FileMode::WavetableImport => "Load wavetable",
                FileMode::WavetableExport => "Save wavetable",
            });
            ui.label(RichText::new(&self.workspace).color(MUTED).size(11.0));
            if let Some(target) = &self.wavetable_file_target {
                ui.label(format!("Module: {}", target.id));
            }
            ui.add_enabled_ui(!preparing, |ui| {
            ui.horizontal(|ui| {
                ui.label("Filename");
                let mut output = egui::TextEdit::singleline(&mut self.filename).desired_width(330.0).show(ui);
                if self.file_focus && !preparing {
                    self.file_focus = false;
                    output.response.request_focus();
                    output.state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),egui::text::CCursor::new(self.filename.chars().count())
                    )));
                    output.state.store(ui.ctx(),output.response.id);
                }
                run |= output.response.lost_focus() && ui.input(|i|i.key_pressed(egui::Key::Enter));
            });
            if matches!(mode, FileMode::Open | FileMode::WavetableImport) {
                egui::ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
                    for file in &self.files {
                        let response = ui.selectable_label(self.filename == *file,file);
                        if response.clicked() { self.filename = file.clone(); }
                        if response.double_clicked() { self.filename = file.clone(); run = true; }
                    }
                    if self.files.is_empty() { ui.label(if mode == FileMode::Open {
                        "No saved projects yet."
                    } else { "No supported wavetable files in this workspace." }); }
                });
            }
            match mode {
                FileMode::Open => { ui.label("Open a native .openpatch project or import an older .json patch."); }
                FileMode::Save => { ui.label("Use a new .openpatch filename. Modules, cables and rack layout are saved together."); }
                FileMode::WavetableImport => {
                    ui.label("Supported: .wav, .f32, .s8/.i8, .s16/.i16, .s24/.i24, .s32/.i32.");
                    ui.label("Raw samples use little-endian encoding. WAV channels remain interleaved.");
                    if let Some(target) = &mut self.wavetable_file_target {
                        ui.horizontal(|ui| {
                            ui.label("Wave points");
                            let entry = ui.add(egui::TextEdit::singleline(&mut target.wave_points)
                                .hint_text("Auto").desired_width(100.0));
                            run |= entry.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                            ui.label(format!("Current: {}", target.current_points));
                        });
                        ui.label("Auto uses a power-of-two WAV sample rate; otherwise it keeps the current wave length. Enter a length such as 32 to override.");
                        ui.label(if target.audio { "VCO wave length must be a power of two, at least 32 points." }
                            else { "LFO wave length must contain at least 2 points." });
                    }
                }
                FileMode::WavetableExport => {
                    let extension = wavetable_file_extension(&self.filename);
                    ui.horizontal(|ui| {
                        if ui.selectable_label(extension == "f32", "Exact float32 (.f32)").clicked() {
                            self.filename = wavetable_filename_with_extension(&self.filename, "f32");
                        }
                        if ui.selectable_label(extension == "wav", "PCM16 WAV (.wav)").clicked() {
                            self.filename = wavetable_filename_with_extension(&self.filename, "wav");
                        }
                    });
                    ui.label(".f32 preserves every source sample bit, including unused trailing points.");
                    ui.label(".wav converts to PCM16 and clips samples to [-1, 1].");
                    ui.label("Choose a new filename. Existing files are never overwritten.");
                }
            }
            });
            ui.separator();
            if preparing {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(if self.wavetable_import.as_ref().is_some_and(|job| job.allowed.load(Ordering::Acquire)) {
                        "Preparing wavetable…"
                    } else { "Finishing cancelled import…" });
                });
                ctx.request_repaint_after(Duration::from_millis(16));
            }
            ui.label(&self.status);
            ui.horizontal(|ui| {
                run |= ui.add_enabled(!preparing, egui::Button::new(match mode {
                    FileMode::Save => "Save project",
                    FileMode::Open => "Open",
                    FileMode::WavetableImport => "Load wavetable",
                    FileMode::WavetableExport => "Save wavetable",
                })).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.cancel_wavetable_import();
            if mode == FileMode::WavetableImport && self.wavetable_import.is_some() {
                self.status = "Wavetable import cancelled".into();
            }
            self.file_mode = None;
            self.wavetable_file_target = None;
            self.after_save = None;
        } else if run && !preparing {
            self.save_or_load(mode);
        }
    }
    fn recovery_window(&mut self, ctx: &egui::Context) {
        if !self.recovery_open {
            return;
        }
        let mut restore = None;
        let mut discard = None;
        let mut dismiss = false;
        egui::Modal::new(egui::Id::new("project-recovery")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.heading("Recover unsaved work");
            ui.label("These copies were left by a previous OpenPatch session. Recovery opens a separate unsaved project with playback paused.");
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                for (index,name) in self.recovery_names.iter().enumerate() {
                    ui.group(|ui| {
                        ui.strong(format!("Recovery {}",index+1)).on_hover_text(name);
                        if let Some(detail) = self.recovery_details.get(name) { ui.label(detail); }
                        ui.horizontal(|ui| {
                            if ui.button("Recover").clicked() { restore = Some(name.clone()); }
                            if ui.button("Discard copy").clicked() { discard = Some(name.clone()); }
                        });
                    });
                }
            });
            ui.label(&self.status);
            dismiss = ui.button("Not now").clicked();
        });
        if let Some(name) = discard {
            if let Some(store) = &self.projects {
                match store.discard_recovery(&name) {
                    Ok(()) => self.recovery_names.retain(|item| *item != name),
                    Err(error) => self.status = format!("Could not discard recovery: {error}"),
                }
            }
        }
        if let Some(name) = restore {
            self.recovery_open = false;
            self.request_action(PendingAction::Recover(name));
        }
        if dismiss
            || self.recovery_names.is_empty()
            || ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.recovery_open = false;
        }
    }
    fn unsaved_window(&mut self, ctx: &egui::Context) {
        if self.confirm_discard.is_none() {
            return;
        }
        let mut decision = 0;
        egui::Modal::new(egui::Id::new("unsaved-project")).show(ctx, |ui| {
            ui.set_width(430.0);
            ui.heading("Save your changes?");
            ui.label("Your project has unsaved changes, or its current live state could not be verified.");
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    decision = 1;
                }
                if ui.button("Discard changes").clicked() {
                    decision = 2;
                }
                if ui.button("Cancel").clicked() {
                    decision = 3;
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            decision = 3;
        }
        match decision {
            1 => {
                self.after_save = self.confirm_discard.take();
                self.save_current();
            }
            2 => {
                self.after_save = None;
                if let Some(action) = self.confirm_discard.take() {
                    self.perform_action(action);
                }
            }
            3 => {
                self.confirm_discard = None;
                self.after_save = None;
                self.deferred_action = None;
            }
            _ => {}
        }
    }
    fn shader_window(&mut self, ctx: &egui::Context) {
        if !self.shader_open {
            return;
        }
        let mut open = true;
        egui::Window::new("GLSL visualizer")
            .open(&mut open)
            .default_size([730.0, 540.0])
            .show(ctx, |ui| {
                ui.label(
                    "GLSL 330 · u_time · u_resolution · u_level · u_wave[256] · u_spectrum[64] · u_ferro",
                );
                ui.horizontal(|ui| {
                    if ui.button("Compile").clicked() {
                        if let (Some(v), Some(gl)) = (&mut self.visualizer, &self.gl) {
                            self.shader_status = match v.compile(gl, &self.shader) {
                                Ok(()) => "Shader compiled".into(),
                                Err(e) => e,
                            };
                        }
                    }
                    if ui.button("Restore source").clicked() {
                        self.shader = DEFAULT_SHADER.into();
                    }
                });
                ui.label(&self.shader_status);
                egui::ScrollArea::both().max_height(430.0).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.shader)
                            .font(egui::TextStyle::Monospace)
                            .code_editor()
                            .desired_rows(24)
                            .desired_width(f32::INFINITY),
                    );
                });
            });
        self.shader_open = open;
    }
    fn rack_scope_smoke_fixture(
        xy: bool,
        external: bool,
    ) -> (Patch, openpatch_core::ScopeSnapshot) {
        let mut pitch = serde_json::Map::new();
        pitch.insert("channels".into(), json!(16));
        for voice in 1..=16 {
            pitch.insert(format!("v{voice}"), json!((voice - 1) as f32 * 0.1));
        }
        let mut patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[{"id":"scope","kind":"scope","params":{"time":0.08,"xy":if xy {1} else {0},"trigger_enabled":1,"threshold":2.5}},
                {"id":"wave","kind":"multi_vco","params":{"frequency":50.0}},
                {"id":"pitch","kind":"poly_constant","params":pitch}],
            "cables":[{"from":"pitch","to":"wave","input":"pitch"},
                {"from":"wave","output":"sine","to":"scope","input":"x"},
                {"from":"wave","output":"triangle","to":"scope","input":"y"}]
        }))
        .expect("Rack Scope smoke patch");
        if external {
            patch.cables.push(
                serde_json::from_value(
                    json!({"from":"wave","output":"square","to":"scope","input":"trigger"}),
                )
                .unwrap(),
            );
        }
        let mut engine = patch.compile().expect("Rack Scope offline engine");
        engine.process_block(&mut vec![0.0; 3841]);
        let handle = engine
            .resolve_module_scope("scope")
            .expect("Rack Scope handle");
        let snapshot = engine
            .module_scope(handle)
            .expect("Rack Scope snapshot")
            .clone();
        (patch, snapshot)
    }

    /// A real single-step commit gives deterministic presentation phases even
    /// if the observing thread is descheduled. Progressive worker sequencing is
    /// covered separately by the host's controlled-worker integration tests.
    fn prepare_motion_fixture(&mut self, origin: Instant) -> Vec<Value> {
        assert!(self.command(
            "replace_patch",
            json!({"patch":{"version":2,"nodes":[
                {"id":"voice","kind":"multi_vco","params":{"frequency":110}},
                {"id":"filter","kind":"ladder_vcf","params":{"cutoff":800}}
            ]}})
        ));
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&json!({"version":1,"zoom":1.0,"pan":[100,64],"positions":{
                "voice":[0,0],"filter":[330,0],"out":[570,0]
            }}))
            .unwrap();
        self.smoke_motion_origin = Some(origin);
        self.rack.set_construction_clock_override(Some(origin));
        let before = self.revision;
        execute_shared(&self.shared, "start_construction", json!({
            "title":"Shape an analog voice","expected_revision":before,"interval_ms":100,
            "steps":[{"label":"Connect and shape the voice","edits":[
                {"tool":"add_module","arguments":{"id":"out","kind":"audio_out"}},
                {"tool":"connect","arguments":{"from":"voice","output":"saw","to":"filter","input":"signal","cable_id":"motion-tone"}},
                {"tool":"connect","arguments":{"from":"filter","to":"out","input":"signal","cable_id":"motion-output"}},
                {"tool":"set_parameter","arguments":{"id":"voice","parameter":"frequency","value":220}},
                {"tool":"set_parameter","arguments":{"id":"filter","parameter":"cutoff","value":2400}},
                {"tool":"set_output","arguments":{"id":"out"}}
            ]}]
        })).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let state = loop {
            let state = self.shared.lock().unwrap().construction_snapshot();
            if state["status"] == "completed" {
                break state;
            }
            assert_eq!(state["status"], "running", "{state}");
            assert!(Instant::now() < deadline, "Motion fixture did not complete");
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(state["completed_steps"], 1);
        assert_eq!(state["last_commit"]["before_revision"], before);
        while self.revision < state["last_commit"]["revision"].as_u64().unwrap() {
            self.sync();
            assert!(
                Instant::now() < deadline,
                "Fixture could not observe commit"
            );
            std::thread::yield_now();
        }
        assert_eq!(state["last_commit"]["revision"], self.revision);
        vec![state["last_commit"].clone()]
    }

    /// Observe a real endpoint move, with a controlled presentation clock only.
    /// Graph routing and the authoritative receipt are never staged or faked.
    fn prepare_cable_move_fixture(&mut self, origin: Instant, progressive: bool) -> Value {
        self.smoke_motion_origin = None;
        assert!(self.command("replace_patch", json!({"patch":{
            "version":2,"nodes":[
                {"id":"voice","kind":"multi_vco","params":{"frequency":110}},
                {"id":"filter-a","kind":"ladder_vcf","params":{"cutoff":800}},
                {"id":"filter-b","kind":"ladder_vcf","params":{"cutoff":2400}},
                {"id":"out","kind":"audio_out"}
            ],"cables":[
                {"id":"moving-tone","from":"voice","output":"saw","to":"filter-a","input":"signal","color":[245,200,70]},
                {"id":"moving-output","from":"filter-a","to":"out","input":"signal","color":[65,190,180]}
            ],"output":"out"
        }})));
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&json!({"version":1,"zoom":1.0,"pan":[72,48],"positions":{
                "voice":[0,0],"filter-a":[330,0],"filter-b":[600,0],"out":[870,0]
            }}))
            .unwrap();
        self.smoke_motion_origin = Some(origin);
        self.rack.set_construction_clock_override(Some(origin));
        let before = self.revision;
        let edits = json!([
            {"tool":"move_cables","arguments":{"cable_ids":["moving-tone"],"end":"input","node":"filter-b","port":"signal"}},
            {"tool":"move_cables","arguments":{"cable_ids":["moving-output"],"end":"output","node":"filter-b","port":"out"}}
        ]);
        if progressive {
            execute_shared(
                &self.shared,
                "start_construction",
                json!({
                    "title":"Repatch the voice","expected_revision":before,"interval_ms":100,
                    "steps":[{"label":"Move the voice to the second filter","edits":edits}]
                }),
            )
            .unwrap();
        } else {
            execute_shared(
                &self.shared,
                "apply_edits",
                json!({
                    "expected_revision":before,"edits":edits
                }),
            )
            .unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let state = loop {
            let state = self.shared.lock().unwrap().construction_snapshot();
            if !progressive || state["status"] == "completed" {
                break state;
            }
            assert_eq!(state["status"], "running", "{state}");
            assert!(
                Instant::now() < deadline,
                "Cable move fixture did not complete"
            );
            std::thread::sleep(Duration::from_millis(2));
        };
        let receipt = if progressive {
            &state["last_commit"]
        } else {
            &state["last_move"]
        };
        assert_eq!(receipt["before_revision"], before);
        assert_eq!(receipt["moved_cables"].as_array().unwrap().len(), 2);
        let revision = receipt["revision"].as_u64().unwrap();
        while self.revision < revision {
            self.sync();
            assert!(
                Instant::now() < deadline,
                "Cable move fixture could not observe commit"
            );
            std::thread::yield_now();
        }
        assert_eq!(self.revision, revision);
        receipt.clone()
    }

    /// Freeze actual offline DSP telemetry for matched room-lighting captures.
    /// There is no audio device or UI blink timer in this fixture.
    fn prepare_room_lighting_fixture(&mut self) {
        self.smoke_motion_origin = None;
        self.mcp_session = crate::mcp_session_ui::Panel::new(false);
        self.select_analysis(None);
        self.keyboard_open = false;
        self.settings_open = false;
        self.library = ModuleLibrary::default();
        self.browser_open = true;
        let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[
                {"id":"wave","kind":"multi_vco","params":{"frequency":50}},
                {"id":"sequence","kind":"seq3"},
                {"id":"push","kind":"push"},
                {"id":"pulses","kind":"pulses"},
                {"id":"scope","kind":"scope","params":{"time":0.08,"trigger_enabled":0}},
                {"id":"viz","kind":"poly_viz"},
                {"id":"sum","kind":"poly_sum"},
                {"id":"volts","kind":"poly_constant","params":{"channels":4,"v1":5,"v2":-2,"v3":3,"v4":1}}
            ],
            "cables":[
                {"from":"wave","output":"sine","to":"scope","input":"x"},
                {"from":"wave","output":"triangle","to":"scope","input":"y"},
                {"from":"volts","to":"viz","input":"signal"},
                {"from":"volts","to":"sum","input":"signal"}
            ]
        })).expect("Room lighting fixture patch");
        let mut engine = patch.compile().expect("Room lighting offline engine");
        let pulses = engine.resolve_module_event("pulses").unwrap();
        for row in [1, 7] {
            assert!(
                engine.handle_module_event(pulses, openpatch_core::ModuleAction::PulsePress(row))
            );
        }
        let push = engine.resolve_module_event("push").unwrap();
        assert!(engine.handle_module_event(push, openpatch_core::ModuleAction::HoldPress));
        assert!(engine.handle_module_event(push, openpatch_core::ModuleAction::HoldRelease));
        let sequence = engine.resolve_module_event("sequence").unwrap();
        for _ in 0..3 {
            assert!(engine.handle_module_event(sequence, openpatch_core::ModuleAction::Clock));
        }
        engine.process_block(&mut vec![0.0; 8192]);
        let mut lights = BTreeMap::new();
        let mut controls = BTreeMap::new();
        for node in &patch.nodes {
            if let Ok(handle) = engine.resolve_module_lights(&node.id) {
                if let Some(state) = engine.module_lights(handle) {
                    lights.insert(node.id.clone(), state);
                }
            }
            if let Ok(handle) = engine.resolve_module_control(&node.id) {
                if let Some(state) = engine.module_control_state(handle) {
                    controls.insert(node.id.clone(), state);
                }
            }
        }
        let scope = engine.resolve_module_scope("scope").unwrap();
        let scope = engine.module_scope(scope).unwrap().clone();
        let sequence = engine.module_telemetry(sequence).unwrap();
        for id in ["pulses", "push", "viz", "sum"] {
            assert!(
                lights[id]
                    .brightness
                    .iter()
                    .flatten()
                    .any(|value| *value > 0.0),
                "Room fixture needs actual lit telemetry for {id}"
            );
        }
        assert!(scope.x_channels > 0 && scope.y_channels > 0);
        assert!(self.command("replace_patch", json!({"patch":patch})));
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&json!({"version":1,"zoom":0.72,"pan":[24,12],"positions":{
                "wave":[0,0],"sequence":[300,0],"push":[1080,0],"pulses":[1260,0],
                "scope":[0,420],"viz":[630,420],"sum":[810,420],"volts":[990,420]
            }}))
            .unwrap();
        self.rack.set_module_states(lights, controls);
        self.rack
            .set_sequence_states(BTreeMap::from([("sequence".into(), sequence)]), false);
        self.rack.set_scope_states(BTreeMap::from([(
            "scope".into(),
            std::sync::Arc::new(scope),
        )]));
        self.title = "Room lighting · actual offline DSP snapshot".into();
        self.status = "Lights on · 100% room · held gates, four CV lanes and captured Scope · no live audio stream".into();
    }

    fn prepare_adsr_eg_fixture(&mut self) {
        let mut patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[{"id":"gates","kind":"poly_constant","params":{"channels":4,"v1":10,"v2":10,"v3":10,"v4":10}},
                {"id":"attack-cv","kind":"poly_constant","params":{"channels":4,"v1":2.5,"v2":-1.5,"v3":-3,"v4":-1}},
                {"id":"decay-cv","kind":"poly_constant","params":{"channels":4,"v1":0,"v2":3,"v3":-2.5,"v4":3}},
                {"id":"envelope","kind":"adsr_eg","params":{"attack":0.4,"decay":0.3,"sustain":0.4,"release":0.6,"attack_amount":1,"decay_amount":1}},
                {"id":"scope","kind":"scope","params":{"time":0.15,"trigger_enabled":0,"x_offset":-5}}],
            "cables":[{"from":"gates","to":"envelope","input":"gate"},
                {"from":"attack-cv","to":"envelope","input":"attack_cv"},
                {"from":"decay-cv","to":"envelope","input":"decay_cv"},
                {"from":"envelope","to":"scope","input":"x"}] })).unwrap();
        let mut engine = patch.compile().unwrap();
        engine.process_block(&mut [0.0; 2048]);
        patch.nodes[0].params.insert("v4".into(), 0.0);
        let mut next = patch.compile().unwrap();
        next.adopt_state_from(&engine);
        next.process_block(&mut [0.0; 6144]);
        let lights = next
            .module_lights(next.resolve_module_lights("envelope").unwrap())
            .unwrap();
        assert_eq!(lights.count, 5);
        assert_eq!(lights.input_channels, Some(4));
        assert!(lights.brightness[..5].iter().all(|pair| pair[0] == 1.0));
        let scope = next
            .module_scope(next.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!(scope.x_channels, 4);
        assert!(scope.x[..4]
            .iter()
            .all(|voice| voice.iter().any(|p| p.max > 0.0)));
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack.restore_layout(&json!({"version":1,"zoom":0.95,"pan":[24,12],"positions":{
            "envelope":[0,0],"scope":[390,0],"gates":[0,420],"attack-cv":[390,420],"decay-cv":[780,420]}})).unwrap();
        self.rack.set_module_states(
            BTreeMap::from([("envelope".into(), lights)]),
            BTreeMap::new(),
        );
        self.rack.set_scope_states(BTreeMap::from([(
            "scope".into(),
            std::sync::Arc::new(scope),
        )]));
        self.browser_open = false;
        self.select_analysis(None);
        self.title = "ADSR EG · four voices, four stages".into();
        self.status = "Actual offline DSP snapshot · attack, decay, sustain and release on separate lanes · no live audio".into();
    }

    fn prepare_port_lights_fixture(&mut self) {
        let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,"nodes":[
            {"id":"positive","kind":"constant","params":{"value":5}},
            {"id":"negative","kind":"constant","params":{"value":-5}},
            {"id":"cancellation","kind":"gain"},
            {"id":"scope","kind":"scope","params":{"trigger_enabled":0}},
            {"id":"polyphonic","kind":"poly_constant","params":{"channels":2,"v1":3,"v2":4}},
            {"id":"poly-vca","kind":"stock_vca","params":{"level":1,"response":1}},
            {"id":"zero","kind":"constant","params":{"value":0}},
            {"id":"silent-vca","kind":"stock_vca","params":{"level":1,"response":1}}
        ],"cables":[
            {"from":"positive","to":"cancellation","input":"signal"},
            {"from":"negative","to":"cancellation","input":"signal"},
            {"from":"cancellation","to":"scope","input":"x"},
            {"from":"polyphonic","to":"poly-vca","input":"signal"},
            {"from":"poly-vca","to":"scope","input":"y"},
            {"from":"zero","to":"silent-vca","input":"signal"},
            {"from":"silent-vca","to":"scope","input":"y"}
        ],"output":"cancellation"}))
        .unwrap();
        let mut engine = patch.compile().unwrap();
        engine.set_port_lights_enabled(true);
        engine.process_block(&mut [0.0; 8192]);
        let check = |id, direction, port| {
            engine
                .port_light(engine.resolve_port_light(id, direction, port).unwrap())
                .unwrap()
        };
        use openpatch_core::PortDirection::{Input, Output};
        assert_eq!(check("positive", Output, "out").raw_rgb[1], 0.5);
        assert_eq!(check("negative", Output, "out").raw_rgb[0], 0.5);
        assert_eq!(check("cancellation", Input, "signal").raw_rgb, [0.0; 3]);
        assert_eq!(check("polyphonic", Output, "out").raw_rgb[2], 0.5);
        assert_eq!(check("silent-vca", Output, "out").raw_rgb, [0.0; 3]);
        let mut lights = BTreeMap::new();
        for id in ["poly-vca", "silent-vca"] {
            lights.insert(
                id.into(),
                engine
                    .module_lights(engine.resolve_module_lights(id).unwrap())
                    .unwrap(),
            );
        }
        assert_eq!(lights["silent-vca"].brightness[0][0], 1.0);
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
                "positive":[0,0],"negative":[240,0],"cancellation":[480,0],"scope":[720,0],
                "polyphonic":[0,420],"poly-vca":[360,420],"zero":[570,420],"silent-vca":[810,420]
            }}))
            .unwrap();
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.rack.set_module_states(lights, BTreeMap::new());
        self.rack.set_scope_states(BTreeMap::from([(
            "scope".into(),
            std::sync::Arc::new(scope),
        )]));
        self.browser_open = false;
        self.select_analysis(None);
        self.title = "Signal at every connected plug".into();
        self.status = "Actual offline DSP · green +5 / red −5 cancel at IN · blue [3,4] V · silent full-gain VCA has dark plugs".into();
    }

    fn prepare_parameter_choices_fixture(&mut self) {
        // Configured settings only: no invented signal activity or device IO.
        let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"waveform","kind":"analog_vco","params":{"waveform":1}},
            {"id":"law","kind":"crossfade","params":{"law":1}},
            {"id":"out","kind":"audio_stereo","params":{"dc_block":1}},
            {"id":"score","kind":"score_sequencer","params":{"running":0}},
            {"id":"drum","kind":"drum_voice","params":{"mode":2}}
        ],"output":"out"}))
        .unwrap();
        patch
            .compile()
            .expect("categorical setting fixture compiles");
        let mut positions = serde_json::Map::new();
        let mut x = 0.0;
        for node in &patch.nodes {
            positions.insert(node.id.clone(), json!([x, 0]));
            x += crate::rack::panel_width(&node.kind);
        }
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&json!({"version":1,"zoom":0.9,"pan":[24,12],"positions":positions}))
            .unwrap();
        self.browser_open = false;
        self.select_analysis(None);
        self.title = "Waveforms, modes and switches".into();
        self.status = "Five configured categorical controls · exact values and shared panel rendering · audio off".into();
    }

    fn prepare_stock_vca_fixture(&mut self) {
        let mut patch: Patch =
            serde_json::from_str(include_str!("../../../examples/stock-vca-voice.json")).unwrap();
        // A separate metrology lane preserves the playable voice and proves
        // effective gain is observable even when one signal voice is silent.
        let proof: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"proof-cv","kind":"poly_constant","params":{"channels":5,"v1":0,"v2":5,"v3":8.408964,"v4":10,"v5":10}},
            {"id":"proof-signal","kind":"poly_constant","params":{"channels":5,"v1":2,"v2":2,"v3":2,"v4":2,"v5":0}},
            {"id":"gain-proof","kind":"stock_vca","params":{"level":1,"response":0}}],
            "cables":[{"from":"proof-cv","to":"gain-proof","input":"cv"},
                {"from":"proof-signal","to":"gain-proof","input":"signal"}]})).unwrap();
        patch.nodes.extend(proof.nodes);
        patch.cables.extend(proof.cables);
        let mut engine = patch.compile().unwrap();
        // Capture 85 ms into the 500 ms clock period, while the real gate is
        // high. A late release-phase capture suppresses Y through EXP's fourth
        // power even though the scope and VCA are functioning correctly.
        let mut audio = vec![0.0; 4096];
        engine.process_block(&mut audio);
        assert!(audio.iter().all(|v| v.is_finite()));
        assert!(audio.iter().any(|v| v.abs() > 0.001));
        let proof = engine
            .module_lights(engine.resolve_module_lights("gain-proof").unwrap())
            .unwrap();
        assert_eq!((proof.count, proof.input_channels), (16, Some(5)));
        for (lane, gain) in [0.0, 0.0625, 0.5, 1.0, 1.0].iter().enumerate() {
            assert!((proof.brightness[lane][0] - gain).abs() < 0.00001);
            assert_eq!(proof.brightness[lane][1], 0.0);
        }
        let amplifier = engine
            .module_lights(engine.resolve_module_lights("amplifier").unwrap())
            .unwrap();
        let envelope = engine
            .module_lights(engine.resolve_module_lights("envelope").unwrap())
            .unwrap();
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
        assert!(scope.x[0].iter().any(|p| p.max.abs() > 0.01));
        assert!(scope.y[0].iter().any(|p| p.max.abs() > 0.01));
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&json!({"version":1,"zoom":0.80,"pan":[24,12],"positions":{
            "voice":[0,0],"filter":[330,0],"amplifier":[630,0],"out":[840,0],"guide":[1050,0],
            "clock":[0,420],"envelope":[180,420],"gain-proof":[570,420],"scope":[780,420],
            "proof-cv":[0,840],"proof-signal":[390,840]}}))
            .unwrap();
        self.rack.set_module_states(
            BTreeMap::from([
                ("amplifier".into(), amplifier),
                ("gain-proof".into(), proof),
                ("envelope".into(), envelope),
            ]),
            BTreeMap::new(),
        );
        self.rack.set_scope_states(BTreeMap::from([(
            "scope".into(),
            std::sync::Arc::new(scope),
        )]));
        self.browser_open = false;
        self.select_analysis(None);
        self.title = "VCA · effective gain, voice by voice".into();
        self.status = "Actual DSP snapshot · proof lanes 0 / 6.25 / 50 / 100 / 100% · fifth input silent, full gain · no live audio".into();
    }

    fn prepare_weak_scope_fixture(
        &mut self,
        automatic: bool,
    ) -> Arc<openpatch_core::ScopeSnapshot> {
        let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[
                {"id":"wave","kind":"multi_vco","params":{"frequency":50}},
                {"id":"x_micro","kind":"attenuverter","params":{"gain":0.00002}},
                {"id":"y_micro","kind":"attenuverter","params":{"gain":0.00002}},
                {"id":"scope","kind":"scope","params":{"time":0.04,"auto_range":if automatic {1}else{0}}}
            ],
            "cables":[
                {"from":"wave","output":"sine","to":"x_micro","input":"signal"},
                {"from":"wave","output":"triangle","to":"y_micro","input":"signal"},
                {"from":"x_micro","to":"scope","input":"x"},
                {"from":"y_micro","to":"scope","input":"y"}
            ]
        })).expect("Weak Scope fixture patch");
        let mut engine = patch.compile().expect("Weak Scope offline engine");
        engine.set_port_lights_enabled(true);
        let taps = [
            ("x_micro", "out"),
            ("y_micro", "out"),
            ("scope", "x"),
            ("scope", "y"),
        ]
        .map(|(id, port)| Some(engine.resolve_tap(id, port, 0).unwrap()));
        let mut audio = vec![[0.0; 2]; 8192];
        let mut raw = vec![[openpatch_core::TapSample::default(); 4]; audio.len()];
        engine.process_audio_block_with_taps(&mut audio, &taps, &mut raw);
        assert!(
            audio.iter().flatten().all(|value| *value == 0.0),
            "CV-only fixture has no audio output"
        );
        for axis in 0..2 {
            let peak = raw
                .iter()
                // Before the delayed source acquires its channel, the
                // attenuverter correctly normals its input to 10 V. Verify
                // the settled 5 V oscillator signal after that startup.
                .skip(4)
                .map(|frame| frame[axis].value.abs())
                .fold(0.0_f32, f32::max);
            assert!(
                (0.00009..0.00011).contains(&peak),
                "Real attenuated signal is approximately 100 µV peak: axis={axis}, peak={peak}"
            );
            for frame in 4..raw.len() {
                assert!(raw[frame][axis].active && raw[frame][axis + 2].active);
                assert_eq!(
                    raw[frame][axis + 2].value,
                    raw[frame - 1][axis].value,
                    "Scope passes unchanged delayed raw volts"
                );
            }
        }
        let scope = Arc::new(
            engine
                .module_scope(engine.resolve_module_scope("scope").unwrap())
                .unwrap()
                .clone(),
        );
        assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
        let measurements = scope.measurements();
        for extrema in [measurements.x.unwrap(), measurements.y.unwrap()] {
            assert_eq!(extrema.observations, 256);
            assert!(extrema.min < -0.00009 && extrema.max > 0.00009);
            assert!((0.00018..0.00022).contains(&extrema.peak_to_peak));
        }
        self.smoke_motion_origin = None;
        self.keyboard_open = false;
        self.settings_open = false;
        self.audio_performance_open = false;
        self.preset_dialog = None;
        self.file_mode = None;
        self.select_analysis(None);
        self.mcp_session = crate::mcp_session_ui::Panel::new(false);
        self.library = ModuleLibrary::default();
        self.browser_open = false;
        assert!(
            self.audio.is_none(),
            "Weak Scope captures use no live audio device"
        );
        assert!(self.command("replace_patch", json!({"patch":patch})));
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&json!({"version":1,"zoom":0.84,"pan":[24,12],"positions":{
                "scope":[0,0],"wave":[0,420],"x_micro":[300,420],"y_micro":[480,420]
            }}))
            .unwrap();
        self.rack.set_agent_follow(false);
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.rack
            .set_scope_states(BTreeMap::from([("scope".into(), scope.clone())]));
        self.title = if automatic {
            "100 µV peak · automatic Scope"
        } else {
            "100 µV peak · manual Scope"
        }
        .into();
        self.status = if automatic {
            "Actual offline Engine · raw 100 µV sine/triangle · calibrated AUTO display · no live audio stream"
        } else {
            "Same actual offline 100 µV capture · manual 20 V/screen · unchanged raw measurements · no live audio stream"
        }.into();
        scope
    }

    fn prepare_random_fixture(&mut self) {
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/random-motion-voice.json"))
                .expect("valid Random motion voice");
        let mut engine = patch.compile().expect("Random motion fixture compiles");
        engine.set_port_lights_enabled(true);
        let motion = engine.resolve_module_lights("motion").unwrap();
        let mut frames = 0;
        loop {
            let mut audio = [[0.0; 2]; 128];
            engine.process_audio_block(&mut audio);
            assert!(audio.iter().flatten().all(|value| value.is_finite()));
            frames += audio.len();
            let state = engine.module_lights(motion).unwrap();
            if state.brightness[2][0] > 1.0
                && audio.iter().flatten().any(|value| value.abs() > 0.001)
            {
                break;
            }
            assert!(
                frames < 96_000,
                "Random fixture needs a real accepted event and audible envelope"
            );
        }
        let lights = ["motion", "envelope", "amplifier"]
            .into_iter()
            .map(|id| {
                (
                    id.to_owned(),
                    engine
                        .module_lights(engine.resolve_module_lights(id).unwrap())
                        .unwrap(),
                )
            })
            .collect();
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
        assert!(scope.x[0].iter().any(|point| point.max.abs() > 0.001));
        assert!(scope.y[0].iter().any(|point| point.max.abs() > 0.001));
        self.smoke_motion_origin = None;
        self.keyboard_open = false;
        self.settings_open = false;
        self.preset_dialog = None;
        self.file_mode = None;
        self.select_analysis(None);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&Self::random_motion_layout())
            .unwrap();
        self.rack.set_agent_follow(false);
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.rack.set_module_states(lights, BTreeMap::new());
        self.rack
            .set_scope_states(BTreeMap::from([("scope".into(), Arc::new(scope))]));
        self.browser_open = false;
        self.title = "Random motion voice".into();
        self.status = "Actual offline DSP snapshot · accepted events, envelope and interpolated Scope traces · no live audio".into();
    }

    fn prepare_merge_fixture(&mut self) -> BTreeMap<String, openpatch_core::ModuleLightState> {
        let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[
                {"id":"source","kind":"poly_constant","params":{"channels":3,"v1":4,"v2":-13}},
                {"id":"silent","kind":"poly_constant","params":{"channels":1,"v1":0}},
                {"id":"auto","kind":"poly_merge"},
                {"id":"zero","kind":"poly_merge","params":{"channels":-1}},
                {"id":"eight","kind":"poly_merge","params":{"channels":8}}],
            "cables":[
                {"from":"source","to":"auto","input":"in1"},
                {"from":"silent","to":"auto","input":"in12"},
                {"from":"source","to":"zero","input":"in1"},
                {"from":"silent","to":"zero","input":"in12"},
                {"from":"source","to":"eight","input":"in1"},
                {"from":"silent","to":"eight","input":"in12"}],"output":"auto"}))
        .expect("Merge count fixture");
        let mut engine = patch.compile().expect("Merge fixture compiles");
        engine.set_port_lights_enabled(true);
        let taps =
            ["auto", "zero", "eight"].map(|id| Some(engine.resolve_tap(id, "out", 0).unwrap()));
        let mut audio = [[0.0; 2]; 64];
        let mut observed = [[openpatch_core::TapSample::default(); 3]; 64];
        engine.process_audio_block_with_taps(&mut audio, &taps, &mut observed);
        let mut lights = BTreeMap::new();
        for (index, (id, expected)) in [("auto", 12), ("zero", 0), ("eight", 8)]
            .into_iter()
            .enumerate()
        {
            let state = engine
                .module_lights(engine.resolve_module_lights(id).unwrap())
                .unwrap();
            assert_eq!(state.output_channels, Some(expected));
            assert_eq!(state.automatic_channels, Some(12));
            assert_eq!(state.count, 0);
            assert_eq!(observed[63][index].active, expected > 0);
            assert_eq!(
                observed[63][index].value,
                if expected > 0 { 4.0 } else { 0.0 }
            );
            lights.insert(id.to_owned(), state);
        }
        self.smoke_motion_origin = None;
        self.keyboard_open = false;
        self.settings_open = false;
        self.audio_performance_open = false;
        self.select_analysis(None);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&json!({"version":1,"zoom":0.85,"pan":[24,12],
            "positions":{"auto":[0,0],"zero":[330,0],"eight":[660,0],
                "source":[0,420],"silent":[390,420]}}))
            .unwrap();
        self.rack.set_agent_follow(false);
        self.rack.set_module_states(lights.clone(), BTreeMap::new());
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.browser_open = false;
        self.title = "Merge · Auto, zero, and eight channels".into();
        self.status = "Actual offline Engine counts: 12 / 0 / 8 · silent occupied input 12 counts in Auto · first input lane only · no live audio".into();
        lights
    }

    fn prepare_split_fixture(&mut self) -> BTreeMap<String, openpatch_core::ModuleLightState> {
        let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[
                {"id":"empty-source","kind":"poly_constant","params":{"channels":0}},
                {"id":"silent-source","kind":"poly_constant","params":{"channels":1,"v1":0}},
                {"id":"poly-source","kind":"poly_constant","params":{"channels":16,"v1":4,"v2":-3,"v16":-7}},
                {"id":"empty","kind":"poly_split"},
                {"id":"silent","kind":"poly_split"},
                {"id":"sixteen","kind":"poly_split"}],
            "cables":[
                {"from":"empty-source","to":"empty","input":"signal"},
                {"from":"silent-source","to":"silent","input":"signal"},
                {"from":"poly-source","to":"sixteen","input":"signal"}],
            "output":"sixteen","output_port":"out16"}))
        .expect("Split count fixture");
        let mut engine = patch.compile().expect("Split fixture compiles");
        engine.set_port_lights_enabled(true);
        let ids = ["empty", "silent", "sixteen"];
        let taps: [_; 48] = std::array::from_fn(|index| {
            Some(
                engine
                    .resolve_tap(ids[index / 16], &format!("out{}", index % 16 + 1), 0)
                    .unwrap(),
            )
        });
        let mut audio = [[0.0; 2]; 64];
        let mut observed = [[openpatch_core::TapSample::default(); 48]; 64];
        engine.process_audio_block_with_taps(&mut audio, &taps, &mut observed);
        let mut lights = BTreeMap::new();
        for (index, (id, expected)) in [("empty", 0), ("silent", 1), ("sixteen", 16)]
            .into_iter()
            .enumerate()
        {
            let state = engine
                .module_lights(engine.resolve_module_lights(id).unwrap())
                .unwrap();
            assert_eq!(state.input_channels, Some(expected));
            assert_eq!(state.count, 0);
            assert!(state.brightness.iter().flatten().all(|v| *v == 0.0));
            for lane in 0..16 {
                let sample = observed[63][index * 16 + lane];
                assert!(
                    sample.active,
                    "Split always emits sixteen independent mono outputs"
                );
                let expected_voltage = if expected == 16 {
                    match lane {
                        0 => 4.0,
                        1 => -3.0,
                        15 => -7.0,
                        _ => lane as f32 + 1.0,
                    }
                } else {
                    0.0
                };
                assert_eq!(sample.value, expected_voltage);
            }
            lights.insert(id.to_owned(), state);
        }
        self.smoke_motion_origin = None;
        self.keyboard_open = false;
        self.settings_open = false;
        self.audio_performance_open = false;
        self.select_analysis(None);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&json!({"version":1,"zoom":0.85,"pan":[24,12],
            "positions":{"empty":[0,0],"silent":[330,0],"sixteen":[660,0],
                "empty-source":[0,420],"silent-source":[390,420],"poly-source":[780,420]}}))
            .unwrap();
        self.rack.set_agent_follow(false);
        self.rack.set_module_states(lights.clone(), BTreeMap::new());
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.browser_open = false;
        self.title = "Split · zero, silent mono, and sixteen channels".into();
        self.status = "Actual offline Engine input counts: 0 / 1 / 16 · silent mono is occupied · sixteen named mono outputs · no live audio".into();
        lights
    }

    fn prepare_stock_lfo_fixture(&mut self) -> BTreeMap<String, openpatch_core::ModuleLightState> {
        let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[
                {"id":"mono","kind":"stock_lfo","params":{"rate":0}},
                {"id":"clocked","kind":"stock_lfo","params":{"rate":0,"invert":1}},
                {"id":"poly","kind":"stock_lfo","params":{"rate":2,"offset":0,"fm_amount":0.5}},
                {"id":"clock","kind":"multi_lfo","params":{"frequency":8,"unipolar":1}},
                {"id":"fm","kind":"poly_constant","params":{"channels":5,"v1":-2,"v2":-1,"v3":0,"v4":1,"v5":2}},
                {"id":"scope","kind":"scope","params":{"time":0.15,"trigger_enabled":0}}],
            "cables":[
                {"from":"clock","output":"square","to":"clocked","input":"clock"},
                {"from":"fm","to":"poly","input":"fm"},
                {"from":"mono","output":"sine","to":"scope","input":"x"},
                {"from":"poly","output":"triangle","to":"scope","input":"y"}],
            "output":"mono","output_port":"sine"})).expect("Stock LFO fixture");
        let mut engine = patch.compile().expect("Stock LFO fixture compiles");
        engine.set_port_lights_enabled(true);
        let ids = ["mono", "clocked", "poly"];
        let ports = ["sine", "triangle", "saw", "square"];
        let taps: [_; 12] = std::array::from_fn(|index| {
            Some(
                engine
                    .resolve_tap(ids[index / 4], ports[index % 4], 0)
                    .unwrap(),
            )
        });
        let mut audio = vec![[0.0; 2]; 18000];
        let mut observed = vec![[openpatch_core::TapSample::default(); 12]; audio.len()];
        engine.process_audio_block_with_taps(&mut audio, &taps, &mut observed);
        assert!(audio.iter().flatten().all(|v| v.is_finite()));
        let mut lights = BTreeMap::new();
        for id in ids {
            let state = engine
                .module_lights(engine.resolve_module_lights(id).unwrap())
                .unwrap();
            assert_eq!(state.count, 5);
            let clock = state.clock_frequency.expect("Actual measured clock");
            if id == "clocked" {
                assert!((clock - 8.0).abs() < 0.1);
            } else {
                assert_eq!(clock, 2.0);
            }
            assert!(state
                .brightness
                .iter()
                .flatten()
                .all(|value| value.is_finite()));
            if id == "poly" {
                assert_eq!(state.brightness[2][0], 1.0);
                assert_eq!(state.brightness[0][0], 0.0);
            } else {
                assert!(state.brightness[0][0] > 0.0);
            }
            lights.insert(id.to_owned(), state);
        }
        assert_eq!(lights["mono"].brightness[4][0], 1.0);
        assert_eq!(lights["clocked"].brightness[3][0], 1.0);
        assert!(observed
            .last()
            .unwrap()
            .iter()
            .all(|sample| sample.active && sample.value.is_finite()));
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!((scope.x_channels, scope.y_channels), (1, 5));
        assert!(scope.x[0]
            .iter()
            .any(|point| point.min.is_finite() && point.max > point.min));
        self.smoke_motion_origin = None;
        self.keyboard_open = false;
        self.settings_open = false;
        self.audio_performance_open = false;
        self.select_analysis(None);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&json!({"version":1,"zoom":0.85,"pan":[24,12],
            "positions":{"mono":[0,0],"clocked":[390,0],"poly":[780,0],
                "scope":[0,420],"clock":[720,420],"fm":[1080,420]}}))
            .unwrap();
        self.rack.set_agent_follow(false);
        self.rack.set_module_states(lights.clone(), BTreeMap::new());
        self.rack
            .set_scope_states(BTreeMap::from([("scope".into(), Arc::new(scope))]));
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.browser_open = false;
        self.title = "Stock LFO · free, clocked, and polyphonic".into();
        self.status = "Actual offline Engine · free Hz / measured clock ratio · mono phase and poly blue lamps · five-voice Scope · no live audio".into();
        lights
    }

    fn prepare_stock_mix_fixture(&mut self) -> openpatch_core::ScopeSnapshot {
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/stock-mix-voice.json"))
                .expect("valid Stock Mix layered voice");
        let mut engine = patch.compile().expect("Stock Mix fixture compiles");
        engine.set_port_lights_enabled(true);
        let mut audio = vec![[0.0; 2]; 24032];
        engine.process_audio_block(&mut audio);
        assert!(audio.iter().flatten().all(|sample| sample.is_finite()));
        for channel in 0..2 {
            assert!(audio.iter().any(|sample| sample[channel].abs() > 0.001));
        }
        assert!(
            audio.iter().all(|sample| sample[0] == sample[1]),
            "Mono mix is explicitly sent to both audio channels"
        );
        let lights: BTreeMap<_, _> = patch
            .nodes
            .iter()
            .filter_map(|node| {
                let handle = engine.resolve_module_lights(&node.id).ok()?;
                Some((node.id.clone(), engine.module_lights(handle)?))
            })
            .collect();
        assert!(lights.get("mix").is_none_or(|state| state.count == 0));
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
        for lane in [&scope.x[0], &scope.y[0]] {
            assert!(lane
                .iter()
                .any(|point| point.min.is_finite() && point.max.abs() > 0.001));
        }
        self.smoke_motion_origin = None;
        self.keyboard_open = false;
        self.settings_open = false;
        self.audio_performance_open = false;
        self.select_analysis(None);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&Self::stock_mix_voice_layout())
            .unwrap();
        self.rack.set_agent_follow(false);
        self.rack.set_module_states(lights, BTreeMap::new());
        self.rack
            .set_scope_states(BTreeMap::from([("scope".into(), Arc::new(scope.clone()))]));
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.browser_open = false;
        self.title = "Stock Mix · layered voice".into();
        self.status = "Actual offline Engine · Scope X: averaged mix, Y: bass reference · slow pitch modulation · centered mono · no Mix lamps · no live audio".into();
        scope
    }

    fn prepare_stock_vca_mix_fixture(&mut self) -> openpatch_core::ScopeSnapshot {
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/stock-vca-mix-voice.json"))
                .expect("valid Stock VCA Mix voice");
        assert_eq!(patch.nodes.len(), 9);
        assert_eq!(patch.cables.len(), 10);
        let mut engine = patch.compile().expect("Stock VCA Mix fixture compiles");
        engine.set_port_lights_enabled(true);
        let mut audio = vec![[0.0; 2]; 24_032];
        engine.process_audio_block(&mut audio);
        assert!(audio.iter().flatten().all(|sample| sample.is_finite()));
        assert!(audio.iter().any(|sample| sample[0].abs() > 0.001));
        assert!(audio.iter().any(|sample| sample[1].abs() > 0.001));
        let lights: BTreeMap<_, _> = patch
            .nodes
            .iter()
            .filter_map(|node| {
                let handle = engine.resolve_module_lights(&node.id).ok()?;
                Some((node.id.clone(), engine.module_lights(handle)?))
            })
            .collect();
        let mixer_lights = lights.get("mix").expect("actual Stock VCA Mix meters");
        assert_eq!(mixer_lights.count, 4, "four native strip meter snapshots");
        for (index, brightness) in mixer_lights.brightness[..4]
            .iter()
            .map(|channels| channels[0])
            .enumerate()
        {
            assert!(
                brightness.is_finite() && brightness > 0.0,
                "strip {} should have actual meter emission",
                index + 1
            );
        }
        assert_ne!(
            mixer_lights.brightness[0][0], mixer_lights.brightness[3][0],
            "layer gains should produce distinct observed meter brightness"
        );
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
        assert!(scope.x[0]
            .iter()
            .any(|point| point.min.is_finite() && point.max.abs() > 0.001));
        assert!(scope.y[0]
            .iter()
            .any(|point| point.min.is_finite() && point.max.abs() > 0.001));

        self.smoke_motion_origin = None;
        self.keyboard_open = false;
        self.settings_open = false;
        self.audio_performance_open = false;
        self.select_analysis(None);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&Self::stock_vca_mix_voice_layout())
            .unwrap();
        self.rack.set_agent_follow(false);
        self.rack.set_module_states(lights, BTreeMap::new());
        self.rack
            .set_scope_states(BTreeMap::from([("scope".into(), Arc::new(scope.clone()))]));
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.browser_open = false;
        self.title = "Stock VCA Mix · four real strip meters".into();
        self.status = "Actual offline Engine · four observed strip VU snapshots · channel and master CV · Scope X: strip 1, Y: summed mix · no live audio".into();
        scope
    }

    fn prepare_stock_octave_fixture(&mut self) {
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/stock-octave-voice.json"))
                .expect("valid Stock Octave voice");
        let mut engine = patch.compile().expect("Stock Octave fixture compiles");
        engine.set_port_lights_enabled(true);
        let mut audio = vec![[0.0; 2]; 24032];
        engine.process_audio_block(&mut audio);
        let lights: BTreeMap<_, _> = patch
            .nodes
            .iter()
            .filter_map(|node| {
                let handle = engine.resolve_module_lights(&node.id).ok()?;
                Some((node.id.clone(), engine.module_lights(handle)?))
            })
            .collect();
        assert!(lights
            .get("octave")
            .and_then(|state| state.octave_shift)
            .is_some());
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        self.smoke_motion_origin = None;
        self.keyboard_open = false;
        self.settings_open = false;
        self.audio_performance_open = false;
        self.select_analysis(None);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&Self::stock_octave_voice_layout())
            .unwrap();
        self.rack.set_agent_follow(false);
        self.rack.set_module_states(lights, BTreeMap::new());
        self.rack
            .set_scope_states(BTreeMap::from([("scope".into(), Arc::new(scope))]));
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.browser_open = false;
        self.title = "Stock Octave · polyphonic pitch shifter".into();
        self.status = "Three pitch lanes · LFO shift CV · actual last channel 0 effective offset · attenuated stereo voice".into();
    }

    fn prepare_stock_mid_side_fixture(&mut self) -> openpatch_core::ScopeSnapshot {
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/stock-mid-side-voice.json"))
                .expect("valid Mid/Side stereo voice");
        let mut engine = patch.compile().expect("Stock Mid/Side fixture compiles");
        engine.set_port_lights_enabled(true);
        let mut audio = vec![[0.0; 2]; 24032];
        engine.process_audio_block(&mut audio);
        assert!(audio.iter().flatten().all(|sample| sample.is_finite()));
        for channel in 0..2 {
            assert!(audio.iter().any(|sample| sample[channel].abs() > 0.001));
        }
        assert!(audio
            .iter()
            .any(|sample| (sample[0] - sample[1]).abs() > 0.001));
        let lights: BTreeMap<_, _> = patch
            .nodes
            .iter()
            .filter_map(|node| {
                let handle = engine.resolve_module_lights(&node.id).ok()?;
                Some((node.id.clone(), engine.module_lights(handle)?))
            })
            .collect();
        assert!(lights.get("width").is_none_or(|state| state.count == 0));
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
        for lane in [&scope.x[0], &scope.y[0]] {
            assert!(lane
                .iter()
                .any(|point| point.min.is_finite() && point.max.abs() > 0.001));
        }
        self.smoke_motion_origin = None;
        self.keyboard_open = false;
        self.settings_open = false;
        self.audio_performance_open = false;
        self.select_analysis(None);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&Self::stock_mid_side_voice_layout())
            .unwrap();
        self.rack.set_agent_follow(false);
        self.rack.set_module_states(lights, BTreeMap::new());
        self.rack
            .set_scope_states(BTreeMap::from([("scope".into(), Arc::new(scope.clone()))]));
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.browser_open = false;
        self.title = "Stock Mid/Side · stereo voice".into();
        self.status = "Actual offline Engine · Scope X: encoded Mid, Y: encoded Side · slow bipolar width CV · no Mid/Side module lamps · no live audio".into();
        scope
    }

    fn prepare_stock_quantizer_fixture(&mut self) -> openpatch_core::ModuleLightState {
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/stock-quantized-voice.json"))
                .expect("valid Quantized stock voice");
        let mut engine = patch.compile().expect("Stock Quantizer fixture compiles");
        engine.set_port_lights_enabled(true);
        let mut audio = vec![[0.0; 2]; 24032];
        engine.process_audio_block(&mut audio);
        assert!(audio.iter().flatten().all(|sample| sample.is_finite()));
        assert!(audio.iter().flatten().any(|sample| sample.abs() > 0.001));
        let lights: BTreeMap<_, _> = patch
            .nodes
            .iter()
            .filter_map(|node| {
                let handle = engine.resolve_module_lights(&node.id).ok()?;
                Some((node.id.clone(), engine.module_lights(handle)?))
            })
            .collect();
        let observed = lights["scale"];
        assert_eq!(observed.count, 0);
        assert_eq!(observed.playing_notes.unwrap().count_ones(), 1);
        let saved = engine
            .module_saved_state(engine.resolve_module_state("scale").unwrap())
            .unwrap();
        let openpatch_core::SavedModuleState::StockQuantizerNotes(mask) = saved else {
            unreachable!()
        };
        assert_eq!(mask, 2741);
        assert_ne!(observed.playing_notes.unwrap() & mask, 0);
        let telemetry = engine
            .module_telemetry(engine.resolve_module_telemetry("sequence").unwrap())
            .unwrap();
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
        self.smoke_motion_origin = None;
        self.keyboard_open = false;
        self.settings_open = false;
        self.audio_performance_open = false;
        self.select_analysis(None);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&Self::stock_quantized_voice_layout())
            .unwrap();
        self.rack.set_agent_follow(false);
        self.rack
            .set_sequence_states(BTreeMap::from([("sequence".into(), telemetry)]), false);
        self.rack
            .set_stock_quantizer_states(BTreeMap::from([("scale".into(), mask)]));
        self.rack.set_module_states(lights, BTreeMap::new());
        self.rack
            .set_scope_states(BTreeMap::from([("scope".into(), Arc::new(scope))]));
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.browser_open = false;
        self.title = "Stock Quantizer · raw pitch to C major".into();
        self.status = "Actual offline Engine · enabled scale and played-note union · Scope compares raw and quantized pitch · no live audio".into();
        observed
    }

    fn prepare_stock_seq3_fixture(&mut self) -> openpatch_core::ModuleTelemetry {
        let mut patch: Patch =
            serde_json::from_str(include_str!("../../../examples/stock-sequenced-voice.json"))
                .expect("valid Stock sequenced voice");
        patch
            .nodes
            .iter_mut()
            .find(|node| node.id == "sequence")
            .unwrap()
            .params
            .insert("steps".into(), 6.0);
        let mut engine = patch.compile().expect("Stock SEQ3 fixture compiles");
        engine.set_port_lights_enabled(true);
        let mut audio = vec![[0.0; 2]; 24032];
        engine.process_audio_block(&mut audio);
        assert!(audio.iter().flatten().all(|value| value.is_finite()));
        assert!(audio.iter().flatten().any(|value| value.abs() > 0.001));
        let telemetry = engine
            .module_telemetry(engine.resolve_module_telemetry("sequence").unwrap())
            .unwrap();
        assert_eq!(telemetry.length, 6);
        assert!(telemetry.running && (2..=3).contains(&telemetry.step));
        let lights: BTreeMap<_, _> = patch
            .nodes
            .iter()
            .filter_map(|node| {
                let handle = engine.resolve_module_lights(&node.id).ok()?;
                Some((node.id.clone(), engine.module_lights(handle)?))
            })
            .collect();
        let lamps = lights["sequence"];
        assert_eq!(lamps.count, 19);
        assert_eq!(lamps.brightness[1][0], 1.0);
        assert_eq!(lamps.brightness[6][0], 0.0); // disabled step4 gate
        assert_eq!(lamps.brightness[17][1], 1.0); // step7 beyond length
        assert_eq!(lamps.brightness[18][1], 1.0); // step8 beyond length
        assert!(lamps
            .brightness
            .iter()
            .flatten()
            .all(|value| value.is_finite()));
        let saved = engine
            .module_saved_state(engine.resolve_module_state("sequence").unwrap())
            .unwrap();
        let openpatch_core::SavedModuleState::StockSeq3 {
            running,
            gates,
            passthrough,
        } = saved
        else {
            unreachable!()
        };
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
        self.smoke_motion_origin = None;
        self.keyboard_open = false;
        self.settings_open = false;
        self.audio_performance_open = false;
        self.select_analysis(None);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&Self::stock_sequenced_voice_layout())
            .unwrap();
        self.rack.set_agent_follow(false);
        self.rack
            .set_sequence_states(BTreeMap::from([("sequence".into(), telemetry)]), false);
        self.rack.set_stock_seq3_states(BTreeMap::from([("sequence".into(),json!({"running":running,
            "gates":(0..8).map(|i| gates & (1 << i) != 0).collect::<Vec<_>>(),"passthrough":passthrough}))]));
        self.rack.set_module_states(lights, BTreeMap::new());
        self.rack
            .set_scope_states(BTreeMap::from([("scope".into(), Arc::new(scope))]));
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.browser_open = false;
        self.title = "Stock SEQ3 · three-row voice".into();
        self.status = "Actual offline Engine · pitch / cutoff / resonance rows · 27 source lamp components · six steps · no live audio".into();
        telemetry
    }

    fn prepare_stock_vco_fixture(&mut self) -> BTreeMap<String, openpatch_core::ModuleLightState> {
        let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[
                {"id":"mono","kind":"stock_vco","params":{"frequency":-12}},
                {"id":"synced","kind":"stock_vco","params":{"frequency":-24,"linear_fm":1,"soft_sync":1}},
                {"id":"poly","kind":"stock_vco","params":{"frequency":0,"pulse_width":0.3}},
                {"id":"clock","kind":"stock_lfo","params":{"rate":3,"offset":0}},
                {"id":"pitch","kind":"poly_constant","params":{"channels":5,"v1":-2,"v2":-1,"v3":0,"v4":1,"v5":2}},
                {"id":"scope","kind":"scope","params":{"time":0.008,"trigger_enabled":0}}],
            "cables":[
                {"from":"clock","output":"square","to":"synced","input":"sync"},
                {"from":"pitch","to":"poly","input":"pitch"},
                {"from":"mono","output":"sine","to":"scope","input":"x"},
                {"from":"poly","output":"saw","to":"scope","input":"y"}],
            "output":"mono","output_port":"sine"})).expect("Stock VCO fixture");
        let mut engine = patch.compile().expect("Stock VCO fixture compiles");
        engine.set_port_lights_enabled(true);
        let ids = ["mono", "synced", "poly"];
        let ports = ["sine", "triangle", "saw", "square"];
        let taps: [_; 12] =
            std::array::from_fn(|i| Some(engine.resolve_tap(ids[i / 4], ports[i % 4], 0).unwrap()));
        let mut audio = vec![[0.0; 2]; 18000];
        let mut observed = vec![[openpatch_core::TapSample::default(); 12]; audio.len()];
        engine.process_audio_block_with_taps(&mut audio, &taps, &mut observed);
        assert!(audio.iter().flatten().all(|v| v.is_finite()));
        assert!(audio.iter().flatten().any(|v| v.abs() > 0.1));
        let mut lights = BTreeMap::new();
        for id in ids {
            let state = engine
                .module_lights(engine.resolve_module_lights(id).unwrap())
                .unwrap();
            assert_eq!(state.count, 5);
            assert_eq!(state.clock_frequency, None);
            assert!(state.brightness.iter().flatten().all(|v| v.is_finite()));
            lights.insert(id.to_owned(), state);
        }
        assert_eq!(lights["poly"].brightness[2][0], 1.0);
        assert_eq!(lights["poly"].brightness[0][0], 0.0);
        assert_eq!(lights["poly"].brightness[1][0], 0.0);
        assert_eq!(lights["synced"].brightness[3][0], 1.0);
        assert_eq!(lights["synced"].brightness[4][0], 1.0);
        assert_eq!(lights["mono"].brightness[3][0], 0.0);
        assert_eq!(lights["mono"].brightness[4][0], 0.0);
        assert!(observed
            .last()
            .unwrap()
            .iter()
            .all(|sample| sample.active && sample.value.is_finite()));
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!((scope.x_channels, scope.y_channels), (1, 5));
        assert!(scope.x[0]
            .iter()
            .any(|point| point.min.is_finite() && point.max > point.min));
        self.smoke_motion_origin = None;
        self.keyboard_open = false;
        self.settings_open = false;
        self.audio_performance_open = false;
        self.select_analysis(None);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&json!({"version":1,"zoom":0.85,"pan":[24,12],
            "positions":{"mono":[0,0],"synced":[390,0],"poly":[780,0],
                "scope":[0,420],"clock":[720,420],"pitch":[1110,420]}}))
            .unwrap();
        self.rack.set_agent_follow(false);
        self.rack.set_module_states(lights.clone(), BTreeMap::new());
        self.rack
            .set_scope_states(BTreeMap::from([("scope".into(), Arc::new(scope))]));
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.browser_open = false;
        self.title = "Stock VCO · mono, soft sync, and five voices".into();
        self.status = "Actual offline Engine · phase RGB and Linear/Soft lamps · eight real jacks · five-voice Scope · no live audio".into();
        lights
    }

    fn prepare_wavetable_fixture(&mut self) {
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/wavetable-voice.json"))
                .expect("Valid wavetable voice");
        let mut engine = patch.compile().expect("Wavetable fixture compiles");
        engine.set_port_lights_enabled(true);
        let morph = engine.resolve_tap("morph", "out", 0).unwrap();
        let mut audio = vec![[0.0; 2]; 4096];
        let mut taps = vec![[openpatch_core::TapSample::default(); 1]; audio.len()];
        engine.process_audio_block_with_taps(&mut audio, &[Some(morph)], &mut taps);
        assert!(audio.iter().flatten().all(|value| value.is_finite()));
        assert!(audio.iter().flatten().any(|value| value.abs() > 0.001));
        let lights: BTreeMap<_, _> = ["voice", "morph", "amplifier", "envelope"]
            .into_iter()
            .map(|id| {
                (
                    id.to_owned(),
                    engine
                        .module_lights(engine.resolve_module_lights(id).unwrap())
                        .unwrap(),
                )
            })
            .collect();
        let actual_position = lights["voice"]
            .waveform_position
            .expect("VCO table position");
        let expected_position = (0.1 + taps[taps.len() - 2][0].value * 0.8 / 10.0).clamp(0.0, 1.0);
        assert!(
            (actual_position - expected_position).abs() < 0.000001,
            "Wavetable display must show the actual delayed LFO modulation"
        );
        assert!(actual_position > 0.1);
        assert_eq!(lights["morph"].waveform_position, Some(0.2));
        for id in ["voice", "morph", "amplifier", "envelope"] {
            assert!(
                lights[id]
                    .brightness
                    .iter()
                    .flatten()
                    .any(|value| *value > 0.0),
                "Wavetable fixture needs actual lit telemetry for {id}"
            );
        }
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
        assert!(scope.x[0]
            .iter()
            .any(|point| point.min.is_finite() && point.max.abs() > 0.001));
        assert!(scope.y[0]
            .iter()
            .any(|point| point.min.is_finite() && point.max.abs() > 0.001));
        self.smoke_motion_origin = None;
        self.keyboard_open = false;
        self.settings_open = false;
        self.preset_dialog = None;
        self.file_mode = None;
        self.select_analysis(None);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&Self::wavetable_voice_layout())
            .unwrap();
        self.rack.set_agent_follow(false);
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.rack.set_module_states(lights, BTreeMap::new());
        self.rack
            .set_scope_states(BTreeMap::from([("scope".into(), Arc::new(scope))]));
        self.browser_open = false;
        self.title = "Wavetable voice · a table in motion".into();
        self.status = "Actual offline DSP snapshot · LFO moves the VCO table · VCA/ADSR lights and both Scope traces share one frame · no live audio".into();
    }

    fn prepare_stock_delay_fixture(&mut self) {
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/stock-delay-voice.json"))
                .expect("Valid stock delay voice");
        let mut engine = patch.compile().expect("Stock delay fixture compiles");
        let clock_handle = engine
            .resolve_module_lights("echo")
            .expect("Actual delay clock observer");
        let mut rendered = 0;
        let clock = loop {
            let mut audio = [0.0; 64];
            engine.process_block(&mut audio);
            assert!(audio.iter().all(|value| value.is_finite()));
            rendered += audio.len();
            let clock = engine.module_lights(clock_handle).unwrap();
            if rendered >= 48000 && clock.brightness[0][0] > 0.8 {
                break clock;
            }
            assert!(
                rendered < 96000,
                "Delay fixture did not observe a real clock pulse"
            );
        };
        assert_eq!(clock.count, 1);
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
        assert!(scope.x[0].iter().any(|point| point.max.abs() > 0.001));
        assert!(scope.y[0].iter().any(|point| point.max.abs() > 0.001));
        self.smoke_motion_origin = None;
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&json!({"version":1,"zoom":0.8,"pan":[24,12],"positions":{
            "voice":[0,0],"amplifier":[330,0],"echo":[540,0],"out":[930,0],"guide":[1140,0],
            "clock":[0,420],"envelope":[180,420],"scope":[570,420]}}))
            .unwrap();
        self.rack.set_agent_follow(false);
        self.rack
            .set_module_states(BTreeMap::from([("echo".into(), clock)]), BTreeMap::new());
        self.rack.set_scope_states(BTreeMap::from([(
            "scope".into(),
            std::sync::Arc::new(scope),
        )]));
        self.browser_open = false;
        self.select_analysis(None);
        self.title = "Delay · a clocked echo voice".into();
        self.status = "Actual offline DSP snapshot · MIX and WET Scope traces · CLK from the delay engine · no live audio".into();
    }

    fn prepare_stock_vcf_fixture(&mut self) {
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/stock-vcf-voice.json")).unwrap();
        let mut engine = patch.compile().unwrap();
        let mut audio = vec![0.0; 16384];
        engine.process_block(&mut audio);
        assert!(audio.iter().all(|v| v.is_finite()));
        assert!(audio.iter().any(|v| v.abs() > 0.001));
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
        assert!(scope.x[0].iter().any(|p| p.max.abs() > 0.01));
        assert!(scope.y[0].iter().any(|p| p.max.abs() > 0.01));
        let envelope = engine
            .module_lights(engine.resolve_module_lights("envelope").unwrap())
            .unwrap();
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&json!({"version":1,"zoom":0.80,"pan":[24,12],"positions":{
            "voice":[0,0],"filter":[330,0],"amplifier":[630,0],"out":[810,0],"guide":[1020,0],
            "clock":[0,420],"envelope":[180,420],"scope":[570,420]}}))
            .unwrap();
        self.rack.set_module_states(
            BTreeMap::from([("envelope".into(), envelope)]),
            BTreeMap::new(),
        );
        self.rack.set_scope_states(BTreeMap::from([(
            "scope".into(),
            std::sync::Arc::new(scope),
        )]));
        self.browser_open = false;
        self.select_analysis(None);
        self.title = "VCF · clocked filter voice".into();
        self.status = "Connected saw → VCF → VCA · actual LP/HP Scope capture · Hz entry and bipolar CV depth".into();
    }

    fn prepare_midi_cc_fixture(&mut self) {
        let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[{"id":"controller","kind":"midi_cc","params":{"cc1":7,"cc7":-1,"mpe":1,"high_resolution":1,"smooth":0}},
                {"id":"scope","kind":"scope","params":{"time":0.15,"trigger_enabled":0,"x_offset":-5}}],
            "cables":[{"from":"controller","output":"out1","to":"scope","input":"x"}] })).unwrap();
        let mut engine = patch.compile().unwrap();
        for frame in 0..8192 {
            if frame % 64 == 0 {
                for channel in 0..16 {
                    let value = ((frame / 64 + usize::from(channel) * 7) % 128) as u8;
                    engine.handle_midi(openpatch_core::MidiEvent::ControlChange {
                        channel,
                        controller: 7,
                        value,
                    });
                    engine.handle_midi(openpatch_core::MidiEvent::ControlChange {
                        channel,
                        controller: 39,
                        value: 64,
                    });
                }
            }
            engine.process_block(&mut [0.0]);
        }
        let handle = engine.resolve_module_state("controller").unwrap();
        assert!(engine.arm_midi_cc_learn(handle, 1, 15));
        let Some(openpatch_core::SavedModuleState::MidiCc {
            assignments,
            values,
        }) = engine.module_saved_state(handle)
        else {
            panic!("actual controller snapshot");
        };
        let learning = engine
            .midi_cc_learning(handle)
            .map(|(serial, cell)| json!({"serial":serial,"cell":cell+1}));
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!(scope.x_channels, 16);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&json!({"version":1,"zoom":1.0,"pan":[24,12],"positions":{
            "controller":[0,0],"scope":[510,0]}}))
            .unwrap();
        self.rack.set_midi_cc_states(BTreeMap::from([(
            "controller".into(),
            json!({
            "assignments":assignments,"values":values.to_vec(),"learning":learning}),
        )]));
        self.rack.set_scope_states(BTreeMap::from([(
            "scope".into(),
            std::sync::Arc::new(scope),
        )]));
        self.browser_open = false;
        self.select_analysis(None);
        self.title = "MIDI CC · sixteen outputs, sixteen voices".into();
        self.status="Actual offline DSP snapshot · 14-bit paired CC7, 16 MPE lanes · output 16 Learn armed · no live MIDI device".into();
    }

    fn prepare_midi_gate_fixture(&mut self) {
        let patch:Patch=serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[{"id":"pads","kind":"midi_gate","params":{"velocity":1,"mpe":1}},
                {"id":"scope","kind":"scope","params":{"time":0.15,"trigger_enabled":0,"x_offset":-5}}],
            "cables":[{"from":"pads","output":"gate1","to":"scope","input":"x"}]})).unwrap();
        let mut engine = patch.compile().unwrap();
        let notes = openpatch_core::midi_gate::DEFAULT_ASSIGNMENTS;
        for (cell, note) in notes.iter().enumerate().skip(1) {
            if cell % 2 == 1 {
                engine.handle_midi(openpatch_core::MidiEvent::NoteOn {
                    channel: 0,
                    note: *note as u8,
                    velocity: ((cell + 1) * 8).min(127) as u8,
                });
            }
        }
        for frame in 0..8192 {
            if frame % 128 == 0 || frame % 128 == 64 {
                for channel in 0..16 {
                    let note = notes[0] as u8;
                    let event = if frame % 128 == 0 {
                        openpatch_core::MidiEvent::NoteOn {
                            channel,
                            note,
                            velocity: (8 + usize::from(channel) * 7 + (frame / 128) % 16).min(127)
                                as u8,
                        }
                    } else {
                        openpatch_core::MidiEvent::NoteOff {
                            channel,
                            note,
                            velocity: 0,
                        }
                    };
                    engine.handle_midi(event);
                }
            }
            engine.process_block(&mut [0.0]);
        }
        let handle = engine.resolve_module_state("pads").unwrap();
        assert!(engine.arm_midi_gate_learn(handle, 1, 15));
        let Some(openpatch_core::SavedModuleState::MidiGateMappings(assignments)) =
            engine.module_saved_state(handle)
        else {
            panic!("actual gate mappings")
        };
        let learning = engine
            .midi_gate_learning(handle)
            .map(|(serial, cell)| json!({"serial":serial,"cell":cell+1}));
        let lights = engine
            .module_lights(engine.resolve_module_lights("pads").unwrap())
            .unwrap();
        assert_eq!(lights.count, 16);
        assert_eq!(lights.brightness.iter().filter(|v| v[0] > 0.0).count(), 8);
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!(scope.x_channels, 16);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack.restore_layout(&json!({"version":1,"zoom":1.0,"pan":[24,12],"positions":{"pads":[0,0],"scope":[510,0]}})).unwrap();
        self.rack.set_midi_gate_states(BTreeMap::from([(
            "pads".into(),
            json!({"assignments":assignments,"learning":learning}),
        )]));
        self.rack
            .set_module_states(BTreeMap::from([("pads".into(), lights)]), BTreeMap::new());
        self.rack.set_scope_states(BTreeMap::from([(
            "scope".into(),
            std::sync::Arc::new(scope),
        )]));
        self.browser_open = false;
        self.select_analysis(None);
        self.title = "MIDI Gate · notes into rhythm".into();
        self.status="Actual offline DSP snapshot · velocity gates, sixteen MPE lanes · eight held pads · Learn armed on output16 · no live MIDI device".into();
    }

    fn prepare_annotations_fixture(&mut self) {
        let patch:Patch=serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"guide","kind":"notes","content":{"type":"notes","text":"PATCH LAB\n日本語 · café · 🎛 · 𝄞\n\nMIDI → Gate → rhythm\n\nZ / X / C play the drum pads.\nLearn assigns your controller.\nPanic releases held gates.\n\nNotes travel with your patch,\npresets and duplicated modules.\n\nSpaces have a purpose, too.\nDrag either Blank edge to resize."}},
            {"id":"thin","kind":"blank","content":{"type":"blank","width_hp":3}},
            {"id":"space","kind":"blank"},
            {"id":"wide","kind":"blank","content":{"type":"blank","width_hp":24}}
        ]})).unwrap();
        assert!(self.command("replace_patch", json!({"patch":patch})));
        self.rack.restore_layout(&json!({"version":1,"zoom":1,"pan":[24,12],"positions":{"guide":[0,0],"thin":[270,0],"space":[345,0],"wide":[525,0]}})).unwrap();
        self.rack.set_room_brightness(1.0);
        self.browser_open = false;
        self.select_analysis(None);
        self.title = "Notes & Blank · a readable instrument".into();
        self.status =
            "Saved Unicode notes · 3, 10 and 24 HP spacers · original shared rack/browser panels"
                .into();
    }

    fn smoke(&mut self, ctx: &egui::Context) {
        let Some((path, frame, captures)) = &mut self.smoke_test else {
            return;
        };
        // After initialization, all smoke scene changes and capture requests
        // use even logical frames. Skip the idle odd frames so software GL
        // keeps ample room inside CI's fixed native-window time budget.
        // Production repaint and construction clocks are unchanged.
        let receipt_id = egui::Id::new("native-smoke-static-receipt");
        let pending = ctx.data(|d| d.get_temp::<crate::smoke_schedule::Receipt>(receipt_id));
        let previous_frame = *frame;
        *frame = crate::smoke_schedule::next_frame(*frame, pending);
        // Keep every existing capture on its original logical frame. The final
        // appended product scene needs odd input/capture milestones, so advance
        // only this tail explicitly and hold its last receipt frame if needed.
        if previous_frame >= 2950 {
            *frame = match previous_frame {
                2950 => 2951,
                2951 => 2953,
                2953 => 2954,
                2954 => 2955,
                2955 => 2956,
                2956 => 2957,
                2957 => 2958,
                2958 | 2959 => 2959,
                _ => *frame,
            };
        }
        let (path, frame, captures) = (path.clone(), *frame, *captures);
        if captures >= 139 {
            assert!(frame < 2960, "Native screenshot timeout");
            self.smoke_noise_tail(ctx, &path, captures);
            return;
        }
        if frame == 1 {
            let v = self
                .visualizer
                .as_mut()
                .expect("Native shader must compile");
            let gl = self.gl.as_ref().expect("GL context required");
            assert!(v.compile(gl, "this is not GLSL").is_err());
            v.compile(gl, DEFAULT_SHADER).expect("Shader recovery");
        }
        if frame <= 28 {
            self.select_analysis(
                ((2..=8).contains(&frame) || captures == 1).then_some(AnalysisTab::Visuals),
            );
        }
        self.visual_mode = match frame {
            2..=4 => 1,
            5..=7 => 2,
            _ => 0,
        };
        if frame == 28 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/clocked-random-voice.json"))
                    .expect("valid bundled random voice");
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("random voice compiles");
            self.replace_rack(Rack::default());
            self.rack.set_zoom(0.75);
            self.title = "Clocked random voice".into();
        }
        if frame == 40 || frame == 52 {
            let (source, title, zoom) = if frame == 40 {
                (
                    include_str!("../../../examples/polyphonic-voice.json"),
                    "Polyphonic voice",
                    0.75,
                )
            } else {
                (
                    include_str!("../../../examples/polyphonic-routing.json"),
                    "Polyphonic routing study",
                    0.85,
                )
            };
            let mut patch: Patch =
                serde_json::from_str(source).expect("valid bundled polyphonic patch");
            let meter_lights = (frame == 52).then(|| {
                for channel in 1..=16 {
                    patch.nodes[0].params.insert(
                        format!("v{channel}"),
                        if channel % 2 == 0 { -4.0 } else { 8.0 },
                    );
                }
                patch
                    .nodes
                    .push(serde_json::from_value(json!({"id":"viz","kind":"poly_viz"})).unwrap());
                patch.cables.push(
                    serde_json::from_value(json!({"from":"voltages","to":"viz","input":"signal"}))
                        .unwrap(),
                );
                let mut engine = patch.compile().expect("routing meter fixture compiles");
                engine.process_block(&mut [0.0; 1024]);
                let mut states = BTreeMap::new();
                for id in ["sum", "viz"] {
                    let handle = engine.resolve_module_lights(id).expect("meter handle");
                    let state = engine.module_lights(handle).expect("meter snapshot");
                    assert_eq!(state.input_channels, Some(16));
                    states.insert(id.into(), state);
                }
                assert_eq!(states["sum"].brightness[0][0], 1.0);
                assert_eq!(states["viz"].brightness[0][0], 0.8);
                assert_eq!(states["viz"].brightness[1][1], 0.4);
                states
            });
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("polyphonic patch compiles");
            self.replace_rack(Rack::default());
            self.rack.set_zoom(zoom);
            self.title = title.into();
            if let Some(lights) = meter_lights {
                self.rack.set_module_states(lights, BTreeMap::new());
                self.title.push_str(" · offline DSP snapshot");
            }
        }
        if frame == 64 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/midi-keyboard-voice.json"))
                    .expect("valid bundled MIDI voice");
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("MIDI voice compiles");
            self.replace_rack(Rack::default());
            self.rack.set_zoom(0.75);
            self.title = "MIDI keyboard voice".into();
            self.keyboard_open = true;
        }
        if frame == 76 {
            self.settings_open = true;
        }
        if frame == 88 {
            self.settings_open = false;
            self.keyboard_open = false;
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/stereo-width-study.json"))
                    .expect("valid bundled stereo study");
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("stereo study compiles");
            self.replace_rack(Rack::default());
            self.rack.set_zoom(1.0);
            self.title = "Stereo width study".into();
        }
        if frame == 100 {
            let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
                "nodes":[{"id":"mix","kind":"mix6"},{"id":"octave","kind":"octave"},
                    {"id":"mult","kind":"mult"},{"id":"stereo","kind":"mid_side"}]}))
            .unwrap();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("routing modules compile");
            self.replace_rack(Rack::default());
            self.title = "Stereo and routing utilities".into();
        }
        if frame == 112 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/clocked-modulation.json"))
                    .expect("valid bundled modulation voice");
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("modulation voice compiles");
            self.replace_rack(Rack::default());
            self.rack.set_zoom(0.75);
            self.title = "Clocked modulation".into();
        }
        if frame == 124 {
            self.file_mode = Some(FileMode::Save);
            self.file_focus = true;
            self.filename = "clocked-modulation.openpatch".into();
        }
        if frame == 136 {
            self.file_mode = None;
            self.confirm_discard = Some(PendingAction::Close);
        }
        if frame == 148 {
            self.confirm_discard = None;
            let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,"nodes":[{"id":"calibration","kind":"multi_lfo","params":{"frequency":100.0}}],"output":"calibration","output_port":"sine"})).unwrap();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("scope calibration compiles");
            self.replace_rack(Rack::default());
            self.title = "Signal inspection".into();
            self.scope.smoke_fixture();
            self.select_analysis(Some(AnalysisTab::Scope));
        }
        if frame == 172 {
            self.select_analysis(None);
            let patch: Patch = serde_json::from_str(include_str!(
                "../../../examples/sequenced-analog-voice.json"
            ))
            .unwrap();
            let mut engine = patch.compile().unwrap();
            let handle = engine.resolve_module_event("sequence").unwrap();
            for _ in 0..3 {
                assert!(engine.handle_module_event(handle, openpatch_core::ModuleAction::Clock));
            }
            engine.process_block(&mut [0.0; 128]);
            let telemetry = engine.module_telemetry(handle).unwrap();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("sequence compiles");
            self.replace_rack(Rack::default());
            let _ = self.rack.restore_layout(&json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
                "sequence":[0,0],"vco":[780,0],"filter":[990,0],"envelope":[0,420],"vca":[240,420],"accent":[390,420],"out":[540,420]
            }}));
            self.rack
                .set_sequence_states(BTreeMap::from([("sequence".into(), telemetry)]), false);
            self.title = "Sequenced analog voice · offline preview".into();
        }
        if frame == 188 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/switched-stereo-voice.json"))
                    .unwrap();
            let mut engine = patch.compile().unwrap();
            engine.process_block(&mut [0.0; 128]);
            let states = ["select", "route"]
                .into_iter()
                .map(|id| {
                    let handle = engine.resolve_module_telemetry(id).unwrap();
                    (id.to_owned(), engine.module_telemetry(handle).unwrap())
                })
                .collect();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("switching voice compiles");
            self.replace_rack(Rack::default());
            let _ = self.rack.restore_layout(&json!({"version":1,"zoom":0.9,"pan":[24,12],"positions":{
                "pitches":[0,0],"split":[360,0],"select":[660,0],"clock":[840,0],
                "vco":[0,420],"route":[210,420],"left":[390,420],"right":[540,420],"out":[690,420]
            }}));
            self.rack.set_sequence_states(states, false);
            self.title = "Switched stereo voice · offline preview".into();
        }
        if frame == 204 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/voltage-utilities.json"))
                    .expect("valid voltage utilities fixture");
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("voltage utilities compile");
            self.replace_rack(Rack::default());
            self.title = "Voltage utilities".into();
        }
        if frame == 220 {
            execute_shared(
                &self.shared,
                "set_bypass",
                json!({"id":"mutes","bypassed":true}),
            )
            .expect("mute bypass fixture");
            self.title = "Voltage utilities · Mutes bypassed".into();
        }
        if frame == 244 || frame == 260 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/gate-shaping-voice.json"))
                    .expect("valid gate shaping fixture");
            let mut engine = patch.compile().expect("gate shaping fixture compiles");
            engine.process_block(&mut vec![0.0; 48_000]);
            if frame == 260 {
                for (id, event) in [
                    ("logic", openpatch_core::ModuleAction::BPress),
                    ("process", openpatch_core::ModuleAction::GatePress),
                ] {
                    let handle = engine.resolve_module_event(id).unwrap();
                    assert!(engine.handle_module_event(handle, event));
                }
                engine.process_block(&mut [0.0; 128]);
            }
            let mut lights = BTreeMap::new();
            let mut controls = BTreeMap::new();
            for id in ["logic", "compare", "process", "rescale"] {
                if let Ok(handle) = engine.resolve_module_lights(id) {
                    if let Some(state) = engine.module_lights(handle) {
                        lights.insert(id.into(), state);
                    }
                }
                if let Ok(handle) = engine.resolve_module_event(id) {
                    if let Some(state) = engine.module_control_state(handle) {
                        controls.insert(id.into(), state);
                    }
                }
            }
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("gate shaping fixture publishes");
            self.replace_rack(Rack::default());
            self.rack.restore_layout(&json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
                "logic":[0,0],"compare":[240,0],"process":[480,0],"rescale":[720,0],
                "motion":[0,420],"clock":[300,420],"notes":[450,420],"vco":[690,420],"out":[900,420]
            }})).expect("fixture layout");
            self.rack.set_module_states(lights, controls);
            self.title = if frame == 260 {
                "Manual gates · offline DSP snapshot"
            } else {
                "Gate shaping voice · offline preview"
            }
            .into();
        }
        if frame == 284 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/layered-stereo-morph.json"))
                    .expect("valid layered stereo fixture");
            let mut engine = patch.compile().expect("layered stereo fixture compiles");
            engine.process_block(&mut vec![0.0; 48_000]);
            let handle = engine
                .resolve_module_lights("mix")
                .expect("mixer meter handle");
            let lights = engine.module_lights(handle).expect("mixer meter snapshot");
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("layered stereo fixture publishes");
            self.replace_rack(Rack::default());
            self.rack
                .restore_layout(&Self::mixing_layout())
                .expect("fixture mixer layout");
            self.rack
                .set_module_states(BTreeMap::from([("mix".into(), lights)]), BTreeMap::new());
            self.title = "Layered stereo morph · offline DSP snapshot".into();
        }
        if frame == 308 {
            let patch:Patch=serde_json::from_value(json!({"version":2,"sample_rate":48000,"nodes":[
                {"id":"tone","kind":"analog_vco","params":{"frequency":130.8128}},
                {"id":"filter","kind":"ladder_vcf","params":{"cutoff":1200}},
                {"id":"pitch","kind":"constant","params":{"value":0.0}},
                {"id":"out","kind":"audio_out"}],
                "cables":[{"from":"pitch","to":"tone","input":"pitch"},{"from":"tone","to":"filter","input":"signal"},{"from":"filter","to":"out","input":"signal"}],"output":"out"})).unwrap();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("duplication fixture compiles");
            self.replace_rack(Rack::default());
            self.rack.restore_layout(&json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{"tone":[0,0],"filter":[210,0],"pitch":[0,420],"out":[210,420]}})).unwrap();
            let mapping = vec![
                ("tone".into(), "tone-copy-1".into()),
                ("filter".into(), "filter-copy-1".into()),
            ];
            let placement = self
                .rack
                .duplicate_placement(&patch, &mapping)
                .expect("duplicate group placement");
            execute_shared(&self.shared,"duplicate_modules",json!({"modules":[{"id":"tone","new_id":"tone-copy-1"},{"id":"filter","new_id":"filter-copy-1"}],"cable_mode":"inputs"})).expect("duplicate group fixture");
            self.rack.apply_duplicate_placement(placement, true);
            self.title = "Duplicated voice pair · two selected".into();
        }
        if frame == 332 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/stacked-pitch-voice.json"))
                    .expect("valid stacked pitch fixture");
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("stacked pitch fixture compiles");
            self.replace_rack(Rack::default());
            self.rack
                .restore_layout(&Self::stacked_layout())
                .expect("stacked fixture layout");
            self.title = "Stacked pitch voice · three cables at V/OCT".into();
        }
        if frame == 356 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/synced-waveform-voice.json"))
                    .expect("valid waveform fixture");
            let mut engine = patch.compile().expect("waveform fixture compiles");
            engine.process_block(&mut vec![0.0; 48_000]);
            let handle = engine
                .resolve_module_lights("mix")
                .expect("waveform meter handle");
            let lights = engine
                .module_lights(handle)
                .expect("waveform meter snapshot");
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("waveform fixture publishes");
            self.replace_rack(Rack::default());
            self.rack
                .restore_layout(&Self::waveform_layout())
                .expect("waveform fixture layout");
            self.rack
                .set_module_states(BTreeMap::from([("mix".into(), lights)]), BTreeMap::new());
            self.title = "Synced waveform voice · offline DSP snapshot".into();
        }
        if frame == 380 {
            self.audio = None;
            self.attach_live_events();
            let patch: Patch = serde_json::from_value(json!({
                "version":2,"sample_rate":48000,
                "nodes":[
                    {"id":"tone","kind":"analog_vco","params":{"frequency":220.0,"waveform":1.0}},
                    {"id":"out","kind":"audio_out"}
                ],
                "cables":[{"from":"tone","to":"out","input":"signal"}],
                "output":"out"
            }))
            .unwrap();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("preset fixture compiles");
            self.replace_rack(Rack::default());
            self.title = "VCO preset library · offline preview".into();
            let revision = self.shared.lock().unwrap().revision();
            self.preset_dialog = Some(PresetDialog::new(
                "tone".into(),
                "analog_vco".into(),
                revision,
                PresetMode::Save,
            ));
            if let Some(dialog) = &mut self.preset_dialog {
                dialog.name = "warm-vco".into();
            }
        }
        if frame == 398 {
            let preset = self
                .preset_dialog
                .as_ref()
                .and_then(|dialog| dialog.capture.clone())
                .expect("offline preset capture is visible");
            execute_shared(
                &self.shared,
                "save_module_preset",
                json!({
                    "filename":"warm-vco.opreset","preset":preset
                }),
            )
            .expect("smoke preset saves");
            self.preset_dialog = None;
        }
        if frame == 404 {
            self.preset_dialog = Some(PresetDialog::new(
                "tone".into(),
                "analog_vco".into(),
                self.revision,
                PresetMode::Browse,
            ));
        }
        if frame == 408 {
            let result = execute_shared(
                &self.shared,
                "read_module_preset",
                json!({
                    "filename":"warm-vco.opreset"
                }),
            )
            .expect("smoke preset reads");
            if let Some(dialog) = &mut self.preset_dialog {
                dialog.preview = result.get("preset").cloned();
                dialog.preview_filename = Some("warm-vco.opreset".into());
            }
        }
        if frame == 422 {
            self.preset_dialog = None;
            let patch: Patch = serde_json::from_value(json!({
                "version":2,"sample_rate":48000,
                "nodes":[
                    {"id":"pitch","kind":"constant","params":{"value":0.0}},
                    {"id":"tone","kind":"analog_vco","params":{"frequency":220.0}},
                    {"id":"out","kind":"audio_out"}
                ],
                "cables":[
                    {"from":"pitch","to":"tone","input":"pitch"},
                    {"from":"tone","to":"out","input":"signal"}
                ],
                "output":"out"
            }))
            .expect("outgoing duplicate fixture");
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("outgoing duplicate fixture compiles");
            self.replace_rack(Rack::default());
            self.rack
                .restore_layout(&json!({
                    "version":1,"zoom":0.85,"pan":[24,12],
                    "positions":{"pitch":[0,0],"tone":[150,0],"out":[720,0]}
                }))
                .expect("outgoing duplicate layout");
            let placement = self
                .rack
                .duplicate_placement(&patch, &[("tone".into(), "tone-copy".into())])
                .expect("outgoing clone placement");
            execute_shared(
                &self.shared,
                "duplicate_modules",
                json!({
                    "modules":[{"id":"tone","new_id":"tone-copy"}],"cable_mode":"all"
                }),
            )
            .expect("outgoing duplicate with cables");
            self.rack.apply_duplicate_placement(placement, false);
            self.title = "VCO copy · incoming pitch and outgoing audio cables".into();
        }
        if frame == 438 {
            self.browse();
        }
        if frame == 454 {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(940.0, 680.0)));
        }
        if frame == 470 {
            let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
                "nodes":[{"id":"calibration","kind":"multi_lfo","params":{"frequency":100.0}},
                    {"id":"tone","kind":"analog_vco"},{"id":"out","kind":"audio_out"}],
                "cables":[{"from":"tone","to":"out","input":"signal"}],"output":"out"}))
            .unwrap();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("compact analysis fixture compiles");
            self.replace_rack(Rack::default());
            self.rack.set_zoom(0.75);
            ctx.data_mut(|data| {
                data.insert_persisted(
                    egui::Id::new("analysis-dock"),
                    egui::containers::panel::PanelState {
                        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 500.0)),
                    },
                )
            });
            self.scope.smoke_fixture();
            self.select_analysis(Some(AnalysisTab::Scope));
            self.keyboard_open = true;
            self.title =
                "Studio Rack · signal inspection with a deliberately long project title".into();
            self.status = "Offline calibration fixture · use the Scope or Visuals tab while keeping the rack and library accessible".into();
        }
        if frame == 494 {
            self.select_analysis(Some(AnalysisTab::Visuals));
        }
        if frame == 518 {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(1440.0, 960.0)));
            self.select_analysis(None);
            self.keyboard_open = false;
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/manual-pulses-voice.json"))
                    .unwrap();
            let mut engine = patch.compile().expect("Pulses fixture compiles");
            let handle = engine.resolve_module_event("buttons").unwrap();
            engine.handle_module_event(handle, openpatch_core::ModuleAction::PulsePress(1));
            engine.handle_module_event(handle, openpatch_core::ModuleAction::PulsePress(10));
            engine.process_block(&mut [0.0; 64]);
            let lights = engine.module_lights(handle).unwrap();
            let controls = engine.module_control_state(handle).unwrap();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
            self.replace_rack(Rack::default());
            self.rack.restore_layout(&Self::pulses_layout()).unwrap();
            self.rack.set_module_states(
                BTreeMap::from([("buttons".into(), lights)]),
                BTreeMap::from([("buttons".into(), controls)]),
            );
            self.title = "Manual pulses voice · rows 1 and 10 held · offline preview".into();
            self.status = "Row 1 plays the envelope · Row 2 advances pitch · Row 3 toggles Run · Row 4 resets".into();
        }
        if frame == 542 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/manual-push-voice.json"))
                    .unwrap();
            let mut engine = patch.compile().expect("Push fixture compiles");
            let handle = engine.resolve_module_event("push").unwrap();
            engine.handle_module_event(handle, openpatch_core::ModuleAction::HoldPress);
            engine.handle_module_event(handle, openpatch_core::ModuleAction::HoldRelease);
            engine.process_block(&mut [0.0; 64]);
            let lights = engine.module_lights(handle).unwrap();
            let controls = engine.module_control_state(handle).unwrap();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
            self.replace_rack(Rack::default());
            self.rack.restore_layout(&Self::push_layout()).unwrap();
            self.rack.set_module_states(
                BTreeMap::from([("push".into(), lights)]),
                BTreeMap::from([("push".into(), controls)]),
            );
            self.title = "Manual push voice · HOLD latched · offline preview".into();
            self.status =
                "PUSH plays a gate · HOLD latches it · While latched, PUSH interrupts the gate"
                    .into();
        }
        if frame == 566 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/delayed-gate-voice.json"))
                    .unwrap();
            let mut engine = patch.compile().unwrap();
            let handle = engine.resolve_module_control("gates").unwrap();
            engine.process_block(&mut [0.0; 64]);
            let lights = engine.module_lights(handle).unwrap();
            let controls = engine.module_control_state(handle).unwrap();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
            self.replace_rack(Rack::default());
            self.rack.restore_layout(&Self::gates_layout()).unwrap();
            self.rack.set_module_states(
                BTreeMap::from([("gates".into(), lights)]),
                BTreeMap::from([("gates".into(), controls)]),
            );
            self.title = "Delayed gate voice · offline preview".into();
            self.status = "Left: timed gate · Right: delayed gate · LENGTH changes both · RESET clears timing".into();
        }
        if frame == 584 {
            // The native smoke runner has no output stream. Keep the screenshot
            // explicit about the simulated data rather than showing a fake live meter.
            self.smoke_diagnostics = Some(AudioTimingSnapshot {
                available: true,
                coherent: true,
                sample_rate: 48_000,
                frames: 256,
                channels: 2,
                revision: self.revision,
                callback_sequence: 321,
                duration_ns: 900_000,
                callbacks: 320,
                total_duration_ns: 393_000_000,
                total_frames: 81_920,
                peak_utilization: Some(0.73),
                utilization: Some(0.16875),
                transition_callbacks: 4,
                adopted_graph: true,
                crossfade: true,
                ..AudioTimingSnapshot::default()
            });
            self.audio_performance_open = true;
            self.status = "Audio performance · simulated screenshot fixture".into();
        }
        if frame == 606 {
            self.audio_performance_open = false;
            self.smoke_diagnostics = None;
            self.keyboard_open = false;
            self.browser_open = false;
            self.select_analysis(None);
            let patch: Patch = serde_json::from_value(json!({
                "version":2,"sample_rate":48000,
                "nodes":[
                    {"id":"tone","kind":"analog_vco","params":{"frequency":523.251,"pulse_width":0.35}},
                    {"id":"vca","kind":"analog_vca"},
                    {"id":"out","kind":"audio_stereo"}
                ],
                "cables":[
                    {"from":"tone","output":"out","to":"vca","input":"signal"},
                    {"from":"vca","output":"out","to":"out","input":"left"}
                ],
                "output":"out"
            })).unwrap();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("parameter context fixture compiles");
            self.replace_rack(Rack::default());
            self.rack
                .restore_layout(&json!({"version":1,"zoom":1.0,"pan":[24,12],"positions":{
                    "tone":[0,0],"vca":[210,0],"out":[420,0]
                }}))
                .unwrap();
            self.rack.select_only("tone".into());
            self.rack
                .request_parameter_menu_for_smoke("tone", "frequency");
            self.title = "Exact frequency · direct value entry".into();
            self.status =
                "Right-click a control to enter a value · Apply creates one undo step".into();
        }
        if frame == 160 {
            self.scope.smoke_xy();
        }
        if frame == 628 {
            egui::Popup::close_all(ctx);
            let patch: Patch = serde_json::from_value(json!({
                "version":2,"nodes":[
                    {"id":"pitch","kind":"constant","params":{"value":1.0}},
                    {"id":"offset","kind":"constant","params":{"value":0.5}},
                    {"id":"sum","kind":"mixer"}, {"id":"out","kind":"output"}
                ], "cables":[
                    {"id":"lead-a","from":"pitch","to":"sum","input":"a","color":[211,101,83],"input_order":2,"output_order":1},
                    {"id":"lead-b","from":"pitch","to":"sum","input":"a","color":[75,139,189],"input_order":1,"output_order":2},
                    {"id":"lead-c","from":"offset","to":"sum","input":"b","color":[72,171,167]},
                    {"id":"lead-out","from":"sum","to":"out","input":"signal"}
                ], "output":"out"
            }))
            .unwrap();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
            self.replace_rack(Rack::default());
            self.rack
                .restore_layout(&json!({"version":1,"zoom":1.0,"pan":[24,12],"positions":{
                    "pitch":[0,0],"offset":[120,0],"sum":[240,0],"out":[390,0]
                }}))
                .unwrap();
            self.rack
                .request_cable_menu_for_smoke("pitch", "out", false);
            self.title = "Cable bundles · output menu".into();
            self.status =
                "Independent plug stacks · Blue on top at source, coral on top at input".into();
        }
        if frame == 654 {
            egui::Popup::close_all(ctx);
            self.rack.request_cable_menu_for_smoke("sum", "a", true);
            self.title = "Cable bundles · input menu".into();
        }
        if frame == 682 {
            egui::Popup::close_all(ctx);
            let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
                {"id":"score","kind":"score_sequencer","params":{"tempo":128},"score":{"length":16,"tracks":[{"steps":[{"step":0,"notes":[0]},{"step":4,"notes":[7]},{"step":8,"notes":[0]},{"step":12,"notes":[7]}]}]}},
                {"id":"drum","kind":"drum_voice"},{"id":"filter","kind":"svf_filter"},
                {"id":"echo","kind":"stereo_delay"},{"id":"space","kind":"stereo_reverb"},
                {"id":"duck","kind":"sidechain_compressor"}
            ],"cables":[{"from":"score","output":"gate1","to":"drum","input":"trigger"},{"from":"drum","to":"filter","input":"signal"}]})).unwrap();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
            self.replace_rack(Rack::default());
            self.rack.restore_layout(&json!({"version":1,"zoom":0.9,"pan":[24,12],"positions":{
                "score":[0,0],"drum":[615,0],"filter":[990,0],"echo":[0,420],"space":[435,420],"duck":[810,420]
            }})).unwrap();
            self.title = "Score and studio modules".into();
            self.status = "Native patterns, percussion, filters and stereo effects".into();
        }
        if matches!(frame, 710 | 738 | 766) {
            egui::Popup::close_all(ctx);
            self.select_analysis(None);
            self.settings_open = false;
            self.keyboard_open = false;
            let xy = frame == 738;
            let external = frame == 766;
            let (patch, snapshot) = Self::rack_scope_smoke_fixture(xy, external);
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("Rack Scope offline fixture publishes");
            self.replace_rack(Rack::default());
            self.rack
                .restore_layout(&json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
                    "scope":[0,0],"wave":[630,0],"pitch":[930,0]
                }}))
                .expect("Rack Scope fixture layout");
            self.rack.set_scope_states(BTreeMap::from([(
                "scope".into(),
                std::sync::Arc::new(snapshot),
            )]));
            self.title = format!(
                "Rack Scope · {} · offline DSP snapshot",
                if xy {
                    "XY"
                } else if external {
                    "external trigger"
                } else {
                    "time"
                }
            );
            self.status = "Offline DSP capture · 16 voices per input · sine X / triangle Y · no live audio stream".into();
        }
        if frame == 794 {
            egui::Popup::close_all(ctx);
            self.select_analysis(None);
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/struck-resonator.json"))
                    .expect("resonator smoke patch");
            let mut engine = patch.compile().expect("resonator smoke engine");
            engine.process_block(&mut vec![0.0; 2500]);
            let handle = engine.resolve_module_scope("scope").unwrap();
            let snapshot = engine.module_scope(handle).unwrap().clone();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
            self.replace_rack(Rack::default());
            self.rack
                .restore_layout(&json!({"version":1,"zoom":0.75,"pan":[24,12],"positions":{
                    "body":[0,0],"scope":[480,0],"clock":[0,420],"out":[360,420]
                }}))
                .unwrap();
            self.rack.set_scope_states(BTreeMap::from([(
                "scope".into(),
                std::sync::Arc::new(snapshot),
            )]));
            self.title = "Struck resonator · physical modal model".into();
            self.status = "Offline DSP snapshot · 16-mode string / beam · configuration preview is not live motion".into();
        }
        if frame == 822 {
            egui::Popup::close_all(ctx);
            self.select_analysis(None);
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/noise-colors.json"))
                    .expect("noise smoke patch");
            let mut engine = patch.compile().expect("noise smoke engine");
            engine.process_block(&mut vec![0.0; 48000]);
            let handle = engine.resolve_module_scope("scope").unwrap();
            let snapshot = engine.module_scope(handle).unwrap().clone();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
            self.replace_rack(Rack::default());
            self.rack
                .restore_layout(&json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{
                    "noise":[0,0],"scope":[270,0],"out":[0,420]
                }}))
                .unwrap();
            self.rack.set_scope_states(BTreeMap::from([(
                "scope".into(),
                std::sync::Arc::new(snapshot),
            )]));
            self.title = "Noise colors · seven simultaneous outputs".into();
            self.status =
                "Offline DSP capture · white X / pink Y · panel curves show filter configuration"
                    .into();
        }
        if frame == 850 {
            egui::Popup::close_all(ctx);
            self.select_analysis(None);
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/random-values-voice.json"))
                    .expect("random values smoke patch");
            let mut engine = patch.compile().expect("random values smoke engine");
            engine.process_block(&mut vec![0.0; 96000]);
            let scope = engine
                .module_scope(engine.resolve_module_scope("scope").unwrap())
                .unwrap()
                .clone();
            let state = engine
                .module_saved_state(engine.resolve_module_state("values").unwrap())
                .unwrap();
            let openpatch_core::SavedModuleState::RandomValues { values, rng } = state else {
                panic!("random values snapshot");
            };
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
            self.replace_rack(Rack::default());
            self.rack
                .restore_layout(&Self::random_values_layout())
                .unwrap();
            self.rack.set_scope_states(BTreeMap::from([(
                "scope".into(),
                std::sync::Arc::new(scope),
            )]));
            self.rack.set_random_values_states(BTreeMap::from([(
                "values".into(),
                json!({"values":values.map(|row| row.map(f32::from_bits)),
                    "rng_state":rng,"active_channels":1}),
            )]));
            self.title = "Random values voice · held voltages".into();
            self.status = "Offline DSP capture · seven held outputs · seeded melody through a pentatonic quantizer".into();
        }
        if frame == 878 {
            egui::Popup::close_all(ctx);
            self.select_analysis(None);
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/shift-register-canon.json"))
                    .expect("shift register smoke patch");
            let mut engine = patch.compile().expect("shift register smoke engine");
            engine.process_block(&mut vec![0.0; 144000]);
            let scope = engine
                .module_scope(engine.resolve_module_scope("scope").unwrap())
                .unwrap()
                .clone();
            let state = engine
                .module_saved_state(engine.resolve_module_state("register").unwrap())
                .unwrap();
            let openpatch_core::SavedModuleState::ShiftRegister { values, rng } = state else {
                panic!("shift register snapshot");
            };
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
            self.replace_rack(Rack::default());
            self.rack
                .restore_layout(&Self::shift_register_layout())
                .unwrap();
            self.rack.set_scope_states(BTreeMap::from([(
                "scope".into(),
                std::sync::Arc::new(scope),
            )]));
            self.rack.set_shift_register_states(BTreeMap::from([(
                "register".into(),
                json!({"values":values.map(f32::from_bits),"rng_state":rng}),
            )]));
            self.title = "Shift register canon · eight linked holds".into();
            self.status =
                "Offline DSP capture · rows 1 / 3 / 5 play a three-voice pentatonic canon".into();
        }
        if frame == 906 {
            egui::Popup::close_all(ctx);
            self.select_analysis(None);
            self.prepare_motion_fixture(Instant::now());
            self.title = "A voice taking shape".into();
            self.status =
                "Motion study · one real construction commit · controlled presentation clock"
                    .into();
        }
        if matches!(frame, 912 | 934 | 956 | 978) {
            let millis = match frame {
                912 => 80,
                934 => 180,
                956 => 340,
                _ => 450,
            };
            self.rack.set_construction_clock_override(Some(
                self.smoke_motion_origin.unwrap() + Duration::from_millis(millis),
            ));
        }
        if frame == 1000 {
            self.mcp_session.open();
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            self.mcp_session
                .set_server(openpatch::local_mcp::LocalServer::start(
                    self.shared.clone(),
                    Path::new(&self.workspace),
                ));
            self.status =
                "Real local MCP bridge · ready for a client · playback remains paused".into();
        }
        if frame == 622 {
            assert!(
                egui::Popup::is_any_open(ctx),
                "Parameter screenshot must contain the open editor"
            );
        }
        if frame == 1030 {
            egui::Popup::close_all(ctx);
            self.prepare_room_lighting_fixture();
        }
        if frame == 1060 {
            self.rack.set_room_brightness(0.1);
            self.status = "Darkroom · 10% room · real module lamps and Scope emission · same offline snapshot".into();
        }
        if frame == 1090 {
            self.rack.set_room_brightness(0.0);
            self.status =
                "Lights out · 0% room · actual lamps and Scope remain · browser stays readable"
                    .into();
        }
        if frame == 1124 {
            self.rack.set_room_brightness(1.0);
            self.browser_open = true;
            assert!(self.library.inspect("seq3", &self.catalog));
            self.title = "Inspect a module · actual panel at natural size".into();
            self.status =
                "Pinned Seq-3 inspection · display-only catalog defaults · scroll to explore"
                    .into();
        }
        if frame == 1154 {
            self.browser_open = false;
            self.library = ModuleLibrary::default();
            let patch: Patch = serde_json::from_value(json!({"version":2,
                "nodes":[{"id":"voice","kind":"multi_vco","params":{"frequency":220,"pulse_width":0.35}},
                    {"id":"filter","kind":"ladder_vcf"},{"id":"amp","kind":"analog_vca"},
                    {"id":"out","kind":"audio_out"}],
                "cables":[{"from":"voice","output":"sine","to":"filter","input":"signal"},
                    {"from":"filter","to":"amp","input":"signal"},{"from":"amp","to":"out","input":"signal"}],
                "output":"out"})).unwrap();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("Precision fixture compiles");
            self.replace_rack(Rack::default());
            self.rack
                .restore_layout(&json!({"version":1,"zoom":1.0,"pan":[24,12],"positions":{
                "voice":[0,0],"filter":[300,0],"amp":[510,0],"out":[720,0]}}))
                .unwrap();
            self.rack.select_only("voice".into());
            self.rack
                .request_precision_control_for_smoke("voice", "pulse_width");
            self.title = "Precision entry · one module, continuous flow".into();
            self.status = "Click a value to type · Tab applies and continues · each commit has its own undo step".into();
        }
        if frame == 1184 {
            self.title = "Patching · guidance follows your hand".into();
            self.status = "Synthetic pointer gesture · real endpoint hit testing · draft cable only, no graph edit".into();
        }
        if frame == 1214 {
            self.rack.set_room_brightness(0.0);
            self.status = "Lights out · patching guidance and active route stay readable".into();
        }
        if frame == 1250 {
            self.rack.set_room_brightness(1.0);
            self.cable_palette = crate::cable_palette::Palette::default();
            self.cable_palette_ui.open();
            self.title = "Cable palettes · compose a personal set".into();
            self.status =
                "Draft palette editor · Apply commits host preferences; the patch stays unchanged"
                    .into();
        }
        if frame == 1272 {
            assert!(
                self.cable_palette_ui.is_open(),
                "Palette capture must contain the actual editor"
            );
            assert_eq!(self.patch.cables.len(), 3);
        }
        if frame == 1290 {
            self.cable_palette_ui = crate::cable_palette_ui::Panel::default();
            self.cable_palette = crate::cable_palette::Palette::default();
            self.cable_palette.select_preset(1);
            self.title = "A reviewed colour · one real cable".into();
            self.status = "Synthetic pointer creates one Bright cable; the next choice advances after acceptance".into();
        }
        if frame == 1310 {
            assert_eq!(self.patch.cables.len(), 4);
            assert_eq!(
                self.patch.cables.last().unwrap().color,
                Some([244, 104, 111])
            );
            assert_eq!(self.cable_palette.next_index(), 1);
            self.status = "One new cable, explicit RGB · next colour advanced only after its successful commit".into();
        }
        if frame == 1350 {
            self.prepare_adsr_eg_fixture();
        }
        if frame == 1400 {
            self.rack.set_room_brightness(0.0);
            self.status =
                "ADSR EG · lights out · four real stage lamps and a four-voice ENV capture".into();
        }
        if frame == 1450 {
            self.prepare_midi_cc_fixture();
        }
        if frame == 1500 {
            self.rack.set_room_brightness(0.0);
            self.status="MIDI CC · lights out · actual sixteen-voice Scope capture · no live audio or MIDI device".into();
        }
        if frame == 1550 {
            self.prepare_midi_gate_fixture();
        }
        if frame == 1650 {
            self.prepare_annotations_fixture();
        }
        if frame == 1700 {
            self.rack.set_room_brightness(0.0);
            self.status =
                "Notes screen emits light; passive Blank panels disappear with room lighting"
                    .into();
        }
        if frame == 1750 {
            self.rack.set_room_brightness(1.0);
            let node = self
                .patch
                .nodes
                .iter()
                .find(|n| n.id == "guide")
                .unwrap()
                .clone();
            self.rack.open_annotation_editor(&node, self.revision);
            self.status = "Actual multiline Notes editor · draft is separate until Apply".into();
        }
        if frame == 1800 {
            assert!(self.command("set_module_content",json!({"id":"guide","content":{"type":"notes","text":"A newer MCP edit is now authoritative. The open editor keeps its previous draft."}})));
            self.status="Actual concurrent Session edit · stale draft retained; Apply blocked until explicit reload".into();
        }
        if frame == 1600 {
            self.rack.set_room_brightness(0.0);
            self.status="MIDI Gate · lights out · eight actual pad lamps and sixteen captured gate lanes · no live audio or MIDI device".into();
        }
        if frame == 1828 {
            self.prepare_stock_vcf_fixture();
        }
        if frame == 1838 {
            self.rack.set_room_brightness(0.0);
            self.status =
                "VCF · lights out · the filter is passive; Scope emits the captured LP/HP signals"
                    .into();
        }
        if frame == 1850 {
            self.prepare_stock_vca_fixture();
        }
        if frame == 1862 {
            self.rack.set_room_brightness(0.0);
            self.status = "VCA · lights out · actual gain columns emit, including the silent full-gain lane · no live audio".into();
        }
        if frame == 1876 {
            self.prepare_parameter_choices_fixture();
        }
        if frame == 1890 {
            self.rack.set_room_brightness(0.0);
            self.status =
                "Lights out · categorical choices are passive panel settings, not signal lamps"
                    .into();
        }
        if frame == 1902 {
            self.prepare_port_lights_fixture();
        }
        if frame == 1920 {
            self.rack.set_room_brightness(0.0);
            self.status = "Lights out · actual signal plugs emit; cancelled and silent signals remain dark · VCA gain lamps are independent".into();
        }
        if frame == 1932 {
            self.rack.set_room_brightness(1.0);
            self.set_cable_presentation(CablePresentation::default());
            let menu = ctx
                .data(|data| data.get_temp::<egui::Id>(egui::Id::new("smoke-view-menu-popup")))
                .expect("Rendered View menu button");
            egui::Popup::open_id(ctx, menu);
            self.status = "View · cable appearance is saved with host preferences, independently of the patch".into();
        }
        if frame == 1944 {
            egui::Popup::close_all(ctx);
            self.prepare_port_lights_fixture();
            self.rack.set_room_brightness(1.0);
            self.set_cable_presentation(CablePresentation {
                opacity: 0.15,
                tension: 1.0,
                plug_brightness: 1.0,
            });
            self.title = "Clear panels · taut cables".into();
            self.status = "Cable bodies 15% · tension 100% · actual offline signal plugs stay bright · no live audio".into();
        }
        if frame == 1958 {
            self.prepare_port_lights_fixture();
            self.rack.set_zoom(0.72);
            self.rack.set_room_brightness(1.0);
            self.set_cable_presentation(CablePresentation {
                opacity: 1.0,
                tension: 0.0,
                plug_brightness: 1.0,
            });
            self.title = "Loose cables · fixed plugs".into();
            self.status = "Cable bodies 100% · tension 0% · short and reversed routes keep their real signal endpoints · no live audio".into();
        }
        if matches!(frame, 1972 | 1988 | 2004 | 2020) {
            egui::Popup::close_all(ctx);
            self.library = ModuleLibrary::default();
            self.library.set_preview_zoom(match frame {
                2004 => 0.25,
                2020 => 2.0,
                _ => 1.0,
            });
            self.browse();
            self.select_analysis(None);
            self.rack.set_room_brightness(1.0);
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(if frame == 2004 {
                Vec2::new(940.0, 680.0)
            } else {
                Vec2::new(1440.0, 960.0)
            }));
            self.title = match frame {
                1972 => "Library · natural-size controls",
                1988 => "Library · wide panels at 100%",
                2004 => "Library · compact 25% previews",
                _ => "Library · wide panels at 200%",
            }
            .into();
            self.status = "Actual catalog panels · preview size is independent of rack zoom and saved patch values".into();
        }
        if frame == 2036 {
            egui::Popup::close_all(ctx);
            let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
                {"id":"vco","kind":"multi_vco","params":{"linear_fm":1,"soft_sync":1}},
                {"id":"lfo","kind":"multi_lfo","params":{"unipolar":1,"invert":1}},
                {"id":"mix","kind":"mix6","params":{"average":1,"invert":1}},
                {"id":"out","kind":"audio_stereo"}
            ],"cables":[
                {"from":"lfo","output":"sine","to":"vco","input":"fm"},
                {"from":"vco","output":"sine","to":"mix","input":"in1"},
                {"from":"mix","to":"out","input":"left"}
            ],"output":"out"}))
            .expect("Small-label rack fixture");
            assert!(self.command("replace_patch", json!({"patch":patch})));
            self.replace_rack(Rack::default());
            self.rack
                .restore_layout(&json!({"version":1,"zoom":0.5,"pan":[24,12],
                    "positions":{"vco":[0,0],"lfo":[330,0],"mix":[660,0],"out":[930,0]}
                }))
                .expect("Small-label rack layout");
            self.rack.set_room_brightness(1.0);
            self.browser_open = false;
            self.select_analysis(None);
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(1440.0, 960.0)));
            self.title = "Compact rack · readable mode labels".into();
            self.status = "Actual shared panels at 50% rack zoom · linear FM, soft sync and mixer modes stay inside their controls".into();
        }
        if frame == 2052 {
            egui::Popup::close_all(ctx);
            self.select_analysis(None);
            self.browser_open = false;
            self.prepare_cable_move_fixture(Instant::now(), true);
            self.rack.set_room_brightness(1.0);
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(1440.0, 960.0)));
            self.title = "Invisible hand · repatch the voice".into();
            self.status =
                "Two real cable moves · input and output ends · controlled presentation clock"
                    .into();
        }
        if matches!(frame, 2060 | 2076 | 2092 | 2108) {
            let millis = match frame {
                2060 => 40,
                2076 => 200,
                2092 => 350,
                _ => 460,
            };
            self.rack.set_construction_clock_override(Some(
                self.smoke_motion_origin.unwrap() + Duration::from_millis(millis),
            ));
        }
        if frame == 2116 {
            self.prepare_room_lighting_fixture();
            self.browser_open = false;
            self.rack.set_room_brightness(1.0);
            let mut appearance = self.rack.cable_presentation();
            appearance.tension = 1.0;
            self.rack.set_cable_presentation(appearance);
            self.title = "Taut cables · a little gravity".into();
            self.status = "Maximum tension · restrained distance-dependent bow · actual shared cable geometry".into();
        }
        if frame == 2132 {
            egui::Popup::close_all(ctx);
            self.smoke_motion_origin = None;
            assert!(self.command(
                "replace_patch",
                json!({"patch":{"version":2,"nodes":[
                    {"id":"near-voice","kind":"multi_vco","params":{"frequency":110}},
                    {"id":"far-voice","kind":"multi_vco","params":{"frequency":220}}
                ]}})
            ));
            self.replace_rack(Rack::default());
            self.rack
                .restore_layout(&json!({"version":1,"zoom":1.0,"pan":[72,48],
                "positions":{"near-voice":[0,0],"far-voice":[1500,0]}}))
                .unwrap();
            self.construction
                .set_motion_mode(crate::construction_motion::MotionMode::Full);
            self.rack
                .set_construction_motion_mode(crate::construction_motion::MotionMode::Full);
            self.rack.set_agent_follow(true);
            self.browser_open = false;
            self.select_analysis(None);
            self.title = "Follow agent edits".into();
            self.status = "Follow enabled · the next edit is beyond the right edge".into();
        }
        if frame == 2140 {
            let pan = self.rack.agent_follow_view_pan();
            ctx.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("smoke-follow-receipt"),
                    json!({
                        "baseline_layout":self.rack.layout(),"before_revision":self.revision,
                        "before_pan":[pan.x,pan.y],"zoom":self.rack.zoom(),"phases_ms":[0,110,260]
                    }),
                )
            });
        }
        if frame == 2144 {
            let origin = Instant::now();
            self.smoke_motion_origin = Some(origin);
            self.rack.set_construction_clock_override(Some(origin));
            let before = self.revision;
            execute_shared(&self.shared, "start_construction", json!({
                "title":"Tune the far voice","expected_revision":before,"interval_ms":100,
                "steps":[{"label":"Tune the offscreen oscillator","edits":[
                    {"tool":"set_parameter","arguments":{"id":"far-voice","parameter":"frequency","value":440}}
                ]}]
            })).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            let receipt = loop {
                let state = self.shared.lock().unwrap().construction_snapshot();
                if state["status"] == "completed" {
                    break state["last_commit"].clone();
                }
                assert_eq!(state["status"], "running", "{state}");
                assert!(Instant::now() < deadline, "Follow fixture did not complete");
                std::thread::sleep(Duration::from_millis(2));
            };
            assert_eq!(receipt["before_revision"], before);
            while self.revision < receipt["revision"].as_u64().unwrap() {
                self.sync();
                assert!(
                    Instant::now() < deadline,
                    "Follow fixture could not observe commit"
                );
                std::thread::yield_now();
            }
            ctx.data_mut(|data| {
                let mut log = data
                    .get_temp::<Value>(egui::Id::new("smoke-follow-receipt"))
                    .unwrap();
                log["commit"] = receipt;
                data.insert_temp(egui::Id::new("smoke-follow-receipt"), log);
            });
            self.status = "Follow enabled · revealing the newly tuned oscillator".into();
        }
        if matches!(frame, 2148 | 2164) {
            self.rack.set_construction_clock_override(Some(
                self.smoke_motion_origin.unwrap()
                    + Duration::from_millis(if frame == 2148 { 110 } else { 260 }),
            ));
        }
        if matches!(frame, 2156 | 2172) {
            let pan = self.rack.agent_follow_view_pan();
            let log = ctx.data_mut(|data| {
                let mut log = data
                    .get_temp::<Value>(egui::Id::new("smoke-follow-receipt"))
                    .unwrap();
                log[if frame == 2156 {
                    "mid_pan"
                } else {
                    "settled_pan"
                }] = json!([pan.x, pan.y]);
                log["final_layout"] = self.rack.layout();
                log["final_revision"] = json!(self.revision);
                data.insert_temp(egui::Id::new("smoke-follow-receipt"), log.clone());
                log
            });
            if frame == 2172 {
                assert_eq!(log["baseline_layout"], log["final_layout"]);
                assert_ne!(log["before_pan"], log["settled_pan"]);
                std::fs::write(
                    path.with_file_name("agent-follow-receipt.json"),
                    serde_json::to_vec_pretty(&log).unwrap(),
                )
                .expect("Save follow receipt");
            }
        }
        if frame == 2184 {
            egui::Popup::close_all(ctx);
            self.prepare_stock_delay_fixture();
            self.rack.set_room_brightness(1.0);
        }
        if frame == 2200 {
            self.rack.set_room_brightness(0.0);
            self.status = "Delay · lights out · actual CLK pulse and MIX/WET Scope traces emit; controls remain passive".into();
        }
        if frame == 2216 {
            self.rack.set_room_brightness(1.0);
            self.library = ModuleLibrary::default();
            self.library.set_preview_zoom(1.0);
            self.browse();
            assert!(self.library.inspect("stock_delay", &self.catalog));
            self.title = "Delay · shared library faceplate".into();
            self.status =
                "The module library uses the rack's exact delay panel and jack geometry".into();
        }
        if frame == 2232 {
            egui::Popup::close_all(ctx);
            self.prepare_wavetable_fixture();
            self.rack.set_room_brightness(1.0);
        }
        if frame == 2248 {
            self.rack.set_room_brightness(0.0);
            self.status = "Wavetable voice · lights out · actual phase and envelope lights with captured Scope traces".into();
        }
        if frame == 2264 {
            self.rack.set_room_brightness(1.0);
            self.library = ModuleLibrary::default();
            self.library.set_preview_zoom(0.5);
            self.browse();
            self.title = "Wavetable · shared library faceplates".into();
            self.status = "Both wavetable modules use the same faceplates and table previews in the library and rack".into();
        }
        if frame == 2284 {
            egui::Popup::close_all(ctx);
            self.prepare_random_fixture();
            self.rack.set_room_brightness(1.0);
        }
        if frame == 2300 {
            self.rack.set_room_brightness(0.0);
            self.status = "Random motion · lights out · real probability/rate events and captured interpolation curves".into();
        }
        if frame == 2316 {
            self.rack.set_room_brightness(1.0);
            self.library = ModuleLibrary::default();
            self.library.set_preview_zoom(0.5);
            self.browse();
            assert!(self.library.inspect("random", &self.catalog));
            self.title = "Random · shared library faceplate".into();
        }
        if frame == 2340 {
            egui::Popup::close_all(ctx);
            let capture = self.prepare_weak_scope_fixture(true);
            ctx.data_mut(|data| data.insert_temp(egui::Id::new("smoke-weak-scope"), capture));
            self.rack.set_room_brightness(1.0);
        }
        if frame == 2356 {
            self.rack.set_room_brightness(0.0);
            self.status = "Actual offline 100 µV capture · lights out · calibrated AUTO trace emits, controls and mounts remain reflective · no live audio".into();
        }
        if frame == 2372 {
            let original = ctx
                .data(|data| {
                    data.get_temp::<Arc<openpatch_core::ScopeSnapshot>>(egui::Id::new(
                        "smoke-weak-scope",
                    ))
                })
                .expect("Original weak Scope capture");
            let manual = self.prepare_weak_scope_fixture(false);
            assert_eq!(
                manual.x, original.x,
                "Manual mode must show the same real raw capture"
            );
            assert_eq!(manual.y, original.y);
            assert_eq!(manual.measurements(), original.measurements());
            self.rack
                .set_scope_states(BTreeMap::from([("scope".into(), original)]));
            self.rack.set_room_brightness(1.0);
        }
        if frame == 2396 {
            // Exercise the same native View popup used by mouse/keyboard input.
            // The final, bounded scene alone advances egui time deterministically
            // so software-rendered captures show distinct real opening phases.
            egui::Popup::close_all(ctx);
            self.browser_open = false;
            self.settings_open = false;
            self.audio_performance_open = false;
            self.select_analysis(None);
            self.rack.set_room_brightness(1.0);
            let menu = ctx
                .data(|data| data.get_temp::<egui::Id>(egui::Id::new("smoke-view-menu-popup")))
                .expect("Rendered View menu button");
            egui::Popup::open_id(ctx, menu);
            self.title = "Native menu motion".into();
            self.status = "View menu · actual quadratic-out opening fade · Full motion · unchanged input regions".into();
        }
        if matches!(frame, 2400 | 2406 | 2418) {
            let menu = ctx
                .data(|data| data.get_temp::<egui::Id>(egui::Id::new("smoke-view-menu-popup")))
                .expect("Rendered View menu button");
            assert!(
                egui::Popup::is_id_open(ctx, menu),
                "Actual View menu must remain open"
            );
            let area = egui::AreaState::load(ctx, menu).expect("Actual native View menu Area");
            assert!(area.rect().is_positive() && area.interactable);
            assert!(area.last_became_visible_at.is_some());
            assert_eq!(ctx.style().animation_time, 0.160);
        }
        if frame == 2430 {
            egui::Popup::close_all(ctx);
            self.prepare_merge_fixture();
            self.rack.set_room_brightness(1.0);
        }
        if frame == 2448 {
            self.rack.set_room_brightness(0.0);
            self.status = "Merge · lights out · measured 12 / 0 / 8 counts emit; explicit zero is an observed value · no live audio".into();
        }
        if frame == 2464 {
            self.rack.set_room_brightness(1.0);
            self.library = ModuleLibrary::default();
            self.library.set_preview_zoom(1.0);
            self.browse();
            assert!(self.library.inspect("poly_merge", &self.catalog));
            self.title = "Merge · shared library faceplate".into();
            self.status = "The browser uses the same Merge panel; its unavailable count is distinct from the actual rack snapshots".into();
        }
        if frame == 2480 {
            self.library = ModuleLibrary::default();
            egui::Popup::close_all(ctx);
            self.prepare_split_fixture();
            self.rack.set_room_brightness(1.0);
        }
        if frame == 2498 {
            self.rack.set_room_brightness(0.0);
            self.status = "Split · lights out · measured input counts 0 / 1 / 16 emit; no fabricated module lamps · no live audio".into();
        }
        if frame == 2514 {
            self.rack.set_room_brightness(1.0);
            self.library = ModuleLibrary::default();
            self.library.set_preview_zoom(1.0);
            self.browse();
            assert!(self.library.inspect("poly_split", &self.catalog));
            self.title = "Split · shared library faceplate".into();
            self.status = "The browser uses the same Split panel; its unavailable count is distinct from actual rack observations".into();
        }
        if frame == 2530 {
            self.library = ModuleLibrary::default();
            egui::Popup::close_all(ctx);
            self.prepare_stock_lfo_fixture();
            self.rack.set_room_brightness(1.0);
        }
        if frame == 2548 {
            self.rack.set_room_brightness(0.0);
            self.status = "Stock LFO · actual phase RGB, Offset/Invert lamps and observed clock rates remain in darkroom · no live audio".into();
        }
        if frame == 2564 {
            self.rack.set_room_brightness(1.0);
            self.library = ModuleLibrary::default();
            self.library.set_preview_zoom(1.0);
            self.browse();
            assert!(self.library.inspect("stock_lfo", &self.catalog));
            self.title = "Stock LFO · shared library faceplate".into();
            self.status = "Shared Stock LFO panel · configured controls remain visible; preview does not invent clock or lamp telemetry".into();
        }
        if frame == 2580 {
            self.library = ModuleLibrary::default();
            egui::Popup::close_all(ctx);
            self.prepare_stock_vco_fixture();
            self.rack.set_room_brightness(1.0);
        }
        if frame == 2598 {
            self.rack.set_room_brightness(0.0);
            self.status = "Stock VCO · actual phase RGB, Linear FM and Soft sync lamps remain emissive · no live audio".into();
        }
        if frame == 2614 {
            self.rack.set_room_brightness(1.0);
            self.library = ModuleLibrary::default();
            self.library.set_preview_zoom(1.0);
            self.browse();
            assert!(self.library.inspect("stock_vco", &self.catalog));
            self.title = "Stock VCO · shared library faceplate".into();
            self.status = "Same Stock VCO panel in rack and browser · configured values, no invented phase or mode telemetry".into();
        }
        if frame == 2630 {
            self.library = ModuleLibrary::default();
            egui::Popup::close_all(ctx);
            self.prepare_stock_seq3_fixture();
            self.rack.set_room_brightness(1.0);
        }
        if frame == 2648 {
            self.rack.set_room_brightness(0.0);
            self.status = "Stock SEQ3 · actual clock / Run / gates / selected and out-of-range step lamps · no live audio".into();
        }
        if frame == 2664 {
            self.rack.set_room_brightness(1.0);
            self.library = ModuleLibrary::default();
            self.library.set_preview_zoom(1.0);
            self.browse();
            assert!(self.library.inspect("stock_seq3", &self.catalog));
            self.title = "Stock SEQ3 · shared library faceplate".into();
            self.status = "Same Stock SEQ3 panel in rack and browser · configured settings, inactive preview lamps".into();
        }
        if frame == 2680 {
            self.library = ModuleLibrary::default();
            egui::Popup::close_all(ctx);
            self.prepare_stock_quantizer_fixture();
            self.rack.set_room_brightness(1.0);
        }
        if frame == 2698 {
            self.rack.set_room_brightness(0.0);
            self.status = "Stock Quantizer · actual played-note display remains emissive · no invented module lamps · no live audio".into();
        }
        if frame == 2716 {
            self.rack.set_room_brightness(1.0);
            self.library = ModuleLibrary::default();
            self.library.set_preview_zoom(0.7);
            self.browse();
            self.library.request_focus();
            self.title = "Stock Quantizer · shared browser thumbnail".into();
            self.status = "Same Stock Quantizer faceplate in rack and filtered browser · actual played notes only on the observed rack".into();
        }
        if frame == 2748 {
            self.library = ModuleLibrary::default();
            egui::Popup::close_all(ctx);
            self.prepare_stock_mid_side_fixture();
            self.rack.set_room_brightness(1.0);
        }
        if frame == 2782 {
            self.rack.set_room_brightness(0.0);
            self.status = "Stock Mid/Side · actual Scope, oscillator lamps and port voltages remain emissive · Mid/Side has no module lamps · no live audio".into();
        }
        if frame == 2814 {
            self.rack.set_room_brightness(1.0);
            self.library = ModuleLibrary::default();
            self.library.set_preview_zoom(0.7);
            self.browse();
            self.library.request_focus();
            self.title = "Stock Mid/Side · shared browser thumbnail".into();
            self.status = "Same Stock Mid/Side faceplate in rack and filtered browser · encoder left, decoder right · inactive preview telemetry".into();
        }
        if frame == 2848 {
            self.library = ModuleLibrary::default();
            egui::Popup::close_all(ctx);
            self.prepare_stock_mix_fixture();
            self.rack.set_room_brightness(1.0);
        }
        if frame == 2882 {
            self.rack.set_room_brightness(0.0);
            self.status = "Stock Mix · actual Scope, oscillator and port emission · no Mix module lamps · no live audio".into();
        }
        if frame == 2914 {
            self.rack.set_room_brightness(1.0);
            self.library = ModuleLibrary::default();
            self.library.set_preview_zoom(0.7);
            self.browse();
            self.library.request_focus();
            self.title = "Stock Mix · shared browser thumbnail".into();
            self.status = "Same Stock Mix panel in rack and filtered browser · saved Average / Invert flags · inactive preview telemetry".into();
        }
        if frame == 2932 {
            self.library = ModuleLibrary::default();
            egui::Popup::close_all(ctx);
            self.prepare_stock_octave_fixture();
            self.rack.set_room_brightness(1.0);
        }
        if frame == 2942 {
            self.rack.set_room_brightness(0.0);
            self.status = "Stock Octave · configured selector remains visible · no module lamps · last actual channel 0 offset shown honestly".into();
        }
        if frame == 2948 {
            self.rack.set_room_brightness(1.0);
            self.library = ModuleLibrary::default();
            self.library.set_preview_zoom(0.7);
            self.browse();
            self.library.request_focus();
            self.title = "Stock Octave · shared browser panel".into();
            self.status = "Same Stock Octave renderer in rack and filtered browser · selector is authored state, offset is observed channel 0 telemetry".into();
        }
        if frame == 2951 {
            self.library = ModuleLibrary::default();
            egui::Popup::close_all(ctx);
            self.prepare_stock_vca_mix_fixture();
            self.rack.set_room_brightness(1.0);
        }
        if frame == 2954 {
            self.rack.set_room_brightness(0.0);
            self.status = "Stock VCA Mix · lights out · four actual offline strip meter snapshots remain emissive · no live audio".into();
        }
        if frame == 2956 {
            self.rack.set_room_brightness(1.0);
            self.library = ModuleLibrary::default();
            self.library.set_preview_zoom(0.7);
            self.browse();
            assert!(self.library.inspect("stock_vca_mix", &self.catalog));
            self.library.request_focus();
            self.title = "Stock VCA Mix · shared browser panel".into();
            self.status = "Same Stock VCA Mix faceplate in rack and browser · preview has no fabricated meter telemetry".into();
        }
        if frame == 1176 {
            assert!(
                self.rack.precision_is_open_for_smoke(),
                "Precision capture must contain the actual navigable editor"
            );
        }
        if matches!(frame, 1206 | 1236) {
            assert!(
                self.rack.cable_is_dragging_for_smoke(),
                "Cable guidance capture must contain a real pointer gesture"
            );
        }
        if matches!(
            frame,
            12 | 24
                | 36
                | 48
                | 60
                | 72
                | 84
                | 96
                | 108
                | 120
                | 132
                | 144
                | 156
                | 168
                | 184
                | 200
                | 216
                | 232
                | 256
                | 272
                | 296
                | 320
                | 344
                | 368
                | 392
                | 416
                | 432
                | 448
                | 466
                | 482
                | 506
                | 530
                | 554
                | 578
                | 600
                | 622
                | 648
                | 676
                | 704
                | 732
                | 760
                | 788
                | 816
                | 844
                | 872
                | 900
                | 928
                | 950
                | 972
                | 994
                | 1022
                | 1052
                | 1082
                | 1112
                | 1146
                | 1176
                | 1206
                | 1236
                | 1272
                | 1328
                | 1372
                | 1422
                | 1472
                | 1522
                | 1572
                | 1622
                | 1672
                | 1722
                | 1772
                | 1822
                | 1832
                | 1844
                | 1854
                | 1868
                | 1882
                | 1896
                | 1910
                | 1928
                | 1940
                | 1952
                | 1964
                | 1980
                | 1996
                | 2012
                | 2028
                | 2044
                | 2060
                | 2076
                | 2092
                | 2108
                | 2124
                | 2140
                | 2156
                | 2172
                | 2192
                | 2208
                | 2224
                | 2240
                | 2256
                | 2276
                | 2292
                | 2308
                | 2328
                | 2352
                | 2368
                | 2384
                | 2400
                | 2406
                | 2418
                | 2440
                | 2456
                | 2472
                | 2490
                | 2506
                | 2522
                | 2540
                | 2556
                | 2572
                | 2590
                | 2606
                | 2622
                | 2640
                | 2656
                | 2672
                | 2690
                | 2708
                | 2730
                | 2766
                | 2798
                | 2830
                | 2866
                | 2898
                | 2930
                | 2938
                | 2944
                | 2950
                | 2953
                | 2955
                | 2958
        ) {
            let token = if let Some(receipt) = crate::smoke_schedule::request(frame, captures) {
                assert!(
                    pending.is_none(),
                    "Previous static screenshot is still pending"
                );
                ctx.data_mut(|d| d.insert_temp(receipt_id, receipt));
                egui::UserData::new(receipt)
            } else {
                egui::UserData::default()
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(token));
        }
        let pending = ctx.data(|d| d.get_temp::<crate::smoke_schedule::Receipt>(receipt_id));
        if let Some(image) = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Screenshot {
                    image, user_data, ..
                } = e
                {
                    if let Some(expected) = pending {
                        let token = user_data
                            .data
                            .as_ref()
                            .and_then(|d| d.downcast_ref::<crate::smoke_schedule::Receipt>())
                            .copied();
                        if !expected.accepts(token, captures) {
                            return None;
                        }
                    }
                    Some(image.clone())
                } else {
                    None
                }
            })
        }) {
            assert!(image.width() >= 900 && image.height() >= 600);
            assert!(
                image.pixels.windows(2).filter(|p| p[0] != p[1]).count() > 1000,
                "Blank window"
            );
            let target = match captures {
                0 => path.clone(),
                1 => path.with_file_name("rack-visuals.png"),
                2 => path.with_file_name("rack-utilities.png"),
                3 => path.with_file_name("rack-polyphonic.png"),
                4 => path.with_file_name("rack-routing.png"),
                5 => path.with_file_name("rack-midi.png"),
                6 => path.with_file_name("audio-midi-settings.png"),
                7 => path.with_file_name("rack-stereo-width.png"),
                8 => path.with_file_name("rack-routing-modules.png"),
                9 => path.with_file_name("rack-modulation.png"),
                10 => path.with_file_name("project-save.png"),
                11 => path.with_file_name("project-unsaved.png"),
                12 => path.with_file_name("signal-scope.png"),
                13 => path.with_file_name("signal-scope-xy.png"),
                14 => path.with_file_name("rack-sequencer.png"),
                15 => path.with_file_name("rack-switches.png"),
                16 => path.with_file_name("rack-voltage-utilities.png"),
                17 => path.with_file_name("rack-bypass.png"),
                18 => path.with_file_name("rack-gate-shaping.png"),
                19 => path.with_file_name("rack-manual-gates.png"),
                20 => path.with_file_name("rack-vca-mix-fade.png"),
                21 => path.with_file_name("rack-duplication.png"),
                22 => path.with_file_name("rack-stacked-routing.png"),
                23 => path.with_file_name("rack-multi-vco.png"),
                24 => path.with_file_name("module-preset-save.png"),
                25 => path.with_file_name("module-preset-browser.png"),
                26 => path.with_file_name("rack-duplication-outgoing.png"),
                27 => path.with_file_name("module-browser.png"),
                28 => path.with_file_name("module-library-compact.png"),
                29 => path.with_file_name("analysis-dock-compact.png"),
                30 => path.with_file_name("analysis-visuals-compact.png"),
                31 => path.with_file_name("rack-pulses.png"),
                32 => path.with_file_name("rack-push.png"),
                33 => path.with_file_name("rack-gates.png"),
                34 => path.with_file_name("audio-performance-simulated.png"),
                35 => path.with_file_name("parameter-context-entry.png"),
                36 => path.with_file_name("cable-output-menu.png"),
                37 => path.with_file_name("cable-input-menu.png"),
                38 => path.with_file_name("rack-score-studio.png"),
                39 => path.with_file_name("rack-scope-time.png"),
                40 => path.with_file_name("rack-scope-xy.png"),
                41 => path.with_file_name("rack-scope-external.png"),
                42 => path.with_file_name("rack-modal-resonator.png"),
                43 => path.with_file_name("rack-colored-noise.png"),
                44 => path.with_file_name("rack-random-values.png"),
                45 => path.with_file_name("rack-shift-register.png"),
                46 => path.with_file_name("construction-motion-080.png"),
                47 => path.with_file_name("construction-motion-180.png"),
                48 => path.with_file_name("construction-motion-340.png"),
                49 => path.with_file_name("construction-motion-settled.png"),
                50 => path.with_file_name("mcp-session.png"),
                51 => path.with_file_name("room-lights-on.png"),
                52 => path.with_file_name("room-dark.png"),
                53 => path.with_file_name("room-lights-out.png"),
                54 => path.with_file_name("module-inspection.png"),
                55 => path.with_file_name("parameter-precision-navigation.png"),
                56 => path.with_file_name("cable-gesture-guidance.png"),
                57 => path.with_file_name("cable-gesture-lights-out.png"),
                58 => path.with_file_name("cable-palette-editor.png"),
                59 => path.with_file_name("cable-palette-created.png"),
                60 => path.with_file_name("rack-adsr-eg.png"),
                61 => path.with_file_name("rack-adsr-eg-lights-out.png"),
                62 => path.with_file_name("rack-midi-cc.png"),
                63 => path.with_file_name("rack-midi-cc-lights-out.png"),
                64 => path.with_file_name("rack-midi-gate.png"),
                65 => path.with_file_name("rack-midi-gate-lights-out.png"),
                66 => path.with_file_name("rack-annotations.png"),
                67 => path.with_file_name("rack-annotations-lights-out.png"),
                68 => path.with_file_name("notes-editor.png"),
                69 => path.with_file_name("notes-editor-conflict.png"),
                70 => path.with_file_name("rack-stock-vcf.png"),
                71 => path.with_file_name("rack-stock-vcf-lights-out.png"),
                72 => path.with_file_name("rack-stock-vca.png"),
                73 => path.with_file_name("rack-stock-vca-lights-out.png"),
                74 => path.with_file_name("rack-parameter-choices.png"),
                75 => path.with_file_name("rack-parameter-choices-lights-out.png"),
                76 => path.with_file_name("rack-port-lights.png"),
                77 => path.with_file_name("rack-port-lights-lights-out.png"),
                78 => path.with_file_name("view-cable-controls.png"),
                79 => path.with_file_name("cable-presentation-clear-panel.png"),
                80 => path.with_file_name("cable-presentation-loose.png"),
                81 => path.with_file_name("library-zoom-100.png"),
                82 => path.with_file_name("library-zoom-wide.png"),
                83 => path.with_file_name("library-zoom-compact.png"),
                84 => path.with_file_name("library-zoom-200.png"),
                85 => path.with_file_name("rack-small-labels.png"),
                86 => path.with_file_name("cable-move-unplug.png"),
                87 => path.with_file_name("cable-move-travel.png"),
                88 => path.with_file_name("cable-move-settle.png"),
                89 => path.with_file_name("cable-move-complete.png"),
                90 => path.with_file_name("cable-tension-tight.png"),
                91 => path.with_file_name("agent-follow-before.png"),
                92 => path.with_file_name("agent-follow-mid.png"),
                93 => path.with_file_name("agent-follow-settled.png"),
                94 => path.with_file_name("rack-stock-delay.png"),
                95 => path.with_file_name("rack-stock-delay-lights-out.png"),
                96 => path.with_file_name("stock-delay-library.png"),
                97 => path.with_file_name("rack-wavetable.png"),
                98 => path.with_file_name("rack-wavetable-lights-out.png"),
                99 => path.with_file_name("wavetable-library.png"),
                100 => path.with_file_name("rack-random.png"),
                101 => path.with_file_name("rack-random-lights-out.png"),
                102 => path.with_file_name("random-library.png"),
                103 => path.with_file_name("rack-weak-scope.png"),
                104 => path.with_file_name("rack-weak-scope-lights-out.png"),
                105 => path.with_file_name("rack-weak-scope-manual.png"),
                106 => path.with_file_name("menu-motion-opening.png"),
                107 => path.with_file_name("menu-motion-mid.png"),
                108 => path.with_file_name("menu-motion-settled.png"),
                109 => path.with_file_name("rack-merge.png"),
                110 => path.with_file_name("rack-merge-lights-out.png"),
                111 => path.with_file_name("merge-library.png"),
                112 => path.with_file_name("rack-split.png"),
                113 => path.with_file_name("rack-split-lights-out.png"),
                114 => path.with_file_name("split-library.png"),
                115 => path.with_file_name("rack-stock-lfo.png"),
                116 => path.with_file_name("rack-stock-lfo-lights-out.png"),
                117 => path.with_file_name("stock-lfo-library.png"),
                118 => path.with_file_name("rack-stock-vco.png"),
                119 => path.with_file_name("rack-stock-vco-lights-out.png"),
                120 => path.with_file_name("stock-vco-library.png"),
                121 => path.with_file_name("rack-stock-seq3.png"),
                122 => path.with_file_name("rack-stock-seq3-lights-out.png"),
                123 => path.with_file_name("stock-seq3-library.png"),
                124 => path.with_file_name("rack-stock-quantizer.png"),
                125 => path.with_file_name("rack-stock-quantizer-lights-out.png"),
                126 => path.with_file_name("stock-quantizer-library.png"),
                127 => path.with_file_name("rack-stock-mid-side.png"),
                128 => path.with_file_name("rack-stock-mid-side-lights-out.png"),
                129 => path.with_file_name("stock-mid-side-library.png"),
                130 => path.with_file_name("rack-stock-mix.png"),
                131 => path.with_file_name("rack-stock-mix-lights-out.png"),
                132 => path.with_file_name("stock-mix-library.png"),
                133 => path.with_file_name("rack-stock-octave.png"),
                134 => path.with_file_name("rack-stock-octave-lights-out.png"),
                135 => path.with_file_name("stock-octave-library.png"),
                136 => path.with_file_name("rack-stock-vca-mix.png"),
                137 => path.with_file_name("rack-stock-vca-mix-lights-out.png"),
                138 => path.with_file_name("stock-vca-mix-library.png"),
                _ => unreachable!("Unexpected native smoke capture index"),
            };
            image::save_buffer(
                &target,
                image.as_raw(),
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
            )
            .expect("Save screenshot");
            if pending.is_some() {
                ctx.data_mut(|d| d.remove::<crate::smoke_schedule::Receipt>(receipt_id));
            }
            eprintln!(
                "Native smoke: capture {}/149 at frame {frame}: {}",
                captures + 1,
                target.display()
            );
            let captures = captures + 1;
            self.smoke_test.as_mut().unwrap().2 = captures;
        }
        assert!(frame < 2960, "Native screenshot timeout");
    }
}
include!("stock_noise_fixture.rs");
include!("stock_process_fixture.rs");
include!("ferrofluid_fixture.rs");
include!("studio_mixer_fixture.rs");
impl eframe::App for OpenPatch {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        let tail_id = egui::Id::new("native-smoke-noise-tail");
        if let Some(mut tail) = ctx.data(|d| d.get_temp::<NoiseSmokeCapture>(tail_id)) {
            if matches!(tail.index, 141 | 144) && !tail.query_sent {
                input.events.push(egui::Event::Text(
                    if tail.index == 141 {
                        "stock_noise"
                    } else {
                        "studio_mixer"
                    }
                    .into(),
                ));
                tail.query_sent = true;
                ctx.data_mut(|d| d.insert_temp(tail_id, tail));
            }
        }
        if self
            .smoke_test
            .as_ref()
            .is_some_and(|(_, frame, _)| *frame >= 606)
        {
            // Xvfb without a window manager need not grant OS focus. This
            // screenshot fixture explicitly models a focused editor; production
            // input and focus-loss cancellation remain untouched.
            input.focused = true;
            input
                .events
                .retain(|event| !matches!(event, egui::Event::WindowFocused(_)));
            if self
                .smoke_test
                .as_ref()
                .is_some_and(|(_, frame, _)| *frame == 612)
            {
                input.events.push(egui::Event::Text("A4*2".into()));
            }
        }
        if let Some((_, frame, _)) = &self.smoke_test {
            if *frame == 2918 {
                input.events.push(egui::Event::Text("stock_mix".into()));
            }
            if *frame == 2950 {
                input.events.push(egui::Event::Text("stock_octave".into()));
            }
            if *frame == 2957 {
                input.events.push(egui::Event::Text("stock_vca_mix".into()));
            }
            if *frame == 2818 {
                input
                    .events
                    .push(egui::Event::Text("stock_mid_side".into()));
            }
            if *frame == 2720 {
                input
                    .events
                    .push(egui::Event::Text("stock_quantizer".into()));
            }
            if *frame == 2268 {
                input.events.push(egui::Event::Text("Wavetable".into()));
            }
            if matches!(*frame, 1972 | 1988 | 2020) {
                input.events.push(egui::Event::Text(
                    if *frame == 1972 { "analog_vco" } else { "seq3" }.into(),
                ));
            }
            if matches!(*frame, 1184 | 1240) {
                input.events.push(egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if matches!(*frame, 1190 | 1192 | 1296 | 1300 | 1304) {
                let canvas = ctx
                    .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("smoke-rack-canvas")))
                    .expect("Rendered smoke rack canvas");
                let position = if matches!(*frame, 1190 | 1296) {
                    self.rack.port_center_for_smoke(
                        canvas,
                        &self.patch,
                        &self.catalog,
                        "voice",
                        "square",
                        false,
                    )
                } else {
                    self.rack.port_center_for_smoke(
                        canvas,
                        &self.patch,
                        &self.catalog,
                        "filter",
                        "signal",
                        true,
                    )
                }
                .expect("Real smoke port");
                input.events.push(egui::Event::PointerMoved(position));
                if matches!(*frame, 1190 | 1296 | 1304) {
                    input.events.push(egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: *frame != 1304,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
            if *frame == 1240 {
                let position = input
                    .events
                    .iter()
                    .find_map(|event| match event {
                        egui::Event::PointerMoved(position) => Some(*position),
                        _ => None,
                    })
                    .unwrap_or(egui::pos2(800.0, 700.0));
                input.events.push(egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_wavetable_import();
        if self.wavetable_import.is_some() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        let menu_motion_capture = self
            .smoke_test
            .as_ref()
            .is_some_and(|(_, frame, _)| (2396..=2418).contains(frame));
        if self
            .smoke_test
            .as_ref()
            .is_some_and(|(_, frame, _)| *frame >= 2396)
        {
            let frame = self.smoke_test.as_ref().unwrap().1;
            ctx.input_mut(|input| {
                // Only the appended menu scene uses a synthetic clock. Native
                // Area opacity still comes entirely from egui's own fade.
                input.time = 1_000_000.0 + (frame - 2396) as f64 * 0.01;
                input.predicted_dt = 1.0 / 60.0;
            });
        }
        crate::menu_motion::configure(
            ctx,
            if menu_motion_capture {
                crate::construction_motion::MotionMode::Full
            } else {
                self.construction.motion_mode()
            },
            self.smoke_test.is_some() && !menu_motion_capture,
        );
        if self.smoke_test.is_some() {
            ctx.style_mut(|style| {
                style.interaction.tooltip_delay = f32::INFINITY;
                // Earlier captures qualify settled UI states; only the appended
                // menu scene enables motion with a deterministic native clock.
                if !menu_motion_capture {
                    style.animation_time = 0.0;
                }
            });
        }
        self.service_agent_follow(ctx);
        self.sync();
        self.service_manual_release(ctx);
        self.observe_document();
        self.toolbar(ctx);
        if let Some(action) = self.construction.show(ctx) {
            self.status = match execute_shared(&self.shared, action.name, action.arguments) {
                Ok(receipt) => format!(
                    "Construction {} · {} steps committed",
                    receipt["status"].as_str().unwrap_or("updated"),
                    receipt["completed_steps"]
                ),
                Err(error) => error,
            };
            self.sync();
        }
        self.rack
            .set_construction_motion_mode(self.construction.motion_mode());
        self.footer(ctx);
        let library_action = self.browser(ctx);
        let palette_enabled = self.library_enabled();
        if self
            .cable_palette_ui
            .show(ctx, &mut self.cable_palette, palette_enabled)
        {
            self.save_host_settings();
        }
        self.global_shortcuts(ctx);
        if let Some(action) = self.deferred_action.take() {
            self.request_action(action);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_action(PendingAction::Close);
        }
        self.performance_keyboard(ctx);
        self.scope.service(&self.patch, self.audio.as_mut());
        self.analysis_dock(ctx);
        if self.smoke_test.is_none() || self.audio.is_some() {
            self.rack.set_port_lights(
                self.audio
                    .as_ref()
                    .and_then(|audio| audio.port_lights(self.revision)),
            );
            let states = self
                .patch
                .nodes
                .iter()
                .filter(|n| {
                    matches!(
                        n.kind.as_str(),
                        "seq3" | "stock_seq3" | "switch_1to4" | "switch_4to1"
                    )
                })
                .filter_map(|n| {
                    self.audio
                        .as_ref()?
                        .sequence_state(&n.id)
                        .map(|state| (n.id.clone(), state))
                })
                .collect();
            self.rack.set_sequence_states(states, self.audio.is_some());
            self.rack.set_stock_quantizer_states(
                self.audio
                    .as_ref()
                    .filter(|_| {
                        self.patch
                            .nodes
                            .iter()
                            .any(|node| node.kind == "stock_quantizer")
                    })
                    .map(|audio| audio.stock_quantizer_states())
                    .unwrap_or_default(),
            );
            self.rack.set_stock_seq3_states(
                self.audio
                    .as_ref()
                    .filter(|_| {
                        self.patch
                            .nodes
                            .iter()
                            .any(|node| node.kind == "stock_seq3")
                    })
                    .map(|audio| audio.stock_seq3_states())
                    .unwrap_or_default(),
            );
            let mut lights = BTreeMap::new();
            let mut controls = BTreeMap::new();
            let mut scopes = BTreeMap::new();
            let mut scope_identities = BTreeMap::new();
            if let Some(audio) = self.audio.as_ref() {
                for node in &self.patch.nodes {
                    if crate::performance::rack_scope_kind(&node.kind) {
                        if let Some((kind, token)) = audio.module_scope_identity(&node.id) {
                            if kind == node.kind {
                                scope_identities.insert(node.id.clone(), token);
                            }
                        }
                        if let Some(frame) = audio.module_scope_frame(&node.id) {
                            if scope_identities.get(&node.id) == Some(&frame.token) {
                                scopes.insert(node.id.clone(), frame.snapshot);
                            }
                        }
                    }
                    if let Some(state) = audio.module_lights(&node.id) {
                        lights.insert(node.id.clone(), state);
                    }
                    if let Some(state) = audio.module_control_state(&node.id) {
                        controls.insert(node.id.clone(), state);
                    }
                }
            }
            let random_values = self
                .audio
                .as_ref()
                .filter(|_| {
                    self.patch
                        .nodes
                        .iter()
                        .any(|node| node.kind == "random_values")
                })
                .map(|audio| audio.random_values_states())
                .unwrap_or_default();
            self.rack.set_random_values_states(random_values);
            let shift_registers = self
                .audio
                .as_ref()
                .filter(|_| self.patch.nodes.iter().any(|node| node.kind == "sh_asr"))
                .map(|audio| audio.shift_register_states())
                .unwrap_or_default();
            self.rack.set_shift_register_states(shift_registers);

            self.rack.set_module_states(lights, controls);
            self.rack.set_scope_identities(scope_identities);
            self.rack.set_scope_states(scopes);
        }
        self.sync_midi_cc_states();
        self.sync_midi_gate_states();
        self.rack_workspace(ctx, library_action);
        self.mcp_session
            .show(ctx, &self.shared, Path::new(&self.workspace));
        self.observe_document();
        self.files_window(ctx);
        self.preset_window(ctx);
        self.shader_window(ctx);
        self.audio_settings(ctx);
        self.audio_performance(ctx);
        self.recovery_window(ctx);
        self.unsaved_window(ctx);
        self.template_window(ctx);
        self.observe_document();
        self.autosave();
        if self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.confirm_engine_upgrade {
            let mut convert = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("engine-upgrade")).show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.heading("Enable polyphony and feedback?");
                ui.label("Engine v2 adds one sample of delay to every cable. Your modules and connections stay in place, but timing and sound can change.");
                ui.label("You can undo this change. The saved file stays unchanged until you save.");
                ui.horizontal(|ui| {
                    cancel = ui.button("Keep legacy timing").clicked();
                    convert = ui.button("Enable engine v2").clicked();
                });
            });
            if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.confirm_engine_upgrade = false;
            } else if convert {
                let mut patch = self.patch.clone();
                patch.version = 2;
                if self.command("replace_patch", json!({"patch":patch})) {
                    self.confirm_engine_upgrade = false;
                    self.status = "Polyphony and feedback enabled".into();
                }
            }
        }
        // Dock placement happens before the rack, but note events must wait for
        // every editor/modal to claim keyboard focus in this frame.
        self.service_performance_keyboard(ctx);
        self.smoke(ctx);
        if self.smoke_test.is_some() {
            // Preserve every fixture frame and screenshot receipt, while avoiding
            // ~23 seconds of idle delay inside the external 45-second smoke cap.
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }
    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        self.cancel_wavetable_import();
        self.mcp_session.shutdown();
        if let (Some(v), Some(gl)) = (&self.visualizer, gl) {
            v.destroy(gl);
        }
    }
}

#[cfg(test)]
mod project_workflow_tests {
    mod toolbar_settings_tests {
        include!("toolbar_settings_tests.rs");
    }
    mod random_factory_tests {
        include!("random_factory_tests.rs");
    }
    mod stock_mix_factory_tests {
        include!("stock_mix_factory_tests.rs");
    }
    mod studio_mixer_factory_tests {
        include!("studio_mixer_factory_tests.rs");
    }
    mod stock_noise_factory_tests {
        include!("stock_noise_factory_tests.rs");
    }
    mod stock_process_factory_tests {
        include!("stock_process_factory_tests.rs");
    }
    mod stock_vca_mix_factory_tests {
        include!("stock_vca_mix_factory_tests.rs");
    }
    mod stock_mid_side_factory_tests {
        include!("stock_mid_side_factory_tests.rs");
    }
    mod wavetable_factory_tests {
        include!("wavetable_factory_tests.rs");
    }
    mod wavetable_file_ui_tests {
        include!("wavetable_file_ui_tests.rs");
    }
    mod agent_follow_app_tests {
        include!("agent_follow_app_tests.rs");
    }
    mod construction_document_tests {
        include!("construction_document_tests.rs");
    }
    mod module_identity_app_tests {
        include!("module_identity_app_tests.rs");
    }
    mod annotation_app_tests {
        include!("annotation_app_tests.rs");
    }
    use super::*;
    use openpatch::control::Session;
    use std::path::Path;

    mod precision_host_tests {
        include!("precision_host_tests.rs");
    }
    mod palette_host_tests {
        include!("palette_host_tests.rs");
    }
    mod cable_presentation_host_tests {
        include!("cable_presentation_host_tests.rs");
    }
    mod library_zoom_host_tests {
        include!("library_zoom_host_tests.rs");
    }
    mod cable_move_host_tests {
        include!("cable_move_host_tests.rs");
    }

    mod midi_gate_app_tests {
        include!("midi_gate_app_tests.rs");
    }
    mod midi_cc_app_tests {
        include!("midi_cc_app_tests.rs");
    }

    // Headless host state: no window, graphics context, device, or MIDI access.
    fn host(path: &Path) -> OpenPatch {
        let mut session = Session::new(path).unwrap();
        session
            .replace_patch(openpatch_core::demo_patch(), None)
            .unwrap();
        let patch = session.patch().clone();
        let revision = session.revision();
        let state_id = session.state_id();
        let workspace = session.workspace().display().to_string();
        let mut rack = Rack::default();
        rack.reconcile(&patch, &catalog());
        let documents = DocumentState::new(&patch, revision, &rack);
        let store = Store::new(path).unwrap();
        let recovery_names = store.recoveries();
        OpenPatch {
            shared: session.into_shared(),
            patch,
            revision,
            state_id,
            history_layouts: BTreeMap::new(),
            audio_revision: None,
            live_events_attach_pending: false,
            audio: None,
            device: String::new(),
            output_device: None,
            output_devices: Vec::new(),
            midi: None,
            midi_ports: Vec::new(),
            midi_device: None,
            midi_epoch: 0,
            settings_open: false,
            settings_status: String::new(),
            audio_performance_open: false,
            smoke_diagnostics: None,
            keyboard: Keyboard::default(),
            keyboard_open: false,
            rack,
            visualizer: None,
            gl: None,
            shader: DEFAULT_SHADER.into(),
            shader_open: false,
            shader_status: String::new(),
            analysis_tab: None,
            visual_mode: 0,
            scope: Scope::default(),
            volume: 0.65,
            status: String::new(),
            filename: "test.openpatch".into(),
            title: "Test".into(),
            files: Vec::new(),
            workspace,
            file_mode: None,
            file_focus: false,
            wavetable_file_target: None,
            wavetable_import: None,
            browser_open: false,
            library: ModuleLibrary::default(),
            preset_dialog: None,
            template_save: None,
            catalog: catalog(),
            start: Instant::now(),
            mcp_session: crate::mcp_session_ui::Panel::new(true),
            construction: crate::construction_ui::Panel::default(),
            cable_palette: crate::cable_palette::Palette::default(),
            cable_palette_ui: crate::cable_palette_ui::Panel::default(),
            smoke_motion_origin: None,
            documents,
            projects: Some(store),
            active_file: None,
            active_token: None,
            after_save: None,
            deferred_action: None,
            deferred_manual_release: None,
            autosave_needed: false,
            recovery_open: !recovery_names.is_empty(),
            recovery_names,
            recovery_details: BTreeMap::new(),
            restored_recovery: None,
            autosaved_document: None,
            last_document_change: Instant::now(),
            last_autosave_attempt: Instant::now(),
            can_undo: false,
            can_redo: false,
            confirm_engine_upgrade: false,
            confirm_discard: None,
            allow_close: false,
            smoke_test: None,
        }
    }

    fn parameter_entry_host(path: &Path) -> OpenPatch {
        let mut app = host(path);
        assert!(app.command(
            "replace_patch",
            json!({"patch":{
                "version":2,"nodes":[
                    {"id":"tone","kind":"analog_vco","params":{"frequency":220.0}},
                    {"id":"level","kind":"constant","params":{"value":1.0}}
                ]
            }})
        ));
        app.rack.select_only("tone".into());
        app.observe_document();
        app
    }

    #[test]
    fn struck_resonator_example_drives_audio_and_scope_without_devices() {
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/struck-resonator.json")).unwrap();
        let restored: Patch =
            serde_json::from_str(&serde_json::to_string(&patch).unwrap()).unwrap();
        let mut engine = restored.compile().unwrap();
        let mut audio = vec![0.0; 2500];
        engine.process_block(&mut audio);
        assert!(audio.iter().all(|x| x.is_finite() && x.abs() < 1.0));
        assert!(audio.iter().any(|x| x.abs() > 0.01));
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap();
        assert_eq!(scope.x_channels, 1);
        assert_eq!(scope.y_channels, 0);
        let extrema = scope.measurements().x.unwrap();
        assert!(extrema.min < 0.0 && extrema.max > 0.0);
        assert_eq!(extrema.observations, openpatch_core::SCOPE_BINS);
    }

    #[test]
    fn rack_scope_smoke_fixtures_capture_real_sixteen_voice_dsp() {
        for (xy, external) in [(false, false), (true, false), (false, true)] {
            let (patch, snapshot) = OpenPatch::rack_scope_smoke_fixture(xy, external);
            assert_eq!(snapshot.xy, xy);
            assert_eq!(snapshot.waiting, !xy);
            assert_eq!(
                snapshot.trigger_input_channels,
                if external { 16 } else { 0 }
            );
            assert_eq!((snapshot.x_channels, snapshot.y_channels), (16, 16));
            assert_eq!(
                patch.nodes.iter().find(|n| n.id == "scope").unwrap().params["time"],
                0.08
            );
            for channel in [&snapshot.x, &snapshot.y] {
                for voice in channel {
                    assert!(voice
                        .iter()
                        .all(|p| p.min.is_finite() && p.max.is_finite() && p.min <= p.max));
                    assert!(voice.iter().any(|p| p.min < -1.0));
                    assert!(voice.iter().any(|p| p.max > 1.0));
                }
            }
            assert_ne!(snapshot.x[0], snapshot.x[15]);
        }
    }

    #[test]
    fn adsr_eg_fast_click_ticket_waits_for_a_distinct_ui_frame_or_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let ctx = egui::Context::default();
        let (ticket, snapshot) = crate::audio::test_manual_release_ticket();
        let mut ticket = Some(ticket);
        let _ = ctx.run(
            egui::RawInput {
                focused: true,
                ..Default::default()
            },
            |ctx| {
                if let Some(ticket) = ticket.take() {
                    app.deferred_manual_release = Some((ctx.cumulative_frame_nr(), ticket));
                }
                app.service_manual_release(ctx);
                app.service_manual_release(ctx);
                assert!(app.deferred_manual_release.is_some());
                assert_eq!(snapshot()["queued"], 1);
                if ctx.current_pass_index() == 0 {
                    ctx.request_discard("verify repeated layout pass");
                }
            },
        );
        assert!(app.deferred_manual_release.is_some());
        let _ = ctx.run(
            egui::RawInput {
                focused: true,
                ..Default::default()
            },
            |ctx| {
                app.service_manual_release(ctx);
            },
        );
        assert!(app.deferred_manual_release.is_none());
        assert_eq!(snapshot()["queued"], 2);
        for focused in [true, false] {
            let (ticket, snapshot) = crate::audio::test_manual_release_ticket();
            let mut ticket = Some(ticket);
            let _ = ctx.run(
                egui::RawInput {
                    focused,
                    ..Default::default()
                },
                |ctx| {
                    if let Some(ticket) = ticket.take() {
                        app.deferred_manual_release = Some((ctx.cumulative_frame_nr(), ticket));
                    }
                    app.file_mode = focused.then_some(FileMode::Open);
                    app.service_manual_release(ctx);
                },
            );
            assert!(app.deferred_manual_release.is_none());
            assert_eq!(snapshot()["queued"], 2);
            app.file_mode = None;
        }
    }

    #[test]
    fn stock_seq3_deferred_click_waits_for_actual_press_and_low_release_before_next_click() {
        for (press, release, mask) in [
            ("run_press", "run_release", 2),
            ("seq_gate1_press", "seq_gate1_release", 8),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = host(dir.path());
            let ctx = egui::Context::default();
            let (ticket, mut render, next_ticket) =
                crate::audio::test_stock_seq3_manual_release_ticket(press, release);
            let mut ticket = Some(ticket);
            for frame in 0..2 {
                let _ = ctx.run(
                    egui::RawInput {
                        focused: true,
                        ..Default::default()
                    },
                    |ctx| {
                        if let Some(ticket) = ticket.take() {
                            app.deferred_manual_release = Some((ctx.cumulative_frame_nr(), ticket));
                        }
                        app.service_manual_release(ctx);
                        assert!(
                            app.deferred_manual_release.is_some(),
                            "Frame {frame}: press has not been sampled"
                        );
                        app.manual_button_click(ctx, "seq", press, release);
                        assert!(app.status.contains("Previous button click is waiting"));
                        assert!(app.deferred_manual_release.is_some());
                    },
                );
            }
            let sampled = render(1);
            assert_eq!(sampled["modules"]["seq"]["controls"]["pressed_mask"], mask);
            let state = &sampled["modules"]["seq"]["state"];
            if mask == 2 {
                assert_eq!(state["running"], false);
            } else {
                assert_eq!(state["gates"][0], false);
            }
            let _ = ctx.run(
                egui::RawInput {
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    app.service_manual_release(ctx);
                    assert!(
                        app.deferred_manual_release.is_some(),
                        "Release is posted but LOW has not been sampled"
                    );
                    app.manual_button_click(ctx, "seq", press, release);
                    assert!(app.status.contains("Previous button click is waiting"));
                    assert!(app.deferred_manual_release.is_some());
                },
            );
            assert!(
                next_ticket().is_err(),
                "Private ticket API must also wait for the pending LOW sample"
            );
            let released = render(1);
            assert_eq!(released["modules"]["seq"]["controls"]["pressed_mask"], 0);
            let _ = ctx.run(
                egui::RawInput {
                    focused: true,
                    ..Default::default()
                },
                |ctx| app.service_manual_release(ctx),
            );
            assert!(app.deferred_manual_release.is_none());
            assert!(app.finish_previous_manual_click());
            let mut second = next_ticket().unwrap();
            let twice = render(1);
            let state = &twice["modules"]["seq"]["state"];
            if mask == 2 {
                assert_eq!(state["running"], true);
            } else {
                assert_eq!(state["gates"][0], true);
            }
            assert!(second.ready_to_release());
            second.begin_release();
            assert!(!second.is_finished());
            render(1);
            assert!(second.is_finished());
        }
    }

    #[test]
    fn midi_gate_offline_fixture_has_actual_gate_lamps_poly_scope_and_learn() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.prepare_midi_gate_fixture();
        assert!(app.audio.is_none());
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            },
            |ctx| app.rack_workspace(ctx, None),
        );
        preset_text(&output, "48 C3");
        preset_text(&output, "LRN");
        preset_text(&output, "Panic");
    }

    #[test]
    fn midi_cc_offline_fixture_uses_actual_paired_values_poly_scope_and_learn() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.prepare_midi_cc_fixture();
        assert!(app.audio.is_none());
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            },
            |ctx| app.rack_workspace(ctx, None),
        );
        preset_text(&output, "CC 7");
        preset_text(&output, "LRN");
        preset_text(&output, "VALUE 127");
    }

    #[test]
    fn adsr_eg_offline_fixture_has_four_stages_and_polyphonic_capture() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.prepare_adsr_eg_fixture();
        assert!(app.audio.is_none());
        assert_eq!(
            app.patch
                .nodes
                .iter()
                .find(|n| n.id == "gates")
                .unwrap()
                .params["v4"],
            0.0
        );
        assert_eq!(app.rack.room_brightness(), 1.0);
        app.rack.set_room_brightness(0.0);
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            },
            |ctx| app.rack_workspace(ctx, None),
        );
        preset_text(&output, "1  100  10k ms");
        preset_text(&output, "CV DEPTH ±100%");
        preset_text(&output, "ADSR EG");
    }

    #[test]
    fn merge_smoke_uses_actual_counts_for_auto_forced_zero_and_truncated_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let lights = app.prepare_merge_fixture();
        for (id, expected) in [("auto", 12), ("zero", 0), ("eight", 8)] {
            assert_eq!(lights[id].output_channels, Some(expected));
            assert_eq!(lights[id].automatic_channels, Some(12));
            assert_eq!(lights[id].count, 0);
        }
        assert_eq!(app.patch.nodes.len(), 5);
        assert_eq!(app.patch.cables.len(), 6);
        assert!(app.status.contains("offline"));
    }

    #[test]
    fn split_smoke_observes_zero_silent_mono_and_all_sixteen_named_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let lights = app.prepare_split_fixture();
        for (id, expected) in [("empty", 0), ("silent", 1), ("sixteen", 16)] {
            assert_eq!(lights[id].input_channels, Some(expected));
            assert_eq!(lights[id].count, 0);
        }
        assert_eq!(app.patch.nodes.len(), 6);
        assert_eq!(app.patch.cables.len(), 3);
        assert!(app.status.contains("offline"));
    }

    #[test]
    fn stock_lfo_smoke_observes_free_clocked_and_polyphonic_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let lights = app.prepare_stock_lfo_fixture();
        assert_eq!(lights.len(), 3);
        assert_eq!(lights["mono"].clock_frequency, Some(2.0));
        assert!((lights["clocked"].clock_frequency.unwrap() - 8.0).abs() < 0.1);
        assert_eq!(lights["poly"].brightness[2][0], 1.0);
        assert_eq!(app.patch.nodes.len(), 6);
        assert_eq!(app.patch.cables.len(), 4);
        assert!(app.status.contains("offline"));
    }

    #[test]
    fn stock_quantizer_factory_voice_observes_actual_notes_audio_and_discard_guard() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let observed = app.prepare_stock_quantizer_fixture();
        assert_eq!(observed.count, 0);
        assert_eq!(observed.playing_notes.unwrap().count_ones(), 1);
        assert_eq!(app.patch.nodes.len(), 9);
        assert!(app
            .patch
            .cables
            .iter()
            .any(|c| c.from == "sequence" && c.to == "scale" && c.input == "pitch"));
        assert!(app
            .patch
            .cables
            .iter()
            .any(|c| c.from == "scale" && c.output == "pitch" && c.to == "voice"));
        assert!(app.status.contains("offline"));
        let playable: Patch =
            serde_json::from_str(include_str!("../../../examples/stock-quantized-voice.json"))
                .unwrap();
        let revision = app.revision;
        app.replace_from_menu(playable, "Quantized stock voice");
        assert_eq!(app.revision, revision);
        let action = app
            .confirm_discard
            .take()
            .expect("modified patch requires discard choice");
        app.perform_action(action);
        assert_eq!(app.title, "Quantized stock voice");
        assert_eq!(app.rack.layout()["positions"]["scale"], json!([0.0, 0.0]));
    }

    #[test]
    fn stock_seq3_smoke_and_bundled_three_row_voice_observe_real_engine() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let telemetry = app.prepare_stock_seq3_fixture();
        assert_eq!(telemetry.length, 6);
        assert!(app.patch.nodes.iter().any(|node| node.kind == "stock_seq3"));
        assert!(app.status.contains("offline"));
        let playable: Patch =
            serde_json::from_str(include_str!("../../../examples/stock-sequenced-voice.json"))
                .unwrap();
        let mut engine = playable.compile().unwrap();
        let mut audio = vec![[0.0; 2]; 48000];
        engine.process_audio_block(&mut audio);
        assert!(audio.iter().flatten().all(|sample| sample.is_finite()));
        assert!(audio.iter().flatten().any(|sample| sample.abs() > 0.001));
        for (port, target, input) in [
            ("cv1", "voice", "pitch"),
            ("cv2", "filter", "cutoff_cv"),
            ("cv3", "filter", "resonance_cv"),
            ("trigger", "envelope", "gate"),
        ] {
            assert!(playable.cables.iter().any(|c| c.from == "sequence"
                && c.output == port
                && c.to == target
                && c.input == input));
        }
        let revision = app.revision;
        app.replace_from_menu(playable, "Stock sequenced voice");
        assert_eq!(app.revision, revision);
        let action = app
            .confirm_discard
            .take()
            .expect("unsaved example must retain discard guard");
        app.perform_action(action);
        assert!(app.revision > revision);
        assert_eq!(app.title, "Stock sequenced voice");
        assert_eq!(app.rack.layout()["zoom"], 0.75);
    }

    #[test]
    fn stock_vco_smoke_and_bundled_voice_observe_real_audio_and_modes() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let lights = app.prepare_stock_vco_fixture();
        assert_eq!(lights.len(), 3);
        assert_eq!(lights["poly"].brightness[2][0], 1.0);
        assert_eq!(lights["synced"].brightness[3][0], 1.0);
        assert_eq!(lights["synced"].brightness[4][0], 1.0);
        assert_eq!(app.patch.nodes.len(), 6);
        assert_eq!(app.patch.cables.len(), 4);
        assert!(app.status.contains("offline"));
        let playable: Patch =
            serde_json::from_str(include_str!("../../../examples/stock-voice.json")).unwrap();
        for kind in [
            "stock_lfo",
            "stock_vco",
            "stock_vcf",
            "stock_vca",
            "adsr_eg",
        ] {
            assert!(playable.nodes.iter().any(|node| node.kind == kind));
        }
        let mut engine = playable.compile().unwrap();
        let mut audio = vec![[0.0; 2]; 48000];
        engine.process_audio_block(&mut audio);
        assert!(audio.iter().flatten().all(|sample| sample.is_finite()));
        assert!(audio.iter().flatten().any(|sample| sample.abs() > 0.001));
        let original = app.revision;
        app.replace_from_menu(playable, "Stock modular voice");
        assert_eq!(
            app.revision, original,
            "The fixture is unsaved; the menu must preserve its discard guard"
        );
        assert!(matches!(
            app.confirm_discard,
            Some(PendingAction::Replace(_, _))
        ));
        let confirmed = app.confirm_discard.take().unwrap();
        app.perform_action(confirmed);
        assert!(app.revision > original);
        assert_eq!(app.rack.layout()["zoom"], 0.75);
        assert_eq!(app.title, "Stock modular voice");
    }

    #[test]
    fn stock_vca_example_and_smoke_use_real_audio_gain_and_fitting_notes() {
        let playable: Patch =
            serde_json::from_str(include_str!("../../../examples/stock-vca-voice.json")).unwrap();
        assert!(playable
            .nodes
            .iter()
            .any(|n| n.id == "amplifier" && n.kind == "stock_vca"));
        assert!(playable
            .cables
            .iter()
            .any(|c| c.from == "filter" && c.to == "amplifier" && c.input == "signal"));
        assert!(playable
            .cables
            .iter()
            .any(|c| c.from == "envelope" && c.to == "amplifier" && c.input == "cv"));
        let mut engine = playable.compile().unwrap();
        let mut audio = vec![0.0; 48000];
        engine.process_block(&mut audio);
        assert!(audio.iter().all(|v| v.is_finite() && v.abs() < 1.0));
        assert!(audio.iter().any(|v| v.abs() > 0.001));
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.prepare_stock_vca_fixture();
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        for room in [1.0, 0.0] {
            app.rack.set_room_brightness(room);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.rack_workspace(ctx, None),
            );
            for text in [
                "VCA",
                "GAIN / VOICE · 0–100%",
                "5 ACTIVE VOICES",
                "EXP",
                "LIN",
                "SCOPE",
            ] {
                preset_text(&output, text);
            }
            fn notes_bounds(shape: &egui::Shape) -> Option<egui::Rect> {
                match shape {
                    egui::Shape::Text(text) if text.galley.text().starts_with("VCA VOICE") => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()))
                    }
                    egui::Shape::Vec(shapes) => shapes.iter().find_map(notes_bounds),
                    _ => None,
                }
            }
            let (clip, bounds) = output
                .shapes
                .iter()
                .find_map(|s| notes_bounds(&s.shape).map(|b| (s.clip_rect, b)))
                .expect("actual Notes text");
            assert!(
                clip.expand(0.5).contains_rect(bounds),
                "VCA Notes are clipped: {bounds:?} outside {clip:?}"
            );
        }
    }

    #[test]
    fn stock_delay_factory_voice_and_smoke_use_real_clock_and_mix_wet_traces() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.prepare_stock_delay_fixture();
        let echo = app
            .patch
            .nodes
            .iter()
            .find(|node| node.id == "echo")
            .unwrap();
        assert_eq!(echo.kind, "stock_delay");
        assert_eq!(echo.params.len(), 8);
        assert!(app
            .patch
            .cables
            .iter()
            .any(|cable| cable.to == "echo" && cable.input == "clock"));
        assert!(app
            .patch
            .cables
            .iter()
            .any(|cable| cable.from == "echo" && cable.output == "mix" && cable.to == "scope"));
        assert!(app
            .patch
            .cables
            .iter()
            .any(|cable| cable.from == "echo" && cable.output == "wet" && cable.to == "scope"));
        assert_eq!(
            serde_json::to_value(app.shared.lock().unwrap().patch()).unwrap(),
            serde_json::to_value(&app.patch).unwrap()
        );
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        for room in [1.0, 0.0] {
            app.rack.set_room_brightness(room);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 960.0),
                    )),
                    focused: true,
                    ..Default::default()
                },
                |ctx| app.rack_workspace(ctx, None),
            );
            for label in ["DELAY", "CLK", "MIX", "WET", "CV DEPTH"] {
                preset_text(&output, label);
            }
        }
    }

    #[test]
    fn stock_vcf_example_and_smoke_use_real_connected_audio_and_scope_capture() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.prepare_stock_vcf_fixture();
        assert_eq!(
            app.patch
                .nodes
                .iter()
                .find(|n| n.id == "filter")
                .unwrap()
                .kind,
            "stock_vcf"
        );
        assert!(app
            .patch
            .cables
            .iter()
            .any(|c| c.from == "voice" && c.to == "filter" && c.input == "signal"));
        assert!(app
            .patch
            .cables
            .iter()
            .any(|c| c.from == "filter" && c.output == "lowpass" && c.to == "amplifier"));
        assert!(app
            .patch
            .cables
            .iter()
            .any(|c| c.from == "envelope" && c.to == "filter" && c.input == "cutoff_cv"));
        assert_eq!(app.rack.room_brightness(), 1.0);
        let mut engine = app.patch.compile().unwrap();
        let mut audio = vec![0.0; 48000];
        engine.process_block(&mut audio);
        assert!(audio.iter().all(|v| v.is_finite() && v.abs() < 1.0));
        assert!(audio.iter().any(|v| v.abs() > 0.001));
        assert!(engine.resolve_module_lights("filter").is_err());
        app.rack.set_room_brightness(0.0);
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 900.0),
                )),
                ..Default::default()
            },
            |ctx| app.rack_workspace(ctx, None),
        );
        preset_text(&output, "VCF");
        preset_text(&output, "CUTOFF");
        preset_text(&output, "SCOPE");
        fn notes_bounds(shape: &egui::Shape) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Text(text) if text.galley.text().starts_with("FILTER VOICE") => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()))
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(notes_bounds),
                _ => None,
            }
        }
        let (clip, bounds) = output
            .shapes
            .iter()
            .find_map(|shape| notes_bounds(&shape.shape).map(|bounds| (shape.clip_rect, bounds)))
            .expect("actual Notes text must be painted");
        assert!(
            clip.expand(0.5).contains_rect(bounds),
            "example Notes text is clipped: {bounds:?} outside {clip:?}"
        );
    }

    #[test]
    fn adsr_eg_example_renders_an_audible_velocity_shaped_chord() {
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/polyphonic-adsr-voice.json"))
                .unwrap();
        let mut engine = patch.compile().unwrap();
        let mut audio = vec![0.0; 48000];
        engine.process_block(&mut audio);
        assert!(audio.iter().all(|v| v.is_finite() && v.abs() < 1.0));
        assert!(audio.iter().any(|v| v.abs() > 0.001));
        let env = engine
            .module_lights(engine.resolve_module_lights("envelope").unwrap())
            .unwrap();
        assert_eq!(env.input_channels, Some(4));
    }

    #[test]
    fn precision_and_cable_smoke_scenes_use_real_descriptors_and_pointer_gestures() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.mcp_session = crate::mcp_session_ui::Panel::new(false);
        app.smoke_test = Some((dir.path().join("unused.png"), 1152, 55));
        let ctx = egui::Context::default();
        // Run the same logical scene progression and raw-input hook as native
        // smoke, without a graphics context, screenshot events or audio devices.
        while app.smoke_test.as_ref().unwrap().1 < 1328 {
            let logical = app.smoke_test.as_ref().unwrap().1;
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(logical as f64 / 60.0),
                focused: false,
                ..Default::default()
            };
            eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
            let output = ctx.run(input, |ctx| {
                app.sync();
                app.toolbar(ctx);
                app.footer(ctx);
                let action = app.browser(ctx);
                let palette_enabled = app.library_enabled();
                app.cable_palette_ui
                    .show(ctx, &mut app.cable_palette, palette_enabled);
                app.global_shortcuts(ctx);
                app.rack_workspace(ctx, action);
                app.smoke(ctx);
            });
            if logical == 1176 {
                preset_text(&output, "voice / pulse_width");
                preset_text(&output, "Apply & Next");
            }
            if matches!(logical, 1206 | 1234) {
                assert!(app.rack.cable_is_dragging_for_smoke());
                assert_eq!(
                    app.patch.cables.len(),
                    3,
                    "the pointer gesture remains a draft"
                );
                assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text().contains("Release: Create · stack"))),
                    "actual drop guidance must remain painted with room lights on or off");
            }
        }
        assert!(!app.rack.cable_is_dragging_for_smoke());
        assert_eq!(app.rack.room_brightness(), 1.0);
        assert_eq!(app.patch.cables.len(), 4);
        assert_eq!(
            app.patch.cables.last().unwrap().color,
            Some([244, 104, 111])
        );
        assert_eq!(app.cable_palette.next_index(), 1);
        assert!(app.audio.is_none());
    }

    #[test]
    fn parameter_smoke_fixture_opens_real_popup_without_a_pointer() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.smoke_test = Some((dir.path().join("unused.png"), 605, 35));
        let ctx = egui::Context::default();
        app.smoke(&ctx);
        app.sync();
        let mut output = None;
        for frame in 0..18 {
            app.smoke_test.as_mut().unwrap().1 = 606 + frame;
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                time: Some(frame as f64),
                focused: false,
                events: vec![egui::Event::WindowFocused(false)],
                ..Default::default()
            };
            eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
            output = Some(ctx.run(input, |ctx| app.rack_workspace(ctx, None)));
        }
        let output = output.unwrap();
        preset_text(&output, "Apply");
        preset_text(&output, "Enter frequency in Hz.");
        preset_text(&output, "Result: 880 Hz");
        assert_eq!(app.patch.nodes[0].params["frequency"], 523.251);
        assert!(egui::Popup::is_any_open(&ctx));
        app.smoke_test = None;
        let mut real_input = egui::RawInput {
            focused: false,
            events: vec![egui::Event::WindowFocused(false)],
            ..Default::default()
        };
        eframe::App::raw_input_hook(&mut app, &ctx, &mut real_input);
        assert!(!real_input.focused);
        assert!(matches!(
            real_input.events.as_slice(),
            [egui::Event::WindowFocused(false)]
        ));
    }

    #[test]
    fn parameter_enter_and_held_repeat_do_not_open_library() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = parameter_entry_host(dir.path());
        app.browser_open = false;
        app.rack
            .request_parameter_menu_for_smoke("tone", "frequency");
        let ctx = egui::Context::default();
        for frame in 0..5 {
            let events = if frame >= 2 {
                vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: frame > 2,
                    modifiers: egui::Modifiers::NONE,
                }]
            } else {
                vec![]
            };
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 800.0),
                    )),
                    time: Some(frame as f64),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    app.global_shortcuts(ctx);
                    app.rack_workspace(ctx, None);
                },
            );
            assert!(!app.browser_open, "frame {frame}");
        }
        assert!(!egui::Popup::is_any_open(&ctx));
    }

    #[test]
    fn captured_parameter_revision_rejects_mcp_races_even_after_app_sync() {
        for sync_before_apply in [false, true] {
            for mutation in ["parameter", "grouped", "remove_readd", "unrelated"] {
                let dir = tempfile::tempdir().unwrap();
                let mut app = parameter_entry_host(dir.path());
                if mutation == "grouped" {
                    assert!(app.command(
                        "set_parameter",
                        json!({
                            "id":"tone","parameter":"frequency","value":330.0,
                            "history_group":"prior-knob-drag"
                        })
                    ));
                }
                let opened_revision = app.revision;
                let opened_state = app.state_id;
                let layout = app.rack.layout();
                let selection = app.rack.selected_ids();
                // This is the emitted Apply command after the UI's own stale
                // check. An MCP edit can win before app.command takes the lock.
                let apply = json!({"id":"tone","parameter":"frequency","value":440.0,
                    "expected_revision":opened_revision});
                match mutation {
                    "parameter" | "grouped" => {
                        let mut args = json!({"id":"tone","parameter":"frequency","value":660.0});
                        if mutation == "grouped" {
                            args["history_group"] = json!("prior-knob-drag");
                        }
                        execute_shared(&app.shared, "set_parameter", args).unwrap();
                    }
                    "remove_readd" => {
                        execute_shared(&app.shared,"apply_edits",json!({"edits":[
                            {"tool":"remove_module","arguments":{"id":"tone"}},
                            {"tool":"add_module","arguments":{"id":"tone","kind":"analog_vco","params":{"frequency":880.0}}}
                        ]})).unwrap();
                    }
                    _ => {
                        execute_shared(
                            &app.shared,
                            "set_parameter",
                            json!({
                                "id":"level","parameter":"value","value":2.0
                            }),
                        )
                        .unwrap();
                    }
                }
                let newer = app.shared.lock().unwrap().snapshot();
                if mutation == "grouped" {
                    assert_eq!(newer["state_id"], opened_state);
                }
                if sync_before_apply {
                    app.sync();
                }
                assert!(
                    !app.command("set_parameter", apply),
                    "{mutation}, synced={sync_before_apply}"
                );
                assert!(app.status.contains("revision conflict"), "{}", app.status);
                assert_eq!(app.shared.lock().unwrap().snapshot(), newer);
                assert_eq!(app.revision, newer["revision"].as_u64().unwrap());
                assert_eq!(app.rack.layout(), layout);
                assert_eq!(app.rack.selected_ids(), selection);
            }
        }
    }

    #[test]
    fn numeric_parameter_apply_is_one_ungrouped_history_action_with_exact_undo_redo() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = parameter_entry_host(dir.path());
        // A prior grouped knob edit must not absorb the numeric Apply action.
        assert!(app.command(
            "set_parameter",
            json!({"id":"tone","parameter":"frequency",
            "value":330.0,"history_group":"prior-knob-drag"})
        ));
        let before = app.shared.lock().unwrap().snapshot();
        let before_patch = app.patch.clone();
        let before_state = app.state_id;
        let layout = app.rack.layout();
        let selection = app.rack.selected_ids();
        let apply = json!({"id":"tone","parameter":"frequency","value":440.0,
            "expected_revision":app.revision});
        assert!(apply.get("history_group").is_none());
        assert!(app.command("set_parameter", apply));
        let after = app.shared.lock().unwrap().snapshot();
        let edited_patch = app.patch.clone();
        let edited_state = app.state_id;
        assert_ne!(edited_state, before_state);
        assert_eq!(
            after["revision"].as_u64().unwrap(),
            before["revision"].as_u64().unwrap() + 1
        );
        assert_eq!(
            after["history"]["undo_steps"].as_u64().unwrap(),
            before["history"]["undo_steps"].as_u64().unwrap() + 1
        );
        assert_eq!(app.patch.nodes[0].params["frequency"], 440.0);
        app.history(false);
        assert_eq!(
            serde_json::to_value(&app.patch).unwrap(),
            serde_json::to_value(&before_patch).unwrap()
        );
        assert_eq!(app.state_id, before_state);
        assert_eq!(
            app.shared.lock().unwrap().snapshot()["history"]["redo_steps"],
            1
        );
        assert_eq!(app.rack.layout(), layout);
        assert_eq!(app.rack.selected_ids(), selection);
        app.history(true);
        assert_eq!(
            serde_json::to_value(&app.patch).unwrap(),
            serde_json::to_value(&edited_patch).unwrap()
        );
        assert_eq!(app.state_id, edited_state);
        assert_eq!(
            app.shared.lock().unwrap().snapshot()["history"]["redo_steps"],
            0
        );
        assert_eq!(app.rack.layout(), layout);
        assert_eq!(app.rack.selected_ids(), selection);
    }

    #[test]
    fn rejected_numeric_parameter_value_preserves_graph_history_layout_and_selection() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = parameter_entry_host(dir.path());
        let before = app.shared.lock().unwrap().snapshot();
        let layout = app.rack.layout();
        let selection = app.rack.selected_ids();
        assert!(!app.command(
            "set_parameter",
            json!({"id":"tone","parameter":"frequency",
            "value":1_000_000.0,"expected_revision":app.revision})
        ));
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
        assert_eq!(app.rack.layout(), layout);
        assert_eq!(app.rack.selected_ids(), selection);
    }

    fn preset_frame(
        ctx: &egui::Context,
        app: &mut OpenPatch,
        time: f64,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                time: Some(time),
                events,
                focused: true,
                ..Default::default()
            },
            |ctx| app.preset_window(ctx),
        )
    }

    fn preset_text(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        fn in_shape(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| in_shape(shape, label)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| in_shape(&shape.shape, label))
            .unwrap_or_else(|| panic!("missing preset UI label {label}"))
    }

    fn preset_click(ctx: &egui::Context, app: &mut OpenPatch, at: egui::Pos2, time: f64) {
        for (offset, pressed) in [(0.0, true), (0.1, false)] {
            preset_frame(
                ctx,
                app,
                time + offset,
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    #[test]
    fn preset_save_uses_reviewed_capture_after_patch_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"tone","kind":"analog_vco","params":{"frequency":220.0}}]
        }))
        .unwrap();
        assert!(app.command("replace_patch", json!({"patch":patch})));
        app.rack.select_only("tone".into());
        let layout = app.rack.layout();
        let selection = app.rack.selected_ids();
        let mut dialog = PresetDialog::new(
            "tone".into(),
            "analog_vco".into(),
            app.revision,
            PresetMode::Save,
        );
        dialog.name = "reviewed-tone".into();
        dialog.await_release = false;
        app.preset_dialog = Some(dialog);
        let ctx = egui::Context::default();
        preset_frame(&ctx, &mut app, 0.0, vec![]);
        let reviewed = app
            .preset_dialog
            .as_ref()
            .unwrap()
            .capture
            .clone()
            .expect("capture");
        assert_eq!(reviewed["params"]["frequency"], 220.0);

        assert!(app.command(
            "set_parameter",
            json!({
                "id":"tone","parameter":"frequency","value":330.0
            })
        ));
        let output = preset_frame(&ctx, &mut app, 1.0, vec![]);
        let save = preset_text(&output, "Save preset");
        preset_click(&ctx, &mut app, save, 2.0);
        assert!(app.preset_dialog.is_none());
        let saved = execute_shared(
            &app.shared,
            "read_module_preset",
            json!({
                "filename":"reviewed-tone.opreset"
            }),
        )
        .unwrap();
        assert_eq!(saved["preset"], reviewed);
        assert_eq!(app.patch.nodes[0].params["frequency"], 330.0);
        assert_eq!(app.rack.layout(), layout);
        assert_eq!(app.rack.selected_ids(), selection);
    }

    #[test]
    fn preset_save_accepts_enter_from_name_field() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"tone","kind":"analog_vco"}]
        }))
        .unwrap();
        assert!(app.command("replace_patch", json!({"patch":patch})));
        let mut dialog = PresetDialog::new(
            "tone".into(),
            "analog_vco".into(),
            app.revision,
            PresetMode::Save,
        );
        dialog.name = "enter-preset".into();
        dialog.await_release = false;
        app.preset_dialog = Some(dialog);
        let ctx = egui::Context::default();
        preset_frame(&ctx, &mut app, 0.0, vec![]);
        preset_frame(
            &ctx,
            &mut app,
            1.0,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.preset_dialog.is_none());
        assert!(execute_shared(
            &app.shared,
            "read_module_preset",
            json!({
                "filename":"enter-preset.opreset"
            })
        )
        .is_ok());
    }

    #[test]
    fn preset_preview_formats_controls_without_changing_saved_precision() {
        let catalog = catalog();
        let spec = &catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|module| module["kind"] == "analog_vco")
            .unwrap()["params"]["frequency"];
        let precise = json!(261.6255798339844_f64);
        assert_eq!(
            preset_control_value("frequency", &precise, spec),
            "261.6 Hz"
        );
        assert_eq!(precise, json!(261.6255798339844_f64));
        assert_eq!(
            preset_control_value("running", &json!(1.0), &Value::Null),
            "On"
        );
        assert_eq!(
            preset_control_value("running", &json!(0.0), &Value::Null),
            "Off"
        );
    }

    #[test]
    fn preset_browse_previews_inline_then_requires_current_target_to_apply() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"tone","kind":"analog_vco","params":{"frequency":220.0}}]
        }))
        .unwrap();
        assert!(app.command("replace_patch", json!({"patch":patch})));
        let captured = execute_shared(
            &app.shared,
            "capture_module_preset",
            json!({
                "id":"tone","expected_revision":app.revision
            }),
        )
        .unwrap();
        execute_shared(
            &app.shared,
            "save_module_preset",
            json!({
                "filename":"warm.opreset","preset":captured["preset"].clone()
            }),
        )
        .unwrap();
        assert!(app.command(
            "set_parameter",
            json!({
                "id":"tone","parameter":"frequency","value":330.0
            })
        ));
        let layout = app.rack.layout();
        let mut dialog = PresetDialog::new(
            "tone".into(),
            "analog_vco".into(),
            app.revision,
            PresetMode::Browse,
        );
        dialog.await_release = false;
        app.preset_dialog = Some(dialog);
        let ctx = egui::Context::default();
        preset_frame(&ctx, &mut app, 0.0, vec![]); // egui sizes a new modal invisibly.
        let output = preset_frame(&ctx, &mut app, 0.2, vec![]);
        let filename = preset_text(&output, "warm.opreset");
        preset_click(&ctx, &mut app, filename, 1.0);
        assert_eq!(
            app.preset_dialog
                .as_ref()
                .unwrap()
                .preview
                .as_ref()
                .unwrap(),
            &captured["preset"]
        );

        assert!(app.command(
            "set_parameter",
            json!({
                "id":"tone","parameter":"frequency","value":440.0
            })
        ));
        let output = preset_frame(&ctx, &mut app, 2.0, vec![]);
        let apply = preset_text(&output, "Apply preset");
        preset_click(&ctx, &mut app, apply, 3.0);
        assert!(
            app.preset_dialog.is_some(),
            "stale target must remain reviewable"
        );
        assert_eq!(app.patch.nodes[0].params["frequency"], 440.0);

        let output = preset_frame(&ctx, &mut app, 4.0, vec![]);
        let refresh = preset_text(&output, "Refresh target");
        preset_click(&ctx, &mut app, refresh, 5.0);
        let output = preset_frame(&ctx, &mut app, 6.0, vec![]);
        let filename = preset_text(&output, "warm.opreset");
        preset_click(&ctx, &mut app, filename, 7.0);
        let output = preset_frame(&ctx, &mut app, 8.0, vec![]);
        let apply = preset_text(&output, "Apply preset");
        preset_click(&ctx, &mut app, apply, 9.0);
        assert!(app.preset_dialog.is_none());
        assert_eq!(app.patch.nodes[0].params["frequency"], 220.0);
        assert_eq!(app.rack.layout(), layout);

        app.preset_dialog = Some(PresetDialog::new(
            "tone".into(),
            "analog_vco".into(),
            app.revision,
            PresetMode::Browse,
        ));
        preset_frame(
            &ctx,
            &mut app,
            10.0,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.preset_dialog.is_none());
    }

    #[test]
    fn seq3_preset_capture_waits_for_live_state_instead_of_using_saved_run() {
        use openpatch::control::LiveModuleEvents;
        use std::sync::Arc;
        struct MissingTelemetry;
        impl LiveModuleEvents for MissingTelemetry {
            fn trigger(&self, _: &str, _: openpatch_core::ModuleAction) -> Result<Value, String> {
                Ok(json!({}))
            }
            fn snapshot(&self) -> Value {
                json!({})
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"steps","kind":"seq3","params":{"running":1.0}}]
        }))
        .unwrap();
        assert!(app.command("replace_patch", json!({"patch":patch})));
        app.shared
            .lock()
            .unwrap()
            .set_live_module_events(Some(Arc::new(MissingTelemetry)))
            .unwrap();
        let mut dialog = PresetDialog::new(
            "steps".into(),
            "seq3".into(),
            app.revision,
            PresetMode::Save,
        );
        app.load_preset_dialog(&mut dialog);
        assert!(dialog.capture.is_none());
        assert!(dialog.status.contains("unavailable"));
        assert!(!dialog.pending);

        app.shared
            .lock()
            .unwrap()
            .set_live_module_events(None)
            .unwrap();
        dialog.pending = true;
        app.load_preset_dialog(&mut dialog);
        assert_eq!(dialog.capture.as_ref().unwrap()["params"]["running"], 1.0);
        assert_eq!(dialog.source, "offline_reset");
    }

    #[test]
    fn preset_search_finds_a_name_beyond_the_first_library_page() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"tone","kind":"analog_vco"}]
        }))
        .unwrap();
        assert!(app.command("replace_patch", json!({"patch":patch})));
        let captured =
            execute_shared(&app.shared, "capture_module_preset", json!({"id":"tone"})).unwrap();
        for index in 0..30 {
            execute_shared(
                &app.shared,
                "save_module_preset",
                json!({
                    "filename":format!("item-{index:02}.opreset"),
                    "preset":captured["preset"].clone()
                }),
            )
            .unwrap();
        }
        execute_shared(
            &app.shared,
            "save_module_preset",
            json!({
                "filename":"z-needle.opreset","preset":captured["preset"].clone()
            }),
        )
        .unwrap();
        let mut dialog = PresetDialog::new(
            "tone".into(),
            "analog_vco".into(),
            app.revision,
            PresetMode::Browse,
        );
        dialog.await_release = false;
        app.preset_dialog = Some(dialog);
        let ctx = egui::Context::default();
        preset_frame(&ctx, &mut app, 0.0, vec![]); // egui sizes a new modal invisibly.
        let output = preset_frame(&ctx, &mut app, 0.2, vec![]);
        assert_eq!(app.preset_dialog.as_ref().unwrap().total, 31);
        assert_eq!(app.preset_dialog.as_ref().unwrap().entries.len(), 30);
        app.preset_dialog.as_mut().unwrap().search = "needle".into();
        let search = preset_text(&output, "Search");
        preset_click(&ctx, &mut app, search, 1.0);
        preset_frame(&ctx, &mut app, 2.0, vec![]);
        let dialog = app.preset_dialog.as_ref().unwrap();
        assert_eq!(dialog.query, "needle");
        assert_eq!(dialog.total, 1);
        assert_eq!(dialog.entries[0]["filename"], "z-needle.opreset");
    }
    fn library_frame(
        app: &mut OpenPatch,
        ctx: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
        focused: bool,
    ) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 800.0),
                )),
                time: Some(time),
                modifiers: events
                    .iter()
                    .rev()
                    .find_map(|event| match event {
                        egui::Event::Key { modifiers, .. } => Some(*modifiers),
                        _ => None,
                    })
                    .unwrap_or_default(),
                events,
                focused,
                ..Default::default()
            },
            |ctx| {
                let action = app.browser(ctx);
                app.rack_workspace(ctx, action);
            },
        );
    }

    fn pointer_event(position: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn library_drop_is_one_history_step_and_preserves_view_and_open_library() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        assert!(app.command("replace_patch", json!({"patch":Patch::new_v2()})));
        app.rack
            .restore_layout(&json!({"version":1,"zoom":0.8,"pan":[24,12],"positions":{}}))
            .unwrap();
        app.browse();
        let ctx = egui::Context::default();
        library_frame(&mut app, &ctx, 0.0, vec![], true);
        let before = app.capture_document();
        let undo = app.shared.lock().unwrap().snapshot()["history"]["undo_steps"]
            .as_u64()
            .unwrap();
        let point = egui::pos2(300.0, 250.0);
        egui::DragAndDrop::set_payload(
            &ctx,
            LibraryDrag {
                kind: "constant".into(),
            },
        );
        library_frame(
            &mut app,
            &ctx,
            1.0,
            vec![egui::Event::PointerMoved(point), pointer_event(point, true)],
            true,
        );
        assert!(app.patch.nodes.is_empty(), "Preview must not commit");
        library_frame(&mut app, &ctx, 1.1, vec![pointer_event(point, false)], true);
        library_frame(&mut app, &ctx, 1.2, vec![], true);
        assert_eq!(app.patch.nodes.len(), 1);
        assert_eq!(app.patch.nodes[0].kind, "constant");
        assert!(app.browser_open);
        let after = app.capture_document();
        assert_eq!(after.layout["pan"], before.layout["pan"]);
        assert_eq!(after.layout["zoom"], before.layout["zoom"]);
        assert_eq!(
            after.layout["positions"]["constant-1"],
            json!([285.0, 420.0])
        );
        assert_eq!(
            app.shared.lock().unwrap().snapshot()["history"]["undo_steps"],
            undo + 1
        );
        app.history(false);
        assert!(app.capture_document().equivalent(&before));
        app.history(true);
        assert!(app.capture_document().equivalent(&after));
    }

    #[test]
    fn library_drop_cancellation_never_mutates_patch_or_geometry() {
        for cancel in ["outside", "escape", "focus", "modal"] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = host(dir.path());
            app.browse();
            let ctx = egui::Context::default();
            library_frame(&mut app, &ctx, 0.0, vec![], true);
            let before = app.capture_document();
            let snapshot = app.shared.lock().unwrap().snapshot();
            let point = egui::pos2(300.0, 250.0);
            egui::DragAndDrop::set_payload(
                &ctx,
                LibraryDrag {
                    kind: "constant".into(),
                },
            );
            library_frame(
                &mut app,
                &ctx,
                1.0,
                vec![egui::Event::PointerMoved(point), pointer_event(point, true)],
                true,
            );
            let mut events = Vec::new();
            if cancel == "escape" {
                events.push(egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            if cancel == "modal" {
                app.file_mode = Some(FileMode::Save);
            }
            let released = if cancel == "outside" {
                egui::pos2(1090.0, 790.0)
            } else {
                point
            };
            events.extend([
                egui::Event::PointerMoved(released),
                pointer_event(released, false),
            ]);
            library_frame(&mut app, &ctx, 1.1, events, cancel != "focus");
            assert!(app.capture_document().equivalent(&before), "{cancel}");
            assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot, "{cancel}");
            assert!(!egui::DragAndDrop::has_payload_of_type::<LibraryDrag>(&ctx));
        }
    }

    #[test]
    fn library_add_failure_preserves_geometry_selection_and_newer_mcp_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.capture_document();
        let original = app.patch.nodes[0].id.clone();
        app.rack.select_only(original.clone());
        let layout = app.rack.layout();
        execute_shared(
            &app.shared,
            "add_module",
            json!({"id":"external","kind":"constant"}),
        )
        .unwrap();
        let snapshot = app.shared.lock().unwrap().snapshot();
        assert!(!app.add_module("constant", Some((egui::pos2(300.0, 420.0), false))));
        assert_eq!(app.rack.layout(), layout);
        assert_eq!(app.rack.selected_ids(), vec![original]);
        assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
        app.sync();
        let full = Patch {
            nodes: (0..openpatch_core::MAX_NODES)
                .map(|i| openpatch_core::Node {
                    content: None,
                    score: None,
                    id: format!("node{i}"),
                    kind: "constant".into(),
                    params: Default::default(),
                    bypassed: false,
                })
                .collect(),
            ..Patch::new_v2()
        };
        assert!(app.command("replace_patch", json!({"patch":full})));
        app.capture_document();
        let layout = app.rack.layout();
        let snapshot = app.shared.lock().unwrap().snapshot();
        assert!(!app.add_module("constant", Some((egui::pos2(300.0, 420.0), false))));
        assert_eq!(app.rack.layout(), layout);
        assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
    }

    #[test]
    fn library_add_receipt_keeps_placement_with_its_commit_during_mcp_race() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let before = app.capture_document();
        let base = app.state_id;
        let receipt=execute_shared(&app.shared,"apply_edits",json!({"edits":[{"tool":"add_module","arguments":{"id":"library-add","kind":"constant"}}],"expected_revision":app.revision})).unwrap();
        let committed_state = receipt["state_id"].as_u64().unwrap();
        let shared = app.shared.clone();
        let mut live = shared.lock().unwrap();
        live.execute("remove_module", json!({"id":"library-add"}))
            .unwrap();
        app.install_add_receipt(
            &receipt,
            "library-add".into(),
            Some((egui::pos2(300.0, 420.0), false)),
            base,
            before.layout.clone(),
        )
        .unwrap();
        let committed = app.capture_document();
        assert_eq!(
            committed.layout["positions"]["library-add"],
            json!([300.0, 420.0])
        );
        assert_eq!(app.history_layouts[&committed_state], committed.layout);
        app.sync();
        assert_eq!(app.state_id, committed_state);
        drop(live);
        app.sync();
        assert!(!app.patch.nodes.iter().any(|node| node.id == "library-add"));
        app.history(false);
        assert!(app.capture_document().equivalent(&committed));
        app.history(false);
        assert!(app.capture_document().equivalent(&before));
    }

    fn movement(app: &OpenPatch, ids: &[String], delta: egui::Vec2) -> ModuleMove {
        let layout = app.rack.layout();
        let before: BTreeMap<_, _> = ids
            .iter()
            .map(|id| {
                let p = &layout["positions"][id];
                (
                    id.clone(),
                    egui::pos2(p[0].as_f64().unwrap() as f32, p[1].as_f64().unwrap() as f32),
                )
            })
            .collect();
        let after = before
            .iter()
            .map(|(id, point)| (id.clone(), *point + delta))
            .collect();
        ModuleMove {
            revision: app.revision,
            before,
            after,
        }
    }

    #[test]
    fn module_moves_share_ordered_history_with_ui_and_mcp_edits_and_saved_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.save_document("moves.openpatch", true);
        let before = app.capture_document();
        let id = app.patch.nodes[0].id.clone();
        let first = movement(&app, std::slice::from_ref(&id), egui::vec2(0.0, 420.0));
        let original_history = app.shared.lock().unwrap().snapshot()["history"]["undo_steps"]
            .as_u64()
            .unwrap();
        assert!(app.move_modules(first));
        let moved = app.capture_document();
        assert!(app.dirty());
        assert_eq!(
            app.shared.lock().unwrap().snapshot()["history"]["undo_steps"],
            original_history + 1
        );
        edit(&mut app, 330.0);
        let edited = app.capture_document();
        let second = movement(&app, std::slice::from_ref(&id), egui::vec2(300.0, 0.0));
        assert!(app.move_modules(second));
        let last = app.capture_document();
        app.history(false);
        assert!(app.capture_document().equivalent(&edited));
        execute_shared(&app.shared, "undo", json!({})).unwrap();
        app.sync();
        assert!(app.capture_document().equivalent(&moved));
        execute_shared(&app.shared, "undo", json!({})).unwrap();
        app.sync();
        assert!(app.capture_document().equivalent(&before));
        assert!(!app.dirty());
        for _ in 0..3 {
            execute_shared(&app.shared, "redo", json!({})).unwrap();
            app.sync();
        }
        assert!(app.capture_document().equivalent(&last));
        app.save_current();
        assert!(!app.dirty());
        app.history(false);
        app.capture_document();
        assert!(app.dirty());
        app.history(true);
        app.capture_document();
        assert!(!app.dirty());
    }

    #[test]
    fn group_move_is_one_step_preserves_view_selection_and_branches_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.rack.set_zoom(0.75);
        let ids: Vec<_> = app.patch.nodes.iter().map(|node| node.id.clone()).collect();
        app.rack.select_only(ids[0].clone());
        let before = app.capture_document();
        let selection = app.rack.selected_ids();
        let proposal = movement(&app, &ids, egui::vec2(30.0, 420.0));
        assert!(app.move_modules(proposal));
        let after = app.capture_document();
        assert_eq!(after.layout["zoom"], before.layout["zoom"]);
        assert_eq!(after.layout["pan"], before.layout["pan"]);
        assert_eq!(app.rack.selected_ids(), selection);
        app.history(false);
        assert!(app.capture_document().equivalent(&before));
        assert!(app.can_redo);
        let branched = movement(&app, &ids, egui::vec2(60.0, 420.0));
        assert!(app.move_modules(branched));
        assert!(!app.can_redo);
        assert!(execute_shared(&app.shared, "redo", json!({})).is_err());
    }

    #[test]
    fn movement_history_eviction_prunes_geometry_to_retained_states() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let id = app.patch.nodes[0].id.clone();
        for _ in 0..132 {
            let proposal = movement(&app, std::slice::from_ref(&id), egui::vec2(15.0, 0.0));
            assert!(app.move_modules(proposal));
        }
        let retained: BTreeSet<_> = app.shared.lock().unwrap().history_state_ids().collect();
        assert_eq!(
            app.history_layouts.keys().copied().collect::<BTreeSet<_>>(),
            retained
        );
        assert_eq!(retained.len(), 129);
        app.history(false);
        app.history(false);
        let branch = movement(&app, std::slice::from_ref(&id), egui::vec2(0.0, 420.0));
        assert!(app.move_modules(branch));
        let retained: BTreeSet<_> = app.shared.lock().unwrap().history_state_ids().collect();
        assert_eq!(
            app.history_layouts.keys().copied().collect::<BTreeSet<_>>(),
            retained
        );
    }

    #[test]
    fn no_op_invalid_and_stale_move_proposals_cannot_create_history_or_replace_newer_geometry() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let id = app.patch.nodes[0].id.clone();
        let before = app.capture_document();
        let state = app.shared.lock().unwrap().snapshot();
        let no_op = movement(&app, std::slice::from_ref(&id), egui::Vec2::ZERO);
        assert!(!app.move_modules(no_op));
        let mut invalid = movement(&app, std::slice::from_ref(&id), egui::vec2(0.0, 420.0));
        invalid.after.get_mut(&id).unwrap().x = f32::NAN;
        assert!(!app.move_modules(invalid));
        assert_eq!(app.shared.lock().unwrap().snapshot(), state);
        assert!(app.capture_document().equivalent(&before));
        let stale = movement(&app, std::slice::from_ref(&id), egui::vec2(0.0, 420.0));
        execute_shared(
            &app.shared,
            "set_parameter",
            json!({"id":id,"parameter":"frequency","value":330}),
        )
        .unwrap();
        let newer = app.shared.lock().unwrap().snapshot();
        assert!(!app.move_modules(stale));
        assert_eq!(app.shared.lock().unwrap().snapshot(), newer);
        assert_eq!(app.rack.layout(), before.layout);
        assert_eq!(app.patch.nodes[0].params["frequency"], 330.0);
    }

    #[test]
    fn move_receipt_binds_geometry_to_its_state_before_newer_mcp_work_is_synced() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let before = app.capture_document();
        let base_state = app.state_id;
        let id = app.patch.nodes[0].id.clone();
        let proposal = movement(&app, std::slice::from_ref(&id), egui::vec2(0.0, 420.0));
        let receipt = app
            .shared
            .lock()
            .unwrap()
            .record_layout_checkpoint(app.revision)
            .unwrap();
        let committed_state = receipt["state_id"].as_u64().unwrap();
        let shared = app.shared.clone();
        let mut live = shared.lock().unwrap();
        live.execute(
            "set_parameter",
            json!({"id":id,"parameter":"frequency","value":330}),
        )
        .unwrap();
        app.install_move_receipt(&receipt, &proposal, base_state, before.layout.clone())
            .unwrap();
        let moved = app.capture_document();
        assert_eq!(app.history_layouts[&committed_state], moved.layout);
        assert_eq!(app.history_layouts[&base_state], before.layout);
        drop(live);
        app.sync();
        assert_eq!(app.patch.nodes[0].params["frequency"], 330.0);
        app.history(false);
        assert!(app.capture_document().equivalent(&moved));
        app.history(false);
        assert!(app.capture_document().equivalent(&before));
    }

    fn start_module_preview(app: &mut OpenPatch, ctx: &egui::Context) -> egui::Pos2 {
        library_frame(app, ctx, 0.0, vec![], true);
        let start = egui::pos2(85.0, 55.0);
        library_frame(
            app,
            ctx,
            0.1,
            vec![egui::Event::PointerMoved(start), pointer_event(start, true)],
            true,
        );
        let target = start + egui::vec2(30.0, 430.0);
        library_frame(app, ctx, 0.2, vec![egui::Event::PointerMoved(target)], true);
        target
    }

    #[test]
    fn saving_and_recovery_during_pointer_drag_keep_committed_geometry() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.save_document("drag.openpatch", true);
        let before = app.capture_document();
        let ctx = egui::Context::default();
        let target = start_module_preview(&mut app, &ctx);
        assert_eq!(app.rack.layout(), before.layout);
        app.save_current();
        assert!(app
            .projects
            .as_ref()
            .unwrap()
            .load("drag.openpatch")
            .unwrap()
            .document
            .equivalent(&before));
        assert!(!app.dirty());
        app.documents.mark_unsaved();
        app.autosave_needed = true;
        make_autosave_due(&mut app);
        app.autosave();
        let recoveries = recovery_files(dir.path());
        assert_eq!(recoveries.len(), 1);
        let recovery: Value =
            serde_json::from_slice(&std::fs::read(&recoveries[0]).unwrap()).unwrap();
        assert_eq!(recovery["layout"], before.layout);
        library_frame(
            &mut app,
            &ctx,
            0.3,
            vec![pointer_event(target, false)],
            true,
        );
        assert_ne!(app.rack.layout()["positions"], before.layout["positions"]);
        app.history(false);
        assert_eq!(app.rack.layout(), before.layout);
    }

    #[test]
    fn new_mcp_revisions_cancel_preview_even_with_unchanged_history_identity() {
        for mutation in ["grouped", "remove", "replace"] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = host(dir.path());
            let id = app.patch.nodes[0].id.clone();
            assert!(app.command(
                "set_parameter",
                json!({"id":id,"parameter":"frequency","value":220,"history_group":"knob"})
            ));
            let before = app.capture_document();
            let base_state = app.state_id;
            let ctx = egui::Context::default();
            let target = start_module_preview(&mut app, &ctx);
            match mutation {
                "grouped" => {
                    execute_shared(
                        &app.shared,
                        "set_parameter",
                        json!({"id":id,"parameter":"frequency","value":330,"history_group":"knob"}),
                    )
                    .unwrap();
                }
                "remove" => {
                    execute_shared(&app.shared, "remove_module", json!({"id":id})).unwrap();
                }
                _ => {
                    execute_shared(
                        &app.shared,
                        "replace_patch",
                        json!({"patch":{"version":2,"nodes":[{"id":"new","kind":"constant"}]}}),
                    )
                    .unwrap();
                }
            }
            let graph = app.shared.lock().unwrap().snapshot();
            app.sync();
            if mutation == "grouped" {
                assert_eq!(app.state_id, base_state);
            }
            assert_eq!(app.history_layouts[&base_state], before.layout);
            library_frame(
                &mut app,
                &ctx,
                0.3,
                vec![pointer_event(target, false)],
                true,
            );
            assert_eq!(app.shared.lock().unwrap().snapshot(), graph);
            if mutation == "grouped" {
                assert_eq!(app.rack.layout(), before.layout);
            } else {
                assert!(app.rack.layout()["positions"].get(&id).is_none());
            }
        }
    }

    #[test]
    fn ferrofluid_can_be_selected_and_other_modes_remain_available() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.select_analysis(Some(AnalysisTab::Visuals));
        let ctx = egui::Context::default();
        let draw = |app: &mut OpenPatch, time: f64, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 760.0),
                    )),
                    time: Some(time),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ctx| app.analysis_dock(ctx),
            );
        };
        draw(&mut app, 0.0, vec![]);
        draw(&mut app, 0.05, vec![]);
        for (step, mode) in [3, 1, 0, 2, 3].into_iter().enumerate() {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("visual-mode", mode))))
                .unwrap();
            let event = |pressed| egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            draw(
                &mut app,
                0.1 + step as f64 * 0.1,
                vec![egui::Event::PointerMoved(rect.center()), event(true)],
            );
            draw(&mut app, 0.15 + step as f64 * 0.1, vec![event(false)]);
            assert_eq!(app.visual_mode, mode);
            assert_eq!(app.analysis_tab, Some(AnalysisTab::Visuals));
        }
    }

    #[test]
    fn analysis_tabs_preserve_scope_capture_until_the_dock_is_closed() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.toggle_analysis(AnalysisTab::Visuals);
        assert_eq!(app.analysis_tab, Some(AnalysisTab::Visuals));
        assert!(!app.scope.open, "visuals alone must not arm probes");
        app.toggle_analysis(AnalysisTab::Scope);
        assert!(app.scope.open);
        app.toggle_analysis(AnalysisTab::Visuals);
        assert!(app.scope.open, "tab switching must retain scope capture");
        app.toggle_analysis(AnalysisTab::Visuals);
        assert_eq!(app.analysis_tab, None);
        assert!(!app.scope.open, "closing disarms capture");
        app.scope.inspect(0, "vco".into(), "out".into());
        app.select_analysis(Some(AnalysisTab::Scope));
        assert!(app.scope.open);
        assert_eq!(app.analysis_tab, Some(AnalysisTab::Scope));
    }

    #[test]
    fn global_shortcuts_respect_windows_popups_focus_and_library_drag() {
        for blocked in [
            "settings",
            "audio-performance",
            "shader",
            "popup",
            "blur",
            "text",
            "drag",
            "none",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = host(dir.path());
            assert!(app.command(
                "add_module",
                json!({"id":"shortcut-target","kind":"constant"})
            ));
            let before = app.shared.lock().unwrap().snapshot();
            app.settings_open = blocked == "settings";
            app.audio_performance_open = blocked == "audio-performance";
            app.shader_open = blocked == "shader";
            let ctx = egui::Context::default();
            let _ = ctx.run(
                egui::RawInput {
                    focused: blocked != "blur",
                    modifiers: egui::Modifiers::COMMAND,
                    events: vec![
                        egui::Event::Key {
                            key: egui::Key::Enter,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::COMMAND,
                        },
                        egui::Event::Key {
                            key: egui::Key::Z,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::COMMAND,
                        },
                    ],
                    ..Default::default()
                },
                |ctx| {
                    if blocked == "popup" {
                        egui::Popup::open_id(ctx, egui::Id::new("shortcut-popup"));
                    }
                    if blocked == "text" {
                        ctx.memory_mut(|m| m.request_focus(egui::Id::new("shortcut-text")));
                    }
                    if blocked == "drag" {
                        egui::DragAndDrop::set_payload(
                            ctx,
                            LibraryDrag {
                                kind: "constant".into(),
                            },
                        );
                    }
                    app.global_shortcuts(ctx);
                },
            );
            if blocked == "none" {
                assert!(app.browser_open);
                assert_ne!(app.shared.lock().unwrap().snapshot(), before);
            } else {
                assert!(!app.browser_open, "{blocked} leaked Enter");
                assert_eq!(
                    app.shared.lock().unwrap().snapshot(),
                    before,
                    "{blocked} leaked Undo"
                );
            }
        }
    }

    #[test]
    fn edit_menu_clear_cables_is_one_undoable_persisted_action_without_moving_modules() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.rack.set_positions_locked(true);
        app.save_document("before-clear.openpatch", true);
        let before = app.shared.lock().unwrap().snapshot();
        let layout = app.rack.layout();
        assert!(!app.patch.cables.is_empty());
        let ctx = egui::Context::default();
        let frame = |app: &mut OpenPatch, time, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 800.0),
                    )),
                    time: Some(time),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    app.toolbar(ctx);
                    app.rack_workspace(ctx, None);
                },
            )
        };
        frame(&mut app, 0.0, vec![]);
        let output = frame(&mut app, 0.1, vec![]);
        let edit = preset_text(&output, "Edit");
        for (time, pressed) in [(1.0, true), (1.1, false)] {
            frame(
                &mut app,
                time,
                vec![
                    egui::Event::PointerMoved(edit),
                    pointer_event(edit, pressed),
                ],
            );
        }
        let output = frame(&mut app, 1.2, vec![]);
        let clear = preset_text(&output, "Clear cables");
        for (time, pressed) in [(2.0, true), (2.1, false)] {
            frame(
                &mut app,
                time,
                vec![
                    egui::Event::PointerMoved(clear),
                    pointer_event(clear, pressed),
                ],
            );
        }
        let after = app.shared.lock().unwrap().snapshot();
        assert!(app.patch.cables.is_empty());
        assert_eq!(
            after["revision"].as_u64(),
            Some(before["revision"].as_u64().unwrap() + 1)
        );
        assert_eq!(
            after["history"]["undo_steps"].as_u64(),
            Some(before["history"]["undo_steps"].as_u64().unwrap() + 1)
        );
        assert_eq!(after["patch"]["nodes"], before["patch"]["nodes"]);
        assert_eq!(after["patch"]["output"], before["patch"]["output"]);
        assert_eq!(app.rack.layout(), layout);
        app.history(false);
        assert_eq!(
            app.shared.lock().unwrap().snapshot()["patch"],
            before["patch"]
        );
        assert_eq!(app.rack.layout(), layout);
        app.history(true);
        assert_eq!(
            app.shared.lock().unwrap().snapshot()["patch"],
            after["patch"]
        );
        assert!(app.rack.positions_locked());
        app.observe_document();
        assert!(app.dirty());
        make_autosave_due(&mut app);
        app.autosave();
        let files = recovery_files(dir.path());
        assert_eq!(files.len(), 1);
        let recovery: Value = serde_json::from_slice(&std::fs::read(&files[0]).unwrap()).unwrap();
        assert_eq!(recovery["patch"]["cables"], json!([]));
        assert_eq!(recovery["layout"], layout);
        app.save_document("cleared.openpatch", true);
        let saved = app
            .projects
            .as_ref()
            .unwrap()
            .load("cleared.openpatch")
            .unwrap();
        assert!(saved.document.equivalent(&app.capture_document()));
        assert!(!app.dirty());
    }

    #[test]
    fn hovered_delete_restores_module_cables_and_layout_with_one_undo() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"source","kind":"constant","params":{"value":2.0}},
            {"id":"sum","kind":"gain"}],"cables":[
            {"id":"first","from":"source","to":"sum","input":"signal"},
            {"id":"second","from":"source","to":"sum","input":"signal"}],"output":"sum"}))
        .unwrap();
        app.replace_now(patch, "Hover deletion");
        app.rack.set_positions_locked(true);
        let ctx = egui::Context::default();
        library_frame(&mut app, &ctx, 0.0, vec![], true);
        assert!(app.rack.selected_ids().is_empty());
        let before = app.shared.lock().unwrap().snapshot();
        let layout = app.rack.layout();
        let point = egui::pos2(
            layout["pan"][0].as_f64().unwrap() as f32 + 35.0,
            layout["pan"][1].as_f64().unwrap() as f32 + 35.0,
        );
        library_frame(
            &mut app,
            &ctx,
            1.0,
            vec![
                egui::Event::PointerMoved(point),
                egui::Event::Key {
                    key: egui::Key::Delete,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            true,
        );
        assert_eq!(app.patch.nodes.len(), 1);
        assert_eq!(app.patch.nodes[0].id, "sum");
        assert!(app.patch.cables.is_empty());
        assert_eq!(app.revision, before["revision"].as_u64().unwrap() + 1);
        app.history(false);
        assert_eq!(
            app.shared.lock().unwrap().snapshot()["patch"],
            before["patch"]
        );
        assert_eq!(app.rack.layout(), layout);
        assert!(app.rack.positions_locked());
        app.history(true);
        assert_eq!(app.patch.nodes.len(), 1);
        assert!(app.patch.cables.is_empty());
    }

    #[test]
    fn stale_clear_cables_cannot_clear_a_newer_mcp_patch() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let revision = app.revision;
        app.shared
            .lock()
            .unwrap()
            .execute("add_module", json!({"id":"new","kind":"constant"}))
            .unwrap();
        let newer = app.shared.lock().unwrap().snapshot();
        assert!(!app.command("clear_cables", json!({"expected_revision":revision})));
        assert_eq!(app.shared.lock().unwrap().snapshot(), newer);
        assert!(!app.patch.cables.is_empty());
        assert!(app.status.contains("revision conflict"));
    }

    #[test]
    fn toolbar_zoom_double_click_resets_and_single_click_allows_exact_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.rack.set_zoom(0.85);
        let ctx = egui::Context::default();
        let frame = |app: &mut OpenPatch, time, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 800.0),
                    )),
                    time: Some(time),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    app.toolbar(ctx);
                    app.rack_workspace(ctx, None);
                },
            )
        };
        frame(&mut app, 0.0, vec![]);
        let output = frame(&mut app, 0.1, vec![]);
        let at = preset_text(&output, "85%");
        for (time, pressed) in [(1.0, true), (1.05, false), (1.1, true), (1.15, false)] {
            frame(
                &mut app,
                time,
                vec![egui::Event::PointerMoved(at), pointer_event(at, pressed)],
            );
        }
        frame(&mut app, 1.3, vec![]);
        assert_eq!(
            app.rack.zoom(),
            1.0,
            "An old text draft must not overwrite double-click reset"
        );
        assert!(!ctx.wants_keyboard_input());
        for (time, pressed) in [(2.0, true), (2.05, false)] {
            frame(
                &mut app,
                time,
                vec![egui::Event::PointerMoved(at), pointer_event(at, pressed)],
            );
        }
        frame(&mut app, 2.1, vec![egui::Event::Text("175".into())]);
        frame(
            &mut app,
            2.2,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(&mut app, 2.3, vec![]);
        assert_eq!(app.rack.zoom(), 1.75);
    }

    #[test]
    fn keyboard_zoom_uses_resized_rack_center_in_the_same_frame() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.rack.set_zoom(0.85);
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 800.0),
                )),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                app.toolbar(ctx);
                app.rack_workspace(ctx, None);
            },
        );
        let old = app.rack.layout();
        let pan = egui::vec2(
            old["pan"][0].as_f64().unwrap() as f32,
            old["pan"][1].as_f64().unwrap() as f32,
        );
        let mut pivot = Vec2::ZERO;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(940.0, 680.0),
                )),
                focused: true,
                modifiers: egui::Modifiers::COMMAND,
                events: vec![egui::Event::Key {
                    key: egui::Key::Equals,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }],
                ..Default::default()
            },
            |ctx| {
                app.toolbar(ctx);
                pivot = ctx.available_rect().size() * 0.5;
                app.global_shortcuts(ctx);
                app.rack_workspace(ctx, None);
            },
        );
        let expected = pivot - (pivot - pan) / 0.85;
        let after = app.rack.layout();
        let actual = egui::vec2(
            after["pan"][0].as_f64().unwrap() as f32,
            after["pan"][1].as_f64().unwrap() as f32,
        );
        assert_eq!(app.rack.zoom(), 1.0);
        assert!(
            (actual - expected).length() < 0.001,
            "{actual:?} vs {expected:?}"
        );
    }

    #[test]
    fn navigation_shortcuts_change_view_without_graph_history() {
        for modifiers in [egui::Modifiers::COMMAND, egui::Modifiers::CTRL] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = host(dir.path());
            let graph = app.shared.lock().unwrap().snapshot();
            let ctx = egui::Context::default();
            let mut layout = app.rack.layout();
            layout["zoom"] = json!(0.85);
            layout["pan"] = json!([-150.0, 80.0]);
            app.rack.restore_layout(&layout).unwrap();
            for (key, expected) in [
                (egui::Key::Equals, 1.0),
                (egui::Key::Minus, std::f32::consts::FRAC_1_SQRT_2),
                (egui::Key::Num0, 1.0),
                (egui::Key::Plus, std::f32::consts::SQRT_2),
            ] {
                let _ = ctx.run(
                    egui::RawInput {
                        focused: true,
                        modifiers,
                        events: vec![egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers,
                        }],
                        ..Default::default()
                    },
                    |ctx| app.global_shortcuts(ctx),
                );
                assert!((app.rack.zoom() - expected).abs() < 0.0001);
                assert_eq!(
                    app.rack.layout()["pan"],
                    layout["pan"],
                    "Before a viewport exists, zoom must not invent a pan reset"
                );
                assert_eq!(app.shared.lock().unwrap().snapshot(), graph);
            }
        }
    }

    #[test]
    fn navigation_shortcuts_respect_editor_focus_and_pointer_gestures() {
        for blocked in [
            "settings",
            "audio-performance",
            "shader",
            "popup",
            "blur",
            "text",
            "drag",
            "pointer",
            "alt",
        ] {
            for key in [egui::Key::Equals, egui::Key::Minus, egui::Key::Num0] {
                let dir = tempfile::tempdir().unwrap();
                let mut app = host(dir.path());
                app.rack.set_zoom(0.85);
                let before = app.rack.layout();
                app.settings_open = blocked == "settings";
                app.audio_performance_open = blocked == "audio-performance";
                app.shader_open = blocked == "shader";
                let ctx = egui::Context::default();
                let modifiers = egui::Modifiers {
                    alt: blocked == "alt",
                    ..egui::Modifiers::COMMAND
                };
                let mut events = vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }];
                if blocked == "pointer" {
                    events.push(pointer_event(egui::pos2(80.0, 80.0), true));
                }
                let _ = ctx.run(
                    egui::RawInput {
                        focused: blocked != "blur",
                        modifiers,
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        if blocked == "popup" {
                            egui::Popup::open_id(ctx, egui::Id::new("zoom-popup"));
                        }
                        if blocked == "text" {
                            ctx.memory_mut(|m| m.request_focus(egui::Id::new("zoom-text")));
                        }
                        if blocked == "drag" {
                            egui::DragAndDrop::set_payload(
                                ctx,
                                LibraryDrag {
                                    kind: "constant".into(),
                                },
                            );
                        }
                        app.global_shortcuts(ctx);
                    },
                );
                assert_eq!(app.rack.layout(), before, "{blocked} leaked {key:?}");
            }
        }
    }

    #[test]
    fn host_view_preferences_survive_new_example_open_import_and_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let expected =
            Document::new(openpatch_core::demo_patch(), Rack::default().layout()).unwrap();
        {
            let previous = Store::new(dir.path()).unwrap();
            previous.save_recovery(&expected).unwrap();
            previous.save_new("locked.openpatch", &expected).unwrap();
        }
        std::fs::write(
            dir.path().join("legacy.json"),
            serde_json::to_vec(&expected.patch).unwrap(),
        )
        .unwrap();
        let mut app = host(dir.path());
        let recovery = app.recovery_names[0].clone();
        app.rack.set_positions_locked(true);
        app.rack.set_room_brightness(0.1);
        app.load_document(&recovery, true);
        assert!(app.rack.positions_locked());
        assert_eq!(app.rack.room_brightness(), 0.1);
        assert_eq!(app.title, "Recovered project");
        app.new_from_template(false);
        assert!(app.rack.positions_locked());
        assert_eq!(app.rack.room_brightness(), 0.1);
        app.replace_now(Patch::new_v2(), "Untitled");
        assert!(app.rack.positions_locked());
        assert_eq!(app.rack.room_brightness(), 0.1);
        assert!(app.patch.nodes.is_empty());
        app.replace_now(openpatch_core::demo_patch(), "Factory example");
        assert!(app.rack.positions_locked());
        assert_eq!(app.rack.room_brightness(), 0.1);
        app.load_document("locked.openpatch", false);
        assert!(app.rack.positions_locked());
        assert_eq!(app.rack.room_brightness(), 0.1);
        assert_eq!(app.active_file.as_deref(), Some("locked.openpatch"));
        app.load_document("legacy.json", false);
        assert!(app.rack.positions_locked());
        assert_eq!(app.rack.room_brightness(), 0.1);
        assert!(app.active_file.is_none());
        assert_eq!(app.patch.nodes.len(), expected.patch.nodes.len());
    }

    #[test]
    fn room_brightness_persists_without_patch_history_or_dirty_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let before = app.shared.lock().unwrap().snapshot();
        let layout = app.rack.layout();
        let layout_revision = app.rack.layout_revision();
        let document = app.capture_document();
        let dirty = app.dirty();
        assert_eq!(app.rack.room_brightness(), 1.0);
        app.rack.set_room_brightness(0.1);
        app.save_host_settings();
        let preferences: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join(".host-settings.json")).unwrap())
                .unwrap();
        assert_eq!(preferences["room_brightness"].as_f64().unwrap() as f32, 0.1);
        app.rack.set_room_brightness(1.0);
        app.apply_host_preferences(&preferences);
        assert_eq!(app.rack.room_brightness(), 0.1);
        assert_eq!(app.rack.layout(), layout);
        assert_eq!(app.rack.layout_revision(), layout_revision);
        assert!(app.capture_document().equivalent(&document));
        assert_eq!(app.dirty(), dirty);
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
        for value in [json!({}), json!({"room_brightness":"dark"}), Value::Null] {
            app.apply_host_preferences(&value);
            assert_eq!(app.rack.room_brightness(), 1.0);
        }
        for (value, expected) in [(json!(-0.5), 0.0), (json!(2.0), 1.0), (json!(0.0), 0.0)] {
            app.apply_host_preferences(&json!({"room_brightness":value}));
            assert_eq!(app.rack.room_brightness(), expected);
        }
        app.rack.set_room_brightness(f32::NAN);
        assert_eq!(app.rack.room_brightness(), 1.0);
    }

    #[test]
    fn position_lock_persists_as_host_preference_without_patch_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let before = app.shared.lock().unwrap().snapshot();
        let layout = app.rack.layout();
        app.rack.set_positions_locked(true);
        app.save_host_settings();
        let preferences: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join(".host-settings.json")).unwrap())
                .unwrap();
        assert_eq!(preferences["lock_module_positions"], true);
        app.rack.set_positions_locked(false);
        app.apply_host_preferences(&preferences);
        assert!(app.rack.positions_locked());
        assert_eq!(app.rack.layout(), layout);
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
        for legacy in [
            json!({}),
            json!({"lock_module_positions":"true"}),
            Value::Null,
        ] {
            app.apply_host_preferences(&legacy);
            assert!(!app.rack.positions_locked());
        }
    }

    #[test]
    fn audio_timing_presentation_never_turns_missing_or_pending_data_into_zero_load() {
        assert_eq!(timing_status(None), TimingStatus::Unavailable);
        assert_eq!(timing_footer(None), "DSP —");
        let mut snapshot = AudioTimingSnapshot {
            available: true,
            coherent: true,
            sample_rate: 48_000,
            frames: 256,
            channels: 2,
            callbacks: 3,
            utilization: Some(0.16875),
            peak_utilization: Some(0.73),
            total_duration_ns: 500_000_000,
            total_frames: 48_000,
            ..AudioTimingSnapshot::default()
        };
        assert_eq!(timing_status(Some(&snapshot)), TimingStatus::Ready);
        assert_eq!(timing_footer(Some(&snapshot)), "DSP 16.9%");
        assert_eq!(weighted_mean_utilization(&snapshot), Some(0.5));
        snapshot.reset_pending = true;
        assert_eq!(timing_status(Some(&snapshot)), TimingStatus::ResetPending);
        assert_eq!(timing_footer(Some(&snapshot)), "DSP reset…");
        snapshot.reset_pending = false;
        snapshot.coherent = false;
        assert_eq!(timing_status(Some(&snapshot)), TimingStatus::Contended);
        assert_eq!(timing_footer(Some(&snapshot)), "DSP —");
        snapshot.coherent = true;
        snapshot.available = false;
        assert_eq!(timing_status(Some(&snapshot)), TimingStatus::Waiting);
        snapshot.available = true;
        snapshot.closed = true;
        assert_eq!(timing_status(Some(&snapshot)), TimingStatus::Stopped);
        assert_eq!(timing_footer(Some(&snapshot)), "DSP —");
    }

    #[test]
    fn audio_performance_window_shows_measured_budget_and_explicit_simulation_label() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.audio_performance_open = true;
        app.smoke_test = Some((dir.path().join("unused.png"), 0, 0));
        app.smoke_diagnostics = Some(AudioTimingSnapshot {
            available: true,
            coherent: true,
            sample_rate: 48_000,
            frames: 256,
            channels: 2,
            duration_ns: 900_000,
            callbacks: 320,
            total_duration_ns: 500_000_000,
            total_frames: 48_000,
            utilization: Some(0.16875),
            peak_utilization: Some(0.73),
            ..AudioTimingSnapshot::default()
        });
        let ctx = egui::Context::default();
        let render = |time: f64, app: &mut OpenPatch| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(940.0, 680.0),
                    )),
                    time: Some(time),
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    app.footer(ctx);
                    egui::CentralPanel::default().show(ctx, |_ui| {});
                    app.audio_performance(ctx);
                },
            )
        };
        render(0.0, &mut app);
        let output = render(1.0, &mut app);
        preset_text(&output, "SIMULATED DATA · SCREENSHOT FIXTURE");
        preset_text(&output, "Current budget use");
        preset_text(&output, "16.9%");
        preset_text(&output, "Weighted mean");
        preset_text(&output, "50.0%");
        preset_text(&output, "Processing budget overruns");
        preset_text(&output, "Reset statistics");
        assert!(app.audio_performance_open);
        assert!(!app.library_enabled());
    }

    #[test]
    fn minimum_window_preserves_rack_space_with_library_keyboard_and_either_dock_tab() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.title = "A very long user project name that must never hide transport or zoom controls"
            .repeat(4);
        app.status = "Long status information ".repeat(20);
        app.browser_open = true;
        app.keyboard_open = true;
        let ctx = egui::Context::default();
        ctx.data_mut(|data| {
            data.insert_persisted(
                egui::Id::new("analysis-dock"),
                egui::containers::panel::PanelState {
                    rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 500.0)),
                },
            );
            data.insert_persisted(
                egui::Id::new("module-library"),
                egui::containers::panel::PanelState {
                    rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(440.0, 1000.0)),
                },
            );
        });
        for (index, tab) in [AnalysisTab::Scope, AnalysisTab::Visuals]
            .into_iter()
            .enumerate()
        {
            app.select_analysis(Some(tab));
            for frame in 0..2 {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(940.0, 680.0),
                        )),
                        time: Some((index * 2 + frame) as f64),
                        focused: true,
                        ..Default::default()
                    },
                    |ctx| {
                        app.toolbar(ctx);
                        app.footer(ctx);
                        app.browser(ctx);
                        app.performance_keyboard(ctx);
                        app.scope.service(&app.patch, None);
                        app.analysis_dock(ctx);
                        let canvas = ctx.available_rect();
                        assert!(
                            canvas.height() >= 220.0,
                            "{tab:?} leaves only {} rack pixels",
                            canvas.height()
                        );
                        assert!(
                            canvas.width() >= 480.0,
                            "{tab:?} leaves only {} rack pixels",
                            canvas.width()
                        );
                        egui::CentralPanel::default().show(ctx, |_ui| {});
                    },
                );
            }
        }
    }

    #[test]
    fn docked_library_allows_rack_shortcuts_but_search_owns_keyboard_input() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        assert!(app.command(
            "replace_patch",
            json!({"patch":{"version":2,"nodes":[{"id":"a","kind":"constant"}]}})
        ));
        app.rack
            .restore_layout(&json!({"version":1,"zoom":1.0,"pan":[24,12],"positions":{"a":[0,0]}}))
            .unwrap();
        app.browse();
        let ctx = egui::Context::default();
        library_frame(&mut app, &ctx, 0.0, vec![], true);
        let header = egui::pos2(84.0, 44.0);
        library_frame(
            &mut app,
            &ctx,
            1.0,
            vec![
                egui::Event::PointerMoved(header),
                pointer_event(header, true),
            ],
            true,
        );
        library_frame(
            &mut app,
            &ctx,
            1.1,
            vec![pointer_event(header, false)],
            true,
        );
        let key = |pressed| egui::Event::Key {
            key: egui::Key::D,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        library_frame(&mut app, &ctx, 1.2, vec![key(true)], true);
        assert_eq!(
            app.patch.nodes.len(),
            2,
            "An open library must not disable rack duplication"
        );
        assert!(app.browser_open);
        library_frame(&mut app, &ctx, 1.3, vec![key(false)], true);
        app.library.request_focus();
        library_frame(&mut app, &ctx, 2.0, vec![], true);
        let snapshot = app.shared.lock().unwrap().snapshot();
        library_frame(&mut app, &ctx, 2.1, vec![key(true)], true);
        assert_eq!(
            app.shared.lock().unwrap().snapshot(),
            snapshot,
            "Search typing must not edit rack modules"
        );
    }

    #[test]
    fn library_favorites_share_host_preferences_without_changing_device_choices() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.output_device = Some("Chosen output".into());
        app.midi_device = Some("Chosen input".into());
        app.library.set_favorites(
            &["analog_vco".into(), "constant".into(), "unknown".into()],
            &app.catalog,
        );
        app.save_host_settings();
        let value: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join(".host-settings.json")).unwrap())
                .unwrap();
        assert_eq!(value["output_device"], "Chosen output");
        assert_eq!(value["midi_device"], "Chosen input");
        assert_eq!(value["module_favorites"], json!(["analog_vco", "constant"]));
    }

    #[test]
    fn browser_viz_add_select_delete_and_history_preserve_monitor_output() {
        let root = tempfile::tempdir().unwrap();
        let mut app = host(root.path());
        assert!(app.command("replace_patch", json!({"patch":{"version":2,"nodes":[]}})));
        assert!(app.add_module("poly_viz", None));
        assert_eq!(app.rack.selected.as_deref(), Some("poly_viz-1"));
        assert!(app.patch.output.is_none());
        assert!(app.command("undo", json!({})));
        assert!(app.patch.nodes.is_empty());
        assert!(app.command("redo", json!({})));
        assert!(app.patch.output.is_none());
        assert_eq!(app.patch.nodes[0].kind, "poly_viz");
        assert!(app.add_module("audio_out", None));
        assert_eq!(app.patch.output.as_deref(), Some("audio_out-1"));
        app.rack.select_only("poly_viz-1".into());
        let revision = app.revision;
        let snapshot = app.shared.lock().unwrap().snapshot();
        assert!(!app.command("set_output", json!({"id":"poly_viz-1","port":"out"})));
        assert_eq!(app.revision, revision);
        assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
        assert!(app.command("remove_module", json!({"id":"poly_viz-1"})));
        assert_eq!(app.patch.output.as_deref(), Some("audio_out-1"));
        assert!(app.command("undo", json!({})));
        assert!(app.patch.nodes.iter().any(|n| n.kind == "poly_viz"));
        assert_eq!(app.patch.output.as_deref(), Some("audio_out-1"));
        assert!(app.command("redo", json!({})));
        assert!(!app.patch.nodes.iter().any(|n| n.kind == "poly_viz"));
        assert_eq!(app.patch.output.as_deref(), Some("audio_out-1"));
    }

    #[test]
    fn browser_add_preserves_runtime_identities_and_sets_first_output_atomically() {
        use openpatch::control::LiveModuleEvents;
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        struct RetirementCounter(AtomicUsize);
        impl LiveModuleEvents for RetirementCounter {
            fn trigger(&self, _: &str, _: openpatch_core::ModuleAction) -> Result<Value, String> {
                Ok(json!({}))
            }
            fn snapshot(&self) -> Value {
                json!({})
            }
            fn retire_modules(&self, ids: &[String]) {
                self.0.fetch_add(ids.len(), Ordering::Relaxed);
            }
        }
        let root = tempfile::tempdir().unwrap();
        let mut app = host(root.path());
        let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[{"id":"seq","kind":"seq3"},{"id":"route","kind":"switch_1to4"}]})).unwrap();
        assert!(app.command("replace_patch", json!({"patch":patch})));
        let counter = Arc::new(RetirementCounter(AtomicUsize::new(0)));
        app.shared
            .lock()
            .unwrap()
            .set_live_module_events(Some(counter.clone()))
            .unwrap();
        let revision = app.revision;
        app.add_module("constant", None);
        assert_eq!(counter.0.load(Ordering::Relaxed), 0);
        assert_eq!(app.revision, revision + 1);
        assert_eq!(app.patch.nodes.len(), 3);
        assert_eq!(app.rack.selected.as_deref(), Some("constant-1"));
        app.add_module("audio_stereo", None);
        assert_eq!(app.patch.output.as_deref(), Some("audio_stereo-1"));
        assert_eq!(app.revision, revision + 2);
        assert_eq!(counter.0.load(Ordering::Relaxed), 0);
        assert!(app.command("undo", json!({})));
        assert!(app.patch.output.is_none());
        assert!(!app.patch.nodes.iter().any(|n| n.id == "audio_stereo-1"));
    }

    fn edit(app: &mut OpenPatch, value: f32) {
        assert!(app.command(
            "set_parameter",
            json!({"id":"tone","parameter":"frequency","value":value})
        ));
        app.capture_document();
    }
    fn make_autosave_due(app: &mut OpenPatch) {
        app.last_document_change = Instant::now() - Duration::from_secs(20);
        app.last_autosave_attempt = Instant::now() - Duration::from_secs(20);
        app.autosave_needed = true;
    }
    fn recovery_files(path: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(path)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with(".openpatch-recovery-")
                    && p.extension().is_some_and(|ext| ext == "openpatch")
            })
            .collect()
    }

    #[test]
    fn waveform_demo_modes_and_placement_survive_native_save_and_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/synced-waveform-voice.json"))
                .unwrap();
        app.replace_now(patch, "Synced waveform voice");
        let geometry = app.rack.layout();
        assert_eq!(app.rack.zoom(), 0.85);
        for (parameter, value) in [
            ("linear_fm", 1.0),
            ("soft_sync", 1.0),
            ("frequency", 523.25116),
            ("pulse_width", 0.35),
        ] {
            assert!(app.command(
                "set_parameter",
                json!({"id":"voice","parameter":parameter,"value":value})
            ));
        }
        app.save_document("waveforms.openpatch", true);
        let saved = app.capture_document();
        let loaded = app
            .projects
            .as_ref()
            .unwrap()
            .load("waveforms.openpatch")
            .unwrap();
        assert!(loaded.document.equivalent(&saved));
        assert_eq!(loaded.document.layout, geometry);
        assert_eq!(loaded.document.patch.cables.len(), 10);
        let voice = loaded
            .document
            .patch
            .nodes
            .iter()
            .find(|n| n.id == "voice")
            .unwrap();
        assert_eq!(voice.params["linear_fm"], 1.0);
        assert_eq!(voice.params["soft_sync"], 1.0);
        assert_eq!(voice.params["pulse_width"], 0.35);
        app.history(false);
        app.capture_document();
        assert!(app.dirty());
        assert_eq!(app.rack.layout(), geometry);
        app.history(true);
        assert!(app.capture_document().equivalent(&saved));
        assert!(!app.dirty());
    }

    #[test]
    fn explicit_cable_gesture_revision_is_preserved_after_app_syncs_a_newer_edit() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/stacked-pitch-voice.json"))
                .unwrap();
        app.replace_now(patch, "Stacked pitch voice");
        let gesture_revision = app.revision;
        execute_shared(
            &app.shared,
            "set_parameter",
            json!({"id":"transpose","parameter":"value","value":0.5}),
        )
        .unwrap();
        app.sync();
        assert!(app.revision > gesture_revision);
        let before = app.shared.lock().unwrap().snapshot();
        let layout = app.rack.layout();
        assert!(!app.command(
            "disconnect",
            json!({"cable_id":"depth-pitch","expected_revision":gesture_revision})
        ));
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
        assert_eq!(app.rack.layout(), layout);
    }

    #[test]
    fn stacked_demo_applies_visible_layout_and_saves_every_named_pitch_cable() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/stacked-pitch-voice.json"))
                .unwrap();
        app.replace_now(patch, "Stacked pitch voice");
        assert_eq!(app.rack.zoom(), 0.85);
        assert_eq!(app.rack.layout()["positions"]["vco"], json!([360.0, 0.0]));
        assert_eq!(
            app.patch
                .cables
                .iter()
                .filter(|c| c.to == "vco" && c.input == "pitch")
                .count(),
            3
        );
        app.save_document("stacked.openpatch", true);
        let loaded = app
            .projects
            .as_ref()
            .unwrap()
            .load("stacked.openpatch")
            .unwrap();
        assert!(loaded.document.equivalent(&app.capture_document()));
        assert!(!app.dirty());
    }

    fn duplication_patch() -> Patch {
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"tone","kind":"analog_vco","params":{"frequency":130.8128},"bypassed":true},{"id":"filter","kind":"ladder_vcf","params":{"cutoff":1200}},{"id":"pitch","kind":"constant","params":{"value":1}},{"id":"out","kind":"audio_out"}],"cables":[{"from":"pitch","to":"tone","input":"pitch"},{"from":"tone","to":"filter","input":"signal"},{"from":"filter","to":"out","input":"signal"}],"output":"out"})).unwrap()
    }

    #[test]
    fn duplicate_group_is_one_undo_step_with_geometry_inputs_and_unchanged_output() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        assert!(app.command("replace_patch", json!({"patch":duplication_patch()})));
        app.rack.restore_layout(&json!({"version":1,"zoom":1.0,"pan":[24,12],"positions":{"tone":[0,0],"filter":[210,0],"pitch":[0,420],"out":[210,420]}})).unwrap();
        let before = app.capture_document();
        let steps = app.shared.lock().unwrap().snapshot()["history"]["undo_steps"]
            .as_u64()
            .unwrap();
        assert!(app.duplicate_modules(
            &["tone".into(), "filter".into()],
            DuplicateCableMode::Inputs,
            app.revision,
            true,
        ));
        let duplicated = app.capture_document();
        assert_eq!(
            app.shared.lock().unwrap().snapshot()["history"]["undo_steps"],
            steps + 1
        );
        assert_eq!(app.patch.output, before.patch.output);
        assert_eq!(app.patch.output_port, before.patch.output_port);
        assert_eq!(app.patch.cables.len(), 5);
        assert!(
            app.patch
                .nodes
                .iter()
                .find(|n| n.id == "tone-copy-1")
                .unwrap()
                .bypassed
        );
        assert!(app
            .patch
            .cables
            .iter()
            .any(|c| c.from == "pitch" && c.to == "tone-copy-1" && c.input == "pitch"));
        assert!(!app
            .patch
            .cables
            .iter()
            .any(|c| c.from == "filter-copy-1" && c.to == "out"));
        assert_eq!(
            app.rack.selected_ids(),
            vec!["filter-copy-1", "tone-copy-1"]
        );
        app.history(false);
        assert!(app.capture_document().equivalent(&before));
        assert!(app.rack.selected_ids().is_empty());
        app.history(true);
        assert!(app.capture_document().equivalent(&duplicated));
        assert!(app.command("apply_edits",json!({"edits":[{"tool":"remove_module","arguments":{"id":"tone-copy-1"}},{"tool":"remove_module","arguments":{"id":"filter-copy-1"}}]})));
        app.capture_document();
        app.history(false);
        assert!(app.capture_document().equivalent(&duplicated));
    }

    #[test]
    fn duplicate_single_with_all_cables_preserves_other_selection_and_undo_is_one_step() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        assert!(app.command("replace_patch", json!({"patch":duplication_patch()})));
        app.rack.restore_layout(&json!({"version":1,"zoom":1.0,"pan":[24,12],"positions":{"tone":[0,0],"filter":[210,0],"pitch":[0,420],"out":[210,420]}})).unwrap();
        app.rack.select_only("pitch".into());
        let before = app.capture_document();
        let steps = app.shared.lock().unwrap().snapshot()["history"]["undo_steps"]
            .as_u64()
            .unwrap();
        assert!(app.duplicate_modules(
            &["filter".into()],
            DuplicateCableMode::All,
            app.revision,
            false,
        ));
        let after = app.capture_document();
        assert_eq!(app.rack.selected_ids(), vec!["pitch"]);
        assert_eq!(app.patch.output, before.patch.output);
        assert_eq!(app.patch.cables.len(), before.patch.cables.len() + 2);
        assert!(app.patch.cables.iter().any(|cable| cable.from == "tone"
            && cable.to == "filter-copy-1"
            && cable.input == "signal"));
        assert!(app
            .patch
            .cables
            .iter()
            .any(|cable| cable.from == "filter-copy-1"
                && cable.to == "out"
                && cable.input == "signal"));
        for id in ["tone", "filter", "pitch", "out"] {
            assert_eq!(
                after.layout["positions"][id],
                before.layout["positions"][id]
            );
        }
        assert_eq!(
            app.shared.lock().unwrap().snapshot()["history"]["undo_steps"],
            steps + 1
        );
        app.history(false);
        assert!(app.capture_document().equivalent(&before));
        app.history(true);
        assert!(app.capture_document().equivalent(&after));
    }

    #[test]
    fn duplicate_all_legacy_guard_allows_incoming_only_and_preserves_rejected_layout() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let patch: Patch = serde_json::from_value(json!({
            "version":1,"nodes":[
                {"id":"pitch","kind":"constant"},
                {"id":"tone","kind":"analog_vco"}
            ],
            "cables":[{"from":"pitch","to":"tone","input":"pitch"}]
        }))
        .unwrap();
        assert!(app.command("replace_patch", json!({"patch":patch})));
        app.rack.select_only("tone".into());
        let before = app.capture_document();
        assert!(!app.duplicate_modules(
            &["pitch".into()],
            DuplicateCableMode::All,
            app.revision,
            false,
        ));
        assert!(app.status.contains("engine v2"));
        assert_eq!(app.rack.layout(), before.layout);
        assert_eq!(app.rack.selected_ids(), vec!["tone"]);
        assert!(app.duplicate_modules(
            &["tone".into()],
            DuplicateCableMode::All,
            app.revision,
            false,
        ));
        assert!(app
            .patch
            .cables
            .iter()
            .any(|cable| cable.from == "pitch" && cable.to == "tone-copy-1"));
        assert_eq!(app.rack.selected_ids(), vec!["tone"]);
    }

    #[test]
    fn stale_duplicate_and_node_limit_failure_preserve_layout_selection_and_graph_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        assert!(app.command("replace_patch", json!({"patch":duplication_patch()})));
        app.capture_document();
        app.rack.select_only("tone".into());
        let layout = app.rack.layout();
        let selection = app.rack.selected_ids();
        execute_shared(
            &app.shared,
            "set_parameter",
            json!({"id":"pitch","parameter":"value","value":2}),
        )
        .unwrap();
        let snapshot = app.shared.lock().unwrap().snapshot();
        assert!(!app.duplicate_modules(
            &["tone".into()],
            DuplicateCableMode::None,
            app.revision,
            true,
        ));
        assert_eq!(app.rack.layout(), layout);
        assert_eq!(app.rack.selected_ids(), selection);
        assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
        app.sync();
        let full = Patch {
            nodes: (0..openpatch_core::MAX_NODES)
                .map(|i| openpatch_core::Node {
                    content: None,
                    score: None,
                    id: format!("node{i}"),
                    kind: "constant".into(),
                    params: Default::default(),
                    bypassed: false,
                })
                .collect(),
            ..Patch::new_v2()
        };
        assert!(app.command("replace_patch", json!({"patch":full})));
        app.capture_document();
        app.rack.select_only("node0".into());
        let layout = app.rack.layout();
        let snapshot = app.shared.lock().unwrap().snapshot();
        assert!(!app.duplicate_modules(
            &["node0".into()],
            DuplicateCableMode::None,
            app.revision,
            true,
        ));
        assert_eq!(app.rack.layout(), layout);
        assert_eq!(app.rack.selected_ids(), vec!["node0"]);
        assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
    }

    #[test]
    fn duplicate_ids_reserve_entire_batch_and_support_maximum_length_sources() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let long = "a".repeat(64);
        let sources = vec![
            long.clone(),
            "tone".into(),
            "tone-copy-1".into(),
            "tone_copy".into(),
            "tone_copy2".into(),
        ];
        let mut original = sources.clone();
        original.push("tone-copy-2".into());
        let patch = Patch {
            nodes: original
                .iter()
                .map(|id| openpatch_core::Node {
                    content: None,
                    score: None,
                    id: id.clone(),
                    kind: "constant".into(),
                    params: Default::default(),
                    bypassed: false,
                })
                .collect(),
            output: Some("tone".into()),
            ..Patch::new_v2()
        };
        assert!(app.command("replace_patch", json!({"patch":patch})));
        app.capture_document();
        assert!(app.duplicate_modules(&sources, DuplicateCableMode::None, app.revision, true));
        assert_eq!(app.patch.nodes.len(), 11);
        let ids: BTreeSet<_> = app.patch.nodes.iter().map(|n| &n.id).collect();
        assert_eq!(ids.len(), 11);
        assert!(ids.iter().all(|id| id.len() <= 64));
        assert!(ids.contains(&"tone-copy-3".to_string()));
        assert_eq!(app.patch.output.as_deref(), Some("tone"));
        assert!(app
            .rack
            .selected_ids()
            .iter()
            .all(|id| !original.contains(id)));
    }

    #[test]
    fn duplicate_receipt_records_exact_commit_even_when_newer_mcp_revision_holds_mutex() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        assert!(app.command("replace_patch", json!({"patch":duplication_patch()})));
        let before = app.capture_document();
        let base_state = app.state_id;
        let mappings = vec![("tone".into(), "copy".into())];
        let placement = app.rack.duplicate_placement(&app.patch, &mappings).unwrap();
        let receipt=execute_shared(&app.shared,"duplicate_modules",json!({"modules":[{"id":"tone","new_id":"copy"}],"cable_mode":"none","expected_revision":app.revision})).unwrap();
        let committed_state = receipt["state_id"].as_u64().unwrap();
        let shared = app.shared.clone();
        let mut live = shared.lock().unwrap();
        live.execute("remove_module", json!({"id":"copy"})).unwrap();
        let later_revision = live.revision();
        app.install_duplicate_receipt(&receipt, placement, base_state, before.layout.clone(), true)
            .unwrap();
        let committed = app.capture_document();
        assert_eq!(app.state_id, committed_state);
        assert_eq!(app.history_layouts[&base_state], before.layout);
        assert_eq!(app.history_layouts[&committed_state], committed.layout);
        assert!(app.patch.nodes.iter().any(|n| n.id == "copy"));
        app.sync();
        assert_eq!(
            app.state_id, committed_state,
            "Busy polling must not relabel the receipt"
        );
        drop(live);
        app.sync();
        assert_eq!(app.revision, later_revision);
        assert!(!app.patch.nodes.iter().any(|n| n.id == "copy"));
        assert!(app.rack.selected_ids().is_empty());
        app.history(false);
        assert!(app.capture_document().equivalent(&committed));
        app.history(false);
        assert!(app.capture_document().equivalent(&before));
    }

    #[test]
    fn layered_demo_replacement_applies_visible_layout_and_saves_connected_stereo_graph() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/layered-stereo-morph.json"))
                .unwrap();
        app.replace_now(patch, "Layered stereo morph");
        assert_eq!(app.rack.zoom(), 0.85);
        assert_eq!(app.rack.layout()["positions"]["morph"], json!([390.0, 0.0]));
        assert_eq!(app.patch.nodes.len(), 8);
        assert_eq!(app.patch.cables.len(), 11);
        app.save_document("morph.openpatch", true);
        let loaded = app
            .projects
            .as_ref()
            .unwrap()
            .load("morph.openpatch")
            .unwrap();
        assert!(loaded.document.equivalent(&app.capture_document()));
        assert!(!app.dirty());
    }

    #[test]
    fn held_voltage_examples_apply_nonoverlapping_layout_only_after_replacement() {
        for (title, source, layout) in [
            (
                "Annotated drum rack",
                include_str!("../../../examples/annotated-drum-rack.json"),
                OpenPatch::annotated_drums_layout(),
            ),
            (
                "MIDI drum pads",
                include_str!("../../../examples/midi-gate-drums.json"),
                OpenPatch::midi_gate_drums_layout(),
            ),
            (
                "MIDI controller voice",
                include_str!("../../../examples/midi-controller-voice.json"),
                OpenPatch::midi_cc_voice_layout(),
            ),
            (
                "Shift register canon",
                include_str!("../../../examples/shift-register-canon.json"),
                OpenPatch::shift_register_layout(),
            ),
            (
                "Random values voice",
                include_str!("../../../examples/random-values-voice.json"),
                OpenPatch::random_values_layout(),
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = host(dir.path());
            edit(&mut app, 330.0);
            let before = app.capture_document();
            let patch: Patch = serde_json::from_str(source).unwrap();
            app.replace_from_menu(patch.clone(), title);
            assert!(matches!(
                app.confirm_discard,
                Some(PendingAction::Replace(_, _))
            ));
            assert!(app.capture_document().equivalent(&before));
            app.confirm_discard = None;
            app.replace_now(patch, title);
            let mut expected_rack = Rack::default();
            expected_rack.restore_layout(&layout).unwrap();
            assert_eq!(app.rack.layout(), expected_rack.layout());
            for (i, a) in app.patch.nodes.iter().enumerate() {
                let a_pos = &layout["positions"][&a.id];
                let ax = a_pos[0].as_f64().unwrap();
                let ay = a_pos[1].as_f64().unwrap();
                for b in &app.patch.nodes[i + 1..] {
                    let b_pos = &layout["positions"][&b.id];
                    let bx = b_pos[0].as_f64().unwrap();
                    let by = b_pos[1].as_f64().unwrap();
                    assert!(
                        ax + f64::from(crate::rack::module_width(a)) <= bx
                            || bx + f64::from(crate::rack::module_width(b)) <= ax
                            || ay + 396.0 <= by
                            || by + 396.0 <= ay,
                        "{} overlaps {}",
                        a.id,
                        b.id
                    );
                }
            }
            app.save_document("example.openpatch", true);
            let saved = app
                .projects
                .as_ref()
                .unwrap()
                .load("example.openpatch")
                .unwrap();
            // Saving materializes the held-state defaults omitted by examples.
            assert!(saved
                .document
                .equivalent(&app.capture_persisted_document().unwrap()));
            assert!(!app.dirty());
        }
    }

    #[test]
    fn bypass_and_mute_edits_participate_in_dirty_undo_and_native_save() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/voltage-utilities.json")).unwrap();
        assert!(app.command("replace_patch", json!({"patch":patch})));
        app.filename = "utilities.openpatch".into();
        app.save_document("utilities.openpatch", true);
        assert!(!app.dirty());
        let before = app.capture_document();
        assert!(app.command("set_bypass", json!({"id":"mutes","bypassed":true})));
        app.capture_document();
        assert!(app.dirty());
        assert!(
            app.patch
                .nodes
                .iter()
                .find(|n| n.id == "mutes")
                .unwrap()
                .bypassed
        );
        app.history(false);
        assert!(app.capture_document().equivalent(&before));
        assert!(!app.dirty());
        app.history(true);
        assert!(
            app.patch
                .nodes
                .iter()
                .find(|n| n.id == "mutes")
                .unwrap()
                .bypassed
        );
        assert!(app.command("invert_mutes", json!({"id":"mutes"})));
        let mutes = app.patch.nodes.iter().find(|n| n.id == "mutes").unwrap();
        assert_eq!(mutes.params["mute10"], 1.0);
        app.save_current();
        let loaded = app
            .projects
            .as_ref()
            .unwrap()
            .load("utilities.openpatch")
            .unwrap();
        assert!(loaded.document.equivalent(&app.capture_document()));
    }

    #[test]
    fn save_dialog_focus_enter_and_cancel_preserve_the_close_continuation() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        edit(&mut app, 330.0);
        app.after_save = Some(PendingAction::Close);
        app.file_dialog(FileMode::Save);
        let ctx = egui::Context::default();
        let draw = |app: &mut OpenPatch, time: f64, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1100.0, 700.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| app.files_window(ctx),
            );
        };
        draw(&mut app, 0.0, vec![]);
        draw(
            &mut app,
            0.1,
            vec![egui::Event::Text("keyboard.openpatch".into())],
        );
        assert_eq!(app.filename, "keyboard.openpatch");
        draw(
            &mut app,
            0.2,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(dir.path().join("keyboard.openpatch").exists());
        assert!(app.allow_close);
        let mut app = host(dir.path());
        edit(&mut app, 440.0);
        app.after_save = Some(PendingAction::Close);
        app.file_dialog(FileMode::Save);
        draw(&mut app, 1.0, vec![]);
        draw(
            &mut app,
            1.1,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.file_mode.is_none());
        assert!(app.after_save.is_none());
        assert!(!app.allow_close);
        assert!(app.dirty());
    }

    #[test]
    fn fast_manual_click_enqueues_pair_under_one_session_lock_without_history() {
        use openpatch::control::LiveModuleEvents;
        use std::sync::{Arc, Mutex};
        struct ClickBackend {
            shared: openpatch::control::SharedSession,
            events: Mutex<Vec<openpatch_core::ModuleAction>>,
            reject_press: std::sync::atomic::AtomicBool,
        }
        impl LiveModuleEvents for ClickBackend {
            fn trigger(
                &self,
                _id: &str,
                action: openpatch_core::ModuleAction,
            ) -> Result<Value, String> {
                assert!(
                    self.shared.try_lock().is_err(),
                    "incarnation is pinned across both actions"
                );
                if self.reject_press.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err("injected queue full".into());
                }
                self.events.lock().unwrap().push(action);
                Ok(json!({"queued":true,"action":action}))
            }
            fn snapshot(&self) -> Value {
                json!({})
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        assert!(app.command(
            "replace_patch",
            json!({"patch":{"version":2,"nodes":[{"id":"buttons","kind":"pulses"}]}})
        ));
        let backend = Arc::new(ClickBackend {
            shared: app.shared.clone(),
            events: Mutex::new(Vec::new()),
            reject_press: std::sync::atomic::AtomicBool::new(false),
        });
        app.shared
            .lock()
            .unwrap()
            .set_live_module_events(Some(backend.clone()))
            .unwrap();
        let before = app.shared.lock().unwrap().snapshot();
        assert!(app.manual_click("buttons", "pulse10_press", "pulse10_release"));
        assert_eq!(
            *backend.events.lock().unwrap(),
            vec![
                openpatch_core::ModuleAction::PulsePress(10),
                openpatch_core::ModuleAction::PulseRelease(10)
            ]
        );
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
        backend
            .reject_press
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(!app.manual_click("buttons", "pulse1_press", "pulse1_release"));
        assert_eq!(backend.events.lock().unwrap().len(), 2);
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
        // Break the test backend's intentional Session reference cycle.
        app.shared
            .lock()
            .unwrap()
            .set_live_module_events(None)
            .unwrap();
    }

    #[test]
    fn random_values_host_native_save_and_recovery_track_triggered_values_and_rng() {
        use openpatch_core::{ModuleAction, SavedModuleState};
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        assert!(app.command(
            "replace_patch",
            json!({"patch":{"version":2,"nodes":[
            {"id":"trigger","kind":"push"},
            {"id":"random","kind":"random_values","params":{"seed":42,"range":3}}
        ],"cables":[{"from":"trigger","output":"gate","to":"random","input":"trig"}]}})
        ));
        app.save_document("random.openpatch", true);
        assert!(!app.dirty());
        let mut engine = app.patch.compile().unwrap();
        let source = engine.resolve_module_event("trigger").unwrap();
        engine.process_block(&mut [0.0; 2]);
        assert!(engine.handle_module_event(source, ModuleAction::PushPress));
        engine.process_block(&mut [0.0; 2]);
        assert!(engine.handle_module_event(source, ModuleAction::PushRelease));
        engine.process_block(&mut [0.0; 2]);
        let saved = engine
            .module_saved_state(engine.resolve_module_state("random").unwrap())
            .unwrap();
        let SavedModuleState::RandomValues { values, rng } = saved else {
            unreachable!()
        };
        assert_ne!(
            Some(saved),
            SavedModuleState::from_node(&app.patch.nodes[1])
        );
        let telemetry = std::sync::Arc::new(ProjectTelemetry(std::sync::Mutex::new(json!({
            "coherent":true,"modules":{
                "trigger":{"kind":"push","available":true,"revision":app.revision,"frame":128,"state":{"holding":false}},
                "random":{"kind":"random_values","available":true,"revision":app.revision,"frame":128,
                    "state":{"values":values.map(|row| row.map(f32::from_bits)),"rng_state":rng,"active_channels":1}}
            }
        }))));
        app.shared
            .lock()
            .unwrap()
            .set_live_module_events(Some(telemetry.clone()))
            .unwrap();
        let raw = app.shared.lock().unwrap().snapshot();
        app.observe_document();
        assert!(
            app.dirty(),
            "an electrical trigger changes persisted content"
        );
        make_autosave_due(&mut app);
        app.autosave();
        let files = recovery_files(dir.path());
        assert_eq!(files.len(), 1);
        std::fs::copy(&files[0], dir.path().join("recovery-copy.openpatch")).unwrap();
        let recovered = app
            .projects
            .as_ref()
            .unwrap()
            .load("recovery-copy.openpatch")
            .unwrap()
            .document;
        assert_eq!(
            SavedModuleState::from_node(&recovered.patch.nodes[1]),
            Some(saved)
        );
        app.save_current();
        assert!(!app.dirty());
        let loaded = app
            .projects
            .as_ref()
            .unwrap()
            .load("random.openpatch")
            .unwrap()
            .document;
        assert!(loaded.equivalent(&recovered));
        assert_eq!(app.shared.lock().unwrap().snapshot(), raw);
        // PRNG position alone is saved content, even if every voltage is unchanged.
        telemetry.0.lock().unwrap()["modules"]["random"]["state"]["rng_state"] = json!(rng ^ 1);
        app.observe_document();
        assert!(app.dirty());
        // A transient read failure cannot silently mark the project clean or save seed-only state.
        telemetry.0.lock().unwrap()["coherent"] = json!(false);
        app.save_current();
        assert!(app.dirty());
        assert!(app.status.contains("Save needs current live state"));
        let unchanged = app
            .projects
            .as_ref()
            .unwrap()
            .load("random.openpatch")
            .unwrap()
            .document;
        assert!(loaded.equivalent(&unchanged));
    }

    struct ProjectTelemetry(std::sync::Mutex<Value>);
    impl openpatch::control::LiveModuleEvents for ProjectTelemetry {
        fn snapshot(&self) -> Value {
            self.0.lock().unwrap().clone()
        }
        fn trigger(&self, _: &str, _: openpatch_core::ModuleAction) -> Result<Value, String> {
            panic!("project capture must never issue runtime actions")
        }
    }
    fn sequencer_host(path: &Path) -> (OpenPatch, std::sync::Arc<ProjectTelemetry>) {
        let mut app = host(path);
        assert!(app.command(
            "replace_patch",
            json!({"patch":{"version":2,"nodes":[
                {"id":"a","kind":"seq3","params":{"running":1}},
                {"id":"b","kind":"seq3","params":{"running":0}}
            ]}})
        ));
        app.save_document("sequence.openpatch", true);
        assert!(!app.dirty());
        let telemetry = std::sync::Arc::new(ProjectTelemetry(std::sync::Mutex::new(json!({
            "coherent":true,"modules":{
                "a":{"available":true,"revision":app.revision,"frame":128,"state":{"running":true,"step":3}},
                "b":{"available":true,"revision":app.revision,"frame":128,"state":{"running":false,"step":6}}
            }
        }))));
        app.shared
            .lock()
            .unwrap()
            .set_live_module_events(Some(telemetry.clone()))
            .unwrap();
        (app, telemetry)
    }
    fn set_project_run(telemetry: &ProjectTelemetry, a: bool, b: bool, frame: u64) {
        let mut value = telemetry.0.lock().unwrap();
        for (id, running) in [("a", a), ("b", b)] {
            value["modules"][id]["state"]["running"] = json!(running);
            value["modules"][id]["frame"] = json!(frame);
            value["modules"][id]["state"]["step"] = json!(frame % 8);
        }
    }

    fn push_host(path: &Path) -> (OpenPatch, std::sync::Arc<ProjectTelemetry>) {
        let mut app = host(path);
        assert!(app.command(
            "replace_patch",
            json!({"patch":{"version":2,"nodes":[
                {"id":"button","kind":"push","params":{"holding":0}}
            ]}})
        ));
        app.save_document("push.openpatch", true);
        assert!(!app.dirty());
        let telemetry = std::sync::Arc::new(ProjectTelemetry(std::sync::Mutex::new(json!({
            "coherent":true,"modules":{
                "button":{"kind":"push","available":true,"revision":app.revision,
                          "frame":128,"state":{"holding":false}}
            }
        }))));
        app.shared
            .lock()
            .unwrap()
            .set_live_module_events(Some(telemetry.clone()))
            .unwrap();
        (app, telemetry)
    }

    fn set_project_hold(telemetry: &ProjectTelemetry, holding: bool, frame: u64) {
        let mut value = telemetry.0.lock().unwrap();
        value["modules"]["button"]["state"]["holding"] = json!(holding);
        value["modules"]["button"]["frame"] = json!(frame);
    }

    #[test]
    fn live_hold_alone_marks_dirty_and_saves_actual_latch_to_recovery_and_project() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, telemetry) = push_host(dir.path());
        let raw_graph = app.shared.lock().unwrap().snapshot();
        let revision = app.revision;
        app.observe_document();
        assert!(!app.dirty());
        set_project_hold(&telemetry, false, 256);
        app.observe_document();
        assert!(!app.dirty(), "callback frame alone is not saved content");
        set_project_hold(&telemetry, true, 256);
        app.observe_document();
        assert!(app.dirty(), "HOLD alone changes saved content");
        assert_eq!(app.revision, revision);
        make_autosave_due(&mut app);
        app.autosave();
        let files = recovery_files(dir.path());
        assert_eq!(files.len(), 1);
        std::fs::copy(&files[0], dir.path().join("recovery-copy.openpatch")).unwrap();
        let recovery = app
            .projects
            .as_ref()
            .unwrap()
            .load("recovery-copy.openpatch")
            .unwrap()
            .document;
        assert_eq!(recovery.patch.nodes[0].params["holding"], 1.0);
        app.save_current();
        assert!(!app.dirty());
        let saved = app
            .projects
            .as_ref()
            .unwrap()
            .load("push.openpatch")
            .unwrap()
            .document;
        assert!(saved.equivalent(&recovery));
        assert_eq!(app.shared.lock().unwrap().snapshot(), raw_graph);
        assert_eq!(app.revision, revision);
        assert!(recovery_files(dir.path()).is_empty());
        set_project_hold(&telemetry, false, 512);
        app.observe_document();
        assert!(
            app.dirty(),
            "same revision but changed live HOLD invalidates cached baseline"
        );
        set_project_hold(&telemetry, true, 512);
        app.observe_document();
        assert!(!app.dirty());
    }

    #[test]
    fn unavailable_or_missing_push_state_blocks_save_and_keeps_recovery_and_close_guard() {
        for missing in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let (mut app, telemetry) = push_host(dir.path());
            set_project_hold(&telemetry, true, 256);
            app.observe_document();
            make_autosave_due(&mut app);
            app.autosave();
            let files = recovery_files(dir.path());
            assert_eq!(files.len(), 1);
            let recovery_bytes = std::fs::read(&files[0]).unwrap();
            let saved_bytes = std::fs::read(dir.path().join("push.openpatch")).unwrap();
            if missing {
                telemetry.0.lock().unwrap()["modules"] = json!({});
            } else {
                telemetry.0.lock().unwrap()["modules"]["button"]["available"] = json!(false);
            }
            app.observe_document();
            assert!(app.dirty());
            app.save_current();
            assert!(app.dirty());
            assert!(app.status.contains("live state"));
            make_autosave_due(&mut app);
            app.autosave();
            assert_eq!(std::fs::read(&files[0]).unwrap(), recovery_bytes);
            assert_eq!(
                std::fs::read(dir.path().join("push.openpatch")).unwrap(),
                saved_bytes
            );
            app.request_action(PendingAction::Close);
            assert!(!app.allow_close);
            assert!(matches!(app.confirm_discard, Some(PendingAction::Close)));
        }
    }

    #[test]
    fn final_clean_close_rechecks_hold_even_without_a_graph_revision_change() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, telemetry) = push_host(dir.path());
        app.observe_document();
        assert!(!app.dirty());
        let revision = app.revision;
        set_project_hold(&telemetry, true, 256);
        app.close_if_saved();
        assert!(!app.allow_close);
        assert!(matches!(app.confirm_discard, Some(PendingAction::Close)));
        assert_eq!(app.revision, revision);
        app.confirm_discard = None;
        set_project_hold(&telemetry, false, 512);
        app.request_action(PendingAction::Close);
        assert!(app.allow_close, "frame change with saved HOLD is clean");
        assert!(app.confirm_discard.is_none());
    }

    #[test]
    fn hold_save_failure_requires_another_close_decision_and_explicit_discard_clears_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, telemetry) = push_host(dir.path());
        set_project_hold(&telemetry, true, 256);
        app.observe_document();
        make_autosave_due(&mut app);
        app.autosave();
        assert_eq!(recovery_files(dir.path()).len(), 1);
        let bytes = std::fs::read(dir.path().join("push.openpatch")).unwrap();
        app.after_save = Some(PendingAction::Close);
        crate::project::fail_next_publish();
        app.save_current();
        assert!(!app.allow_close);
        assert!(matches!(app.confirm_discard, Some(PendingAction::Close)));
        assert!(app.after_save.is_none());
        assert!(app.dirty());
        assert_eq!(
            std::fs::read(dir.path().join("push.openpatch")).unwrap(),
            bytes
        );
        app.request_action(PendingAction::DiscardClose);
        assert!(app.allow_close);
        assert!(recovery_files(dir.path()).is_empty());
    }

    #[test]
    fn mixed_live_run_and_hold_are_saved_from_one_coherent_frame() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        assert!(app.command(
            "replace_patch",
            json!({"patch":{"version":2,"nodes":[
                {"id":"seq","kind":"seq3","params":{"running":1}},
                {"id":"button","kind":"push","params":{"holding":0}}
            ]}})
        ));
        app.save_document("mixed.openpatch", true);
        let telemetry = std::sync::Arc::new(ProjectTelemetry(std::sync::Mutex::new(json!({
            "coherent":true,"modules":{
                "seq":{"kind":"seq3","available":true,"revision":app.revision,"frame":256,"state":{"running":false}},
                "button":{"kind":"push","available":true,"revision":app.revision,"frame":256,"state":{"holding":true}}
            }
        }))));
        app.shared
            .lock()
            .unwrap()
            .set_live_module_events(Some(telemetry.clone()))
            .unwrap();
        app.observe_document();
        assert!(app.dirty());
        app.save_current();
        assert!(!app.dirty());
        let saved = app
            .projects
            .as_ref()
            .unwrap()
            .load("mixed.openpatch")
            .unwrap()
            .document;
        assert_eq!(saved.patch.nodes[0].params["running"], 0.0);
        assert_eq!(saved.patch.nodes[1].params["holding"], 1.0);
        telemetry.0.lock().unwrap()["modules"]["button"]["frame"] = json!(512);
        assert!(app.capture_persisted_document().is_err());
        let still_saved = app
            .projects
            .as_ref()
            .unwrap()
            .load("mixed.openpatch")
            .unwrap()
            .document;
        assert!(still_saved.equivalent(&saved));
    }

    #[test]
    fn live_run_save_and_recovery_roundtrip_without_editing_graph_or_history() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, telemetry) = sequencer_host(dir.path());
        let before = app.shared.lock().unwrap().snapshot();
        app.observe_document();
        assert!(!app.dirty());
        set_project_run(&telemetry, true, false, 257);
        app.observe_document();
        assert!(!app.dirty(), "steps and frames are not project edits");
        set_project_run(&telemetry, false, true, 512);
        app.observe_document();
        assert!(app.dirty(), "Run alone changes saved content");
        make_autosave_due(&mut app);
        app.autosave();
        let files = recovery_files(dir.path());
        assert_eq!(files.len(), 1);
        std::fs::copy(&files[0], dir.path().join("recovery-copy.openpatch")).unwrap();
        let recovery = app
            .projects
            .as_ref()
            .unwrap()
            .load("recovery-copy.openpatch")
            .unwrap()
            .document;
        assert_eq!(recovery.patch.nodes[0].params["running"], 0.0);
        assert_eq!(recovery.patch.nodes[1].params["running"], 1.0);
        app.save_current();
        assert!(!app.dirty());
        let saved = app
            .projects
            .as_ref()
            .unwrap()
            .load("sequence.openpatch")
            .unwrap()
            .document;
        assert!(saved.equivalent(&recovery));
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
        assert!(recovery_files(dir.path()).is_empty());
        // Reload with no device resets to the saved current Run values.
        app.shared
            .lock()
            .unwrap()
            .set_live_module_events(None)
            .unwrap();
        app.load_document("sequence.openpatch", false);
        assert!(!app.dirty());
        assert_eq!(app.patch.nodes[0].params["running"], 0.0);
        assert_eq!(app.patch.nodes[1].params["running"], 1.0);
    }

    #[test]
    fn live_run_change_during_publication_preserves_exact_bytes_and_blocks_close() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, telemetry) = sequencer_host(dir.path());
        set_project_run(&telemetry, false, true, 256);
        let captured = app.capture_persisted_document().unwrap();
        crate::project::before_next_publish(move || set_project_run(&telemetry, true, true, 512));
        app.after_save = Some(PendingAction::Close);
        app.save_current();
        let saved = app
            .projects
            .as_ref()
            .unwrap()
            .load("sequence.openpatch")
            .unwrap()
            .document;
        assert!(saved.equivalent(&captured));
        assert!(app.dirty());
        assert!(!app.allow_close);
        assert!(matches!(app.confirm_discard, Some(PendingAction::Close)));
        assert!(app.after_save.is_none());
    }

    #[test]
    fn unavailable_live_state_preserves_file_recovery_and_allows_explicit_discard() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, telemetry) = sequencer_host(dir.path());
        set_project_run(&telemetry, false, true, 256);
        app.observe_document();
        make_autosave_due(&mut app);
        app.autosave();
        let files = recovery_files(dir.path());
        let recovery = std::fs::read(&files[0]).unwrap();
        let bytes = std::fs::read(dir.path().join("sequence.openpatch")).unwrap();
        *telemetry.0.lock().unwrap() = json!({"coherent":false});
        app.observe_document();
        app.save_current();
        assert!(app.dirty());
        assert!(app.status.contains("live state"));
        make_autosave_due(&mut app);
        app.autosave();
        assert_eq!(std::fs::read(&files[0]).unwrap(), recovery);
        assert_eq!(
            std::fs::read(dir.path().join("sequence.openpatch")).unwrap(),
            bytes
        );
        app.request_action(PendingAction::Close);
        assert!(!app.allow_close);
        assert!(app.confirm_discard.is_some());
        let shared = app.shared.clone();
        let guard = shared.lock().unwrap();
        let action = app.confirm_discard.take().unwrap();
        app.perform_action(action);
        assert!(matches!(
            app.deferred_action,
            Some(PendingAction::DiscardClose)
        ));
        drop(guard);
        let pending = app.deferred_action.take().unwrap();
        app.request_action(pending);
        assert!(app.allow_close);
        assert!(app.confirm_discard.is_none());
        assert!(recovery_files(dir.path()).is_empty());
    }

    #[test]
    fn final_clean_close_rechecks_run_but_ignores_frame_changes() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, telemetry) = sequencer_host(dir.path());
        app.observe_document();
        assert!(!app.dirty());
        set_project_run(&telemetry, false, false, 256);
        app.close_if_saved();
        assert!(!app.allow_close);
        assert!(app.confirm_discard.is_some());
        app.confirm_discard = None;
        set_project_run(&telemetry, true, false, 512);
        app.request_action(PendingAction::Close);
        assert!(app.allow_close);
        assert!(app.confirm_discard.is_none());
    }

    #[test]
    fn final_replacement_guard_rechecks_run_before_loading_or_starting_new() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, telemetry) = sequencer_host(dir.path());
        app.observe_document();
        assert!(!app.dirty());
        let before = app.shared.lock().unwrap().snapshot();
        set_project_run(&telemetry, false, true, 256);
        app.load_document_guarded("sequence.openpatch", false, true);
        assert!(matches!(app.confirm_discard, Some(PendingAction::Load(_))));
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
        app.confirm_discard = None;
        app.replace_now_guarded(openpatch_core::demo_patch(), "New", true);
        assert!(matches!(
            app.confirm_discard,
            Some(PendingAction::Replace(_, _))
        ));
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
        app.confirm_discard = None;
        app.replace_now(openpatch_core::demo_patch(), "New");
        assert_ne!(app.shared.lock().unwrap().snapshot(), before);
    }

    #[test]
    fn failed_save_continuation_stays_visible_and_cancel_prevents_later_close() {
        for unavailable in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let (mut app, telemetry) = sequencer_host(dir.path());
            set_project_run(&telemetry, false, true, 256);
            app.after_save = Some(PendingAction::Close);
            if unavailable {
                *telemetry.0.lock().unwrap() = json!({});
            } else {
                crate::project::fail_next_publish();
            }
            app.save_current();
            assert!(app.after_save.is_none());
            assert!(matches!(app.confirm_discard, Some(PendingAction::Close)));
            let ctx = egui::Context::default();
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ctx| app.unsaved_window(ctx),
            );
            assert!(app.confirm_discard.is_none());
            assert!(app.deferred_action.is_none());
            app.shared
                .lock()
                .unwrap()
                .set_live_module_events(None)
                .unwrap();
            app.save_current();
            assert!(!app.allow_close);
            assert!(!app.dirty());
        }
    }

    #[test]
    fn failed_live_save_keeps_saved_baseline_file_token_and_runtime_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, telemetry) = sequencer_host(dir.path());
        let path = dir.path().join("sequence.openpatch");
        let bytes = std::fs::read(&path).unwrap();
        let token = format!("{:?}", app.active_token.as_ref().unwrap());
        set_project_run(&telemetry, false, true, 256);
        let live = telemetry.0.lock().unwrap().clone();
        let graph = app.shared.lock().unwrap().snapshot();
        crate::project::fail_next_publish();
        app.save_current();
        assert!(app.status.contains("injected failure"));
        assert!(app.dirty());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert_eq!(format!("{:?}", app.active_token.as_ref().unwrap()), token);
        assert_eq!(*telemetry.0.lock().unwrap(), live);
        assert_eq!(app.shared.lock().unwrap().snapshot(), graph);
        set_project_run(&telemetry, true, false, 512);
        app.observe_document();
        assert!(!app.dirty(), "failed save did not change the baseline");
    }

    #[test]
    fn save_current_roundtrip_revert_and_conflicts_preserve_the_working_project() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.save_document("voice.openpatch", true);
        assert!(!app.dirty());
        let original = std::fs::read(dir.path().join("voice.openpatch")).unwrap();
        edit(&mut app, 330.0);
        app.rack.set_zoom(0.75);
        app.save_current();
        assert!(!app.dirty());
        let saved = app
            .projects
            .as_ref()
            .unwrap()
            .load("voice.openpatch")
            .unwrap();
        assert_eq!(saved.document.patch.nodes[0].params["frequency"], 330.0);
        assert_eq!(saved.document.layout["zoom"], 0.75);
        edit(&mut app, 440.0);
        app.load_document("voice.openpatch", false);
        assert!(!app.dirty());
        assert_eq!(app.patch.nodes[0].params["frequency"], 330.0);
        edit(&mut app, 550.0);
        std::fs::write(dir.path().join("voice.openpatch"), &original).unwrap();
        app.save_current();
        assert!(app.dirty());
        assert!(app.status.contains("changed on disk"));
        assert_eq!(
            std::fs::read(dir.path().join("voice.openpatch")).unwrap(),
            original
        );
        assert_eq!(app.patch.nodes[0].params["frequency"], 550.0);
        assert_eq!(app.active_file.as_deref(), Some("voice.openpatch"));
        let before = app.capture_document();
        std::fs::write(dir.path().join("bad.openpatch"), b"{}").unwrap();
        app.load_document("bad.openpatch", false);
        assert!(app.capture_document().equivalent(&before));
        assert_eq!(app.active_file.as_deref(), Some("voice.openpatch"));
    }

    #[test]
    fn mcp_edit_during_save_keeps_matching_disk_layout_and_newer_work_unsaved() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let captured = app.capture_document();
        let shared = app.shared.clone();
        crate::project::before_next_publish(move || {
            execute_shared(
                &shared,
                "add_module",
                json!({"id":"agent-added","kind":"constant"}),
            )
            .unwrap();
        });
        app.after_save = Some(PendingAction::Close);
        app.save_document("snapshot.openpatch", true);
        let on_disk = app
            .projects
            .as_ref()
            .unwrap()
            .load("snapshot.openpatch")
            .unwrap();
        assert!(on_disk.document.equivalent(&captured));
        assert!(app.patch.nodes.iter().any(|n| n.id == "agent-added"));
        assert!(app.rack.layout()["positions"].get("agent-added").is_some());
        assert!(app.dirty());
        assert!(!app.allow_close);
        assert!(matches!(app.confirm_discard, Some(PendingAction::Close)));
    }

    #[test]
    fn discard_close_does_not_recreate_recovery_and_rejects_late_mcp_edits() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        edit(&mut app, 330.0);
        make_autosave_due(&mut app);
        app.autosave();
        assert_eq!(recovery_files(dir.path()).len(), 1);
        app.perform_action(PendingAction::Close);
        make_autosave_due(&mut app);
        app.autosave();
        assert!(app.allow_close);
        assert!(recovery_files(dir.path()).is_empty());
        assert!(execute_shared(
            &app.shared,
            "add_module",
            json!({"id":"late","kind":"constant"})
        )
        .unwrap_err()
        .contains("closing"));
    }

    #[test]
    fn stale_or_busy_clean_close_defers_until_newer_mcp_edits_are_observed() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let shared = app.shared.clone();
        let mut guard = shared.lock().unwrap();
        app.request_action(PendingAction::Close);
        assert!(!app.allow_close);
        assert!(app.deferred_action.is_some());
        guard
            .execute("add_module", json!({"id":"latest","kind":"constant"}))
            .unwrap();
        drop(guard);
        let action = app.deferred_action.take().unwrap();
        app.request_action(action);
        assert!(!app.allow_close);
        assert!(app.dirty());
        assert!(app.confirm_discard.is_some());
        assert!(app.patch.nodes.iter().any(|n| n.id == "latest"));
    }

    #[test]
    fn close_confirmation_pauses_construction_and_dismissal_keeps_it_paused() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        edit(&mut app, 330.0);
        let plan = execute_shared(&app.shared, "start_construction", json!({
            "expected_revision": app.revision,
            "interval_ms": 2000,
            "steps": [{"label": "Add a source", "edits": [
                {"tool": "add_module", "arguments": {"id": "construction_source", "kind": "constant"}}
            ]}]
        })).unwrap();
        // Close intentionally defers while the construction worker owns the
        // session lock. Exercise that path explicitly instead of assuming the
        // request acquires both nonblocking locks during a loaded test run.
        let shared = app.shared.clone();
        let guard = shared.lock().unwrap();
        app.request_action(PendingAction::Close);
        assert!(app.deferred_action.is_some());
        assert!(!app.allow_close);
        drop(guard);
        let deadline = Instant::now() + Duration::from_secs(5);
        while let Some(action) = app.deferred_action.take() {
            assert!(Instant::now() < deadline, "Close remained deferred");
            app.request_action(action);
            if app.deferred_action.is_some() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        assert!(app.confirm_discard.is_some());
        assert!(!app.allow_close);
        assert_eq!(
            app.shared.lock().unwrap().construction_snapshot()["status"],
            "paused"
        );
        // Cancelling the document dialog must not implicitly resume agent work.
        app.confirm_discard = None;
        app.sync();
        assert_eq!(
            app.shared.lock().unwrap().construction_snapshot()["status"],
            "paused"
        );
        execute_shared(
            &app.shared,
            "cancel_construction",
            json!({"plan_id": plan["plan_id"]}),
        )
        .unwrap();
    }

    #[test]
    fn room_capture_uses_real_offline_dsp_and_keeps_the_same_patch_across_lighting() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.prepare_room_lighting_fixture();
        assert_eq!(app.patch.nodes.len(), 8);
        assert_eq!(app.patch.cables.len(), 4);
        assert!(app.browser_open);
        let state = app.shared.lock().unwrap().snapshot();
        let layout = app.rack.layout();
        for amount in [1.0, 0.1, 0.0] {
            app.rack.set_room_brightness(amount);
            assert_eq!(app.rack.room_brightness(), amount);
            assert_eq!(app.shared.lock().unwrap().snapshot(), state);
            assert_eq!(app.rack.layout(), layout);
        }
    }

    #[test]
    fn actual_construction_receipts_drive_cues_without_changing_final_history_or_view() {
        use crate::construction_motion::MotionMode;
        let mut results = Vec::new();
        for mode in [MotionMode::Full, MotionMode::Off] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = host(dir.path());
            app.construction.set_motion_mode(mode);
            let origin = Instant::now();
            let receipts = app.prepare_motion_fixture(origin);
            for (index, receipt) in receipts.iter().enumerate() {
                assert_eq!(receipt["step_index"], index);
                assert_eq!(
                    receipt["revision"].as_u64().unwrap(),
                    receipt["before_revision"].as_u64().unwrap() + 1
                );
            }
            let mid = origin + Duration::from_millis(180);
            assert_eq!(
                app.rack.construction_effect_counts_at(mid),
                if mode == MotionMode::Full {
                    (1, 2, 2)
                } else {
                    (0, 0, 0)
                }
            );
            let snapshot = app.shared.lock().unwrap().snapshot();
            let layout = app.rack.layout();
            assert_eq!(
                app.rack
                    .construction_effect_counts_at(origin + Duration::from_millis(450)),
                (0, 0, 0)
            );
            app.sync();
            assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
            assert_eq!(app.rack.layout(), layout);
            results.push((snapshot, layout));
        }
        assert_eq!(results[0], results[1]);
    }

    #[test]
    fn a_new_construction_plan_clears_prior_motion_before_any_new_revision() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let origin = Instant::now();
        app.prepare_motion_fixture(origin);
        assert_eq!(app.rack.construction_effect_counts_at(origin), (1, 2, 2));
        let revision = app.revision;
        let plan = execute_shared(&app.shared, "start_construction", json!({"expected_revision":revision,"interval_ms":2000,
            "steps":[{"label":"A new plan","edits":[{"tool":"set_parameter","arguments":{"id":"voice","parameter":"frequency","value":330}}]}]})).unwrap();
        // The construction worker can briefly own Session after returning its
        // receipt. Production sync intentionally uses try_lock and retries on
        // the next UI frame; the test must allow that same publication retry.
        let deadline = Instant::now() + Duration::from_secs(1);
        while app.rack.construction_effect_counts_at(origin) != (0, 0, 0) {
            assert!(
                Instant::now() < deadline,
                "New plan presentation was not observed"
            );
            app.sync();
            std::thread::yield_now();
        }
        assert_eq!(app.revision, revision);
        assert_eq!(app.rack.construction_effect_counts_at(origin), (0, 0, 0));
        execute_shared(
            &app.shared,
            "cancel_construction",
            json!({"plan_id":plan["plan_id"]}),
        )
        .unwrap();
    }

    #[test]
    fn mcp_connection_window_owns_shortcuts_and_blocks_rack_gestures() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.mcp_session.open();
        assert!(!app.library_enabled());
        let before = app.shared.lock().unwrap().snapshot();
        let layout = app.rack.layout();
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                focused: true,
                modifiers: egui::Modifiers::COMMAND,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(940.0, 680.0),
                )),
                events: vec![performance_key(
                    egui::Key::Z,
                    true,
                    egui::Modifiers::COMMAND,
                )],
                ..Default::default()
            },
            |ctx| {
                app.global_shortcuts(ctx);
                assert!(!app.performance_keyboard_enabled(ctx, true));
                app.rack_workspace(ctx, None);
                app.mcp_session
                    .show(ctx, &app.shared, Path::new(&app.workspace));
            },
        );
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
        assert_eq!(app.rack.layout(), layout);
    }

    #[test]
    fn human_receipt_installation_clears_construction_reveals_even_without_sync_revision_change() {
        for action in ["add", "duplicate", "move", "replace"] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = host(dir.path());
            let cable = app.patch.cables[0].clone();
            let mut before = app.patch.clone();
            before.cables.clear();
            let now = Instant::now();
            app.rack.observe_construction_transition(
                &before,
                &app.patch,
                app.revision - 1,
                app.revision,
                &json!({"plan_id":1,"completed_steps":1,"last_revision":app.revision,"current_revision":app.revision,
                    "last_commit":{"plan_id":1,"step_index":0,"before_revision":app.revision-1,
                        "revision":app.revision,"label":"Connect signal","age_ms":0,"invalidate_existing_cables":false, "new_modules":[],
                        "new_cables":[{"id":cable.id,"from":cable.from,"output":cable.output,"to":cable.to,"input":cable.input}],
                        "parameter_changes":[]}}),
                true,
            );
            assert!(
                app.rack.construction_progress_at(&cable, now).is_some(),
                "{action}"
            );
            let id = app.patch.nodes[0].id.clone();
            match action {
                "add" => assert!(app.add_module("constant", None)),
                "duplicate" => assert!(app.duplicate_modules(
                    &[id],
                    DuplicateCableMode::None,
                    app.revision,
                    true
                )),
                "move" => {
                    let layout = app.rack.layout();
                    let p = &layout["positions"][&id];
                    let position =
                        egui::pos2(p[0].as_f64().unwrap() as f32, p[1].as_f64().unwrap() as f32);
                    assert!(app.move_modules(ModuleMove {
                        revision: app.revision,
                        before: BTreeMap::from([(id.clone(), position)]),
                        after: BTreeMap::from([(id, position + egui::vec2(6000.0, 4200.0))]),
                    }));
                }
                "replace" => app.replace_now(app.patch.clone(), "Replaced"),
                _ => unreachable!(),
            }
            assert!(
                app.rack.construction_progress_at(&cable, now).is_none(),
                "{action}"
            );
        }
    }

    #[test]
    fn recovery_opens_unsaved_and_save_as_preserves_the_original_project() {
        let dir = tempfile::tempdir().unwrap();
        let mut old = host(dir.path());
        old.save_document("original.openpatch", true);
        let original = std::fs::read(dir.path().join("original.openpatch")).unwrap();
        edit(&mut old, 330.0);
        make_autosave_due(&mut old);
        old.autosave();
        drop(old);
        let mut app = host(dir.path());
        assert_eq!(app.recovery_names.len(), 1);
        let recovery = app.recovery_names[0].clone();
        app.load_document(&recovery, true);
        assert!(app.dirty());
        assert!(app.active_file.is_none());
        assert!(dir.path().join(&recovery).exists());
        app.save_current();
        assert!(matches!(app.file_mode, Some(FileMode::Save)));
        app.save_document("recovered.openpatch", true);
        assert!(!app.dirty());
        assert_eq!(
            std::fs::read(dir.path().join("original.openpatch")).unwrap(),
            original
        );
        assert!(!dir.path().join(&recovery).exists());
        assert_eq!(
            app.projects
                .as_ref()
                .unwrap()
                .load("recovered.openpatch")
                .unwrap()
                .document
                .patch
                .nodes[0]
                .params["frequency"],
            330.0
        );
    }

    #[test]
    fn new_template_installs_layout_clears_history_and_preserves_host_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.save_document("original.openpatch", true);
        let original_file = std::fs::read(dir.path().join("original.openpatch")).unwrap();
        edit(&mut app, 330.0);
        app.shared
            .lock()
            .unwrap()
            .execute("undo", json!({}))
            .unwrap();
        app.sync();
        let mut template = crate::startup::factory_document();
        template.layout["zoom"] = json!(0.75);
        app.projects
            .as_ref()
            .unwrap()
            .save_template(&template, None)
            .unwrap();
        app.volume = 0.42;
        app.output_device = Some("chosen output".into());
        app.midi_device = Some("chosen MIDI".into());
        app.rack.set_positions_locked(true);
        let revision = app.revision;
        app.new_from_template(false);
        assert!(app.capture_document().equivalent(&template));
        assert!(app.revision > revision);
        let snapshot = app.shared.lock().unwrap().snapshot();
        assert_eq!(snapshot["history"]["undo_steps"], 0);
        assert_eq!(snapshot["history"]["redo_steps"], 0);
        assert!(app.active_file.is_none() && app.active_token.is_none());
        assert_eq!(app.title, "Untitled");
        assert!(!app.dirty());
        assert_eq!(app.volume, 0.42);
        assert_eq!(app.output_device.as_deref(), Some("chosen output"));
        assert_eq!(app.midi_device.as_deref(), Some("chosen MIDI"));
        assert!(app.rack.positions_locked());
        assert_eq!(
            std::fs::read(dir.path().join("original.openpatch")).unwrap(),
            original_file
        );
    }

    #[test]
    fn new_template_saved_content_guard_preserves_current_project_and_history() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, telemetry) = sequencer_host(dir.path());
        app.observe_document();
        let before = app.shared.lock().unwrap().snapshot();
        let layout = app.rack.layout();
        let active = app.active_file.clone();
        set_project_run(&telemetry, false, true, 256);
        app.new_from_template(true);
        assert!(matches!(
            app.confirm_discard,
            Some(PendingAction::NewTemplate)
        ));
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
        assert_eq!(app.rack.layout(), layout);
        assert_eq!(app.active_file, active);
    }

    #[test]
    fn save_template_publishes_reviewed_snapshot_without_changing_project_identity_or_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.save_document("working.openpatch", true);
        let saved_file = std::fs::read(dir.path().join("working.openpatch")).unwrap();
        edit(&mut app, 330.0);
        app.rack.set_zoom(0.75);
        app.begin_save_template();
        let reviewed = app.template_save.as_ref().unwrap().document.clone();
        edit(&mut app, 550.0);
        app.rack.set_zoom(0.85);
        let live = app.shared.lock().unwrap().snapshot();
        let layout = app.rack.layout();
        let title = app.title.clone();
        let filename = app.filename.clone();
        app.save_confirmed_template();
        let published = app
            .projects
            .as_ref()
            .unwrap()
            .load_template()
            .unwrap()
            .unwrap();
        assert!(published.equivalent(&reviewed));
        assert_eq!(published.patch.nodes[0].params["frequency"], 330.0);
        assert_eq!(published.layout["zoom"], 0.75);
        assert_eq!(app.shared.lock().unwrap().snapshot(), live);
        assert_eq!(app.rack.layout(), layout);
        assert_eq!(app.active_file.as_deref(), Some("working.openpatch"));
        assert!(app.active_token.is_some());
        assert_eq!(app.title, title);
        assert_eq!(app.filename, filename);
        assert!(app.dirty());
        assert_eq!(
            std::fs::read(dir.path().join("working.openpatch")).unwrap(),
            saved_file
        );
        edit(&mut app, 220.0);
        app.rack.set_zoom(1.0);
        app.observe_document();
        assert!(
            !app.dirty(),
            "template publication must retain the original project baseline"
        );
    }

    #[test]
    fn startup_view_keeps_early_mcp_edits_and_marks_them_dirty_against_prepared_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let (mut session, startup) = crate::startup::prepare(dir.path(), None, false).unwrap();
        let (_, clean) = OpenPatch::startup_view(&startup, session.patch(), session.revision());
        assert!(!clean.dirty());
        session
            .execute("add_module", json!({"id":"early_mcp","kind":"constant"}))
            .unwrap();
        let (rack, dirty) = OpenPatch::startup_view(&startup, session.patch(), session.revision());
        assert!(session
            .patch()
            .nodes
            .iter()
            .any(|node| node.id == "early_mcp"));
        assert!(rack.layout()["positions"].get("early_mcp").is_some());
        assert!(dirty.dirty());
        session.execute("undo", json!({})).unwrap();
        let (_, restored) = OpenPatch::startup_view(&startup, session.patch(), session.revision());
        assert!(!restored.dirty());
    }

    fn performance_key(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn performance_keyboard_guards_release_notes_without_changing_the_document() {
        use openpatch_core::MidiEvent;
        for blocked in [
            "closed",
            "stopped",
            "blur",
            "text",
            "popup",
            "file",
            "settings",
            "audio-performance",
            "shader",
            "recovery",
            "discard",
            "preset",
            "template",
            "engine",
            "drag",
            "closing",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = host(dir.path());
            app.keyboard_open = true;
            let before = app.shared.lock().unwrap().snapshot();
            let layout = app.rack.layout();
            let ctx = egui::Context::default();
            let mut events = Vec::new();
            let _ = ctx.run(
                egui::RawInput {
                    focused: true,
                    events: vec![performance_key(egui::Key::Z, true, egui::Modifiers::NONE)],
                    ..Default::default()
                },
                |ctx| {
                    events = app.update_performance_keyboard(ctx, true);
                },
            );
            assert_eq!(
                events,
                vec![MidiEvent::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 127
                }],
                "{blocked}"
            );
            app.keyboard_open = blocked != "closed";
            app.file_mode = (blocked == "file").then_some(FileMode::Open);
            app.settings_open = blocked == "settings";
            app.audio_performance_open = blocked == "audio-performance";
            app.shader_open = blocked == "shader";
            app.recovery_open = blocked == "recovery";
            app.confirm_discard = (blocked == "discard").then_some(PendingAction::Close);
            app.confirm_engine_upgrade = blocked == "engine";
            app.allow_close = blocked == "closing";
            if blocked == "preset" {
                app.preset_dialog = Some(PresetDialog::new(
                    "tone".into(),
                    "oscillator".into(),
                    app.revision,
                    PresetMode::Save,
                ));
            }
            if blocked == "template" {
                app.template_save = Some(TemplateSave {
                    document: app.capture_document(),
                    token: None,
                });
            }
            let _ = ctx.run(
                egui::RawInput {
                    focused: blocked != "blur",
                    events: vec![performance_key(egui::Key::Q, true, egui::Modifiers::NONE)],
                    ..Default::default()
                },
                |ctx| {
                    if blocked == "text" {
                        ctx.memory_mut(|m| m.request_focus(egui::Id::new("keyboard-guard-text")));
                    }
                    if blocked == "popup" {
                        egui::Popup::open_id(ctx, egui::Id::new("keyboard-guard-popup"));
                    }
                    if blocked == "drag" {
                        egui::DragAndDrop::set_payload(
                            ctx,
                            LibraryDrag {
                                kind: "constant".into(),
                            },
                        );
                    }
                    assert!(
                        !app.performance_keyboard_enabled(ctx, blocked != "stopped"),
                        "{blocked}"
                    );
                    events = app.update_performance_keyboard(ctx, blocked != "stopped");
                },
            );
            assert_eq!(
                events,
                vec![MidiEvent::NoteOff {
                    channel: 0,
                    note: 60,
                    velocity: 127
                }],
                "{blocked}"
            );
            assert!(app.keyboard.release().is_empty(), "{blocked} released once");
            assert_eq!(app.shared.lock().unwrap().snapshot(), before, "{blocked}");
            assert_eq!(app.rack.layout(), layout, "{blocked}");
        }
    }

    #[test]
    fn same_frame_text_focus_prevents_performance_note_after_dock_reserves_space() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.keyboard_open = true;
        let before = app.shared.lock().unwrap().snapshot();
        let ctx = egui::Context::default();
        let mut text = String::new();
        let _ = ctx.run(
            egui::RawInput {
                focused: true,
                events: vec![performance_key(egui::Key::Z, true, egui::Modifiers::NONE)],
                ..Default::default()
            },
            |ctx| {
                assert!(app.performance_keyboard_enabled(ctx, true));
                app.performance_keyboard(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut text))
                        .request_focus();
                });
                assert!(ctx.wants_keyboard_input());
                assert!(app.update_performance_keyboard(ctx, true).is_empty());
            },
        );
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
        assert!(app.keyboard.release().is_empty());
    }

    #[test]
    fn command_and_control_host_shortcuts_remain_actions_and_never_performance_notes() {
        for ctrl in [false, true] {
            for key in [
                egui::Key::N,
                egui::Key::O,
                egui::Key::S,
                egui::Key::Z,
                egui::Key::D,
            ] {
                let dir = tempfile::tempdir().unwrap();
                let mut app = host(dir.path());
                app.keyboard_open = true;
                app.rack.select_only("tone".into());
                let before = app.shared.lock().unwrap().snapshot();
                assert!(app.patch.nodes.iter().any(|node| node.id == "tone"));
                assert_eq!(app.rack.selected_ids(), vec!["tone"]);
                assert!(before["history"]["undo_steps"].as_u64().unwrap() > 0);
                let count = app.patch.nodes.len();
                let modifiers = egui::Modifiers {
                    command: true,
                    ctrl,
                    ..Default::default()
                };
                let ctx = egui::Context::default();
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100.0, 800.0),
                        )),
                        focused: true,
                        modifiers,
                        // Rack duplication intentionally requires the pointer over
                        // its canvas even when a module is already selected.
                        events: vec![
                            egui::Event::PointerMoved(egui::pos2(84.0, 44.0)),
                            performance_key(key, true, modifiers),
                        ],
                        ..Default::default()
                    },
                    |ctx| {
                        app.global_shortcuts(ctx);
                        app.rack_workspace(ctx, None);
                        assert!(
                            app.update_performance_keyboard(ctx, true).is_empty(),
                            "{key:?}, ctrl={ctrl}"
                        );
                    },
                );
                match key {
                    egui::Key::N => {
                        assert_eq!(app.title, "Untitled");
                        assert_eq!(
                            app.shared.lock().unwrap().snapshot()["history"]["undo_steps"],
                            0
                        );
                    }
                    egui::Key::O => assert!(app.file_mode == Some(FileMode::Open)),
                    egui::Key::S => assert!(app.file_mode == Some(FileMode::Save)),
                    egui::Key::Z => assert_ne!(app.shared.lock().unwrap().snapshot(), before),
                    egui::Key::D => assert_eq!(
                        app.patch.nodes.len(),
                        count + 1,
                        "ctrl={ctrl}: {}",
                        app.status
                    ),
                    _ => unreachable!(),
                }
                assert!(app.keyboard.release().is_empty());
            }
        }
    }
}
