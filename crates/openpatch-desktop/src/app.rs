use crate::{
    audio::Audio,
    keyboard::Keyboard,
    midi::{MidiInputManager, MidiPortInfo},
    rack::{Rack, RackAction},
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
    volume: f32,
    status: String,
    filename: String,
    title: String,
    files: Vec<String>,
    workspace: String,
    file_mode: Option<FileMode>,
    browser_open: bool,
    browser_focus: bool,
    search: String,
    catalog: Value,
    start: Instant,
    mcp: bool,
    saved_revision: u64,
    can_undo: bool,
    can_redo: bool,
    confirm_close: bool,
    confirm_engine_upgrade: bool,
    confirm_discard: Option<PendingAction>,
    layout_dirty: bool,
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
        let audio_revision = audio.as_ref().map(|_| revision);
        let volume = audio.as_ref().map_or(0.65, Audio::volume);
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
            rack: Rack::default(),
            visualizer,
            gl: cc.gl.clone(),
            shader: DEFAULT_SHADER.into(),
            shader_open: false,
            shader_status,
            visuals_open: false,
            visual_mode: 0,
            volume,
            status: "Ready — press Play to hear the patch".into(),
            filename: "my-patch.json".into(),
            title: "Reference analog voice".into(),
            files: vec![],
            workspace,
            file_mode: None,
            browser_open: false,
            browser_focus: false,
            search: String::new(),
            catalog: catalog(),
            start: Instant::now(),
            mcp,
            saved_revision: revision,
            can_undo: false,
            can_redo: false,
            confirm_close: false,
            confirm_engine_upgrade: false,
            confirm_discard: None,
            layout_dirty: false,
            allow_close: false,
            smoke_test: smoke_test.map(|path| (path, 0, 0)),
        };
        app.refresh_files();
        app.refresh_devices();
        app.connect_selected_midi();
        app
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
            self.audio_revision = Some(self.revision);
            self.status = "Audio ready — press Play".into();
            self.refresh_devices();
            self.connect_selected_midi();
            self.save_host_settings();
        } else {
            self.settings_status = label;
        }
    }
    fn send_notes(&mut self, events: Vec<MidiEvent>) {
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
            && !self.confirm_close
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
                name.ends_with(".json") && !name.starts_with('.') && !name.ends_with(".layout.json")
            })
            .collect();
        self.files.sort();
    }
    fn command(&mut self, name: &str, mut args: Value) -> bool {
        if !matches!(name, "save_patch" | "render_audio") {
            args["expected_revision"] = json!(self.revision);
        }
        match execute_shared(&self.shared, name, args) {
            Ok(result) => {
                self.status = match name {
                    "save_patch" => {
                        self.saved_revision = result["revision"].as_u64().unwrap_or(self.revision);
                        self.title = self.filename.trim_end_matches(".json").into();
                        self.refresh_files();
                        format!("Saved {}", self.filename)
                    }
                    "load_patch" => {
                        self.saved_revision = result["revision"].as_u64().unwrap_or(self.revision);
                        self.title = self.filename.trim_end_matches(".json").into();
                        format!("Loaded {}", self.filename)
                    }
                    _ => "Patch updated".into(),
                };
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
                match audio.submit_patch(&self.patch) {
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
        self.file_mode = Some(mode);
    }
    fn dirty(&self) -> bool {
        self.revision != self.saved_revision || self.layout_dirty
    }
    fn replace_from_menu(&mut self, patch: Patch, title: &str) {
        if self.dirty() {
            self.confirm_discard = Some(PendingAction::Replace(patch, title.into()));
            return;
        }
        self.replace_now(patch, title);
    }
    fn replace_now(&mut self, patch: Patch, title: &str) {
        self.panic_notes();
        if self.command("replace_patch", json!({"patch":patch})) {
            self.title = title.into();
            self.rack = Rack::default();
            self.layout_dirty = false;
        }
    }
    fn add_module(&mut self, kind: &str) {
        let id = (1..)
            .map(|i| format!("{kind}-{i}"))
            .find(|id| !self.patch.nodes.iter().any(|node| node.id == *id))
            .unwrap();
        let mut patch = self.patch.clone();
        patch.nodes.push(openpatch_core::Node {
            id: id.clone(),
            kind: kind.into(),
            params: Default::default(),
        });
        if matches!(kind, "output" | "audio_out" | "audio_stereo") && patch.output.is_none() {
            patch.output = Some(id.clone());
        }
        if self.command("replace_patch", json!({"patch":patch})) {
            self.rack.selected = Some(id);
            self.browser_open = false;
        }
    }
    fn save_or_load(&mut self, mode: FileMode, discard: bool) {
        if mode == FileMode::Open && self.dirty() && !discard {
            self.confirm_discard = Some(PendingAction::Load(self.filename.clone()));
            self.file_mode = None;
            return;
        }
        let op = if mode == FileMode::Save {
            "save_patch"
        } else {
            "load_patch"
        };
        if mode == FileMode::Open {
            self.panic_notes();
        }
        if !self.command(op, json!({"filename":self.filename})) {
            return;
        }
        let layout_path =
            PathBuf::from(&self.workspace).join(format!("{}.layout.json", self.filename));
        if mode == FileMode::Save {
            // Patch save already validated this basename. Layout writes never follow symlinks or overwrite files.
            let result = (|| -> Result<(), String> {
                let data =
                    serde_json::to_vec_pretty(&self.rack.layout()).map_err(|e| e.to_string())?;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(layout_path)
                    .map_err(|e| e.to_string())?;
                file.write_all(&data).map_err(|e| e.to_string())
            })();
            if let Err(e) = result {
                self.status = format!("Patch saved; layout could not be saved: {e}");
                self.layout_dirty = true;
            } else {
                self.layout_dirty = false;
            }
        } else {
            self.layout_dirty = false;
            self.rack = Rack::default();
            if let Ok(metadata) = std::fs::symlink_metadata(&layout_path) {
                if metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && metadata.len() <= 1_048_576
                {
                    if let Ok(bytes) = std::fs::read(layout_path) {
                        if let Ok(layout) = serde_json::from_slice(&bytes) {
                            if let Err(e) = self.rack.restore_layout(&layout) {
                                self.status = format!("Patch loaded; layout ignored: {e}");
                            }
                        }
                    }
                }
            }
        }
        self.file_mode = None;
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
                        if ui.button("Save as…  ⌘S").clicked() {
                            self.file_dialog(FileMode::Save);
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
                        format!("{kind} {title} {description} {model} {inputs} {params}")
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
        let mut open = true;
        let mut run = false;
        egui::Window::new(if mode == FileMode::Save {
            "Save patch as"
        } else {
            "Open patch"
        })
        .open(&mut open)
        .default_width(460.0)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label(RichText::new(&self.workspace).color(MUTED).size(11.0));
            ui.horizontal(|ui| {
                ui.label("Filename");
                ui.add(egui::TextEdit::singleline(&mut self.filename).desired_width(330.0));
            });
            if mode == FileMode::Open {
                egui::ScrollArea::vertical()
                    .max_height(250.0)
                    .show(ui, |ui| {
                        for file in &self.files {
                            let r = ui.selectable_label(self.filename == *file, file);
                            if r.clicked() {
                                self.filename = file.clone();
                            }
                            if r.double_clicked() {
                                self.filename = file.clone();
                                run = true;
                            }
                        }
                        if self.files.is_empty() {
                            ui.label("No saved patches yet.");
                        }
                    });
            } else {
                ui.label(
                    RichText::new(
                        "Choose a new filename ending in .json. Existing patches are kept.",
                    )
                    .size(11.0)
                    .color(MUTED),
                );
            }
            ui.separator();
            ui.label(&self.status);
            if ui
                .button(if mode == FileMode::Save {
                    "Save patch"
                } else {
                    "Open patch"
                })
                .clicked()
            {
                run = true;
            }
        });
        if !open {
            self.file_mode = None;
        } else if run {
            self.save_or_load(mode, false);
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
        if matches!(*frame, 12 | 24 | 36 | 48 | 60 | 72 | 84) {
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
                _ => path.with_file_name("audio-midi-settings.png"),
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
            if *captures == 7 {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        assert!(*frame < 180, "Native screenshot timeout");
    }
}
impl eframe::App for OpenPatch {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sync();
        if !ctx.wants_keyboard_input()
            && !self.confirm_engine_upgrade
            && !self.confirm_close
            && self.confirm_discard.is_none()
        {
            if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                self.toggle_play();
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.browse();
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
                self.file_dialog(FileMode::Save);
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
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty() && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
        self.toolbar(ctx);
        self.footer(ctx);
        self.performance_keyboard(ctx);
        self.visual_dock(ctx);
        let mut action = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(0))
            .show(ctx, |ui| {
                let rate = self
                    .audio
                    .as_ref()
                    .map_or(self.patch.sample_rate, |a| a.sample_rate);
                action = self.rack.show(ui, &self.patch, &self.catalog, rate);
            });
        match action {
            Some(RackAction::Command(name, args)) => {
                self.command(name, args);
            }
            Some(RackAction::OpenBrowser) => self.browse(),
            Some(RackAction::LayoutChanged) => {
                self.layout_dirty = true;
                self.status = "Module moved".into();
            }
            None => {}
        }
        self.browser(ctx);
        self.files_window(ctx);
        self.shader_window(ctx);
        self.audio_settings(ctx);
        if self.confirm_close {
            egui::Window::new("Unsaved patch")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Save your patch before closing, or discard this session's edits.");
                    ui.horizontal(|ui| {
                        if ui.button("Keep working").clicked() {
                            self.confirm_close = false;
                        }
                        if ui.button("Save as…").clicked() {
                            self.confirm_close = false;
                            self.file_dialog(FileMode::Save);
                        }
                        if ui.button("Discard and close").clicked() {
                            self.allow_close = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
        }
        if self.confirm_discard.is_some() {
            let mut decision = None;
            egui::Window::new("Replace unsaved patch?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Your current patch has unsaved edits. Save a copy first if you want to keep them.");
                    ui.horizontal(|ui| {
                        if ui.button("Keep editing").clicked() {
                            decision = Some(false);
                        }
                        if ui.button("Discard edits and continue").clicked() {
                            decision = Some(true);
                        }
                    });
                });
            if let Some(proceed) = decision {
                if let Some(action) = self.confirm_discard.take() {
                    if proceed {
                        match action {
                            PendingAction::Replace(patch, title) => self.replace_now(patch, &title),
                            PendingAction::Load(filename) => {
                                self.filename = filename;
                                self.save_or_load(FileMode::Open, true);
                            }
                        }
                    }
                }
            }
        }
        if self.confirm_engine_upgrade {
            let mut convert = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("engine-upgrade")).show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.heading("Enable polyphony and feedback?");
                ui.label("Engine v2 adds one sample of delay to every cable. Your modules and connections stay in place, but timing and sound can change.");
                ui.label("You can undo this change. The saved file stays unchanged until you save a new copy.");
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
