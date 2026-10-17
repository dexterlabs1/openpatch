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
const GENRE_COLOR: Color32 = Color32::from_rgb(232, 122, 82);
const MOOD_COLOR: Color32 = Color32::from_rgb(122, 162, 255);
const TEXTURE_COLOR: Color32 = Color32::from_rgb(86, 200, 150);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode { Reveal, Showtime, Instant }

/// Requests the app carries out after the panel draws.
pub(crate) enum Request { StartPlayback }

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
}

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
            Some((find(&GENRES, p.first()?)?, find(&MOODS, p.get(1)?)?, find(&TEXTURES, p.get(2)?)?, p.get(3).and_then(|s| s.parse().ok()).unwrap_or(1)))
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

    fn ready(&self) -> bool { self.genre.is_some() && self.mood.is_some() && self.texture.is_some() && !self.building && self.pending.is_none() }

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
        let session = session.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let result = (|| -> Result<Built, String> {
                let r = Recipe::new(GENRES[g], MOODS[m], TEXTURES[t], seed)?;
                let rev = revision(&session)?;
                execute_shared(&session, "replace_patch", json!({"patch": {"version": 2, "sample_rate": 48000, "nodes": [], "cables": []}, "expected_revision": rev}))?;
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
        let font = FontId::proportional(24.0);
        let plain = TextFormat { font_id: font.clone(), color: Color32::from_gray(225), ..Default::default() };
        let blank = TextFormat { font_id: font.clone(), color: Color32::from_gray(95), ..Default::default() };
        let colored = |c| TextFormat { font_id: font.clone(), color: c, ..Default::default() };
        job.append("A ", 0.0, plain.clone());
        match self.mood { Some(m) => job.append(&MOODS[m].to_lowercase(), 0.0, colored(MOOD_COLOR)), None => job.append("mood", 0.0, blank.clone()) }
        job.append(" ", 0.0, plain.clone());
        match self.genre { Some(g) => job.append(genre_noun(GENRES[g]), 0.0, colored(GENRE_COLOR)), None => job.append("genre", 0.0, blank.clone()) }
        job.append(" ", 0.0, plain.clone());
        match self.texture { Some(t) => job.append(texture_phrase(TEXTURES[t]), 0.0, colored(TEXTURE_COLOR)), None => job.append("texture", 0.0, blank) }
        job.append(".", 0.0, plain);
        job
    }

    fn word_row(ui: &mut egui::Ui, title: &str, keys: &str, words: &[&str], selected: &mut Option<usize>, color: Color32, focused: bool) {
        ui.horizontal(|ui| {
            let t = RichText::new(format!("{} {title}", if focused { ">" } else { " " })).monospace();
            ui.label(if focused { t.color(color).strong() } else { t.weak() });
            ui.label(RichText::new(keys).small().weak());
        });
        ui.horizontal_wrapped(|ui| {
            for (i, w) in words.iter().enumerate() {
                let on = *selected == Some(i);
                let text = if on { RichText::new(*w).strong().size(15.0).color(Color32::BLACK) } else { RichText::new(*w).size(15.0) };
                let mut b = egui::Button::new(text).corner_radius(14.0).min_size(egui::vec2(0.0, 34.0));
                if on { b = b.fill(color); }
                if ui.add(b).clicked() { *selected = Some(i); }
            }
        });
        ui.add_space(4.0);
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
        self.genre = Some((next_seed(&mut self.rng) % 8) as usize);
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

    pub(crate) fn show(&mut self, ctx: &egui::Context, session: &SharedSession) -> Option<Request> {
        let was_idle = !self.building && self.pending.is_none();
        self.run_demo(ctx, session);
        self.poll(session);
        if !self.open { return None; }
        if self.building || self.pending.is_some() { ctx.request_repaint_after(Duration::from_millis(50)); }
        self.keyboard(ctx, session);
        let mut open = self.open;
        egui::Window::new("Patch Engine")
            .open(&mut open)
            .resizable(false)
            .collapsible(true)
            .vscroll(true)
            .max_height(ctx.content_rect().height() - 80.0)
            .default_width(560.0)
            .anchor(egui::Align2::RIGHT_TOP, [-14.0, 62.0])
            .show(ctx, |ui| {
                ui.set_width(560.0);
                ui.label(self.sentence());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let build = ui.add_enabled(self.ready(), egui::Button::new(RichText::new("  Build it  ").strong().size(15.0)).min_size(egui::vec2(120.0, 32.0)));
                    if build.clicked() { self.start(session); }
                    if ui.add_enabled(!self.building && self.pending.is_none(), egui::Button::new(RichText::new("Surprise me").size(15.0)).min_size(egui::vec2(0.0, 32.0))).clicked() { self.surprise(session); }
                    ui.checkbox(&mut self.keep_seed, if self.seed == 0 { "Keep seed".to_string() } else { format!("Keep seed {}", self.seed) })
                        .on_hover_text("Every build composes a new song. Keep the seed to rebuild this one.");
                });
                ui.horizontal(|ui| {
                    for (m, label, tip) in [(Mode::Reveal, "Reveal", "Place the rack, then patch it section by section (~2 s)"),
                                            (Mode::Showtime, "Showtime", "Modules arrive one by one (~5 s)"),
                                            (Mode::Instant, "Instant", "Everything at once")] {
                        if ui.selectable_label(self.mode == m, label).on_hover_text(tip).clicked() { self.mode = m; }
                    }
                    if self.mode != Mode::Instant {
                        ui.add(egui::Slider::new(&mut self.interval_ms, 100..=300).suffix(" ms").text("step"));
                    }
                    ui.checkbox(&mut self.autoplay, "Play");
                });
                let secs = self.elapsed.or_else(|| self.started.filter(|_| self.building || self.pending.is_some()).map(|t| t.elapsed()));
                ui.horizontal(|ui| {
                    ui.label(RichText::new(secs.map_or("–.–– s".to_string(), |d| format!("{:.2} s", d.as_secs_f32()))).monospace().size(22.0));
                    let frac = if self.total == 0 { 0.0 } else { self.done as f32 / self.total as f32 };
                    ui.add(egui::ProgressBar::new(frac).desired_width(300.0).text(self.label.clone()));
                });
                if let Some(e) = &self.error { ui.colored_label(Color32::from_rgb(240, 130, 90), e); }
                if self.remaining.is_some() && ui.button("Resume build").clicked() { self.resume(session); }
                ui.separator();
                Self::word_row(ui, "GENRE", "1–8", &GENRES, &mut self.genre, GENRE_COLOR, self.cursor == 0);
                Self::word_row(ui, "MOOD", "Q–I", &MOODS, &mut self.mood, MOOD_COLOR, self.cursor == 1);
                Self::word_row(ui, "TEXTURE", "A–K", &TEXTURES, &mut self.texture, TEXTURE_COLOR, self.cursor == 2);
                if let Some(t) = self.texture { ui.label(RichText::new(texture_text(TEXTURES[t])).small().weak()); }
                ui.separator();
                if let Some(info) = &self.info {
                    ui.add_space(6.0);
                    ui.label(RichText::new(info["title"].as_str().unwrap_or("")).strong().size(16.0));
                    ui.horizontal_wrapped(|ui| {
                        for fact in [format!("{} BPM", info["bpm"]), info["key"].as_str().unwrap_or("").to_string(), format!("swing {}%", info["swing"]),
                                     format!("{} modules · {} cables", info["modules"], info["cables"]), format!("seed {}", info["seed"])] {
                            ui.label(RichText::new(fact).monospace().small());
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        for c in info["chords"].as_array().into_iter().flatten() { ui.label(RichText::new(c.as_str().unwrap_or("")).strong()); }
                    });
                    ui.label(RichText::new(info["description"].as_str().unwrap_or("")).small());
                    let notes: Vec<&str> = info["notes"].as_array().into_iter().flatten().filter_map(|n| n.as_str()).collect();
                    ui.label(RichText::new(notes.join(" · ")).small().weak());
                }
                if self.history.len() > 1 {
                    let mut replay = None;
                    egui::CollapsingHeader::new(RichText::new("Recent").small().weak()).id_salt("patch-engine-recent").show(ui, |ui| {
                        for (i, &(g, m, t, seed, secs)) in self.history.iter().enumerate().skip(1).take(5) {
                            let text = format!("{} {} · {} · seed {}{}", MOODS[m], GENRES[g], TEXTURES[t], seed, if secs > 0.0 { format!(" · {secs:.2} s") } else { String::new() });
                            if ui.button(text).on_hover_text("Select these words and seed").clicked() { replay = Some(i); }
                        }
                    });
                    if let Some(i) = replay {
                        let (g, m, t, seed, _) = self.history[i];
                        (self.genre, self.mood, self.texture, self.seed, self.keep_seed) = (Some(g), Some(m), Some(t), seed, true);
                    }
                }
                ui.add_space(4.0);
                ui.label(RichText::new("Steam Deck: D-pad up/down picks a row, left/right a word · A builds · X surprises · B closes. Building replaces the rack; Undo brings it back.").small().weak());
            });
        self.open = open;
        let started = was_idle && (self.building || self.pending.is_some());
        (started && self.autoplay).then_some(Request::StartPlayback)
    }
}
