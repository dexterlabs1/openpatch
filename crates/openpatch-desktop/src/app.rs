use crate::{
    audio::Audio,
    document_state::{apply_document, DocumentState},
    keyboard::Keyboard,
    midi::{MidiInputManager, MidiPortInfo},
    project::{Document, FileToken, Store},
    rack::{Rack, RackAction},
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

const CHROME: Color32 = Color32::from_rgb(37, 39, 43);
const TEXT: Color32 = Color32::from_rgb(218, 220, 223);
const MUTED: Color32 = Color32::from_rgb(151, 157, 167);
const ACCENT: Color32 = Color32::from_rgb(120, 185, 164);

#[derive(Clone, Copy, PartialEq)]
enum FileMode {
    Open,
    Save,
}

enum PendingAction {
    Replace(Patch, String),
    Load(String),
    Recover(String),
    Close,
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
    keyboard: Keyboard,
    keyboard_open: bool,
    rack: Rack,
    visualizer: Option<Visualizer>,
    gl: Option<std::sync::Arc<glow::Context>>,
    shader: String,
    shader_open: bool,
    shader_status: String,
    visuals_open: bool,
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
    browser_focus: bool,
    search: String,
    catalog: Value,
    start: Instant,
    mcp: bool,
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
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        shared: SharedSession,
        mcp: bool,
        smoke_test: Option<PathBuf>,
    ) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = CHROME;
        style.visuals.window_fill = Color32::from_rgb(43, 46, 51);
        style.visuals.extreme_bg_color = Color32::from_rgb(24, 26, 30);
        style.visuals.override_text_color = Some(TEXT);
        style.visuals.selection.bg_fill = Color32::from_rgb(65, 95, 90);
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(53, 57, 63);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(71, 78, 86);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(64, 103, 90);
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
        let mut rack = Rack::default();
        rack.reconcile(&patch, &catalog());
        let documents = DocumentState::new(&patch, revision, &rack);
        let (projects, recovery_names, storage_error) =
            match Store::new(std::path::Path::new(&workspace)) {
                Ok(store) => {
                    let recoveries = store.recoveries();
                    (Some(store), recoveries, None)
                }
                Err(error) => (None, Vec::new(), Some(error)),
            };
        let recovery_open = !recovery_names.is_empty() && smoke_test.is_none();
        let mut app = Self {
            shared,
            patch,
            revision,
            state_id,
            history_layouts: BTreeMap::new(),
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
            keyboard: Keyboard::default(),
            keyboard_open: false,
            rack,
            visualizer,
            gl: cc.gl.clone(),
            shader: DEFAULT_SHADER.into(),
            shader_open: false,
            shader_status,
            visuals_open: false,
            visual_mode: 0,
            scope: Scope::default(),
            volume,
            status: "Ready — press Play to hear the patch".into(),
            filename: "my-patch.openpatch".into(),
            title: "Reference analog voice".into(),
            files: vec![],
            workspace,
            file_mode: None,
            file_focus: false,
            browser_open: false,
            browser_focus: false,
            search: String::new(),
            catalog: catalog(),
            start: Instant::now(),
            mcp,
            documents,
            projects,
            active_file: None,
            active_token: None,
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
        if let Some(error) = storage_error {
            app.status = format!("Project storage unavailable: {error}");
        }
        app.refresh_recovery_list();
        app.refresh_files();
        app.refresh_devices();
        app.connect_selected_midi();
        app.attach_live_events();
        app
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
                "output_device": self.output_device, "midi_device": self.midi_device
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
            self.settings_status =
                format!("Device selected; preferences could not be saved: {error}");
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
    fn performance_keyboard(&mut self, ctx: &egui::Context) {
        let enabled = self.keyboard_open
            && self.audio.as_ref().is_some_and(Audio::is_playing)
            && !ctx.wants_keyboard_input()
            && ctx.input(|i| i.focused)
            && !self.browser_open
            && self.file_mode.is_none()
            && !self.shader_open
            && !self.settings_open
            && !self.recovery_open
            && !self.confirm_engine_upgrade
            && self.confirm_discard.is_none();
        let events = self.keyboard.update(ctx, enabled);
        self.send_notes(events);
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
    fn sync(&mut self) {
        if let Ok(s) = self.shared.try_lock() {
            self.can_undo = s.can_undo();
            self.can_redo = s.can_redo();
            if s.revision() != self.revision {
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
                self.patch = s.patch().clone();
                self.rack.reconcile_selection(&self.patch);
                self.revision = s.revision();
            }
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
        self.browser_focus = true;
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
        if self
            .documents
            .observe(&self.patch, self.revision, &self.rack)
        {
            self.last_document_change = Instant::now();
            self.autosave_needed = true;
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
        } else {
            self.perform_action(action);
        }
    }
    fn replace_from_menu(&mut self, patch: Patch, title: &str) {
        self.request_action(PendingAction::Replace(patch, title.into()));
    }
    fn replace_now(&mut self, patch: Patch, title: &str) {
        if self.command("replace_patch", json!({"patch":patch})) {
            self.panic_notes();
            self.clear_recovery();
            self.title = title.into();
            self.active_file = None;
            self.active_token = None;
            self.rack = Rack::default();
            if title == "Sequenced analog voice" {
                self.sequence_layout();
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
    }
    fn switched_stereo_layout(&mut self) {
        let _ = self.rack.restore_layout(
            &json!({"version":1,"zoom":1.0,"pan":[24,12],"positions":{
                "pitches":[0,0],"split":[360,0],"select":[660,0],"clock":[840,0],
                "vco":[0,420],"route":[210,420],"left":[390,420],"right":[540,420],"out":[690,420]
            }}),
        );
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
            PendingAction::Replace(patch, title) => self.replace_now(patch, &title),
            PendingAction::Load(name) => self.load_document(&name, false),
            PendingAction::Recover(name) => self.load_document(&name, true),
            PendingAction::Close => {
                let sealed = self
                    .shared
                    .try_lock()
                    .ok()
                    .map(|mut live| live.seal_for_close(self.revision));
                if matches!(sealed, Some(Ok(()))) {
                    self.clear_recovery();
                    self.allow_close = true;
                } else {
                    self.deferred_action = Some(PendingAction::Close);
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
        let document = self.capture_document();
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
                self.status = format!("Save failed: {error}. Use Save As to keep a separate copy.")
            }
        }
    }
    fn load_document(&mut self, name: &str, recovery: bool) {
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
                let applied = apply_document(&self.shared, expected_revision, &loaded.document)?;
                Ok((loaded, applied))
            });
        match result {
            Ok((loaded, applied)) => {
                self.history_layouts
                    .insert(self.state_id, self.rack.layout());
                self.patch = applied.patch;
                self.rack = applied.rack;
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
                self.sync();
                self.capture_document();
            }
        }
    }
    fn autosave(&mut self) {
        if self.smoke_test.is_some() || self.allow_close {
            return;
        }
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
        let document = self.capture_document();
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
    fn add_module(&mut self, kind: &str) {
        let id = (1..)
            .map(|i| format!("{kind}-{i}"))
            .find(|id| !self.patch.nodes.iter().any(|node| node.id == *id))
            .unwrap();
        let mut edits = vec![json!({"tool":"add_module","arguments":{"id":id,"kind":kind}})];
        if matches!(kind, "output" | "audio_out" | "audio_stereo") && self.patch.output.is_none() {
            edits.push(json!({"tool":"set_output","arguments":{"id":id,"port":"out"}}));
        }
        if self.command("apply_edits", json!({"edits":edits})) {
            self.rack.select_only(id);
            self.browser_open = false;
        }
    }
    fn duplicate_modules(&mut self, ids: &[String], with_inputs: bool) -> bool {
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
            json!({"modules":modules,"cable_mode":if with_inputs{"inputs"}else{"none"},"expected_revision":self.revision}),
        ) {
            Ok(receipt) => receipt,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        self.install_duplicate_receipt(&receipt, placement, base_state_id, base_layout)
            .expect("shared duplicate command returns a committed graph snapshot");
        self.sync();
        self.status = format!(
            "Duplicated {} module{}{}",
            ids.len(),
            if ids.len() == 1 { "" } else { "s" },
            if with_inputs { " with inputs" } else { "" }
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
    ) -> Result<(), String> {
        let patch: Patch =
            serde_json::from_value(receipt["patch"].clone()).map_err(|error| error.to_string())?;
        let revision = receipt["revision"]
            .as_u64()
            .ok_or("duplicate receipt has no revision")?;
        let state_id = receipt["state_id"]
            .as_u64()
            .ok_or("duplicate receipt has no state identity")?;
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
        self.rack.apply_duplicate_placement(placement);
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
    fn toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu")
            .frame(
                egui::Frame::new()
                    .fill(CHROME)
                    .inner_margin(egui::Margin::symmetric(12, 7)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("OPENPATCH").strong().size(14.0));
                    ui.add_space(10.0);
                    ui.menu_button("File", |ui| {
                        if ui.button("New patch").clicked() {
                            self.replace_from_menu(Patch::new_v2(), "Untitled");
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
                            self.replace_from_menu(openpatch_core::demo_patch(), "Tremolo study");
                            ui.close();
                        }
                    });
                    ui.menu_button("Edit", |ui| {
                        if ui
                            .add_enabled(self.can_undo, egui::Button::new("Undo patch edit   ⌘Z"))
                            .clicked()
                        {
                            self.history(false);
                            ui.close();
                        }
                        if ui
                            .add_enabled(self.can_redo, egui::Button::new("Redo patch edit   ⇧⌘Z"))
                            .clicked()
                        {
                            self.history(true);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Add module…   Enter").clicked() {
                            self.browse();
                            ui.close();
                        }
                    });
                    ui.menu_button("View", |ui| {
                        ui.checkbox(&mut self.visuals_open, "Visualizers");
                        ui.checkbox(&mut self.scope.open, "Signal scope");
                        ui.checkbox(&mut self.keyboard_open, "Performance keyboard");
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
                    if ui.button("+  Add module").clicked() {
                        self.browse();
                    }
                    ui.toggle_value(&mut self.visuals_open, "Visuals");
                    ui.toggle_value(&mut self.scope.open, "Scope");
                    if ui.button("Audio / MIDI").clicked() {
                        self.settings_open = !self.settings_open;
                        if self.settings_open {
                            self.refresh_devices();
                        }
                    }
                    ui.add_space(10.0);
                    let mut zoom = self.rack.zoom() * 100.0;
                    ui.label(RichText::new("Zoom").color(MUTED));
                    if ui
                        .add(
                            egui::DragValue::new(&mut zoom)
                                .range(40.0..=180.0)
                                .suffix("%")
                                .speed(1.0),
                        )
                        .changed()
                    {
                        self.rack.set_zoom(zoom / 100.0);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(2.0);
                        if ui
                            .add(egui::Slider::new(&mut self.volume, 0.0..=1.0).show_value(false))
                            .on_hover_text("Master volume")
                            .changed()
                        {
                            if let Some(audio) = &self.audio {
                                audio.set_volume(self.volume);
                            }
                        }
                        ui.label(RichText::new("Level").size(11.0).color(MUTED));
                        let playing = self.audio.as_ref().is_some_and(Audio::is_playing);
                        if ui
                            .add_enabled(
                                self.audio.is_some(),
                                egui::Button::new(if playing { "Pause" } else { "Play" })
                                    .fill(if playing {
                                        Color32::from_rgb(63, 99, 87)
                                    } else {
                                        Color32::from_rgb(59, 66, 72)
                                    })
                                    .min_size(Vec2::new(65.0, 24.0)),
                            )
                            .clicked()
                        {
                            self.toggle_play();
                        }
                    });
                });
            });
    }
    fn footer(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(CHROME)
                    .inner_margin(egui::Margin::symmetric(12, 5)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let dirty = if self.dirty() { " *" } else { "" };
                    ui.label(RichText::new(format!("{}{}", self.title, dirty)).size(11.0));
                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "{} modules · {} cables · {}",
                            self.patch.nodes.len(),
                            self.patch.cables.len(),
                            if self.patch.version == 2 { "Poly engine" } else { "Legacy engine" }
                        ))
                        .size(10.0)
                        .color(MUTED),
                    );
                    if self.rack.selection_count() > 1 {
                        ui.separator();
                        ui.label(
                            RichText::new(format!("{} selected", self.rack.selection_count()))
                                .size(10.0)
                                .color(ACCENT),
                        )
                        .on_hover_text("Shift-click headers to select modules. Cmd/Ctrl+D duplicates the selection; add Shift to include incoming cables.");
                    }
                    ui.separator();
                    ui.add(
                        egui::Label::new(RichText::new(&self.status).size(10.0).color(ACCENT))
                            .truncate(),
                    )
                    .on_hover_text(&self.status);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.audio.is_none() {
                            if ui
                                .button("Retry audio")
                                .on_hover_text(&self.device)
                                .clicked()
                            {
                                self.reopen_audio(self.output_device.clone());
                            }
                        } else if let Some(audio) = &self.audio {
                            if audio.solver_failures() != 0 {
                                ui.colored_label(Color32::from_rgb(235, 167, 85), format!("Solver holds: {}", audio.solver_failures()))
                                    .on_hover_text("A circuit solve did not converge within its bounded work limit. The last valid capacitor state was retained. Reduce drive, resonance, or modulation.");
                            }
                            ui.label(
                                RichText::new(format!("{} Hz", audio.sample_rate))
                                    .size(10.0)
                                    .color(MUTED),
                            )
                            .on_hover_text(&self.device);
                            let peak = self.audio.as_ref().map_or(0.0, Audio::peak);
                            ui.add(
                                egui::ProgressBar::new(peak.min(1.0))
                                    .desired_width(65.0)
                                    .desired_height(5.0)
                                    .fill(ACCENT),
                            );
                        }
                        if self.mcp {
                            ui.label(RichText::new("MCP").color(ACCENT).size(10.0));
                        }
                    });
                });
            });
    }
    fn visual_dock(&mut self, ctx: &egui::Context) {
        if !self.visuals_open {
            return;
        }
        egui::TopBottomPanel::bottom("visualizers")
            .default_height(200.0)
            .height_range(130.0..=500.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(20, 23, 29))
                    .inner_margin(10),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("VISUALIZERS").strong().size(11.0));
                    for (mode, name) in [(0, "Aurora"), (1, "Spectrum"), (2, "Scope")] {
                        ui.selectable_value(&mut self.visual_mode, mode, name);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Hide").clicked() {
                            self.visuals_open = false;
                        }
                        if ui.button("Edit GLSL").clicked() {
                            self.shader_open = true;
                        }
                    });
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
            });
    }
    fn browser(&mut self, ctx: &egui::Context) {
        if !self.browser_open {
            return;
        }
        let mut open = true;
        let mut chosen: Option<String> = None;
        let modules = self.catalog["modules"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        egui::Window::new("Add a module")
            .open(&mut open)
            .default_size([800.0, 560.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .show(ctx, |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search names, models, controls, or inputs")
                        .desired_width(f32::INFINITY),
                );
                if self.browser_focus {
                    response.request_focus();
                    self.browser_focus = false;
                }
                ui.label(
                    RichText::new("OpenPatch built-in modules")
                        .color(MUTED)
                        .size(11.0),
                );
                if self.patch.version == 1 {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Legacy timing: polyphonic modules require engine v2.");
                        if ui.button("Enable polyphony…").clicked() {
                            self.confirm_engine_upgrade = true;
                            self.browser_open = false;
                        }
                    });
                }
                let query = self.search.trim().to_lowercase();
                let visible: Vec<_> = modules
                    .iter()
                    .filter(|module| {
                        module["min_patch_version"].as_u64().unwrap_or(1)
                            <= self.patch.version as u64
                    })
                    .filter(|module| {
                        let kind = module["kind"].as_str().unwrap_or("");
                        let title = module["title"].as_str().unwrap_or(kind);
                        let description = module["description"].as_str().unwrap_or("");
                        let model = module["model"].as_str().unwrap_or("legacy");
                        let inputs = module["inputs"].to_string();
                        let params = module["params"]
                            .as_object()
                            .map(|params| params.keys().cloned().collect::<Vec<_>>().join(" "))
                            .unwrap_or_default();
                        let tags = module["tags"].to_string();
                        format!("{kind} {title} {description} {model} {inputs} {params} {tags}")
                            .to_lowercase()
                            .contains(&query)
                    })
                    .collect();
                let mut categories: Vec<String> = ["circuit-derived", "idealized", "legacy"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect();
                for module in &visible {
                    let category = module["model"].as_str().unwrap_or("legacy");
                    if !categories.iter().any(|item| item == category) {
                        categories.push(category.into());
                    }
                }
                egui::ScrollArea::vertical()
                    .max_height(440.0)
                    .show(ui, |ui| {
                        if visible.is_empty() {
                            ui.add_space(20.0);
                            ui.label("No modules match this search.");
                        }
                        for category in &categories {
                            let members: Vec<_> = visible
                                .iter()
                                .filter(|module| {
                                    module["model"].as_str().unwrap_or("legacy") == category
                                })
                                .collect();
                            if members.is_empty() {
                                continue;
                            }
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new(match category.as_str() {
                                    "circuit-derived" => "Circuit-derived models",
                                    "idealized" => "Idealized utilities",
                                    "legacy" => "Legacy modules",
                                    "functional" => "Voltage utilities",
                                    other => other,
                                })
                                .strong()
                                .size(12.0),
                            );
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                for module in members {
                                    let Some(kind) = module["kind"].as_str() else {
                                        continue;
                                    };
                                    let title = module["title"].as_str().unwrap_or(kind);
                                    let desc = module["description"]
                                        .as_str()
                                        .unwrap_or("Built-in OpenPatch module");
                                    let (rect, response) = ui.allocate_exact_size(
                                        Vec2::new(178.0, 212.0),
                                        egui::Sense::click(),
                                    );
                                    let p = ui.painter();
                                    p.rect_filled(
                                        rect,
                                        2.0,
                                        if response.hovered() {
                                            Color32::from_rgb(229, 231, 224)
                                        } else {
                                            Color32::from_rgb(204, 207, 200)
                                        },
                                    );
                                    p.rect_stroke(
                                        rect,
                                        2.0,
                                        egui::Stroke::new(1.0_f32, Color32::from_gray(90)),
                                        egui::StrokeKind::Inside,
                                    );
                                    let mut title_job = egui::text::LayoutJob::simple(
                                        title.into(),
                                        egui::FontId::proportional(15.0),
                                        Color32::from_gray(31),
                                        rect.width() - 18.0,
                                    );
                                    title_job.wrap.max_rows = 1;
                                    let title_galley = p.layout_job(title_job);
                                    p.galley(
                                        rect.center_top()
                                            + Vec2::new(-title_galley.size().x * 0.5, 14.0),
                                        title_galley,
                                        Color32::from_gray(31),
                                    );
                                    p.text(
                                        rect.center_top() + Vec2::new(0.0, 41.0),
                                        egui::Align2::CENTER_CENTER,
                                        category,
                                        egui::FontId::monospace(9.0),
                                        Color32::from_rgb(63, 89, 83),
                                    );
                                    let knob = rect.center_top() + Vec2::new(0.0, 82.0);
                                    p.circle_filled(
                                        knob + Vec2::new(1.0, 2.0),
                                        23.0,
                                        Color32::from_black_alpha(50),
                                    );
                                    p.circle_filled(knob, 22.0, Color32::from_gray(41));
                                    p.circle_stroke(
                                        knob,
                                        20.0,
                                        egui::Stroke::new(1.5_f32, Color32::from_gray(74)),
                                    );
                                    p.line_segment(
                                        [knob - Vec2::new(0.0, 7.0), knob - Vec2::new(0.0, 17.0)],
                                        egui::Stroke::new(2.0_f32, Color32::from_gray(230)),
                                    );
                                    let mut description_job = egui::text::LayoutJob::simple(
                                        desc.into(),
                                        egui::FontId::proportional(11.0),
                                        Color32::from_gray(53),
                                        rect.width() - 22.0,
                                    );
                                    description_job.wrap.max_rows = 4;
                                    p.galley(
                                        rect.left_top() + Vec2::new(11.0, 117.0),
                                        p.layout_job(description_job),
                                        Color32::from_gray(53),
                                    );
                                    p.text(
                                        rect.center_bottom() - Vec2::new(0.0, 16.0),
                                        egui::Align2::CENTER_CENTER,
                                        kind,
                                        egui::FontId::monospace(9.0),
                                        Color32::from_gray(86),
                                    );
                                    response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            true,
                                            format!("Add {title}, {category}. {desc}"),
                                        )
                                    });
                                    if response.clicked() {
                                        chosen = Some(kind.into());
                                    }
                                    response.on_hover_text(format!("{title}\n{category}\n{desc}"));
                                }
                            });
                        }
                    });
            });
        self.browser_open = open;
        if let Some(kind) = chosen {
            self.add_module(&kind);
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
            ui.label("Your current project has unsaved changes to its modules or rack layout.");
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
                if let Some(action) = self.confirm_discard.take() {
                    self.perform_action(action);
                }
            }
            3 => {
                self.confirm_discard = None;
                self.after_save = None;
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
    fn smoke(&mut self, ctx: &egui::Context) {
        let Some((path, frame, captures)) = &mut self.smoke_test else {
            return;
        };
        *frame += 1;
        if *frame == 1 {
            let v = self
                .visualizer
                .as_mut()
                .expect("Native shader must compile");
            let gl = self.gl.as_ref().expect("GL context required");
            assert!(v.compile(gl, "this is not GLSL").is_err());
            v.compile(gl, DEFAULT_SHADER).expect("Shader recovery");
        }
        self.visuals_open = (*frame >= 2 && *frame <= 8) || *captures == 1;
        self.visual_mode = match *frame {
            2..=4 => 1,
            5..=7 => 2,
            _ => 0,
        };
        if *frame == 28 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/clocked-random-voice.json"))
                    .expect("valid bundled random voice");
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("random voice compiles");
            self.rack = Rack::default();
            self.rack.set_zoom(0.75);
            self.title = "Clocked random voice".into();
        }
        if *frame == 40 || *frame == 52 {
            let (source, title, zoom) = if *frame == 40 {
                (
                    include_str!("../../../examples/polyphonic-voice.json"),
                    "Polyphonic voice",
                    0.75,
                )
            } else {
                (
                    include_str!("../../../examples/polyphonic-routing.json"),
                    "Polyphonic routing study",
                    1.0,
                )
            };
            let patch: Patch =
                serde_json::from_str(source).expect("valid bundled polyphonic patch");
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("polyphonic patch compiles");
            self.rack = Rack::default();
            self.rack.set_zoom(zoom);
            self.title = title.into();
        }
        if *frame == 64 {
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
        if *frame == 76 {
            self.settings_open = true;
        }
        if *frame == 88 {
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
        if *frame == 100 {
            let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
                "nodes":[{"id":"mix","kind":"mix6"},{"id":"octave","kind":"octave"},
                    {"id":"mult","kind":"mult"},{"id":"stereo","kind":"mid_side"}]}))
            .unwrap();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("routing modules compile");
            self.rack = Rack::default();
            self.title = "Stereo and routing utilities".into();
        }
        if *frame == 112 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/clocked-modulation.json"))
                    .expect("valid bundled modulation voice");
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("modulation voice compiles");
            self.rack = Rack::default();
            self.rack.set_zoom(0.75);
            self.title = "Clocked modulation".into();
        }
        if *frame == 124 {
            self.file_mode = Some(FileMode::Save);
            self.file_focus = true;
            self.filename = "clocked-modulation.openpatch".into();
        }
        if *frame == 136 {
            self.file_mode = None;
            self.confirm_discard = Some(PendingAction::Close);
        }
        if *frame == 148 {
            self.confirm_discard = None;
            let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,"nodes":[{"id":"calibration","kind":"multi_lfo","params":{"frequency":100.0}}],"output":"calibration","output_port":"sine"})).unwrap();
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("scope calibration compiles");
            self.rack = Rack::default();
            self.title = "Signal inspection".into();
            self.scope.smoke_fixture();
        }
        if *frame == 172 {
            self.scope.open = false;
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
        if *frame == 188 {
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
        if *frame == 204 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/voltage-utilities.json"))
                    .expect("valid voltage utilities fixture");
            execute_shared(&self.shared, "replace_patch", json!({"patch":patch}))
                .expect("voltage utilities compile");
            self.rack = Rack::default();
            self.title = "Voltage utilities".into();
        }
        if *frame == 220 {
            execute_shared(
                &self.shared,
                "set_bypass",
                json!({"id":"mutes","bypassed":true}),
            )
            .expect("mute bypass fixture");
            self.title = "Voltage utilities · Mutes bypassed".into();
        }
        if *frame == 244 || *frame == 260 {
            let patch: Patch =
                serde_json::from_str(include_str!("../../../examples/gate-shaping-voice.json"))
                    .expect("valid gate shaping fixture");
            let mut engine = patch.compile().expect("gate shaping fixture compiles");
            engine.process_block(&mut vec![0.0; 48_000]);
            if *frame == 260 {
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
            self.title = if *frame == 260 {
                "Manual gates · offline DSP snapshot"
            } else {
                "Gate shaping voice · offline preview"
            }
            .into();
        }
        if *frame == 284 {
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
        if *frame == 308 {
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
            self.rack.apply_duplicate_placement(placement);
            self.title = "Duplicated voice pair · two selected".into();
        }
        if *frame == 332 {
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
        if *frame == 356 {
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
        if *frame == 160 {
            self.scope.smoke_xy();
        }
        if matches!(
            *frame,
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
            let target = match *captures {
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
                _ => path.with_file_name("rack-multi-vco.png"),
            };
            image::save_buffer(
                target,
                image.as_raw(),
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
            )
            .expect("Save screenshot");
            *captures += 1;
            if *captures == 24 {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        assert!(*frame < 394, "Native screenshot timeout");
    }
}
impl eframe::App for OpenPatch {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.smoke_test.is_some() {
            ctx.style_mut(|style| style.interaction.tooltip_delay = f32::INFINITY);
        }
        self.sync();
        self.observe_document();
        if !ctx.wants_keyboard_input()
            && !self.confirm_engine_upgrade
            && !self.recovery_open
            && self.file_mode.is_none()
            && self.confirm_discard.is_none()
        {
            if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                self.toggle_play();
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
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
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O)) {
                self.file_dialog(FileMode::Open);
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) {
                self.history(ctx.input(|i| i.modifiers.shift));
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Num0)) {
                self.rack.reset_view();
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.browser_open = false;
        }
        if let Some(action) = self.deferred_action.take() {
            self.request_action(action);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_action(PendingAction::Close);
        }
        self.toolbar(ctx);
        self.footer(ctx);
        self.performance_keyboard(ctx);
        self.visual_dock(ctx);
        self.scope
            .show(ctx, &self.patch, &self.catalog, self.audio.as_mut());
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
            if let Some(audio) = self.audio.as_ref() {
                for node in &self.patch.nodes {
                    if let Some(state) = audio.module_lights(&node.id) {
                        lights.insert(node.id.clone(), state);
                    }
                    if let Some(state) = audio.module_control_state(&node.id) {
                        controls.insert(node.id.clone(), state);
                    }
                }
            }
            self.rack.set_module_states(lights, controls);
        }
        let mut action = None;
        self.rack
            .set_shortcuts_enabled(!self.browser_open && !self.settings_open && !self.shader_open);
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
                    && !self.allow_close;
                ui.add_enabled_ui(interactive, |ui| {
                    action = self
                        .rack
                        .show(ui, &self.patch, &self.catalog, rate, self.revision);
                });
            });
        match action {
            Some(RackAction::Command(name, args)) => {
                self.command(name, args);
            }
            Some(RackAction::Duplicate { ids, with_inputs }) => {
                self.duplicate_modules(&ids, with_inputs);
            }
            Some(RackAction::OpenBrowser) => self.browse(),
            Some(RackAction::Inspect { lane, id, port }) => self.scope.inspect(lane, id, port),
            Some(RackAction::LayoutChanged) => {
                self.status = "Module moved".into();
            }
            None => {}
        }
        self.observe_document();
        self.browser(ctx);
        self.files_window(ctx);
        self.shader_window(ctx);
        self.audio_settings(ctx);
        self.recovery_window(ctx);
        self.unsaved_window(ctx);
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
        self.smoke(ctx);
        ctx.request_repaint_after(Duration::from_millis(33));
    }
    fn on_exit(&mut self, gl: Option<&glow::Context>) {
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
            keyboard: Keyboard::default(),
            keyboard_open: false,
            rack,
            visualizer: None,
            gl: None,
            shader: DEFAULT_SHADER.into(),
            shader_open: false,
            shader_status: String::new(),
            visuals_open: false,
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
            browser_focus: false,
            search: String::new(),
            catalog: catalog(),
            start: Instant::now(),
            mcp: true,
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
        app.add_module("constant");
        assert_eq!(counter.0.load(Ordering::Relaxed), 0);
        assert_eq!(app.revision, revision + 1);
        assert_eq!(app.patch.nodes.len(), 3);
        assert_eq!(app.rack.selected.as_deref(), Some("constant-1"));
        app.add_module("audio_stereo");
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
        assert!(app.duplicate_modules(&["tone".into(), "filter".into()], true));
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
        assert!(!app.duplicate_modules(&["tone".into()], false));
        assert_eq!(app.rack.layout(), layout);
        assert_eq!(app.rack.selected_ids(), selection);
        assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
        app.sync();
        let full = Patch {
            nodes: (0..openpatch_core::MAX_NODES)
                .map(|i| openpatch_core::Node {
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
        assert!(!app.duplicate_modules(&["node0".into()], false));
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
        assert!(app.duplicate_modules(&sources, false));
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
        app.install_duplicate_receipt(&receipt, placement, base_state, before.layout.clone())
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
}
