//! Patch Engine: pick a genre, a mood and a texture; the engine composes a new piece and builds it in the rack.
use eframe::egui::{self, text::LayoutJob, Color32, FontId, RichText, TextFormat};
use openpatch::control::{execute_shared, SharedSession};
use openpatch::patch_engine::{genre_noun, texture_phrase, texture_text, Recipe, GENRES, MOODS, TEXTURES};
use serde_json::{json, Value};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const GENRE_KEYS: [egui::Key; 8] = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6, egui::Key::Num7, egui::Key::Num8];
const MOOD_KEYS: [egui::Key; 8] = [egui::Key::Q, egui::Key::W, egui::Key::E, egui::Key::R, egui::Key::T, egui::Key::Y, egui::Key::U, egui::Key::I];
const TEXTURE_KEYS: [egui::Key; 8] = [egui::Key::A, egui::Key::S, egui::Key::D, egui::Key::F, egui::Key::G, egui::Key::H, egui::Key::J, egui::Key::K];
/// Genre index used for the Minecraft (C418-style) mode, after the eight genre words.
const MINECRAFT: usize = 8;
fn genre_name(i: usize) -> &'static str { if i == MINECRAFT { "Minecraft" } else { GENRES[i] } }
// Palette: the app's chrome, warm ivory and amber, with one muted hue per word list.
const GENRE_COLOR: Color32 = Color32::from_rgb(222, 138, 94);
const MOOD_COLOR: Color32 = Color32::from_rgb(134, 166, 218);
const TEXTURE_COLOR: Color32 = Color32::from_rgb(134, 192, 152);
const C418_COLOR: Color32 = Color32::from_rgb(150, 188, 112);
const SHEET: Color32 = Color32::from_rgb(25, 27, 27);
const WELL: Color32 = Color32::from_rgb(15, 16, 16);
const RAISED: Color32 = Color32::from_rgb(39, 42, 42);
const LINE: Color32 = Color32::from_rgb(55, 59, 57);
const INK: Color32 = Color32::from_rgb(235, 233, 226);
const DIM: Color32 = Color32::from_rgb(146, 149, 143);
const AMBER: Color32 = Color32::from_rgb(229, 186, 106);
const DARK_INK: Color32 = Color32::from_rgb(24, 24, 22);
const IVORY: Color32 = Color32::from_rgb(231, 225, 209);
const IVORY_INK: Color32 = Color32::from_rgb(40, 38, 33);
const IVORY_LINE: Color32 = Color32::from_rgb(190, 183, 169);

fn face(ui: &egui::Ui, size: f32) -> FontId { crate::panel_skin::heading_font(ui.ctx(), size) }

/// A word chip: raised and quiet until chosen, then filled with its list's colour.
fn chip(ui: &mut egui::Ui, text: &str, selected: bool, color: Color32, tip: Option<&str>) -> egui::Response {
    let font = face(ui, 15.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, if selected { DARK_INK } else { INK });
    let size = egui::vec2(galley.size().x + 18.0, 28.0);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hover = response.hovered();
        let fill = if selected { color } else if hover { Color32::from_rgb(47, 51, 50) } else { RAISED };
        let stroke = if selected { color } else if hover { color.gamma_multiply(0.7) } else { LINE };
        ui.painter().rect(rect, 7.0_f32, fill, egui::Stroke::new(1.0_f32, stroke), egui::StrokeKind::Inside);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, INK);
    }
    if let Some(tip) = tip { response = response.on_hover_text(tip); }
    response
}

/// A small segmented control; returns the newly chosen index.
fn segmented(ui: &mut egui::Ui, labels: &[&str], selected: usize, width: f32) -> Option<usize> {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 26.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect(rect, 6.0_f32, WELL, egui::Stroke::new(1.0_f32, LINE), egui::StrokeKind::Inside);
    let w = rect.width() / labels.len() as f32;
    let mut picked = None;
    for (i, label) in labels.iter().enumerate() {
        let r = egui::Rect::from_min_size(rect.min + egui::vec2(w * i as f32, 0.0), egui::vec2(w, rect.height())).shrink(2.0);
        let response = ui.interact(r, ui.id().with(("seg", label, i)), egui::Sense::click());
        let on = i == selected;
        if on { ui.painter().rect_filled(r, 5.0_f32, Color32::from_rgb(58, 54, 44)); }
        else if response.hovered() { ui.painter().rect_filled(r, 5.0_f32, RAISED); }
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, *label, face(ui, 13.0), if on { AMBER } else { DIM });
        if response.clicked() { picked = Some(i); }
    }
    picked
}

