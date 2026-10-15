use crate::{
    audio::{Audio, AudioTimingSnapshot},
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
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::PathBuf,
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
                            ui.label(if name == "running" {
                                "RUN"
                            } else {
                                parameter_label(name)
                            });
                            let spec = module
                                .and_then(|module| module["params"].get(name))
                                .unwrap_or(&Value::Null);
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
    browser_open: bool,
    library: ModuleLibrary,
    preset_dialog: Option<PresetDialog>,
    template_save: Option<TemplateSave>,
    catalog: Value,
    start: Instant,
    mcp: bool,
    construction: crate::construction_ui::Panel,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    local_mcp: Option<openpatch::local_mcp::LocalServer>,
    local_mcp_error: Option<String>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    local_mcp_command: String,
    documents: DocumentState,
    projects: Option<Store>,
    active_file: Option<String>,
    active_token: Option<FileToken>,
    after_save: Option<PendingAction>,
    deferred_action: Option<PendingAction>,
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
        match server {
            Ok(server) => {
                let cli = std::env::current_exe()
                    .ok()
                    .and_then(|path| path.parent().map(|parent| parent.join("openpatch")))
                    .filter(|path| path.is_file());
                let executable = cli
                    .map(|path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''")))
                    .unwrap_or_else(|| "openpatch".into());
                self.local_mcp_command =
                    format!("{executable} attach --session {}", server.session_id());
                self.local_mcp = Some(server);
            }
            Err(error) => self.local_mcp_error = Some(error),
        }
    }

    fn mcp_status(&self, ui: &mut egui::Ui) {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if let Some(server) = &self.local_mcp {
            let clients = server.client_count();
            let label = if clients == 0 {
                "MCP ready".to_owned()
            } else {
                format!("MCP · {clients} attached")
            };
            let command = &self.local_mcp_command;
            if ui
                .small_button(RichText::new(label).color(ACCENT).size(10.0))
                .on_hover_text(format!(
                    "Connect an MCP client to this rack. Click to copy:\n{command}\nPlayback stays under your control.{}",
                    if self.mcp { "\nA stdio connection is also enabled." } else { "" }
                ))
                .clicked()
            {
                ui.ctx().copy_text(command.clone());
            }
            return;
        }
        if self.mcp {
            ui.label(RichText::new("MCP stdio").color(ACCENT).size(10.0))
                .on_hover_text(
                    "The client that launched this window controls its live patch over stdio.",
                );
        }
        if let Some(error) = &self.local_mcp_error {
            ui.label(
                RichText::new("MCP attach unavailable")
                    .color(ACCENT)
                    .size(10.0),
            )
            .on_hover_text(error);
        }
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
        let (patch, revision, state_id, workspace) = {
            let s = shared.lock().unwrap();
            (
                s.patch().clone(),
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
        let (mut audio, device) = Self::open_audio(&patch, output_device.as_deref());
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
            browser_open: false,
            library: ModuleLibrary::default(),
            preset_dialog: None,
            template_save: None,
            catalog: catalog(),
            start: Instant::now(),
            mcp,
            construction: crate::construction_ui::Panel::default(),
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            local_mcp: None,
            local_mcp_error: None,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            local_mcp_command: String::new(),
            documents,
            projects: startup.store,
            active_file: startup.active_file,
            active_token: startup.active_token,
            after_save: None,
            deferred_action: None,
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

    fn attach_live_events(&self) {
        let sender = self.audio.as_ref().map(Audio::module_event_sender);
        if let Ok(mut session) = self.shared.lock() {
            session.set_live_module_events(sender);
        }
    }
    fn open_audio(patch: &Patch, device: Option<&str>) -> (Option<Audio>, String) {
        let opened = match device {
            Some(device) => Audio::new_with_device(patch, Some(device)),
            None => Audio::new(patch),
        };
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
    fn apply_host_preferences(&mut self, preferences: &Value) {
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
                "lock_module_positions": self.rack.positions_locked()
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
        let (audio, label) = Self::open_audio(&self.patch, selected.as_deref());
        if let Some(mut audio) = audio {
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
            && ctx.input(|i| i.focused)
            && !egui::DragAndDrop::has_payload_of_type::<LibraryDrag>(ctx)
            && self.preset_dialog.is_none()
            && self.template_save.is_none()
            && self.file_mode.is_none()
            && !self.shader_open
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
        let mut open = true;
        egui::Window::new("Audio & MIDI")
            .default_pos(ctx.content_rect().center() - Vec2::new(220.0, 190.0))
            .open(&mut open)
            .resizable(false)
            .default_width(430.0)
            .show(ctx, |ui| {
                ui.heading("Audio output");
                let mut selected = self.output_device.clone();
                egui::ComboBox::from_id_salt("audio-device")
                    .width(390.0)
                    .selected_text(selected.as_deref().unwrap_or("System default"))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut selected, None, "System default");
                        for device in &self.output_devices {
                            ui.selectable_value(&mut selected, Some(device.clone()), device);
                        }
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
                ui.label("Changing the output pauses playback. Sample rate follows the device.");
                ui.separator();
                ui.heading("MIDI input");
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
                    .width(390.0)
                    .selected_text(midi_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut midi, None, "None — use the on-screen keyboard");
                        for port in &self.midi_ports {
                            ui.selectable_value(&mut midi, Some(port.id.clone()), &port.name);
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
                ui.label("Pause, reconnect, and voice allocation changes release held notes.");
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
        if !matches!(name, "save_patch" | "render_audio") && args.get("expected_revision").is_none()
        {
            args["expected_revision"] = json!(self.revision);
        }
        match execute_shared(&self.shared, name, args) {
            Ok(_) => {
                self.status = if name == "trigger_module" {
                    "Performance action queued"
                } else {
                    "Patch updated"
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
    fn sync(&mut self) {
        if let Ok(s) = self.shared.try_lock() {
            let construction = s.construction_snapshot();
            self.can_undo = s.can_undo();
            self.can_redo = s.can_redo();
            if s.revision() != self.revision {
                // Preview positions never belong to the outgoing history state.
                // Cancel even when a grouped edit retains the same state ID.
                self.rack.cancel_module_drag();
                self.history_layouts
                    .insert(self.state_id, self.rack.layout());
                if self.state_id != s.state_id() {
                    self.state_id = s.state_id();
                    if let Some(layout) = self.history_layouts.get(&self.state_id) {
                        let _ = self.rack.restore_layout(layout);
                    }
                }
                let retained: BTreeSet<_> = s.history_state_ids().collect();
                self.history_layouts.retain(|id, _| retained.contains(id));
                self.rack.observe_construction_transition(
                    &self.patch,
                    s.patch(),
                    self.revision,
                    s.revision(),
                    &construction,
                    self.construction.animate_cables(),
                );
                self.patch = s.patch().clone();
                self.rack.reconcile_selection(&self.patch);
                self.revision = s.revision();
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
            if self.audio_revision != Some(self.revision) {
                match audio.submit_patch_at_revision(&self.patch, self.revision) {
                    Ok(()) => self.audio_revision = Some(self.revision),
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
                self.patch = applied.patch;
                self.rack = applied.rack;
                self.rack.set_positions_locked(locked);
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
        if let Err(error) = result {
            self.status = error;
            if require_saved {
                self.confirm_discard = Some(PendingAction::Replace(patch, title.into()));
            }
            self.sync();
            self.observe_document();
            return;
        }
        self.sync();
        self.panic_notes();
        self.clear_recovery();
        self.title = title.into();
        self.active_file = None;
        self.active_token = None;
        let positions_locked = self.rack.positions_locked();
        self.rack = Rack::default();
        self.rack.set_positions_locked(positions_locked);
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
        } else if title == "Layered stereo morph" {
            self.rack
                .restore_layout(&Self::mixing_layout())
                .expect("bundled mixer layout");
        }
        self.documents.mark_unsaved();
        self.autosave_needed = true;
        self.capture_document();
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
                self.rack = applied.rack;
                self.rack.set_positions_locked(positions_locked);
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
        let name = self.filename.clone();
        if mode == FileMode::Save {
            self.save_document(&name, true);
        } else {
            self.file_mode = None;
            self.request_action(PendingAction::Load(name));
        }
    }
    fn global_shortcuts(&mut self, ctx: &egui::Context) {
        if !self.library_enabled()
            || !ctx.input(|i| i.focused)
            || ctx.wants_keyboard_input()
            || egui::Popup::is_any_open(ctx)
            || egui::DragAndDrop::has_payload_of_type::<LibraryDrag>(ctx)
        {
            return;
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
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) {
            self.history(ctx.input(|i| i.modifiers.shift));
        }
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

    fn toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu")
            .frame(
                egui::Frame::new()
                    .fill(CHROME)
                    .inner_margin(egui::Margin::symmetric(12, 7)),
            )
            .show(ctx, |ui| {
                ui.add_enabled_ui(self.library_enabled(), |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.spacing_mut().button_padding = Vec2::new(7.0, 6.0);
                        ui.set_min_height(30.0);
                        ui.label(RichText::new("OpenPatch").strong().size(14.0));
                        ui.add_space(6.0);
                        ui.menu_button("File", |ui| {
                            if ui.button("New patch     ⌘N").clicked() {
                                self.request_action(PendingAction::NewTemplate);
                                ui.close();
                            }
                            if ui.button("Save template…").clicked() {
                                self.begin_save_template();
                                ui.close();
                            }
                            if ui.button("Open…     ⌘O").clicked() {
                                self.file_dialog(FileMode::Open);
                                ui.close();
                            }
                            if ui.button("Save     ⌘S").clicked() {
                                self.save_current();
                                ui.close();
                            }
                            if ui.button("Save as…     ⇧⌘S").clicked() {
                                self.after_save = None;
                                self.file_dialog(FileMode::Save);
                                ui.close();
                            }
                            if ui
                                .add_enabled(
                                    self.active_file.is_some(),
                                    egui::Button::new("Revert to saved"),
                                )
                                .clicked()
                            {
                                if let Some(name) = self.active_file.clone() {
                                    self.request_action(PendingAction::Load(name));
                                }
                                ui.close();
                            }
                            if ui
                                .add_enabled(
                                    !self.recovery_names.is_empty(),
                                    egui::Button::new("Recover unsaved work…"),
                                )
                                .clicked()
                            {
                                self.recovery_open = true;
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("Reference analog voice").clicked() {
                                let mut patch = openpatch_core::analog_demo_patch();
                                patch.version = 2;
                                self.replace_from_menu(patch, "Reference analog voice");
                                ui.close();
                            }
                            if ui.button("Clocked random voice").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/clocked-random-voice.json"
                                ))
                                .expect("valid bundled random voice");
                                self.replace_from_menu(patch, "Clocked random voice");
                                ui.close();
                            }
                            if ui.button("Synced waveform voice").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/synced-waveform-voice.json"
                                ))
                                .expect("valid bundled waveform voice");
                                self.replace_from_menu(patch, "Synced waveform voice");
                                ui.close();
                            }
                            if ui.button("Stacked pitch voice").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/stacked-pitch-voice.json"
                                ))
                                .expect("valid bundled stacked pitch voice");
                                self.replace_from_menu(patch, "Stacked pitch voice");
                                ui.close();
                            }
                            if ui.button("Layered stereo morph").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/layered-stereo-morph.json"
                                ))
                                .expect("valid bundled layered stereo morph");
                                self.replace_from_menu(patch, "Layered stereo morph");
                                ui.close();
                            }
                            if ui.button("Delayed gate voice").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/delayed-gate-voice.json"
                                ))
                                .unwrap();
                                ui.close();
                                self.replace_from_menu(patch, "Delayed gate voice");
                            }
                            if ui.button("Manual push voice").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/manual-push-voice.json"
                                ))
                                .unwrap();
                                ui.close();
                                self.replace_from_menu(patch, "Manual push voice");
                            }
                            if ui.button("Random values voice").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/random-values-voice.json"
                                ))
                                .expect("valid bundled random values voice");
                                ui.close();
                                self.replace_from_menu(patch, "Random values voice");
                                self.rack.set_zoom(0.75);
                            }
                            if ui.button("Noise colors").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/noise-colors.json"
                                ))
                                .expect("valid bundled noise colors");
                                ui.close();
                                self.replace_from_menu(patch, "Noise colors");
                                self.rack.set_zoom(0.85);
                            }
                            if ui.button("Struck resonator").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/struck-resonator.json"
                                ))
                                .expect("valid bundled physical resonator");
                                ui.close();
                                self.replace_from_menu(patch, "Struck resonator");
                                self.rack.set_zoom(0.75);
                            }
                            if ui.button("Manual pulses voice").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/manual-pulses-voice.json"
                                ))
                                .unwrap();
                                ui.close();
                                self.replace_from_menu(patch, "Manual pulses voice");
                            }
                            if ui.button("Gate shaping voice").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/gate-shaping-voice.json"
                                ))
                                .expect("valid bundled gate shaping voice");
                                self.replace_from_menu(patch, "Gate shaping voice");
                                ui.close();
                            }
                            if ui.button("Voltage utilities").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/voltage-utilities.json"
                                ))
                                .expect("valid bundled voltage utilities");
                                self.replace_from_menu(patch, "Voltage utilities");
                                ui.close();
                            }
                            if ui.button("Switched stereo voice").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/switched-stereo-voice.json"
                                ))
                                .expect("valid bundled switching voice");
                                self.replace_from_menu(patch, "Switched stereo voice");
                                ui.close();
                            }
                            if ui.button("Sequenced analog voice").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/sequenced-analog-voice.json"
                                ))
                                .expect("valid bundled sequence");
                                self.replace_from_menu(patch, "Sequenced analog voice");
                                ui.close();
                            }
                            if ui.button("Clocked modulation").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/clocked-modulation.json"
                                ))
                                .expect("valid bundled modulation voice");
                                self.replace_from_menu(patch, "Clocked modulation");
                                self.rack.set_zoom(0.75);
                                ui.close();
                            }
                            if ui.button("Stereo width study").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/stereo-width-study.json"
                                ))
                                .expect("valid bundled stereo study");
                                self.replace_from_menu(patch, "Stereo width study");
                                ui.close();
                            }
                            if ui.button("MIDI keyboard voice").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/midi-keyboard-voice.json"
                                ))
                                .expect("valid bundled MIDI voice");
                                self.replace_from_menu(patch, "MIDI keyboard voice");
                                self.keyboard_open = true;
                                ui.close();
                            }
                            if ui.button("Polyphonic voice").clicked() {
                                let patch = serde_json::from_str(include_str!(
                                    "../../../examples/polyphonic-voice.json"
                                ))
                                .expect("valid bundled polyphonic voice");
                                self.replace_from_menu(patch, "Polyphonic voice");
                                ui.close();
                            }
                            if self.patch.version == 1
                                && ui.button("Enable polyphony and feedback…").clicked()
                            {
                                self.confirm_engine_upgrade = true;
                                ui.close();
                            }
                            if ui.button("Tremolo study").clicked() {
                                self.replace_from_menu(
                                    openpatch_core::demo_patch(),
                                    "Tremolo study",
                                );
                                ui.close();
                            }
                        });
                        ui.menu_button("Edit", |ui| {
                            if ui
                                .add_enabled(
                                    self.can_undo,
                                    egui::Button::new("Undo patch edit   ⌘Z"),
                                )
                                .clicked()
                            {
                                self.history(false);
                                ui.close();
                            }
                            if ui
                                .add_enabled(
                                    self.can_redo,
                                    egui::Button::new("Redo patch edit   ⇧⌘Z"),
                                )
                                .clicked()
                            {
                                self.history(true);
                                ui.close();
                            }
                            ui.separator();
                            if ui
                                .add_enabled(!self.patch.cables.is_empty(), egui::Button::new("Clear cables"))
                                .clicked()
                            {
                                self.command("clear_cables", json!({"expected_revision":self.revision}));
                                ui.close();
                            }
                            if ui.button("Add module…   Enter").clicked() {
                                self.browse();
                                ui.close();
                            }
                        });
                        ui.menu_button("View", |ui| {
                            for (tab, label) in [
                                (AnalysisTab::Visuals, "Visualizers"),
                                (AnalysisTab::Scope, "Signal scope"),
                            ] {
                                let mut selected = self.analysis_tab == Some(tab);
                                if ui.checkbox(&mut selected, label).changed() {
                                    self.select_analysis(selected.then_some(tab));
                                }
                            }
                            ui.checkbox(&mut self.keyboard_open, "Performance keyboard");
                            if ui.button("Audio performance…").clicked() {
                                self.audio_performance_open = true;
                                ui.close();
                            }
                            ui.separator();
                            let mut locked = self.rack.positions_locked();
                            if ui.checkbox(&mut locked, "Lock module positions").changed() {
                                self.rack.set_positions_locked(locked);
                                self.save_host_settings();
                            }
                            if ui.button("Reset rack view").clicked() {
                                self.rack.reset_view();
                                ui.close();
                            }
                            if ui.button("Shader editor").clicked() {
                                self.shader_open = true;
                                ui.close();
                            }
                        });
                        ui.separator();
                        // Allocate fixed controls from the right, then give the title
                        // the remaining width. Long project names cannot hide transport.
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(
                                ui,
                                Icon::Settings,
                                "Audio / MIDI settings",
                                self.settings_open,
                                true,
                            )
                            .clicked()
                            {
                                self.settings_open = !self.settings_open;
                                if self.settings_open {
                                    self.refresh_devices();
                                }
                            }
                            if icon_button(ui, Icon::Plus, "Zoom in", false, true).clicked() {
                                self.rack.zoom_step(true);
                            }
                            let mut zoom = self.rack.zoom() * 100.0;
                            let zoom_response = ui
                                .add_sized(
                                    [48.0, 30.0],
                                    egui::DragValue::new(&mut zoom)
                                        .range(25.0..=400.0)
                                        .suffix("%")
                                        .speed(1.0),
                                )
                                .on_hover_text("Rack zoom · Cmd/Ctrl −/+ · double-click or Cmd/Ctrl+0 for 100%");
                            if zoom_response.double_clicked() {
                                self.rack.set_zoom(1.0);
                                zoom_response.surrender_focus();
                                ui.data_mut(|data| data.remove::<String>(zoom_response.id));
                            } else if zoom_response.changed() {
                                self.rack.set_zoom(zoom / 100.0);
                            }
                            if icon_button(ui, Icon::Minus, "Zoom out", false, true).clicked() {
                                self.rack.zoom_step(false);
                            }
                            ui.separator();
                            if icon_button(
                                ui,
                                Icon::Visuals,
                                "Visuals",
                                self.analysis_tab == Some(AnalysisTab::Visuals),
                                true,
                            )
                            .clicked()
                            {
                                self.toggle_analysis(AnalysisTab::Visuals);
                            }
                            if icon_button(
                                ui,
                                Icon::Scope,
                                "Signal scope",
                                self.analysis_tab == Some(AnalysisTab::Scope),
                                true,
                            )
                            .clicked()
                            {
                                self.toggle_analysis(AnalysisTab::Scope);
                            }
                            if icon_button(
                                ui,
                                Icon::Library,
                                "Module library · Enter",
                                self.browser_open,
                                true,
                            )
                            .clicked()
                            {
                                if self.browser_open {
                                    self.browser_open = false;
                                } else {
                                    self.browse();
                                }
                            }
                            ui.separator();
                            ui.scope(|ui| {
                                ui.spacing_mut().slider_width = 64.0;
                                if ui
                                    .add(
                                        egui::Slider::new(&mut self.volume, 0.0..=1.0)
                                            .show_value(false),
                                    )
                                    .on_hover_text(format!(
                                        "Master volume · {:.0}%",
                                        self.volume * 100.0
                                    ))
                                    .changed()
                                {
                                    if let Some(audio) = &self.audio {
                                        audio.set_volume(self.volume);
                                    }
                                }
                            });
                            let playing = self.audio.as_ref().is_some_and(Audio::is_playing);
                            if icon_button(
                                ui,
                                if playing { Icon::Pause } else { Icon::Play },
                                if playing {
                                    "Pause audio · Space"
                                } else {
                                    "Play audio · Space"
                                },
                                playing,
                                self.audio.is_some(),
                            )
                            .clicked()
                            {
                                self.toggle_play();
                            }
                            ui.separator();
                            if icon_button(ui, Icon::Save, "Save patch · Cmd/Ctrl+S", false, true)
                                .clicked()
                            {
                                self.save_current();
                            }
                            if icon_button(
                                ui,
                                Icon::Redo,
                                "Redo patch edit · Shift+Cmd/Ctrl+Z",
                                false,
                                self.can_redo,
                            )
                            .clicked()
                            {
                                self.history(true);
                            }
                            if icon_button(
                                ui,
                                Icon::Undo,
                                "Undo patch edit · Cmd/Ctrl+Z",
                                false,
                                self.can_undo,
                            )
                            .clicked()
                            {
                                self.history(false);
                            }
                            let title =
                                format!("{}{}", self.title, if self.dirty() { " *" } else { "" });
                            ui.add(
                                egui::Label::new(RichText::new(&title).size(13.0).color(TEXT))
                                    .truncate(),
                            )
                            .on_hover_text(title);
                        });
                    });
                });
            });
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
                            for (mode, name) in [(0, "Aurora"), (1, "Spectrum"), (2, "Waveform")] {
                                ui.selectable_value(&mut self.visual_mode, mode, name);
                            }
                        });
                        let wave = self.audio.as_ref().map_or([0.0; 256], Audio::waveform);
                        let peak = self.audio.as_ref().map_or(0.0, Audio::peak);
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

    fn rack_workspace(&mut self, ctx: &egui::Context, library_action: Option<LibraryAction>) {
        let mut action = None;
        let mut pending_add = None;
        if !self.library_enabled() || !ctx.input(|i| i.focused) {
            egui::DragAndDrop::clear_payload(ctx);
        }
        let drag = egui::DragAndDrop::payload::<LibraryDrag>(ctx);
        self.rack.set_shortcuts_enabled(
            drag.is_none()
                && !self.settings_open
                && !self.audio_performance_open
                && !self.shader_open
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
                    && self.confirm_discard.is_none()
                    && !self.recovery_open
                    && !self.confirm_engine_upgrade
                    && self.preset_dialog.is_none()
                    && self.template_save.is_none()
                    && !self.audio_performance_open
                    && !self.allow_close;
                let canvas = ui.available_rect_before_wrap();
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
                self.command(name, args);
            }
            Some(RackAction::ManualClick { id, press, release }) => {
                self.manual_click(&id, press, release);
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
        if self.library.take_favorites_changed() {
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
                        "id":dialog.id,"expected_revision":dialog.revision
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
            ui.label(RichText::new("Presets store module controls and bypass only. Rack cables and position stay in the project.").color(MUTED).size(11.0));
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
                    json!({"filename":filename}),
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
        let Some(mode) = self.file_mode else {
            return;
        };
        let mut run = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("project-file-dialog")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(if mode == FileMode::Save { "Save project as" } else { "Open project" });
            ui.label(RichText::new(&self.workspace).color(MUTED).size(11.0));
            ui.horizontal(|ui| {
                ui.label("Filename");
                let mut output = egui::TextEdit::singleline(&mut self.filename).desired_width(330.0).show(ui);
                if self.file_focus {
                    self.file_focus = false;
                    output.response.request_focus();
                    output.state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),egui::text::CCursor::new(self.filename.chars().count())
                    )));
                    output.state.store(ui.ctx(),output.response.id);
                }
                run |= output.response.lost_focus() && ui.input(|i|i.key_pressed(egui::Key::Enter));
            });
            if mode == FileMode::Open {
                egui::ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
                    for file in &self.files {
                        let response = ui.selectable_label(self.filename == *file,file);
                        if response.clicked() { self.filename = file.clone(); }
                        if response.double_clicked() { self.filename = file.clone(); run = true; }
                    }
                    if self.files.is_empty() { ui.label("No saved projects yet."); }
                });
                ui.label("Open a native .openpatch project or import an older .json patch.");
            } else {
                ui.label("Use a new .openpatch filename. Modules, cables and rack layout are saved together.");
            }
            ui.separator();
            ui.label(&self.status);
            ui.horizontal(|ui| {
                run |= ui.button(if mode == FileMode::Save { "Save project" } else { "Open" }).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.file_mode = None;
            self.after_save = None;
        } else if run {
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
                    "GLSL 330 · u_time · u_resolution · u_level · u_wave[256] · u_spectrum[64]",
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

    fn smoke(&mut self, ctx: &egui::Context) {
        let Some((path, frame, captures)) = &mut self.smoke_test else {
            return;
        };
        *frame += 1;
        let (path, frame, captures) = (path.clone(), *frame, *captures);
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
            self.title = "Stereo and routing utilities".into();
        }
        if frame == 112 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/clocked-modulation.json"))
                    .expect("valid bundled modulation voice");
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("modulation voice compiles");
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
            self.rack
                .restore_layout(&json!({"version":1,"zoom":1.0,"pan":[24,12],"positions":{
                    "tone":[0,0],"vca":[210,0],"out":[420,0]
                }}))
                .unwrap();
            self.rack.select_only("tone".into());
            self.rack
                .request_parameter_menu_for_smoke("tone", "frequency");
            self.title = "Exact frequency · direct knob entry".into();
            self.status =
                "Right-click a knob to enter a value · Apply creates one undo step".into();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
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
            self.rack = Rack::default();
            self.rack
                .restore_layout(&json!({"version":1,"zoom":0.75,"pan":[24,12],"positions":{
                    "values":[0,0],"scope":[330,0],"clock":[0,420],
                    "notes":[240,420],"vco":[420,420],"out":[750,420]
                }}))
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
        if frame == 622 {
            assert!(
                egui::Popup::is_any_open(ctx),
                "Parameter screenshot must contain the open editor"
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
        ) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        if let Some(image) = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Screenshot { image, .. } = e {
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
                _ => path.with_file_name("rack-random-values.png"),
            };
            image::save_buffer(
                &target,
                image.as_raw(),
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
            )
            .expect("Save screenshot");
            eprintln!(
                "Native smoke: capture {}/45 at frame {frame}: {}",
                captures + 1,
                target.display()
            );
            let captures = captures + 1;
            self.smoke_test.as_mut().unwrap().2 = captures;
            if captures == 45 {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        assert!(frame < 906, "Native screenshot timeout");
    }
}
impl eframe::App for OpenPatch {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
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
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.smoke_test.is_some() {
            ctx.style_mut(|style| {
                style.interaction.tooltip_delay = f32::INFINITY;
                // These captures qualify settled UI states. Their frame-driven
                // fixtures must not depend on a shared runner's wall-clock pace.
                style.animation_time = 0.0;
            });
        }
        self.sync();
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
        if !self.construction.animate_cables() {
            self.rack.clear_construction_animation();
        }
        self.footer(ctx);
        let library_action = self.browser(ctx);
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
            let states = self
                .patch
                .nodes
                .iter()
                .filter(|n| matches!(n.kind.as_str(), "seq3" | "switch_1to4" | "switch_4to1"))
                .filter_map(|n| {
                    self.audio
                        .as_ref()?
                        .sequence_state(&n.id)
                        .map(|state| (n.id.clone(), state))
                })
                .collect();
            self.rack.set_sequence_states(states, self.audio.is_some());
            let mut lights = BTreeMap::new();
            let mut controls = BTreeMap::new();
            let mut scopes = BTreeMap::new();
            if let Some(audio) = self.audio.as_ref() {
                for node in &self.patch.nodes {
                    if node.kind == "scope" {
                        if let Some(state) = audio.module_scope(&node.id) {
                            scopes.insert(node.id.clone(), state);
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
            self.rack.set_module_states(lights, controls);
            self.rack.set_scope_states(scopes);
        }
        self.rack_workspace(ctx, library_action);
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
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        drop(self.local_mcp.take());
        if let (Some(v), Some(gl)) = (&self.visualizer, gl) {
            v.destroy(gl);
        }
    }
}

#[cfg(test)]
mod project_workflow_tests {
    use super::*;
    use openpatch::control::Session;
    use std::path::Path;

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
            browser_open: false,
            library: ModuleLibrary::default(),
            preset_dialog: None,
            template_save: None,
            catalog: catalog(),
            start: Instant::now(),
            mcp: true,
            construction: crate::construction_ui::Panel::default(),
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            local_mcp: None,
            local_mcp_error: None,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            local_mcp_command: String::new(),
            documents,
            projects: Some(store),
            active_file: None,
            active_token: None,
            after_save: None,
            deferred_action: None,
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
            .set_live_module_events(Some(Arc::new(MissingTelemetry)));
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

        app.shared.lock().unwrap().set_live_module_events(None);
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
    fn position_lock_survives_new_open_import_and_recovery() {
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
        app.load_document(&recovery, true);
        assert!(app.rack.positions_locked());
        assert_eq!(app.title, "Recovered project");
        app.replace_now(Patch::new_v2(), "Untitled");
        assert!(app.rack.positions_locked());
        assert!(app.patch.nodes.is_empty());
        app.load_document("locked.openpatch", false);
        assert!(app.rack.positions_locked());
        assert_eq!(app.active_file.as_deref(), Some("locked.openpatch"));
        app.load_document("legacy.json", false);
        assert!(app.rack.positions_locked());
        assert!(app.active_file.is_none());
        assert_eq!(app.patch.nodes.len(), expected.patch.nodes.len());
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
            .set_live_module_events(Some(counter.clone()));
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
            .set_live_module_events(Some(backend.clone()));
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
        app.shared.lock().unwrap().set_live_module_events(None);
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
            .set_live_module_events(Some(telemetry.clone()));
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
            .set_live_module_events(Some(telemetry.clone()));
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
            .set_live_module_events(Some(telemetry.clone()));
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
            .set_live_module_events(Some(telemetry.clone()));
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
        app.shared.lock().unwrap().set_live_module_events(None);
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
            app.shared.lock().unwrap().set_live_module_events(None);
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
        app.request_action(PendingAction::Close);
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
                &json!({"last_revision":app.revision}),
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
