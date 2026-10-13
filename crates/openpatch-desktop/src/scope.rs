//! Calibrated, sample-timed inspection. Collection is on the UI thread; the
//! callback supplies bounded, generation-tagged packets from the audible engine.
use crate::audio::{Audio, ScopeEndpoint, ScopePacket, ScopeSelection};
use eframe::egui::{self, pos2, Color32, FontId, RichText, Stroke, Vec2};
use openpatch_core::Patch;
use serde_json::Value;
use std::collections::VecDeque;

const COLORS: [Color32; 2] = [
    Color32::from_rgb(241, 184, 87),
    Color32::from_rgb(82, 205, 205),
];
const MUTED: Color32 = Color32::from_rgb(156, 160, 153);

trait ScopeSource {
    fn generation(&self) -> u64;
    fn bind(&mut self, patch: &Patch, selection: Option<ScopeSelection>) -> Result<u64, String>;
    fn drain(&mut self, receive: impl FnMut(ScopePacket));
}

impl ScopeSource for Audio {
    fn generation(&self) -> u64 {
        self.scope_generation()
    }

    fn bind(&mut self, patch: &Patch, selection: Option<ScopeSelection>) -> Result<u64, String> {
        self.set_scope(patch, selection)
    }

    fn drain(&mut self, mut receive: impl FnMut(ScopePacket)) {
        for packet in self.scope_packets() {
            receive(packet);
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    values: [f32; 2],
    active: [bool; 2],
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Trigger {
    Auto,
    Normal,
    Free,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Settings {
    seconds: f64,
    trigger: Trigger,
    source: usize,
    level: f32,
    falling: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            seconds: 0.02,
            trigger: Trigger::Auto,
            source: 0,
            level: 0.0,
            falling: false,
        }
    }
}
#[derive(Default)]
struct Acquisition {
    generation: Option<u64>,
    rate: u32,
    next_frame: Option<u64>,
    history: VecDeque<Sample>,
    display: Vec<Sample>,
    armed: bool,
    finish_at: Option<u64>,
    last_publish: Option<u64>,
    triggered: bool,
    gaps: u64,
    first_frame: u64,
}
impl Acquisition {
    fn reset(&mut self) {
        *self = Self::default();
    }
    fn reset_stream(&mut self, generation: u64, rate: u32) {
        let gaps = self.gaps;
        self.reset();
        self.gaps = gaps;
        self.generation = Some(generation);
        self.rate = rate;
    }
    fn push(&mut self, packet: &ScopePacket, expected_generation: u64, settings: Settings) {
        if packet.generation != expected_generation {
            return;
        }
        if self.generation != Some(packet.generation) || self.rate != packet.sample_rate {
            self.reset_stream(packet.generation, packet.sample_rate);
        }
        if self
            .next_frame
            .is_some_and(|next| next != packet.start_frame)
        {
            self.gaps += 1;
            self.reset_stream(packet.generation, packet.sample_rate);
        }
        let length = (settings.seconds * self.rate as f64)
            .round()
            .clamp(2.0, 384_000.0) as usize;
        let pre = length / 4;
        for i in 0..packet.x.len() {
            let frame = packet.start_frame + i as u64;
            let sample = Sample {
                values: [packet.x[i], packet.y[i]],
                active: [packet.x_active[i], packet.y_active[i]],
            };
            if sample.values.iter().any(|x| !x.is_finite()) {
                self.gaps += 1;
                self.reset_stream(packet.generation, packet.sample_rate);
                continue;
            }
            self.history.push_back(sample);
            if self.history.len() > length {
                self.history.pop_front();
            }
            let value = sample.values[settings.source] * if settings.falling { -1.0 } else { 1.0 };
            let level = settings.level * if settings.falling { -1.0 } else { 1.0 };
            if !sample.active[settings.source] {
                self.armed = false;
            } else if value <= level - 0.01 {
                self.armed = true;
            } else if value >= level + 0.01 && self.armed {
                self.armed = false;
                if self.finish_at.is_none()
                    && self.history.len() > pre
                    && settings.trigger != Trigger::Free
                {
                    // Trigger is at index pre; complete the remaining post-trigger samples.
                    self.finish_at = Some(frame + (length - pre - 1) as u64);
                }
            }
            if self.history.len() == length {
                let triggered = self.finish_at == Some(frame);
                let timeout = self
                    .last_publish
                    .is_none_or(|last| frame.saturating_sub(last) >= length as u64);
                let automatic =
                    settings.trigger != Trigger::Normal && self.finish_at.is_none() && timeout;
                if triggered || automatic {
                    self.display.clear();
                    self.display.extend(self.history.iter().copied());
                    self.first_frame = frame + 1 - length as u64;
                    self.triggered = triggered;
                    self.finish_at = None;
                    self.last_publish = Some(frame);
                }
            }
        }
        self.next_frame = Some(packet.start_frame + packet.x.len() as u64);
    }
}

pub struct Scope {
    pub open: bool,
    selection: ScopeSelection,
    applied: Option<Option<ScopeSelection>>,
    generation: Option<u64>,
    acquisition: Acquisition,
    settings: Settings,
    scales: [f32; 2],
    offsets: [f32; 2],
    xy: bool,
    frozen: bool,
    status: String,
    fixture: bool,
}
impl Default for Scope {
    fn default() -> Self {
        Self {
            open: false,
            selection: ScopeSelection { x: None, y: None },
            applied: None,
            generation: None,
            acquisition: Acquisition::default(),
            settings: Settings::default(),
            scales: [2.0; 2],
            offsets: [0.0; 2],
            xy: false,
            frozen: false,
            status: String::new(),
            fixture: false,
        }
    }
}
impl Scope {
    pub fn inspect(&mut self, lane: usize, id: String, port: String) {
        let endpoint = Some(ScopeEndpoint {
            id,
            port,
            channel: 0,
        });
        if lane == 0 {
            self.selection.x = endpoint;
        } else {
            self.selection.y = endpoint;
        }
        self.open = true;
        // Selecting the same ID explicitly also rebinds a replaced module.
        self.reset_connection();
    }
    pub fn reset_connection(&mut self) {
        self.applied = None;
        self.generation = None;
        self.acquisition.reset();
    }
    #[cfg(test)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        patch: &Patch,
        catalog: &Value,
        audio: Option<&mut Audio>,
    ) {
        self.service(patch, audio);
        if self.open {
            egui::TopBottomPanel::bottom("signal-scope-test")
                .default_height(255.0)
                .height_range(220.0..=560.0)
                .show(ctx, |ui| self.contents(ui, patch, catalog));
        }
    }
    /// Run once every UI frame, whether Scope or another dock tab is visible.
    /// Closing the dock disarms capture and still drains queued packets.
    pub fn service(&mut self, patch: &Patch, audio: Option<&mut Audio>) {
        if self.fixture {
            return;
        }
        if let Some(audio) = audio {
            self.service_source(patch, audio);
        } else {
            self.applied = None;
            self.generation = None;
            self.acquisition.reset();
            self.status =
                "Audio engine unavailable — use Retry audio to enable live inspection".into();
        }
    }
    fn service_source(&mut self, patch: &Patch, source: &mut impl ScopeSource) {
        if self
            .generation
            .is_some_and(|generation| generation != source.generation())
        {
            self.generation = Some(source.generation());
            self.acquisition.reset();
        }
        let requested = self.open.then(|| self.selection.clone());
        if self.applied.as_ref() != Some(&requested) {
            self.acquisition.reset();
            match source.bind(patch, requested.clone()) {
                Ok(generation) => {
                    self.applied = Some(requested);
                    self.generation = Some(generation);
                    self.status.clear();
                }
                Err(error) => {
                    self.status = error;
                    self.applied = Some(requested);
                    self.generation = None;
                }
            }
        }
        let generation = source.generation();
        if self.generation.is_some() && self.generation != Some(generation) {
            self.generation = Some(generation);
            self.acquisition.reset();
        }
        source.drain(|packet| {
            if self.open && !self.frozen && self.generation.is_some() {
                self.acquisition.push(&packet, generation, self.settings);
            }
        });
    }
    /// Draw the selected Scope tab inside the application's shared analysis dock.
    /// Call `service` before this method each frame.
    pub fn contents(&mut self, ui: &mut egui::Ui, patch: &Patch, catalog: &Value) {
        if !self.open {
            return;
        }
        let old_settings = self.settings;
        let old_frozen = self.frozen;
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 3.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.xy, false, "Time");
            ui.selectable_value(&mut self.xy, true, "X/Y");
            egui::ComboBox::from_id_salt("scope-time")
                .width(78.0)
                .selected_text(format!("{} ms/div", self.settings.seconds * 100.0))
                .show_ui(ui, |ui| {
                    for seconds in [0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0] {
                        ui.selectable_value(
                            &mut self.settings.seconds,
                            seconds,
                            format!("{} ms/div", seconds * 100.0),
                        );
                    }
                });
            ui.toggle_value(&mut self.frozen, "Freeze");
            ui.menu_button("Advanced…", |ui| {
                ui.set_width(380.0);
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| self.advanced_controls(ui, patch, catalog));
            })
            .response
            .on_hover_text("Trigger, voice, center offsets, reconnection, and diagnostics");
        });
        self.source_row(ui, 0, patch, catalog);
        self.source_row(ui, 1, patch, catalog);
        if old_settings != self.settings || (old_frozen && !self.frozen) {
            self.acquisition.reset();
        }
        // Never paint an old trace under newly selected endpoint labels.
        if self.applied.as_ref() != Some(&Some(self.selection.clone())) && !self.fixture {
            self.acquisition.reset();
        }
        self.paint(ui, patch, catalog);
    }
    fn source_row(&mut self, ui: &mut egui::Ui, lane: usize, patch: &Patch, catalog: &Value) {
        ui.push_id(("source", lane), |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(COLORS[lane], if lane == 0 { "X" } else { "Y" });
                let endpoint = if lane == 0 {
                    &mut self.selection.x
                } else {
                    &mut self.selection.y
                };
                let label = endpoint
                    .as_ref()
                    .map_or("Choose an output…".to_owned(), |e| {
                        format!("{} · {}", e.id, e.port)
                    });
                let source_width = (ui.available_width() - 140.0).clamp(165.0, 260.0);
                let compact_label = compact_label(&label, (source_width / 7.5) as usize);
                egui::ComboBox::from_id_salt("endpoint")
                    .width(source_width)
                    .selected_text(compact_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(endpoint, None, "Disconnected");
                        for node in &patch.nodes {
                            if let Some(module) = catalog["modules"]
                                .as_array()
                                .and_then(|modules| modules.iter().find(|m| m["kind"] == node.kind))
                            {
                                if let Some(ports) = module["outputs"].as_array() {
                                    for port in ports.iter().filter_map(Value::as_str) {
                                        let item = ScopeEndpoint {
                                            id: node.id.clone(),
                                            port: port.into(),
                                            channel: endpoint.as_ref().map_or(0, |e| e.channel),
                                        };
                                        ui.selectable_value(
                                            endpoint,
                                            Some(item),
                                            format!("{} · {}", node.id, port),
                                        );
                                    }
                                }
                            }
                        }
                    })
                    .response
                    .on_hover_text(label);
                let units = units(endpoint.as_ref(), patch, catalog);
                egui::ComboBox::from_id_salt("scale")
                    .width(88.0)
                    .selected_text(format!("{} {units}/div", self.scales[lane]))
                    .show_ui(ui, |ui| {
                        for scale in [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0] {
                            ui.selectable_value(
                                &mut self.scales[lane],
                                scale,
                                format!("{scale} {units}/div"),
                            );
                        }
                    });
            });
        });
    }
    fn advanced_controls(&mut self, ui: &mut egui::Ui, patch: &Patch, catalog: &Value) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Trigger").color(MUTED));
            for (mode, name) in [
                (Trigger::Auto, "Auto"),
                (Trigger::Normal, "Normal"),
                (Trigger::Free, "Free"),
            ] {
                ui.selectable_value(&mut self.settings.trigger, mode, name);
            }
            ui.selectable_value(&mut self.settings.source, 0, "X");
            ui.selectable_value(&mut self.settings.source, 1, "Y");
            ui.selectable_value(&mut self.settings.falling, false, "Rising");
            ui.selectable_value(&mut self.settings.falling, true, "Falling");
            ui.add(
                egui::DragValue::new(&mut self.settings.level)
                    .range(-100.0..=100.0)
                    .speed(0.05)
                    .prefix("Level "),
            );
        });
        let selection_current =
            self.applied.as_ref() == Some(&Some(self.selection.clone())) || self.fixture;
        for (lane, color) in COLORS.iter().enumerate() {
            ui.push_id(("advanced", lane), |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(*color, if lane == 0 { "X" } else { "Y" });
                    let endpoint = if lane == 0 {
                        &mut self.selection.x
                    } else {
                        &mut self.selection.y
                    };
                    ui.label(RichText::new("Voice").color(MUTED));
                    let mut voice = endpoint.as_ref().map_or(1, |e| e.channel + 1);
                    if ui
                        .add_enabled(
                            endpoint.is_some(),
                            egui::DragValue::new(&mut voice).range(1..=16),
                        )
                        .changed()
                    {
                        if let Some(e) = endpoint {
                            e.channel = voice - 1;
                        }
                    }
                    let units = units(endpoint.as_ref(), patch, catalog);
                    ui.label(RichText::new("Center").color(MUTED));
                    ui.add(
                        egui::DragValue::new(&mut self.offsets[lane])
                            .range(-100.0..=100.0)
                            .speed(0.05)
                            .suffix(format!(" {units}")),
                    );
                    if let Some(stats) = selection_current
                        .then(|| statistics(&self.acquisition.display, lane))
                        .flatten()
                    {
                        ui.label(
                            RichText::new(format!(
                                "{:.2}…{:.2}  DC {:.2}  RMS {:.2} {units}",
                                stats.min, stats.max, stats.dc, stats.rms
                            ))
                            .size(11.0)
                            .color(*color),
                        );
                    } else {
                        ui.label(RichText::new("No active samples").size(11.0).color(MUTED));
                    }
                });
            });
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Reconnect sources")
                .on_hover_text("Rebind the selected sources after a module was replaced")
                .clicked()
            {
                self.reset_connection();
            }
            ui.label(
                RichText::new("25% pre-trigger · ±0.01 hysteresis")
                    .size(10.0)
                    .color(MUTED),
            );
        });
        ui.horizontal_wrapped(|ui| {
            if self.acquisition.rate != 0 {
                ui.label(
                    RichText::new(format!(
                        "{} Hz · {} frames · gaps {}",
                        self.acquisition.rate,
                        self.acquisition.display.len(),
                        self.acquisition.gaps
                    ))
                    .size(10.0)
                    .color(MUTED),
                );
            }
            if let Some(generation) = self.acquisition.generation {
                ui.label(
                    RichText::new(format!("generation {generation}"))
                        .size(10.0)
                        .color(MUTED),
                );
            }
            ui.label(
                RichText::new("Raw output · before master level / graph crossfade")
                    .size(10.0)
                    .color(MUTED),
            );
        });
    }
    fn paint(&self, ui: &mut egui::Ui, patch: &Patch, catalog: &Value) {
        let display = &self.acquisition.display;
        let state = if self.fixture {
            "Offline calibration fixture"
        } else if self.frozen {
            "Frozen"
        } else if display.is_empty() {
            "Waiting for signal / trigger"
        } else if self.acquisition.triggered {
            "Triggered capture · waiting for next trigger"
        } else {
            "Auto / free running"
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(state).color(COLORS[0]).size(11.0));
            if !self.status.is_empty() {
                ui.add(
                    egui::Label::new(RichText::new(&self.status).color(Color32::LIGHT_RED))
                        .truncate(),
                );
            }
        });
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(
                ui.available_width().max(0.0),
                ui.available_height().max(0.0),
            ),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_rgb(13, 25, 28));
        if rect.width() < 75.0 || rect.height() < 40.0 {
            response.on_hover_text("Resize the analysis dock to see the Scope plot");
            return;
        }
        painter.line_segment(
            [
                rect.left_top() + Vec2::new(1.0, 1.0),
                rect.right_top() + Vec2::new(-1.0, 1.0),
            ],
            Stroke::new(1.0_f32, Color32::from_rgb(63, 76, 75)),
        );
        let plot = rect.shrink2(Vec2::new(35.0, 17.0));
        for x in 0..=10 {
            let x = plot.left() + plot.width() * x as f32 / 10.0;
            painter.line_segment(
                [pos2(x, plot.top()), pos2(x, plot.bottom())],
                Stroke::new(1.0_f32, Color32::from_rgb(36, 51, 52)),
            );
        }
        for y in 0..=8 {
            let y = plot.top() + plot.height() * y as f32 / 8.0;
            painter.line_segment(
                [pos2(plot.left(), y), pos2(plot.right(), y)],
                Stroke::new(1.0_f32, Color32::from_rgb(36, 51, 52)),
            );
        }
        painter.line_segment(
            [
                pos2(plot.left(), plot.center().y),
                pos2(plot.right(), plot.center().y),
            ],
            Stroke::new(1.0_f32, Color32::from_rgb(66, 80, 77)),
        );
        let y = |value: f32, lane: usize| {
            plot.center().y - (value - self.offsets[lane]) / self.scales[lane] * plot.height() / 8.0
        };
        for (lane, color) in COLORS.iter().enumerate() {
            if self.xy && lane == 0 {
                continue;
            }
            for (divisions, row) in [
                (4.0, plot.top()),
                (0.0, plot.center().y),
                (-4.0, plot.bottom()),
            ] {
                let (x, align) = if lane == 0 {
                    (plot.left() - 4.0, egui::Align2::RIGHT_CENTER)
                } else {
                    (plot.right() + 4.0, egui::Align2::LEFT_CENTER)
                };
                painter.text(
                    pos2(x, row),
                    align,
                    format!("{:.1}", self.offsets[lane] + divisions * self.scales[lane]),
                    FontId::monospace(9.0),
                    *color,
                );
            }
        }
        let trace = painter.with_clip_rect(plot);
        if self.xy {
            let stride = display.len().div_ceil(2048).max(1);
            let mut previous = None;
            for chunk in display.chunks(stride) {
                if chunk.iter().any(|s| !s.active[0] || !s.active[1]) {
                    previous = None;
                    continue;
                }
                let s = chunk[0];
                let point = pos2(
                    plot.center().x
                        + (s.values[0] - self.offsets[0]) / self.scales[0] * plot.width() / 10.0,
                    y(s.values[1], 1),
                );
                if let Some(old) = previous {
                    trace.line_segment([old, point], Stroke::new(1.3_f32, COLORS[0]));
                }
                previous = Some(point);
            }
        } else {
            let bins = display.len().min(plot.width().ceil() as usize).max(1);
            for (lane, color) in COLORS.iter().enumerate() {
                let mut previous = None;
                for bin in 0..bins {
                    let start = bin * display.len() / bins;
                    let end = (bin + 1) * display.len() / bins;
                    let slice = &display[start..end];
                    if let Some(stats) = statistics(slice, lane) {
                        let x = plot.left() + (bin as f32 + 0.5) / bins as f32 * plot.width();
                        trace.line_segment(
                            [pos2(x, y(stats.min, lane)), pos2(x, y(stats.max, lane))],
                            Stroke::new(1.0_f32, *color),
                        );
                        let point = pos2(x, y(stats.dc as f32, lane));
                        let continuous = slice.iter().all(|s| s.active[lane]);
                        if continuous {
                            if let Some(old) = previous {
                                trace.line_segment([old, point], Stroke::new(1.1_f32, *color));
                            }
                        }
                        previous = if continuous { Some(point) } else { None };
                    } else {
                        previous = None;
                    }
                }
            }
            if self.acquisition.triggered {
                let x = plot.left() + plot.width() * 0.25;
                painter.line_segment(
                    [pos2(x, plot.top()), pos2(x, plot.bottom())],
                    Stroke::new(1.0_f32, Color32::from_rgb(132, 103, 59)),
                );
                painter.text(
                    pos2(x, rect.top() + 2.0),
                    egui::Align2::CENTER_TOP,
                    "T",
                    FontId::monospace(10.0),
                    MUTED,
                );
            }
        }
        let left = if self.xy {
            format!(
                "X: {} {}/div",
                self.scales[0],
                units(self.selection.x.as_ref(), patch, catalog)
            )
        } else {
            format!(
                "{:.1} ms",
                if self.acquisition.triggered {
                    -self.settings.seconds * 250.0
                } else {
                    0.0
                }
            )
        };
        let right = if self.xy {
            format!(
                "Y: {} {}/div",
                self.scales[1],
                units(self.selection.y.as_ref(), patch, catalog)
            )
        } else {
            format!(
                "{:.1} ms",
                self.settings.seconds
                    * 1000.0
                    * if self.acquisition.triggered {
                        0.75
                    } else {
                        1.0
                    }
            )
        };
        painter.text(
            pos2(plot.left(), rect.bottom() - 2.0),
            egui::Align2::LEFT_BOTTOM,
            left,
            FontId::monospace(10.0),
            MUTED,
        );
        painter.text(
            pos2(plot.right(), rect.bottom() - 2.0),
            egui::Align2::RIGHT_BOTTOM,
            right,
            FontId::monospace(10.0),
            MUTED,
        );
        response.on_hover_text("Time view preserves each pixel column’s minimum and maximum. Statistics use every active captured sample. Missing voices leave gaps. X/Y is a decimated phase portrait. FS means normalized digital amplitude, not volts. Scope controls are session-only.");
    }
    pub fn smoke_xy(&mut self) {
        self.xy = true;
    }
    /// Uses actual engine output in Linux CI without accessing an audio device.
    pub fn smoke_fixture(&mut self) {
        self.open = true;
        self.fixture = true;
        self.status.clear();
        self.settings.seconds = 0.02;
        self.selection = ScopeSelection {
            x: Some(ScopeEndpoint {
                id: "calibration".into(),
                port: "sine".into(),
                channel: 0,
            }),
            y: Some(ScopeEndpoint {
                id: "calibration".into(),
                port: "triangle".into(),
                channel: 0,
            }),
        };
        let mut patch: Patch = serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,"nodes":[{"id":"calibration","kind":"multi_lfo","params":{"frequency":100.0}}],"output":"calibration","output_port":"sine"})).unwrap();
        let mut x = patch.compile().unwrap();
        patch.output_port = "triangle".into();
        let mut y = patch.compile().unwrap();
        let mut xb = [0.0; 64];
        let mut yb = [0.0; 64];
        for index in 0..100 {
            x.process_block(&mut xb);
            y.process_block(&mut yb);
            self.acquisition.push(
                &ScopePacket {
                    start_frame: index * 64,
                    generation: 1,
                    sample_rate: 48000,
                    x: xb,
                    y: yb,
                    x_active: [true; 64],
                    y_active: [true; 64],
                },
                1,
                self.settings,
            );
        }
    }
}
fn compact_label(label: &str, max_chars: usize) -> String {
    let mut chars = label.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}
