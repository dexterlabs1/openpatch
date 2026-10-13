use crate::{
    audio::Audio,
    visuals::{Visualizer, DEFAULT_SHADER},
};
use eframe::{
    egui::{self, Color32, FontId, RichText, Stroke, Vec2},
    glow,
};
use openpatch::control::{execute_shared, SharedSession};
use openpatch_core::{catalog, Patch};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const BG: Color32 = Color32::from_rgb(13, 17, 25);
const PANEL: Color32 = Color32::from_rgb(21, 27, 38);
const LINE: Color32 = Color32::from_rgb(44, 54, 70);
const INK: Color32 = Color32::from_rgb(227, 233, 242);
const MUTED: Color32 = Color32::from_rgb(137, 153, 174);
const MINT: Color32 = Color32::from_rgb(89, 224, 187);
const PURPLE: Color32 = Color32::from_rgb(170, 140, 244);

pub struct OpenPatch {
    shared: SharedSession,
    patch: Patch,
    revision: u64,
    audio_revision: Option<u64>,
    audio: Option<Audio>,
    device: String,
    visualizer: Option<Visualizer>,
    gl: Option<std::sync::Arc<glow::Context>>,
    shader: String,
    shader_open: bool,
    shader_status: String,
    visual_mode: i32,
    volume: f32,
    status: String,
    source: Option<String>,
    selected: Option<String>,
    filename: String,
    files: Vec<String>,
    workspace: String,
    catalog: Value,
    start: Instant,
    mcp: bool,
    saved_revision: u64,
    undo_patch: Option<Patch>,
    confirm_close: bool,
    allow_close: bool,
    smoke_test: Option<(std::path::PathBuf, u32)>,
}
impl OpenPatch {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        shared: SharedSession,
        mcp: bool,
        smoke_test: Option<std::path::PathBuf>,
    ) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = BG;
        style.visuals.window_fill = PANEL;
        style.visuals.extreme_bg_color = BG;
        style.visuals.override_text_color = Some(INK);
        style.visuals.selection.bg_fill = Color32::from_rgb(30, 92, 81);
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(33, 43, 58);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 67, 81);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(38, 103, 86);
        style.spacing.item_spacing = Vec2::new(10.0, 10.0);
        style.spacing.button_padding = Vec2::new(12.0, 7.0);
        cc.egui_ctx.set_style(style);
        let (patch, revision, workspace) = {
            let session = shared.lock().unwrap();
            (
                session.patch().clone(),
                session.revision(),
                session.workspace().display().to_string(),
            )
        };
        let (audio, device) = match Audio::new(&patch) {
            Ok(audio) => {
                let device = format!("{} · {} Hz", audio.device_name, audio.sample_rate);
                (Some(audio), device)
            }
            Err(e) => (None, format!("Audio unavailable: {e}")),
        };
        let (visualizer, shader_status) = match cc.gl.as_ref().map(|gl| Visualizer::new(gl)) {
            Some(Ok(v)) => (Some(v), "Native GLSL 330 · ready".into()),
            Some(Err(e)) => (None, e),
            None => (None, "OpenGL is unavailable".into()),
        };
        let audio_revision = audio.as_ref().map(|_| revision);
        let volume = audio.as_ref().map_or(0.65, Audio::volume);
        let mut app = Self {
            shared,
            patch,
            revision,
            audio_revision,
            audio,
            device,
            visualizer,
            gl: cc.gl.clone(),
            shader: DEFAULT_SHADER.into(),
            shader_open: false,
            shader_status,
            visual_mode: 0,
            volume,
            status: "Ready. Press Play to hear the patch.".into(),
            source: None,
            selected: None,
            filename: "my-patch.json".into(),
            files: vec![],
            workspace,
            catalog: catalog(),
            start: Instant::now(),
            mcp,
            saved_revision: revision,
            undo_patch: None,
            confirm_close: false,
            allow_close: false,
            smoke_test: smoke_test.map(|path| (path, 0)),
        };
        app.refresh_files();
        app
    }
    fn refresh_files(&mut self) {
        self.files = std::fs::read_dir(&self.workspace)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.ends_with(".json") && !name.starts_with('.'))
            .collect();
        self.files.sort();
    }
    fn command(&mut self, name: &str, mut args: Value) {
        if !matches!(name, "save_patch" | "render_audio") {
            args["expected_revision"] = json!(self.revision);
        }
        match execute_shared(&self.shared, name, args) {
            Ok(result) => {
                self.status = match name {
                    "save_patch" => {
                        self.saved_revision = result["revision"].as_u64().unwrap_or(self.revision);
                        self.refresh_files();
                        format!("Saved {}", self.filename)
                    }
                    "load_patch" => {
                        self.saved_revision = result["revision"].as_u64().unwrap_or(self.revision);
                        format!("Loaded {}", self.filename)
                    }
                    _ => "Patch updated".into(),
                };
            }
            Err(e) => self.status = e,
        }
        self.sync();
    }
    fn sync(&mut self) {
        if let Ok(session) = self.shared.try_lock() {
            if session.revision() != self.revision {
                self.undo_patch = Some(self.patch.clone());
                self.patch = session.patch().clone();
                self.revision = session.revision();
            }
        }
        let mut audio_failure = None;
        if let Some(audio) = &mut self.audio {
            audio.service();
            if self.audio_revision != Some(self.revision) {
                match audio.submit_patch(&self.patch) {
                    Ok(()) => self.audio_revision = Some(self.revision),
                    Err(e) => {
                        audio.set_playing(false);
                        audio_failure = Some(format!("Audio stopped: {e}"));
                    }
                }
            }
            if audio.take_stream_error() {
                audio.set_playing(false);
                audio_failure =
                    Some("Audio device disconnected or stream failed. Use Retry audio.".into());
            }
        }
        if let Some(error) = audio_failure {
            self.audio = None;
            self.audio_revision = None;
            self.status = error;
        }
    }
    fn add(&mut self, kind: &str) {
        let id = (1..)
            .map(|i| format!("{kind}-{i}"))
            .find(|id| !self.patch.nodes.iter().any(|node| node.id == *id))
            .unwrap();
        self.command("add_module", json!({"id":id,"kind":kind}));
        self.selected = Some(id);
    }
    fn toggle_play(&mut self) {
        if let Some(audio) = &self.audio {
            audio.set_playing(!audio.is_playing());
            self.status = if audio.is_playing() {
                "Playing live audio"
            } else {
                "Playback paused"
            }
            .into();
        }
    }
    fn header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("transport")
            .frame(egui::Frame::new().fill(PANEL).inner_margin(18))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("◈").size(29.0).color(MINT));
                    ui.vertical(|ui| {
                        ui.label(RichText::new("OPENPATCH").size(23.0).strong());
                        ui.label(
                            RichText::new("MODULAR SOUND / MOVING LIGHT")
                                .size(10.0)
                                .color(MUTED),
                        );
                    });
                    ui.add_space(30.0);
                    let playing = self.audio.as_ref().is_some_and(Audio::is_playing);
                    let label = if playing { "Ⅱ  Pause" } else { "▶  Play" };
                    if ui
                        .add_enabled(
                            self.audio.is_some(),
                            egui::Button::new(RichText::new(label).size(17.0).color(BG))
                                .fill(MINT)
                                .min_size(Vec2::new(110.0, 42.0)),
                        )
                        .clicked()
                    {
                        self.toggle_play();
                    }
                    ui.add_space(12.0);
                    ui.label(RichText::new("MASTER").size(10.0).color(MUTED));
                    if ui
                        .add(egui::Slider::new(&mut self.volume, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        if let Some(audio) = &self.audio {
                            audio.set_volume(self.volume);
                        }
                    }
                    ui.label(format!("{}%", (self.volume * 100.0).round()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(if self.mcp {
                                "●  MCP SESSION"
                            } else {
                                "●  NATIVE / 0.2"
                            })
                            .color(MINT)
                            .size(11.0),
                        );
                    });
                });
            });
    }
    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("library").exact_width(218.0).resizable(false).frame(egui::Frame::new().fill(PANEL).inner_margin(18)).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui,|ui| {
                section(ui,"MODULE LIBRARY");
                ui.label(RichText::new("Build your instrument.").color(MUTED).size(12.0));
                ui.add_space(8.0);
                for (kind,name,desc) in [
                    ("oscillator","Oscillator","Sine tone · FM input"),
                    ("lfo","LFO","Slow motion · modulation"),
                    ("gain","VCA","Level + control voltage"),
                    ("lowpass","Low-pass","One-pole tone shaping"),
                    ("mixer","Mixer","Blend two signals"),
                    ("constant","Constant","Steady control voltage"),
                    ("output","Output","Send to your speakers"),
                ] {
                    if ui.add_sized([180.0,29.0],egui::Button::new(format!("+   {name}"))).clicked() { self.add(kind); }
                    ui.label(RichText::new(desc).size(10.0).color(MUTED));
                }
                ui.add_space(14.0); ui.separator(); section(ui,"PATCH FILES");
                ui.add(egui::TextEdit::singleline(&mut self.filename).desired_width(176.0).hint_text("patch.json"));
                ui.horizontal(|ui| {
                    if ui.button("Save new").clicked() { self.command("save_patch",json!({"filename":self.filename})); }
                    if ui.button("Load").clicked() { self.command("load_patch",json!({"filename":self.filename})); }
                });
                egui::ComboBox::from_id_salt("saved-patches").selected_text("Saved patches").width(176.0).show_ui(ui,|ui| {
                    for file in &self.files { ui.selectable_value(&mut self.filename,file.clone(),file); }
                });
                ui.label(RichText::new(&self.workspace).size(10.0).color(MUTED));
                ui.add_space(12.0);
                if ui.button("Load tremolo demo").clicked() { self.command("replace_patch",json!({"patch":openpatch_core::demo_patch()})); }
                if ui.add_enabled(self.undo_patch.is_some(), egui::Button::new("Undo last edit / load")).clicked() {
                    if let Some(patch) = self.undo_patch.take() {
                        self.command("replace_patch", json!({"patch":patch}));
                    }
                }
                ui.add_space(14.0); ui.separator(); section(ui,"PATCHING");
                ui.label(RichText::new("Click OUT, then an input jack to connect. Right-click an input to disconnect. Esc cancels.").color(MUTED).size(11.0));
                if let Some(id) = self.selected.clone() {
                    ui.add_space(8.0); ui.label(RichText::new(&id).color(MINT));
                    if ui.button("Use as audio output").clicked() { self.command("set_output",json!({"id":id})); }
                    if ui.button("Remove module").clicked() { self.command("remove_module",json!({"id":id})); self.selected=None; }
                }
                ui.add_space(12.0);
                ui.label(RichText::new("Space  play / pause").color(MUTED).size(10.0));
            });
        });
    }
    fn visual_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            section(ui, "VISUAL FIELD");
            ui.add_space(16.0);
            for (mode, name) in [(0, "Aurora"), (1, "Spectrum"), (2, "Scope")] {
                ui.selectable_value(&mut self.visual_mode, mode, name);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Edit GLSL").clicked() {
                    self.shader_open = true;
                }
            });
        });
        let wave = self.audio.as_ref().map_or([0.0; 256], Audio::waveform);
        let peak = self.audio.as_ref().map_or(0.0, Audio::peak);
        let size = Vec2::new(
            ui.available_width(),
            (ui.available_height() * 0.38).clamp(150.0, 265.0),
        );
        if let Some(visualizer) = &self.visualizer {
            visualizer.paint(
                ui,
                size,
                self.start.elapsed().as_secs_f32(),
                peak,
                wave,
                self.visual_mode,
            );
        } else {
            ui.colored_label(PURPLE, &self.shader_status);
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("LIVE AUDIO REACTIVE").color(MUTED).size(10.0));
            ui.add_space(12.0);
            ui.add(
                egui::ProgressBar::new(peak.min(1.0))
                    .desired_width(100.0)
                    .desired_height(4.0)
                    .fill(MINT),
            );
            ui.label(
                RichText::new(format!("PEAK  {:.1} dB", 20.0 * peak.max(0.00001).log10()))
                    .color(MUTED)
                    .size(10.0),
            );
        });
    }
    fn rack(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            section(ui, "THE RACK");
            ui.label(
                RichText::new(format!(
                    "{} MODULES  /  {} CABLES",
                    self.patch.nodes.len(),
                    self.patch.cables.len()
                ))
                .size(10.0)
                .color(MUTED),
            );
            if let Some(source) = &self.source {
                ui.colored_label(MINT, format!("Patching from {source} → choose an input"));
            }
        });
        let nodes = self.patch.nodes.clone();
        let cables = self.patch.cables.clone();
        let mut outputs = BTreeMap::new();
        let mut inputs = BTreeMap::new();
        let mut action: Option<(&str, Value)> = None;
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let width = (nodes.len() as f32 * 181.0).max(ui.available_width());
                let (rack_rect, _) =
                    ui.allocate_exact_size(Vec2::new(width, 354.0), egui::Sense::hover());
                let painter = ui.painter().clone();
                for y in [rack_rect.top() + 8.0, rack_rect.top() + 295.0] {
                    painter.line_segment(
                        [
                            egui::pos2(rack_rect.left(), y),
                            egui::pos2(rack_rect.right(), y),
                        ],
                        Stroke::new(4.0_f32, LINE),
                    );
                }
                if nodes.is_empty() {
                    painter.text(
                        rack_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Add a module from the library to begin.",
                        FontId::proportional(16.0),
                        MUTED,
                    );
                }
                for (index, node) in nodes.iter().enumerate() {
                    let rect = egui::Rect::from_min_size(
                        rack_rect.min + Vec2::new(index as f32 * 181.0, 16.0),
                        Vec2::new(171.0, 270.0),
                    );
                    let accent = if node.kind == "lfo" || node.kind == "constant" {
                        PURPLE
                    } else {
                        MINT
                    };
                    painter.rect_filled(rect, 8.0, PANEL);
                    painter.rect_stroke(
                        rect,
                        8.0,
                        Stroke::new(
                            1.0_f32,
                            if self.selected.as_ref() == Some(&node.id) {
                                accent
                            } else {
                                LINE
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    painter.line_segment(
                        [
                            rect.left_top() + Vec2::new(12.0, 0.0),
                            rect.right_top() - Vec2::new(12.0, 0.0),
                        ],
                        Stroke::new(2.0_f32, accent),
                    );
                    for x in [rect.left() + 8.0, rect.right() - 8.0] {
                        for y in [rect.top() + 9.0, rect.bottom() - 9.0] {
                            painter.circle_filled(egui::pos2(x, y), 2.0, LINE);
                        }
                    }
                    let header_rect = egui::Rect::from_min_size(
                        rect.min + Vec2::new(14.0, 13.0),
                        Vec2::new(145.0, 42.0),
                    );
                    if ui
                        .interact(
                            header_rect,
                            ui.id().with((&node.id, "select")),
                            egui::Sense::click(),
                        )
                        .clicked()
                    {
                        self.selected = Some(node.id.clone());
                    }
                    painter.text(
                        header_rect.min,
                        egui::Align2::LEFT_TOP,
                        node.kind.to_uppercase(),
                        FontId::proportional(15.0),
                        accent,
                    );
                    painter.text(
                        header_rect.min + Vec2::new(0.0, 22.0),
                        egui::Align2::LEFT_TOP,
                        &node.id,
                        FontId::monospace(10.0),
                        MUTED,
                    );
                    let module = self.catalog["modules"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|v| v["kind"] == node.kind)
                        .unwrap();
                    let content = egui::Rect::from_min_max(
                        rect.min + Vec2::new(14.0, 64.0),
                        rect.max - Vec2::new(14.0, 80.0),
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new().max_rect(content).id_salt(&node.id),
                        |ui| {
                            ui.set_width(143.0);
                            ui.spacing_mut().item_spacing.y = 4.0;
                            ui.spacing_mut().slider_width = 130.0;
                            for (name, spec) in module["params"].as_object().unwrap() {
                                let mut value = node
                                    .params
                                    .get(name)
                                    .copied()
                                    .unwrap_or(spec["default"].as_f64().unwrap() as f32);
                                let min = spec["min"].as_f64().unwrap() as f32;
                                let mut max = spec["max"].as_f64().unwrap() as f32;
                                if let Some(ratio) = spec["sample_rate_max_ratio"].as_f64() {
                                    max = max.min(
                                        self.patch.sample_rate.min(
                                            self.audio
                                                .as_ref()
                                                .map_or(self.patch.sample_rate, |a| a.sample_rate),
                                        ) as f32
                                            * ratio as f32,
                                    );
                                }
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(name.to_uppercase()).size(9.0).color(MUTED),
                                    );
                                    ui.label(RichText::new(format!("{value:.2}")).size(10.0));
                                });
                                let slider = egui::Slider::new(&mut value, min..=max)
                                    .show_value(false)
                                    .logarithmic(name == "frequency" || name == "cutoff");
                                if ui.add(slider).changed() {
                                    action = Some((
                                        "set_parameter",
                                        json!({"id":node.id,"parameter":name,"value":value}),
                                    ));
                                }
                            }
                            if node.kind == "output" {
                                ui.label(RichText::new("MONO → DEVICE").color(MUTED).size(10.0));
                                if self.patch.output.as_ref() == Some(&node.id) {
                                    ui.colored_label(MINT, "●  Master output");
                                }
                            }
                            if node.kind == "mixer" {
                                ui.label(RichText::new("A + B").color(MUTED).size(24.0));
                            }
                        },
                    );
                    let ports = module["inputs"].as_array().unwrap();
                    for (p, port) in ports.iter().enumerate() {
                        let name = port.as_str().unwrap();
                        let pos = egui::pos2(
                            rect.left() + rect.width() * (p + 1) as f32 / (ports.len() + 1) as f32,
                            rect.bottom() - 64.0,
                        );
                        inputs.insert((node.id.clone(), name.to_string()), pos);
                        let connected = cables.iter().any(|c| c.to == node.id && c.input == name);
                        let response = jack(
                            ui,
                            &painter,
                            pos,
                            ui.id().with((&node.id, name)),
                            name,
                            if connected { accent } else { MUTED },
                        );
                        if response.clicked() {
                            self.selected = Some(node.id.clone());
                            if let Some(source) = self.source.take() {
                                action = Some((
                                    "connect",
                                    json!({"from":source,"to":node.id,"input":name}),
                                ));
                            }
                        }
                        if response.secondary_clicked() && connected {
                            action = Some(("disconnect", json!({"to":node.id,"input":name})));
                        }
                    }
                    let pos = egui::pos2(rect.center().x, rect.bottom() - 27.0);
                    outputs.insert(node.id.clone(), pos);
                    if jack(
                        ui,
                        &painter,
                        pos,
                        ui.id().with((&node.id, "out")),
                        "OUT",
                        accent,
                    )
                    .clicked()
                    {
                        self.source = Some(node.id.clone());
                        self.selected = Some(node.id.clone());
                    }
                }
                for (i, cable) in cables.iter().enumerate() {
                    if let (Some(&from), Some(&to)) = (
                        outputs.get(&cable.from),
                        inputs.get(&(cable.to.clone(), cable.input.clone())),
                    ) {
                        let color = if i % 2 == 0 { MINT } else { PURPLE };
                        let depth = rack_rect.top() + 315.0 + (i % 4) as f32 * 9.0;
                        let points = [from, egui::pos2(from.x, depth), egui::pos2(to.x, depth), to];
                        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                            points,
                            false,
                            Color32::TRANSPARENT,
                            Stroke::new(6.0_f32, Color32::from_black_alpha(80)),
                        ));
                        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                            points,
                            false,
                            Color32::TRANSPARENT,
                            Stroke::new(2.0_f32, color.gamma_multiply(0.75)),
                        ));
                    }
                }
                if let Some(source) = &self.source {
                    if let (Some(&from), Some(pointer)) =
                        (outputs.get(source), ui.input(|i| i.pointer.hover_pos()))
                    {
                        painter.line_segment([from, pointer], Stroke::new(1.0_f32, MINT));
                    }
                }
            });
        if let Some((name, args)) = action {
            self.command(name, args);
        }
    }
    fn shader_window(&mut self, ctx: &egui::Context) {
        let mut open = self.shader_open;
        egui::Window::new("Native GLSL studio").open(&mut open).default_size([730.0,540.0]).show(ctx,|ui| {
            ui.label("GLSL 330 core · u_time · u_resolution · u_level · u_wave[256] · u_spectrum[64]");
            ui.label(RichText::new("Shaders run on your GPU. Compile errors keep the previous visual running.").color(MUTED));
            ui.horizontal(|ui| {
                if ui.button("Compile shader").clicked() { if let (Some(v),Some(gl))=(&mut self.visualizer,&self.gl) { self.shader_status=match v.compile(gl,&self.shader) {Ok(())=>"Shader compiled successfully".into(),Err(e)=>e}; } }
                if ui.button("Restore source").clicked() { self.shader=DEFAULT_SHADER.into(); }
            });
            ui.label(&self.shader_status);
            egui::ScrollArea::both().max_height(430.0).show(ui,|ui| { ui.add(egui::TextEdit::multiline(&mut self.shader).font(egui::TextStyle::Monospace).code_editor().desired_rows(24).desired_width(f32::INFINITY)); });
        });
        self.shader_open = open;
    }
}
impl eframe::App for OpenPatch {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sync();
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.source = None;
        }
        if !ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.toggle_play();
        }
        if ctx.input(|i| i.viewport().close_requested())
            && self.revision != self.saved_revision
            && !self.allow_close
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
        self.header(ctx);
        egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::new().fill(PANEL).inner_margin(10))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.status).size(11.0).color(MINT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("r{}  ·  {}", self.revision, self.device))
                                .size(10.0)
                                .color(MUTED),
                        );
                    });
                });
                if (self.audio.is_none() || self.status.starts_with("Audio"))
                    && ui.button("Retry audio").clicked()
                {
                    match Audio::new(&self.patch) {
                        Ok(a) => {
                            a.set_volume(self.volume);
                            self.device = format!("{} · {} Hz", a.device_name, a.sample_rate);
                            self.audio_revision = Some(self.revision);
                            self.audio = Some(a);
                            self.status = "Audio ready. Press Play.".into();
                        }
                        Err(e) => self.status = format!("Audio unavailable: {e}"),
                    }
                }
            });
        self.sidebar(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(20))
            .show(ctx, |ui| {
                self.visual_panel(ui);
                ui.add_space(12.0);
                ui.separator();
                self.rack(ui);
            });
        if self.shader_open {
            self.shader_window(ctx);
        }
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
                        if ui.button("Discard and close").clicked() {
                            self.allow_close = true;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
        }
        if let Some((path, frame)) = &mut self.smoke_test {
            *frame += 1;
            if *frame == 1 {
                let visualizer = self
                    .visualizer
                    .as_mut()
                    .expect("Native shader must compile");
                let gl = self.gl.as_ref().expect("Native GL context is required");
                assert!(visualizer.compile(gl, "this is not GLSL").is_err());
                visualizer
                    .compile(gl, DEFAULT_SHADER)
                    .expect("Valid shader must recover");
            }
            // Exercise each native render mode before capturing the default view.
            self.visual_mode = match *frame {
                2..=4 => 1,
                5..=7 => 2,
                _ => 0,
            };
            if *frame == 12 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            let screenshot = ctx.input(|input| {
                input.events.iter().find_map(|event| {
                    if let egui::Event::Screenshot { image, .. } = event {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
            });
            if let Some(image) = screenshot {
                assert!(image.width() >= 900 && image.height() >= 600);
                assert!(
                    image
                        .pixels
                        .windows(2)
                        .filter(|pixels| pixels[0] != pixels[1])
                        .count()
                        > 1000,
                    "Rendered window is blank"
                );
                image::save_buffer(
                    path,
                    image.as_raw(),
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .expect("Save native screenshot");
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            assert!(*frame < 180, "Timed out waiting for native screenshot");
        }
        ctx.request_repaint_after(Duration::from_millis(33));
    }
    fn on_exit(&mut self, gl: Option<&glow::Context>) {
        if let (Some(v), Some(gl)) = (&self.visualizer, gl) {
            v.destroy(gl);
        }
    }
}
fn section(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).strong().color(MUTED));
}
fn jack(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    pos: egui::Pos2,
    id: egui::Id,
    label: &str,
    color: Color32,
) -> egui::Response {
    let response = ui.interact(
        egui::Rect::from_center_size(pos, Vec2::splat(24.0)),
        id,
        egui::Sense::click(),
    );
    painter.circle_filled(pos, 8.0, BG);
    painter.circle_stroke(
        pos,
        7.0,
        Stroke::new(if response.hovered() { 2.5_f32 } else { 1.5_f32 }, color),
    );
    painter.circle_filled(pos, 2.5, color);
    painter.text(
        pos - Vec2::new(0.0, 12.0),
        egui::Align2::CENTER_BOTTOM,
        label.to_uppercase(),
        FontId::monospace(9.0),
        MUTED,
    );
    response.on_hover_text(format!(
        "{label}: click to patch; right-click an input to unplug"
    ))
}