/// A pill toggle with a small lamp.
fn pill(ui: &mut egui::Ui, label: &str, on: &mut bool, tip: &str) {
    let font = face(ui, 13.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, if *on { INK } else { DIM });
    let (rect, response) = ui.allocate_exact_size(egui::vec2(galley.size().x + 30.0, 24.0), egui::Sense::click());
    let response = response.on_hover_text(tip);
    if response.clicked() { *on = !*on; }
    let p = ui.painter();
    p.rect(rect, 12.0_f32, if response.hovered() { RAISED } else { SHEET }, egui::Stroke::new(1.0_f32, if *on { AMBER.gamma_multiply(0.6) } else { LINE }), egui::StrokeKind::Inside);
    let lamp = egui::pos2(rect.left() + 12.0, rect.center().y);
    p.circle_filled(lamp, 3.5, if *on { AMBER } else { Color32::from_rgb(70, 74, 71) });
    if *on { p.circle_filled(lamp, 6.0, AMBER.gamma_multiply(0.18)); }
    p.galley(egui::pos2(rect.left() + 21.0, rect.center().y - galley.size().y / 2.0), galley, INK);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode { Reveal, Showtime, Instant }

/// Requests the app carries out after the panel draws.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Request { StartPlayback, ShowPatch, Prepare }

/// CPU budget for Light mode (Patch Engine cost units ≈ % of an Apple M-series core); about 60% of a Steam Deck core.
const LIGHT_BUDGET: f64 = 18.0;

struct Built { info: Value, steps: Vec<Value>, plan: bool }

pub(crate) struct Panel {
    open: bool,
    genre: Option<usize>,
    mood: Option<usize>,
    texture: Option<usize>,
    seed: u64,
    keep_seed: bool,
    mode: Mode,
    interval_ms: u32,
    autoplay: bool,
    rng: u64,
    pending: Option<mpsc::Receiver<Result<Built, String>>>,
    started: Option<Instant>,
    elapsed: Option<Duration>,
    building: bool,
    plan_steps: Vec<Value>,
    done: usize,
    total: usize,
    label: String,
    remaining: Option<Vec<Value>>,
    info: Option<Value>,
    error: Option<String>,
    history: Vec<(usize, usize, usize, u64, f32)>,
    last_poll: Option<Instant>,
    cursor: usize,
    demo: Option<Demo>,
    light: bool,
    first_frame: bool,
    armed: Option<Armed>,
    prepare_sent: bool,
    just_launched: bool,
    face_family: egui::FontFamily,
}

/// A build waiting for the app to start a fresh document (which asks about unsaved changes first).
struct Armed { g: usize, m: usize, t: usize, seed: u64, mode: Mode, interval: u32, budget: Option<f64> }

/// Demo/capture mode for pitches and documentation screenshots, driven by environment variables:
/// `OPENPATCH_PATCH_ENGINE_DEMO="Genre|Mood|Texture|seed"` opens the panel and builds that piece on launch;
/// `OPENPATCH_CAPTURE="first.png@0.9,second.png@6"` saves the window at those times (seconds after launch);
/// `OPENPATCH_WINDOW="1280x800"` sizes the window (Steam Deck screen).
struct Demo { start: Instant, words: Option<(usize, usize, usize, u64)>, captures: Vec<(std::path::PathBuf, f32)>, size: Option<egui::Vec2>, began: bool, waiting: Option<std::path::PathBuf> }

impl Demo {
    fn from_env() -> Option<Demo> {
        let words = std::env::var("OPENPATCH_PATCH_ENGINE_DEMO").ok().and_then(|v| {
            let p: Vec<&str> = v.split('|').collect();
            let find = |list: &[&str], w: &str| list.iter().position(|x| x.eq_ignore_ascii_case(w));
            let genre = if p.first()?.eq_ignore_ascii_case("minecraft") { MINECRAFT } else { find(&GENRES, p.first()?)? };
            Some((genre, find(&MOODS, p.get(1)?)?, find(&TEXTURES, p.get(2)?)?, p.get(3).and_then(|s| s.parse().ok()).unwrap_or(1)))
        });
        let captures: Vec<_> = std::env::var("OPENPATCH_CAPTURE").ok().map(|v| v.split(',').filter_map(|c| {
            let (path, t) = c.rsplit_once('@')?;
            Some((std::path::PathBuf::from(path), t.parse().ok()?))
        }).collect()).unwrap_or_default();
        let size = std::env::var("OPENPATCH_WINDOW").ok().and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some(egui::vec2(w.parse().ok()?, h.parse().ok()?))
        });
        (words.is_some() || !captures.is_empty() || size.is_some()).then(|| Demo { start: Instant::now(), words, captures, size, began: false, waiting: None })
    }
}