fn units(endpoint: Option<&ScopeEndpoint>, patch: &Patch, catalog: &Value) -> &'static str {
    let Some(e) = endpoint else {
        return "V";
    };
    let node = patch.nodes.iter().find(|n| n.id == e.id);
    let metadata = node
        .and_then(|n| {
            catalog["modules"]
                .as_array()?
                .iter()
                .find(|m| m["kind"] == n.kind)
        })
        .and_then(|m| {
            m["output_metadata"]
                .as_array()?
                .iter()
                .find(|p| p["name"] == e.port)
        });
    match metadata.and_then(|m| m["units"].as_str()) {
        Some("V") => "V",
        Some("normalized digital amplitude") => "FS",
        Some("legacy normalized") => "norm",
        _ => "units",
    }
}
struct Statistics {
    min: f32,
    max: f32,
    dc: f64,
    rms: f64,
}
fn statistics(samples: &[Sample], lane: usize) -> Option<Statistics> {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    let mut sum = 0.0;
    let mut squares = 0.0;
    let mut count = 0;
    for sample in samples.iter().filter(|s| s.active[lane]) {
        let value = sample.values[lane];
        min = min.min(value);
        max = max.max(value);
        sum += value as f64;
        squares += (value as f64).powi(2);
        count += 1;
    }
    (count != 0).then(|| Statistics {
        min,
        max,
        dc: sum / count as f64,
        rms: (squares / count as f64).sqrt(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text_center(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        fn find(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
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
            .unwrap_or_else(|| panic!("missing Scope label {label}"))
    }
    #[test]
    fn compact_scope_keeps_sources_scales_and_plot_visible_at_minimum_width() {
        let mut scope = Scope::default();
        scope.smoke_fixture();
        let ctx = egui::Context::default();
        let patch = Patch::new_v2();
        let catalog = openpatch_core::catalog();
        let mut draw = |time| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(630.0, 600.0),
                    )),
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    scope.show(ctx, &patch, &catalog, None);
                    egui::CentralPanel::default().show(ctx, |_ui| {});
                },
            )
        };
        let _ = draw(0.0);
        let output = draw(1.0);
        for label in [
            "Time",
            "X/Y",
            "Freeze",
            "Advanced…",
            "calibration · sine",
            "calibration · triangle",
            "2 units/div",
        ] {
            assert!(text_center(&output, label).x <= 630.0);
        }
        let plot = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.fill == Color32::from_rgb(13, 25, 28) => {
                Some(rect.rect)
            }
            _ => None,
        });
        let plot = plot.expect("scope plot remains visible");
        assert!(plot.height() >= 70.0, "plot height {}", plot.height());
        assert!(plot.width() > 500.0, "plot width {}", plot.width());
    }
    #[test]
    fn advanced_popup_exposes_trigger_voice_center_and_reconnect() {
        let mut scope = Scope::default();
        scope.smoke_fixture();
        let ctx = egui::Context::default();
        let patch = Patch::new_v2();
        let catalog = openpatch_core::catalog();
        let mut time = 0.0;
        let mut draw = |events: Vec<egui::Event>| {
            time += 0.1;
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(630.0, 600.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    scope.show(ctx, &patch, &catalog, None);
                    egui::CentralPanel::default().show(ctx, |_ui| {});
                },
            )
        };
        let _ = draw(vec![]);
        let output = draw(vec![]);
        let advanced = text_center(&output, "Advanced…");
        for pressed in [true, false] {
            let _ = draw(vec![
                egui::Event::PointerMoved(advanced),
                egui::Event::PointerButton {
                    pos: advanced,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        let output = draw(vec![]);
        for label in ["Trigger", "Normal", "Voice", "Center", "Reconnect sources"] {
            let _ = text_center(&output, label);
        }
        let normal = text_center(&output, "Normal");
        for pressed in [true, false] {
            let _ = draw(vec![
                egui::Event::PointerMoved(normal),
                egui::Event::PointerButton {
                    pos: normal,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        assert_eq!(scope.settings.trigger, Trigger::Normal);
    }
    #[test]
    fn native_freeze_button_holds_capture_and_resume_rearms() {
        let mut scope = Scope::default();
        scope.smoke_fixture();
        let ctx = egui::Context::default();
        let patch = Patch::new_v2();
        let catalog = openpatch_core::catalog();
        let mut time = 0.0;
        let mut draw = |scope: &mut Scope, events: Vec<egui::Event>| {
            time += 0.033;
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(1320.0, 880.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ctx| scope.show(ctx, &patch, &catalog, None),
            )
        };
        let _ = draw(&mut scope, vec![]);
        let output = draw(&mut scope, vec![]);
        let center = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "Freeze" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("Freeze is rendered");
        let before = scope.acquisition.first_frame;
        let count = scope.acquisition.display.len();
        assert!(count > 0);
        for pressed in [true, false] {
            draw(
                &mut scope,
                vec![
                    egui::Event::PointerMoved(center),
                    egui::Event::PointerButton {
                        pos: center,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(scope.frozen);
        assert_eq!(scope.acquisition.first_frame, before);
        assert_eq!(scope.acquisition.display.len(), count);
        for pressed in [true, false] {
            draw(
                &mut scope,
                vec![egui::Event::PointerButton {
                    pos: center,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert!(!scope.frozen);
        assert!(scope.acquisition.display.is_empty());
    }
    fn packet(start: u64) -> ScopePacket {
        ScopePacket {
            start_frame: start,
            generation: 7,
            sample_rate: 8000,
            x: [-1.0; 64],
            y: [2.0; 64],
            x_active: [true; 64],
            y_active: [false; 64],
        }
    }
    #[derive(Default)]
    struct FakeSource {
        generation: u64,
        bindings: Vec<Option<ScopeSelection>>,
        packets: VecDeque<ScopePacket>,
        drained: usize,
    }
    impl ScopeSource for FakeSource {
        fn generation(&self) -> u64 {
            self.generation
        }
        fn bind(
            &mut self,
            _patch: &Patch,
            selection: Option<ScopeSelection>,
        ) -> Result<u64, String> {
            self.generation += 1;
            self.bindings.push(selection);
            Ok(self.generation)
        }
        fn drain(&mut self, mut receive: impl FnMut(ScopePacket)) {
            while let Some(packet) = self.packets.pop_front() {
                self.drained += 1;
                receive(packet);
            }
        }
    }
    #[test]
    fn service_captures_while_other_tab_shows_and_disarms_on_close() {
        let patch = Patch::new_v2();
        let mut scope = Scope {
            open: true,
            settings: Settings {
                seconds: 0.008,
                trigger: Trigger::Free,
                ..Settings::default()
            },
            selection: ScopeSelection {
                x: Some(ScopeEndpoint {
                    id: "source".into(),
                    port: "out".into(),
                    channel: 0,
                }),
                y: None,
            },
            ..Scope::default()
        };
        let mut source = FakeSource::default();
        let mut first = packet(0);
        first.generation = 1;
        source.packets.push_back(first);
        // No contents call: the Visuals tab is visible instead of Scope.
        scope.service_source(&patch, &mut source);
        assert_eq!(scope.acquisition.display.len(), 64);
        assert_eq!(scope.acquisition.first_frame, 0);
        assert_eq!(source.bindings.len(), 1);

        let mut next = packet(64);
        next.generation = 1;
        source.packets.push_back(next);
        scope.service_source(&patch, &mut source);
        assert_eq!(scope.acquisition.first_frame, 64);

        scope.frozen = true;
        let mut skipped = packet(128);
        skipped.generation = 1;
        source.packets.push_back(skipped);
        scope.service_source(&patch, &mut source);
        assert_eq!(scope.acquisition.first_frame, 64);
        assert_eq!(scope.acquisition.display.len(), 64);
        assert_eq!(source.drained, 3);

        scope.selection.x.as_mut().unwrap().port = "other".into();
        scope.service_source(&patch, &mut source);
        assert!(scope.acquisition.display.is_empty());
        assert_eq!(source.bindings.len(), 2);

        scope.open = false;
        let mut after_close = packet(192);
        after_close.generation = 3;
        source.packets.push_back(after_close);
        scope.service_source(&patch, &mut source);
        assert_eq!(source.bindings.last(), Some(&None));
        assert!(source.packets.is_empty());
        assert_eq!(source.drained, 4);
        assert!(scope.acquisition.display.is_empty());
    }
    #[test]
    fn trigger_pre_roll_and_inactive_statistics_are_exact() {
        let mut a = Acquisition::default();
        let settings = Settings {
            seconds: 0.008,
            trigger: Trigger::Normal,
            ..Settings::default()
        };
        let mut p = packet(0);
        p.x[32..].fill(3.0);
        a.push(&p, 7, settings);
        assert!(a.display.is_empty());
        let mut q = packet(64);
        q.x.fill(3.0);
        a.push(&q, 7, settings);
        assert_eq!(a.display.len(), 64);
        assert_eq!(a.first_frame, 16);
        assert!(a.triggered);
        assert!(a.display[..16].iter().all(|s| s.values[0] == -1.0));
        assert!(a.display[16..].iter().all(|s| s.values[0] == 3.0));
        let stats = statistics(&a.display, 0).unwrap();
        assert_eq!((stats.min, stats.max, stats.dc), (-1.0, 3.0, 2.0));
        assert_eq!(stats.rms, 7.0_f64.sqrt());
        assert!(statistics(&a.display, 1).is_none());
    }
    #[test]
    fn gaps_generations_and_missing_trigger_channels_reset_acquisition() {
        let settings = Settings {
            seconds: 0.008,
            trigger: Trigger::Free,
            ..Settings::default()
        };
        let mut a = Acquisition::default();
        a.push(&packet(0), 7, settings);
        assert_eq!(a.display.len(), 64);
        a.push(&packet(1000), 8, settings);
        assert_eq!(a.next_frame, Some(64));
        let mut p = packet(1024);
        p.x_active.fill(false);
        a.push(&p, 7, settings);
        assert_eq!(a.gaps, 1);
        assert!(statistics(&a.display, 0).is_none());
        assert_eq!(a.first_frame, 1024);
        p.generation = 8;
        a.push(
            &p,
            8,
            Settings {
                trigger: Trigger::Normal,
                ..settings
            },
        );
        assert!(a.display.is_empty());
        assert!(!a.armed);
    }
    #[test]
    fn auto_captures_dc_and_min_max_preserves_single_sample_impulses() {
        let mut a = Acquisition::default();
        let mut p = packet(0);
        p.x.fill(0.0);
        p.x[31] = 12.0;
        a.push(
            &p,
            7,
            Settings {
                seconds: 0.008,
                trigger: Trigger::Free,
                ..Settings::default()
            },
        );
        let s = statistics(&a.display, 0).unwrap();
        assert_eq!(s.max, 12.0);
        assert_eq!(s.dc, 12.0 / 64.0);
        assert_eq!(s.rms, 1.5);
    }
}
