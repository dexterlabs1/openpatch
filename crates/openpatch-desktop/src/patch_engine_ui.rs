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
const GENRE_COLOR: Color32 = Color32::from_rgb(232, 122, 82);
const MOOD_COLOR: Color32 = Color32::from_rgb(122, 162, 255);
const TEXTURE_COLOR: Color32 = Color32::from_rgb(86, 200, 150);

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
            light: cfg!(target_os = "linux"), first_frame: true, armed: None, prepare_sent: false, just_launched: false,
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
        let font = FontId::proportional(19.0);
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
        job
    }

    fn word_row(ui: &mut egui::Ui, title: &str, words: &[&str], selected: &mut Option<usize>, color: Color32, focused: bool, tips: Option<fn(&str) -> &'static str>) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            let t = RichText::new(format!("{}{title}", if focused { "› " } else { "  " })).monospace().small();
            ui.add_sized([62.0, 26.0], egui::Label::new(if focused { t.color(color).strong() } else { t.weak() }));
            for (i, w) in words.iter().enumerate() {
                let on = *selected == Some(i);
                let text = if on { RichText::new(*w).strong().size(13.5).color(Color32::BLACK) } else { RichText::new(*w).size(13.5) };
                let mut b = egui::Button::new(text).corner_radius(12.0).min_size(egui::vec2(0.0, 28.0));
                if on { b = b.fill(color); }
                let mut r = ui.add(b);
                if let Some(tip) = tips { r = r.on_hover_text(tip(w)); }
                if r.clicked() { *selected = Some(i); }
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
        let mut open = self.open;
        egui::Window::new("Patch Engine")
            .open(&mut open)
            .resizable(false)
            .collapsible(true)
            .vscroll(true)
            .max_height(ctx.content_rect().height() - 80.0)
            .default_width(470.0)
            .anchor(egui::Align2::RIGHT_TOP, [-12.0, 58.0])
            .show(ctx, |ui| {
                ui.set_width(470.0);
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 5.0);
                ui.label(self.sentence());
                // build row: actions + live timer/progress
                ui.horizontal(|ui| {
                    let build = ui.add_enabled(self.ready(), egui::Button::new(RichText::new("Build it").strong().size(14.0)).min_size(egui::vec2(92.0, 30.0)));
                    if build.clicked() { self.start(session); }
                    if ui.add_enabled(!self.building && self.pending.is_none() && self.armed.is_none(), egui::Button::new("Surprise").min_size(egui::vec2(0.0, 30.0))).clicked() { self.surprise(session); }
                    let secs = self.elapsed.or_else(|| self.started.filter(|_| self.building || self.pending.is_some()).map(|t| t.elapsed()));
                    ui.label(RichText::new(secs.map_or("–.–– s".to_string(), |d| format!("{:.2} s", d.as_secs_f32()))).monospace().size(15.0));
                    let frac = if self.total == 0 { 0.0 } else { self.done as f32 / self.total as f32 };
                    ui.add(egui::ProgressBar::new(frac).desired_width(ui.available_width()).desired_height(18.0).text(RichText::new(self.label.clone()).small()));
                });
                if let Some(e) = &self.error { ui.colored_label(Color32::from_rgb(240, 130, 90), RichText::new(e).small()); }
                if self.remaining.is_some() && ui.button("Resume build").clicked() { self.resume(session); }
                // words
                let mc = self.genre == Some(MINECRAFT);
                ui.add_enabled_ui(!mc, |ui| Self::word_row(ui, "GENRE", &GENRES, &mut self.genre, GENRE_COLOR, self.cursor == 0, None));
                Self::word_row(ui, "MOOD", &MOODS, &mut self.mood, MOOD_COLOR, self.cursor == 1, None);
                Self::word_row(ui, "TEXTURE", &TEXTURES, &mut self.texture, TEXTURE_COLOR, self.cursor == 2, Some(texture_text));
                // one slim options row
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(5.0, 3.0);
                    let mc_button = egui::Button::new(RichText::new("C418 mode").small().color(if mc { Color32::BLACK } else { Color32::from_gray(200) }))
                        .fill(if mc { Color32::from_rgb(120, 190, 110) } else { Color32::from_gray(45) }).corner_radius(10.0);
                    if ui.add(mc_button).on_hover_text("Minecraft-style: gentle piano, slow drifting harmony and lots of space (original music in the spirit of C418)").clicked() {
                        self.genre = if mc { None } else { Some(MINECRAFT) };
                    }
                    for (m, label, tip) in [(Mode::Reveal, "Reveal", "Place the rack, then patch it section by section (~2 s)"),
                                            (Mode::Showtime, "Show", "Modules arrive one by one (~5 s)"),
                                            (Mode::Instant, "Instant", "Everything at once")] {
                        if ui.selectable_label(self.mode == m, RichText::new(label).small()).on_hover_text(tip).clicked() { self.mode = m; }
                    }
                    if self.mode != Mode::Instant {
                        ui.spacing_mut().slider_width = 60.0;
                        ui.add(egui::Slider::new(&mut self.interval_ms, 100..=300).suffix(" ms").show_value(true));
                    }
                    ui.checkbox(&mut self.autoplay, RichText::new("Play").small());
                    ui.checkbox(&mut self.light, RichText::new("Light").small()).on_hover_text("Keep the build within a Steam Deck's CPU: optional layers are left out when a song would be too heavy.");
                    ui.checkbox(&mut self.keep_seed, RichText::new(if self.seed == 0 { "Keep seed".to_string() } else { format!("Seed {}", self.seed) }).small())
                        .on_hover_text("Every build composes a new song. Keep the seed to rebuild this one.");
                });
                if let Some(info) = &self.info {
                    let head = format!("{} · {} BPM · {} · {}", info["title"].as_str().unwrap_or(""), info["bpm"], info["key"].as_str().unwrap_or(""),
                                       info["chords"].as_array().into_iter().flatten().filter_map(|c| c.as_str()).collect::<Vec<_>>().join(" "));
                    egui::CollapsingHeader::new(RichText::new(head).small().strong()).id_salt("patch-engine-recipe").show(ui, |ui| {
                        ui.label(RichText::new(info["description"].as_str().unwrap_or("")).small());
                        let notes: Vec<&str> = info["notes"].as_array().into_iter().flatten().filter_map(|n| n.as_str()).collect();
                        ui.label(RichText::new(format!("{} · {} modules · {} cables · seed {}", notes.join(" · "), info["modules"], info["cables"], info["seed"])).small().weak());
                    });
                }
                if self.history.len() > 1 {
                    let mut replay = None;
                    egui::CollapsingHeader::new(RichText::new("Recent").small().weak()).id_salt("patch-engine-recent").show(ui, |ui| {
                        for (i, &(g, m, t, seed, secs)) in self.history.iter().enumerate().skip(1).take(5) {
                            let text = format!("{} {} · {} · seed {}{}", MOODS[m], genre_name(g), TEXTURES[t], seed, if secs > 0.0 { format!(" · {secs:.2} s") } else { String::new() });
                            if ui.small_button(text).on_hover_text("Select these words and seed").clicked() { replay = Some(i); }
                        }
                    });
                    if let Some(i) = replay {
                        let (g, m, t, seed, _) = self.history[i];
                        (self.genre, self.mood, self.texture, self.seed, self.keep_seed) = (Some(g), Some(m), Some(t), seed, true);
                    }
                }
                ui.label(RichText::new("D-pad row/word · A build · X surprise · B close · F1 panel · PgUp/PgDn zoom · Home view · F11 full screen").small().weak());
            });
        self.open = open;
        let started = std::mem::take(&mut self.just_launched) || (was_idle && (self.building || self.pending.is_some()));
        let mut requests = vec![];
        if self.armed.is_some() && !self.prepare_sent { requests.push(Request::Prepare); self.prepare_sent = true; }
        if started && self.autoplay { requests.push(Request::StartPlayback); }
        if started { requests.push(Request::ShowPatch); }
        requests
    }
}