impl Default for Panel {
    fn default() -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        Self {
            open: false, genre: None, mood: None, texture: None, seed: 0, keep_seed: false, mode: Mode::Reveal,
            interval_ms: 120, autoplay: true, rng: now | 1, pending: None, started: None, elapsed: None, building: false,
            plan_steps: vec![], done: 0, total: 0, label: String::new(), remaining: None, info: None, error: None,
            history: vec![], last_poll: None, cursor: 0, demo: Demo::from_env(),
            light: cfg!(target_os = "linux"), first_frame: true, armed: None, prepare_sent: false, just_launched: false, face_family: egui::FontFamily::Proportional,
        }
    }
}

fn next_seed(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    1 + *state % 99_999
}

fn revision(session: &SharedSession) -> Result<u64, String> {
    Ok(execute_shared(session, "inspect_construction", json!({}))?["current_revision"].as_u64().unwrap_or(0))
}

impl Panel {
    pub(crate) fn is_open(&self) -> bool { self.open }
    pub(crate) fn toggle(&mut self) { self.open = !self.open; }

    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui) {
        let button = egui::Button::new(RichText::new("Patch Engine").strong()).selected(self.open);
        if ui.add(button).on_hover_text("Compose a new patch from three words").clicked() { self.toggle(); }
    }

    fn ready(&self) -> bool {
        self.genre.is_some() && self.mood.is_some() && self.texture.is_some() && !self.building && self.pending.is_none() && self.armed.is_none()
    }

    /// Document title for the next build; the app names the fresh document with it.
    pub(crate) fn prepared_title(&self) -> Option<String> {
        self.armed.as_ref().map(|a| format!("{} {} · {}", MOODS[a.m], genre_name(a.g), TEXTURES[a.t]))
    }

    /// The user cancelled the unsaved-changes prompt: forget the armed build.
    pub(crate) fn cancel_armed(&mut self) {
        if self.armed.take().is_some() {
            self.label = "Build cancelled; your patch is unchanged".into();
            self.started = None;
            if let Some(h) = self.history.first() { if h.4 == 0.0 { self.history.remove(0); } }
        }
    }

    fn start(&mut self, session: &SharedSession) {
        let (Some(g), Some(m), Some(t)) = (self.genre, self.mood, self.texture) else { return };
        if !self.ready() { return; }
        if !self.keep_seed || self.seed == 0 { self.seed = next_seed(&mut self.rng); }
        let seed = self.seed;
        let (mode, interval) = (self.mode, self.interval_ms);
        self.started = Some(Instant::now());
        self.elapsed = None;
        self.error = None;
        self.remaining = None;
        self.done = 0;
        self.total = 0;
        self.label = "Composing…".into();
        self.history.insert(0, (g, m, t, seed, 0.0));
        self.history.truncate(8);
        let budget = self.light.then_some(LIGHT_BUDGET);
        let _ = session;
        self.armed = Some(Armed { g, m, t, seed, mode, interval, budget });
        self.prepare_sent = false;
        self.label = "Starting a new document…".into();
    }

    /// The app has installed a fresh, empty document for this build: compose and construct it.
    pub(crate) fn launch(&mut self, session: &SharedSession) {
        let Some(Armed { g, m, t, seed, mode, interval, budget }) = self.armed.take() else { return };
        self.label = "Composing…".into();
        self.just_launched = true;
        let session = session.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let result = (|| -> Result<Built, String> {
                let r = Recipe::with_budget(genre_name(g), MOODS[m], TEXTURES[t], seed, budget)?;
                let title = format!("Patch Engine · {}", r.title);
                let steps = match mode {
                    Mode::Instant => {
                        for chunk in r.edits().chunks(256) {
                            execute_shared(&session, "apply_edits", json!({"edits": chunk, "expected_revision": revision(&session)?}))?;
                        }
                        return Ok(Built { info: r.info(), steps: vec![], plan: false });
                    }
                    Mode::Reveal => {
                        let (modules, steps) = r.reveal();
                        for chunk in modules.chunks(256) {
                            execute_shared(&session, "apply_edits", json!({"edits": chunk, "expected_revision": revision(&session)?}))?;
                        }
                        steps
                    }
                    Mode::Showtime => r.showtime(),
                };
                execute_shared(&session, "start_construction", json!({"expected_revision": revision(&session)?, "interval_ms": interval, "steps": steps, "title": title}))?;
                Ok(Built { info: r.info(), steps, plan: true })
            })();
            let _ = tx.send(result);
        });
    }

    fn resume(&mut self, session: &SharedSession) {
        let Some(steps) = self.remaining.take() else { return };
        let result = revision(session).and_then(|rev| execute_shared(session, "start_construction",
            json!({"expected_revision": rev, "interval_ms": self.interval_ms, "steps": steps.clone(), "title": "Patch Engine · resume"})));
        match result {
            Ok(_) => { self.plan_steps = steps; self.building = true; self.error = None; self.started = Some(Instant::now()); self.elapsed = None; }
            Err(e) => { self.error = Some(e); self.remaining = Some(steps); }
        }
    }

    /// Track the running construction plan.
    fn poll(&mut self, session: &SharedSession) {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(Ok(built)) => {
                    self.pending = None;
                    self.info = Some(built.info);
                    if built.plan {
                        self.total = built.steps.len();
                        self.plan_steps = built.steps;
                        self.building = true;
                        self.label = "Building…".into();
                    } else {
                        self.finish("Built in one step");
                    }
                }
                Ok(Err(e)) => { self.pending = None; self.error = Some(e); self.label.clear(); }
                Err(mpsc::TryRecvError::Disconnected) => { self.pending = None; }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if !self.building || self.last_poll.is_some_and(|t| t.elapsed() < Duration::from_millis(60)) { return; }
        self.last_poll = Some(Instant::now());
        let Ok(s) = execute_shared(session, "inspect_construction", json!({})) else { return };
        self.done = s["completed_steps"].as_u64().unwrap_or(0) as usize;
        self.total = s["total_steps"].as_u64().unwrap_or(self.total as u64) as usize;
        if let Some(l) = s["last_label"].as_str() { self.label = l.to_string(); }
        match s["status"].as_str() {
            Some("completed") => self.finish("Built"),
            Some("conflicted") => {
                self.building = false;
                self.remaining = Some(self.plan_steps[self.done.min(self.plan_steps.len())..].to_vec());
                self.error = Some(format!("The rack was edited during the build, so it stopped after {}/{} steps. Resume finishes the rest.", self.done, self.total));
            }
            Some("cancelled") | Some("failed") => { self.building = false; self.label = format!("Build {}", s["status"].as_str().unwrap_or("stopped")); }
            _ => {}
        }
    }

    fn finish(&mut self, label: &str) {
        self.building = false;
        self.elapsed = self.started.map(|t| t.elapsed());
        self.label = label.into();
        self.done = self.total;
        if let (Some(h), Some(e)) = (self.history.first_mut(), self.elapsed) { h.4 = e.as_secs_f32(); }
    }

    fn sentence(&self) -> LayoutJob {
        let mut job = LayoutJob::default();
        let font = FontId::new(24.0, self.face_family.clone());
        let plain = TextFormat { font_id: font.clone(), color: Color32::from_gray(225), ..Default::default() };
        let blank = TextFormat { font_id: font.clone(), color: Color32::from_gray(95), ..Default::default() };
        let colored = |c| TextFormat { font_id: font.clone(), color: c, ..Default::default() };
        job.append("A ", 0.0, plain.clone());
        match self.mood { Some(m) => job.append(&MOODS[m].to_lowercase(), 0.0, colored(MOOD_COLOR)), None => job.append("mood", 0.0, blank.clone()) }
        job.append(" ", 0.0, plain.clone());
        match self.genre { Some(g) => job.append(genre_noun(genre_name(g)), 0.0, colored(GENRE_COLOR)), None => job.append("genre", 0.0, blank.clone()) }
        job.append(" ", 0.0, plain.clone());
        match self.texture { Some(t) => job.append(texture_phrase(TEXTURES[t]), 0.0, colored(TEXTURE_COLOR)), None => job.append("texture", 0.0, blank) }
        job.append(".", 0.0, plain);
        job.wrap.max_width = f32::INFINITY;
        job
    }

    fn section(ui: &mut egui::Ui, title: &str, keys: &str, color: Color32, focused: bool) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), egui::Sense::hover());
        let p = ui.painter();
        if focused { p.rect_filled(egui::Rect::from_min_size(rect.left_top() + egui::vec2(-14.0, 1.0), egui::vec2(3.0, 14.0)), 1.5_f32, color); }
        p.text(rect.left_center(), egui::Align2::LEFT_CENTER, title, face(ui, 13.0), if focused { color } else { color.gamma_multiply(0.85) });
        p.text(rect.right_center(), egui::Align2::RIGHT_CENTER, keys, face(ui, 12.0), Color32::from_rgb(96, 100, 96));
    }

    fn words(ui: &mut egui::Ui, words: &[&str], selected: &mut Option<usize>, color: Color32, tips: Option<fn(&str) -> &'static str>) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(5.0, 5.0);
            for (i, w) in words.iter().enumerate() {
                if chip(ui, w, *selected == Some(i), color, tips.map(|t| t(w))).clicked() { *selected = Some(i); }
            }
        });
    }

    fn keyboard(&mut self, ctx: &egui::Context, session: &SharedSession) {
        if ctx.wants_keyboard_input() { return; }
        let pressed = |k: egui::Key| ctx.input(|i| i.key_pressed(k) && i.modifiers.is_none());
        for (i, k) in GENRE_KEYS.iter().enumerate() { if pressed(*k) { self.genre = Some(i); } }
        for (i, k) in MOOD_KEYS.iter().enumerate() { if pressed(*k) { self.mood = Some(i); } }
        for (i, k) in TEXTURE_KEYS.iter().enumerate() { if pressed(*k) { self.texture = Some(i); } }
        if pressed(egui::Key::ArrowUp) { self.cursor = (self.cursor + 2) % 3; }
        if pressed(egui::Key::ArrowDown) { self.cursor = (self.cursor + 1) % 3; }
        let step = if pressed(egui::Key::ArrowRight) { Some(1) } else if pressed(egui::Key::ArrowLeft) { Some(7) } else { None };
        if let Some(step) = step {
            let slot = match self.cursor { 0 => &mut self.genre, 1 => &mut self.mood, _ => &mut self.texture };
            *slot = Some(slot.map_or(0, |i| (i + step) % 8));
        }
        if pressed(egui::Key::Enter) { self.start(session); }
        if pressed(egui::Key::Slash) || pressed(egui::Key::Space) { self.surprise(session); }
        if pressed(egui::Key::Escape) { self.open = false; }
    }

    fn surprise(&mut self, session: &SharedSession) {
        self.genre = Some((next_seed(&mut self.rng) % 9) as usize);
        self.mood = Some((next_seed(&mut self.rng) % 8) as usize);
        self.texture = Some((next_seed(&mut self.rng) % 8) as usize);
        self.keep_seed = false;
        self.start(session);
    }

    fn run_demo(&mut self, ctx: &egui::Context, session: &SharedSession) {
        let Some(mut demo) = self.demo.take() else { return };
        if !demo.began {
            demo.began = true;
            if let Some(size) = demo.size { ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size)); }
            if let Some((g, m, t, seed)) = demo.words {
                (self.genre, self.mood, self.texture, self.seed, self.keep_seed, self.open) = (Some(g), Some(m), Some(t), seed, true, true);
            }
        }
        if demo.words.is_some() && demo.start.elapsed() > Duration::from_millis(1500) && self.history.is_empty() { self.start(session); demo.start = Instant::now(); }
        let t = demo.start.elapsed().as_secs_f32();
        if demo.waiting.is_none() && !self.history.is_empty() {
            if let Some(i) = demo.captures.iter().position(|c| t >= c.1) {
                let (path, _) = demo.captures.remove(i);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                demo.waiting = Some(path);
            }
        }
        if let Some(path) = demo.waiting.clone() {
            let shot = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
            if let Some(image) = shot {
                match image::save_buffer(&path, image.as_raw(), image.width() as u32, image.height() as u32, image::ColorType::Rgba8) {
                    Ok(()) => eprintln!("Patch Engine capture: {}", path.display()),
                    Err(e) => eprintln!("Patch Engine capture failed: {e}"),
                }
                demo.waiting = None;
            }
        }
        ctx.request_repaint_after(Duration::from_millis(30));
        self.demo = Some(demo);
    }

    pub(crate) fn show(&mut self, ctx: &egui::Context, session: &SharedSession) -> Vec<Request> {
        if std::mem::take(&mut self.first_frame) && ctx.content_rect().height() <= 900.0 {
            self.open = true;   // small screens (Steam Deck): start with the Patch Engine open
        }
        let was_idle = !self.building && self.pending.is_none();
        self.run_demo(ctx, session);
        self.poll(session);
        if !self.open { return vec![]; }
        if self.building || self.pending.is_some() { ctx.request_repaint_after(Duration::from_millis(50)); }
        self.keyboard(ctx, session);
        self.face_family = crate::panel_skin::heading_font(ctx, 1.0).family;
        let narrow = ctx.content_rect().width() < 1400.0;
        let mut close = false;
        egui::SidePanel::right("patch-engine-sheet")
            .resizable(false)
            .exact_width(if narrow { 380.0 } else { 420.0 })
            .frame(egui::Frame::new().fill(SHEET).inner_margin(egui::Margin { left: 16, right: 14, top: 12, bottom: 10 })
                .stroke(egui::Stroke::new(1.0_f32, LINE)))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                // header
                ui.horizontal(|ui| {
                    let (bar, _) = ui.allocate_exact_size(egui::vec2(3.0, 16.0), egui::Sense::hover());
                    ui.painter().rect_filled(bar, 1.5_f32, AMBER);
                    ui.label(RichText::new("PATCH ENGINE").font(face(ui, 16.0)).color(INK));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                        let c = if resp.hovered() { INK } else { DIM };
                        let k = r.shrink(7.0);
                        ui.painter().line_segment([k.left_top(), k.right_bottom()], egui::Stroke::new(1.5_f32, c));
                        ui.painter().line_segment([k.right_top(), k.left_bottom()], egui::Stroke::new(1.5_f32, c));
                        if resp.on_hover_text("Close (F1 or B)").clicked() { close = true; }
                        ui.label(RichText::new("F1").font(face(ui, 12.0)).color(Color32::from_rgb(96, 100, 96)));
                    });
                });
                // display: the sentence on a recessed screen
                let width = ui.available_width();
                let mut job = self.sentence();
                job.wrap.max_width = width - 24.0;
                let galley = ui.painter().layout_job(job);
                let caption = match &self.info {
                    Some(info) => format!("{} BPM · {} · {}", info["bpm"], info["key"].as_str().unwrap_or(""),
                                          info["chords"].as_array().into_iter().flatten().filter_map(|c| c.as_str()).collect::<Vec<_>>().join("  ")),
                    None => "Choose a genre, a mood and a texture.".into(),
                };
                let height = galley.size().y + 42.0;
                let (screen, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                ui.painter().rect(screen, 9.0_f32, WELL, egui::Stroke::new(1.0_f32, Color32::from_rgb(44, 47, 46)), egui::StrokeKind::Inside);
                ui.painter().galley(screen.left_top() + egui::vec2(12.0, 10.0), galley, INK);
                ui.painter().text(screen.left_bottom() + egui::vec2(12.0, -10.0), egui::Align2::LEFT_BOTTOM, caption, face(ui, 13.0), DIM);
                ui.add_space(2.0);
                // primary action: the build bar doubles as the progress meter
                let secs = self.elapsed.or_else(|| self.started.filter(|_| self.building || self.pending.is_some()).map(|t| t.elapsed()));
                let busy = self.building || self.pending.is_some() || self.armed.is_some();
                let (bar, bar_resp) = ui.allocate_exact_size(egui::vec2(width, 44.0), egui::Sense::click());
                {
                    let p = ui.painter();
                    let ready = self.ready();
                    let hover = bar_resp.hovered() && ready;
                    if busy {
                        p.rect(bar, 9.0_f32, RAISED, egui::Stroke::new(1.0_f32, AMBER.gamma_multiply(0.5)), egui::StrokeKind::Inside);
                        let frac = if self.total == 0 { 0.04 } else { (self.done as f32 / self.total as f32).max(0.04) };
                        let fill = egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * frac, bar.height()));
                        p.rect_filled(fill, 9.0_f32, AMBER.gamma_multiply(0.85));
                        p.text(bar.left_center() + egui::vec2(14.0, 0.0), egui::Align2::LEFT_CENTER, "BUILDING", face(ui, 19.0), DARK_INK);
                        p.text(bar.right_center() - egui::vec2(14.0, 0.0), egui::Align2::RIGHT_CENTER,
                               format!("{}  {:.2} s", self.label, secs.map_or(0.0, |d| d.as_secs_f32())), face(ui, 13.0), INK);
                    } else {
                        let fill = if !ready { RAISED } else if hover { Color32::from_rgb(240, 200, 125) } else { AMBER };
                        p.rect_filled(bar, 9.0_f32, fill);
                        let (title, ink) = if ready { (if self.history.is_empty() { "BUILD" } else { "BUILD AGAIN" }, DARK_INK) } else { ("PICK THREE WORDS", DIM) };
                        p.text(bar.left_center() + egui::vec2(14.0, 0.0), egui::Align2::LEFT_CENTER, title, face(ui, 20.0), ink);
                        let right = match secs { Some(d) if ready => format!("last build {:.2} s   A", d.as_secs_f32()), _ if ready => "A".into(), _ => String::new() };
                        p.text(bar.right_center() - egui::vec2(14.0, 0.0), egui::Align2::RIGHT_CENTER, right, face(ui, 13.0), ink.gamma_multiply(0.75));
                    }
                }
                if bar_resp.clicked() && self.ready() { self.start(session); }
                // secondary row: surprise · seed
                ui.horizontal(|ui| {
                    let half = (width - 6.0) / 2.0;
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(half, 30.0), egui::Sense::click());
                    let enabled = !busy;
                    ui.painter().rect(r, 8.0_f32, if resp.hovered() && enabled { RAISED } else { SHEET }, egui::Stroke::new(1.0_f32, LINE), egui::StrokeKind::Inside);
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "SURPRISE ME   X", face(ui, 14.0), if enabled { INK } else { DIM });
                    if resp.on_hover_text("Random words, new seed").clicked() && enabled { self.surprise(session); }
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(half, 30.0), egui::Sense::click());
                    let kept = self.keep_seed;
                    ui.painter().rect(r, 8.0_f32, if resp.hovered() { RAISED } else { SHEET }, egui::Stroke::new(1.0_f32, if kept { AMBER.gamma_multiply(0.6) } else { LINE }), egui::StrokeKind::Inside);
                    let text = if self.seed == 0 { "NEW SONG EVERY BUILD".to_string() } else if kept { format!("KEEPING SEED {}", self.seed) } else { format!("SEED {}  ·  KEEP", self.seed) };
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, text, face(ui, 14.0), if kept { AMBER } else { DIM });
                    if resp.on_hover_text("Every build composes a new song. Keep the seed to rebuild this one.").clicked() { self.keep_seed = !self.keep_seed; }
                });
                if let Some(e) = &self.error { ui.label(RichText::new(e).font(face(ui, 13.0)).color(Color32::from_rgb(232, 140, 100))); }
                if self.remaining.is_some() && ui.button("Resume build").clicked() { self.resume(session); }
                ui.add_space(4.0);
                // word lists
                egui::ScrollArea::vertical().id_salt("patch-engine-words").auto_shrink([false, true]).max_height(ui.available_height() - 112.0).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    let mc = self.genre == Some(MINECRAFT);
                    Self::section(ui, "GENRE", "1–8", GENRE_COLOR, self.cursor == 0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(5.0, 5.0);
                        for (i, w) in GENRES.iter().enumerate() {
                            if chip(ui, w, self.genre == Some(i), GENRE_COLOR, None).clicked() { self.genre = Some(i); }
                        }
                        if chip(ui, "C418 · Minecraft", mc, C418_COLOR, Some("Gentle piano, slow drifting harmony, lots of space: original music in the spirit of C418")).clicked() {
                            self.genre = Some(MINECRAFT);
                        }
                    });
                    ui.add_space(4.0);
                    Self::section(ui, "MOOD", "Q–I", MOOD_COLOR, self.cursor == 1);
                    Self::words(ui, &MOODS, &mut self.mood, MOOD_COLOR, None);
                    ui.add_space(4.0);
                    Self::section(ui, "TEXTURE", "A–K", TEXTURE_COLOR, self.cursor == 2);
                    Self::words(ui, &TEXTURES, &mut self.texture, TEXTURE_COLOR, Some(texture_text));
                    ui.add_space(6.0);
                    // recipe card, styled like a rack module face
                    if let Some(info) = &self.info {
                        egui::Frame::new().fill(IVORY).corner_radius(8.0).stroke(egui::Stroke::new(1.0_f32, IVORY_LINE)).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
                            ui.label(RichText::new(info["title"].as_str().unwrap_or("").to_uppercase()).font(face(ui, 16.0)).color(IVORY_INK));
                            ui.label(RichText::new(info["description"].as_str().unwrap_or("")).font(face(ui, 13.5)).color(IVORY_INK));
                            let notes: Vec<&str> = info["notes"].as_array().into_iter().flatten().filter_map(|n| n.as_str()).collect();
                            ui.label(RichText::new(format!("{}  ·  {} modules  ·  {} cables  ·  seed {}", notes.join("  ·  "), info["modules"], info["cables"], info["seed"]))
                                .font(face(ui, 12.0)).color(Color32::from_rgb(110, 104, 92)));
                        });
                    }
                    if self.history.len() > 1 {
                        ui.add_space(4.0);
                        Self::section(ui, "RECENT", "", DIM, false);
                        let mut replay = None;
                        for (i, &(g, m, t, seed, secs)) in self.history.iter().enumerate().skip(1).take(4) {
                            let text = format!("{} {} · {}   seed {}{}", MOODS[m], genre_name(g), TEXTURES[t], seed, if secs > 0.0 { format!("   {secs:.2} s") } else { String::new() });
                            let resp = ui.add(egui::Label::new(RichText::new(text).font(face(ui, 13.0)).color(DIM)).sense(egui::Sense::click()));
                            if resp.on_hover_text("Choose these words and seed").clicked() { replay = Some(i); }
                        }
                        if let Some(i) = replay {
                            let (g, m, t, seed, _) = self.history[i];
                            (self.genre, self.mood, self.texture, self.seed, self.keep_seed) = (Some(g), Some(m), Some(t), seed, true);
                        }
                    }
                });
                // options, pinned to the bottom of the sheet
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    ui.label(RichText::new("D-PAD ROW / WORD   ·   A BUILD   ·   X SURPRISE   ·   B CLOSE   ·   PGUP/PGDN ZOOM   ·   F11 FULL SCREEN")
                        .font(face(ui, 11.0)).color(Color32::from_rgb(92, 96, 92)));
                    ui.horizontal(|ui| {
                        pill(ui, "PLAY", &mut self.autoplay, "Start playback when a build begins");
                        pill(ui, "LIGHT", &mut self.light, "Keep the build within a Steam Deck's CPU: optional layers are left out when a song would be too heavy");
                        if self.mode != Mode::Instant {
                            let speeds = [100u32, 150, 250];
                            let cur = speeds.iter().position(|&v| v == self.interval_ms).unwrap_or(1);
                            if let Some(i) = segmented(ui, &["FAST", "MEDIUM", "SLOW"], cur, ui.available_width()) { self.interval_ms = speeds[i]; }
                        }
                    });
                    let modes = [Mode::Reveal, Mode::Showtime, Mode::Instant];
                    let cur = modes.iter().position(|&m| m == self.mode).unwrap_or(0);
                    if let Some(i) = segmented(ui, &["REVEAL", "SHOWTIME", "INSTANT"], cur, width) { self.mode = modes[i]; }
                    ui.label(RichText::new("BUILD STYLE").font(face(ui, 12.0)).color(Color32::from_rgb(110, 114, 108)));
                });
            });
        if close { self.open = false; }
        let started = std::mem::take(&mut self.just_launched) || (was_idle && (self.building || self.pending.is_some()));
        let mut requests = vec![];
        if self.armed.is_some() && !self.prepare_sent { requests.push(Request::Prepare); self.prepare_sent = true; }
        if started && self.autoplay { requests.push(Request::StartPlayback); }
        if started { requests.push(Request::ShowPatch); }
        requests
    }
}
