//! Patch Engine: pick a genre, a mood and a texture; the engine composes a new piece and builds it in the rack.
use eframe::egui::{self, text::LayoutJob, Color32, FontId, RichText, TextFormat};
pub(crate) use openpatch::control::DocumentExecutionFence as DocumentFence;
use openpatch::control::{execute_shared, execute_shared_fenced, Session, SharedSession};
use openpatch::patch_engine::{
    genre_noun, texture_phrase, texture_text, Controls, Recipe, GENRES, MOODS, TEXTURES,
};
use serde_json::{json, Value};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const GENRE_KEYS: [egui::Key; 8] = [
    egui::Key::Num1,
    egui::Key::Num2,
    egui::Key::Num3,
    egui::Key::Num4,
    egui::Key::Num5,
    egui::Key::Num6,
    egui::Key::Num7,
    egui::Key::Num8,
];
const MOOD_KEYS: [egui::Key; 8] = [
    egui::Key::Q,
    egui::Key::W,
    egui::Key::E,
    egui::Key::R,
    egui::Key::T,
    egui::Key::Y,
    egui::Key::U,
    egui::Key::I,
];
const TEXTURE_KEYS: [egui::Key; 8] = [
    egui::Key::A,
    egui::Key::S,
    egui::Key::D,
    egui::Key::F,
    egui::Key::G,
    egui::Key::H,
    egui::Key::J,
    egui::Key::K,
];
/// Genre index used for the Minecraft (C418-style) mode, after the genre words.
const MINECRAFT: usize = GENRES.len();
fn genre_name(i: usize) -> &'static str {
    if i == MINECRAFT {
        "Minecraft"
    } else {
        GENRES[i]
    }
}
// Palette: the app's chrome, warm ivory and a single amber accent.
const SHEET: Color32 = Color32::from_rgb(25, 27, 27);
const WELL: Color32 = Color32::from_rgb(15, 16, 16);
const RAISED: Color32 = Color32::from_rgb(39, 42, 42);
const LINE: Color32 = Color32::from_rgb(55, 59, 57);
const INK: Color32 = Color32::from_rgb(235, 233, 226);
const DIM: Color32 = Color32::from_rgb(146, 149, 143);
const AMBER: Color32 = Color32::from_rgb(229, 186, 106);
const DARK_INK: Color32 = Color32::from_rgb(24, 24, 22);

fn face(ui: &egui::Ui, size: f32) -> FontId {
    crate::panel_skin::heading_font(ui.ctx(), size)
}

const CELL: Color32 = Color32::from_rgb(31, 34, 34);
const CELL_ON: Color32 = Color32::from_rgb(50, 46, 38);

/// One cell of a word matrix: flat, equal width, an amber edge and amber text when chosen.
fn cell(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    text: &str,
    selected: bool,
    tip: Option<&str>,
) -> egui::Response {
    let mut response = ui.interact(rect, ui.id().with(("cell", text)), egui::Sense::click());
    let p = ui.painter();
    let fill = if selected {
        CELL_ON
    } else if response.hovered() {
        RAISED
    } else {
        CELL
    };
    p.rect(
        rect,
        3.0_f32,
        fill,
        egui::Stroke::new(
            1.0_f32,
            if selected {
                Color32::from_rgb(92, 78, 52)
            } else {
                LINE
            },
        ),
        egui::StrokeKind::Inside,
    );
    if selected {
        p.rect_filled(
            egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(0.0, 4.0),
                egui::vec2(2.0, rect.height() - 8.0),
            ),
            1.0_f32,
            AMBER,
        );
    }
    p.text(
        rect.left_center() + egui::vec2(10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        text,
        face(ui, 14.5),
        if selected { AMBER } else { INK },
    );
    if let Some(tip) = tip.filter(|t| !t.is_empty()) {
        response = response.on_hover_text(tip);
    }
    response
}

/// Words laid out as a fixed matrix (`cols` per row); returns the clicked index.
fn matrix(
    ui: &mut egui::Ui,
    words: &[&str],
    selected: Option<usize>,
    cols: usize,
    tips: Option<fn(&str) -> &'static str>,
) -> Option<usize> {
    let gap = 4.0;
    let width = ui.available_width();
    let w = (width - gap * (cols as f32 - 1.0)) / cols as f32;
    let rows = words.len().div_ceil(cols);
    let (area, _) = ui.allocate_exact_size(
        egui::vec2(width, rows as f32 * 30.0 + (rows as f32 - 1.0) * gap),
        egui::Sense::hover(),
    );
    let mut clicked = None;
    for (i, word) in words.iter().enumerate() {
        let (r, c) = (i / cols, i % cols);
        let rect = egui::Rect::from_min_size(
            area.min + egui::vec2(c as f32 * (w + gap), r as f32 * (30.0 + gap)),
            egui::vec2(w, 30.0),
        );
        if cell(ui, rect, word, selected == Some(i), tips.map(|t| t(word))).clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// A small segmented control; returns the newly chosen index.
fn segmented(ui: &mut egui::Ui, labels: &[&str], selected: usize, width: f32) -> Option<usize> {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 26.0), egui::Sense::hover());
    ui.painter().rect(
        rect,
        3.0_f32,
        CELL,
        egui::Stroke::new(1.0_f32, LINE),
        egui::StrokeKind::Inside,
    );
    let w = rect.width() / labels.len() as f32;
    let mut picked = None;
    for (i, label) in labels.iter().enumerate() {
        let r = egui::Rect::from_min_size(
            rect.min + egui::vec2(w * i as f32, 0.0),
            egui::vec2(w, rect.height()),
        )
        .shrink(2.0);
        let response = ui.interact(r, ui.id().with(("seg", label, i)), egui::Sense::click());
        let on = i == selected;
        if on {
            ui.painter().rect_filled(r, 2.0_f32, CELL_ON);
        } else if response.hovered() {
            ui.painter().rect_filled(r, 2.0_f32, RAISED);
        }
        if i > 0 && !on && i != selected + 1 {
            ui.painter().line_segment(
                [
                    r.left_top() + egui::vec2(-2.0, 5.0),
                    r.left_bottom() + egui::vec2(-2.0, -5.0),
                ],
                egui::Stroke::new(1.0_f32, LINE),
            );
        }
        ui.painter().text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            face(ui, 13.5),
            if on { AMBER } else { DIM },
        );
        if response.clicked() {
            picked = Some(i);
        }
    }
    picked
}

/// A plain check box: a small square that fills amber.
fn check(ui: &mut egui::Ui, label: &str, on: &mut bool, tip: &str) {
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        face(ui, 13.5),
        if *on { INK } else { DIM },
    );
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(galley.size().x + 22.0, 22.0),
        egui::Sense::click(),
    );
    let response = response.on_hover_text(tip);
    if response.clicked() {
        *on = !*on;
    }
    let bx = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 6.5, rect.center().y),
        egui::vec2(11.0, 11.0),
    );
    ui.painter().rect(
        bx,
        2.0_f32,
        if *on { AMBER } else { CELL },
        egui::Stroke::new(
            1.0_f32,
            if *on {
                AMBER
            } else {
                Color32::from_rgb(78, 82, 79)
            },
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(
        egui::pos2(rect.left() + 18.0, rect.center().y - galley.size().y / 2.0),
        galley,
        INK,
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Reveal,
    Showtime,
    Instant,
}

/// Requests the app carries out after the panel draws.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Request {
    StartPlayback,
    ShowPatch,
    Prepare,
}

/// CPU budget for Light mode (Patch Engine cost units ≈ % of an Apple M-series core); about 60% of a Steam Deck core.
const LIGHT_BUDGET: f64 = 18.0;

pub(crate) struct GeneratedDocument {
    fence: DocumentFence,
    content: Value,
}

impl GeneratedDocument {
    pub(crate) fn capture(session: &SharedSession, fence: DocumentFence) -> Option<Self> {
        let live = session.lock().ok()?;
        if !fence.matches(&live) {
            return None;
        }
        Some(Self {
            fence,
            content: live.project_edit_content(Some(fence.revision)).ok()?,
        })
    }

    pub(crate) fn matches(&self, session: &Session) -> bool {
        self.fence.matches(session)
            && session
                .project_edit_content(Some(self.fence.revision))
                .is_ok_and(|content| {
                    let status = session
                        .project_save_baseline()
                        .status_from_content(Some(&content));
                    content == self.content
                        && status["target"].is_null()
                        && status["saved_revision"].is_null()
                })
    }
}

struct Built {
    info: Value,
    steps: Vec<Value>,
    plan: bool,
    fence: DocumentFence,
}

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
    /// Energy, complexity, weirdness, space and length for the next build.
    ctl: Controls,
    first_frame: bool,
    armed: Option<Armed>,
    prepare_sent: bool,
    just_launched: bool,
    face_family: egui::FontFamily,
    details: bool,
    build_fence: Option<DocumentFence>,
    generated: Option<GeneratedDocument>,
}

/// A build waiting for the app to start a fresh document (which asks about unsaved changes first).
struct Armed {
    g: usize,
    m: usize,
    t: usize,
    seed: u64,
    mode: Mode,
    interval: u32,
    budget: Option<f64>,
    ctl: Controls,
}

/// Demo/capture mode for pitches and documentation screenshots, driven by environment variables:
/// `OPENPATCH_PATCH_ENGINE_DEMO="Genre|Mood|Texture|seed"` opens the panel and builds that piece on launch;
/// `OPENPATCH_CAPTURE="first.png@0.9,second.png@6"` saves the window at those times (seconds after launch);
/// `OPENPATCH_WINDOW="1280x800"` sizes the window (Steam Deck screen).
struct Demo {
    start: Instant,
    words: Option<(usize, usize, usize, u64)>,
    captures: Vec<(std::path::PathBuf, f32)>,
    size: Option<egui::Vec2>,
    began: bool,
    waiting: Option<std::path::PathBuf>,
}

impl Demo {
    fn from_env() -> Option<Demo> {
        let words = std::env::var("OPENPATCH_PATCH_ENGINE_DEMO")
            .ok()
            .and_then(|v| {
                let p: Vec<&str> = v.split('|').collect();
                let find =
                    |list: &[&str], w: &str| list.iter().position(|x| x.eq_ignore_ascii_case(w));
                let genre = if p.first()?.eq_ignore_ascii_case("minecraft") {
                    MINECRAFT
                } else {
                    find(&GENRES, p.first()?)?
                };
                Some((
                    genre,
                    find(&MOODS, p.get(1)?)?,
                    find(&TEXTURES, p.get(2)?)?,
                    p.get(3).and_then(|s| s.parse().ok()).unwrap_or(1),
                ))
            });
        let captures: Vec<_> = std::env::var("OPENPATCH_CAPTURE")
            .ok()
            .map(|v| {
                v.split(',')
                    .filter_map(|c| {
                        let (path, t) = c.rsplit_once('@')?;
                        Some((std::path::PathBuf::from(path), t.parse().ok()?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let size = std::env::var("OPENPATCH_WINDOW").ok().and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some(egui::vec2(w.parse().ok()?, h.parse().ok()?))
        });
        (words.is_some() || !captures.is_empty() || size.is_some()).then(|| Demo {
            start: Instant::now(),
            words,
            captures,
            size,
            began: false,
            waiting: None,
        })
    }
}

impl Default for Panel {
    fn default() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        Self {
            open: false,
            genre: None,
            mood: None,
            texture: None,
            seed: 0,
            keep_seed: false,
            mode: Mode::Reveal,
            interval_ms: 120,
            autoplay: true,
            rng: now | 1,
            pending: None,
            started: None,
            elapsed: None,
            building: false,
            plan_steps: vec![],
            done: 0,
            total: 0,
            label: String::new(),
            remaining: None,
            info: None,
            error: None,
            history: vec![],
            last_poll: None,
            cursor: 0,
            demo: Demo::from_env(),
            light: cfg!(target_os = "linux"),
            ctl: Controls::default(),
            first_frame: true,
            armed: None,
            prepare_sent: false,
            just_launched: false,
            face_family: egui::FontFamily::Proportional,
            details: false,
            build_fence: None,
            generated: None,
        }
    }
}

fn next_seed(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    1 + *state % 99_999
}

fn start_build_construction(
    session: &SharedSession,
    fence: DocumentFence,
    interval: u32,
    steps: &[Value],
    title: &str,
) -> Result<Value, String> {
    execute_shared_fenced(
        session,
        "start_construction",
        json!({"expected_revision": fence.revision, "interval_ms": interval, "steps": steps, "title": title}),
        fence,
    ).map(|(result, _)| result)
}

fn apply_build_edits(
    session: &SharedSession,
    fence: &mut DocumentFence,
    edits: &[Value],
) -> Result<(), String> {
    let (_, committed) = execute_shared_fenced(
        session,
        "apply_edits",
        json!({"edits": edits, "expected_revision": fence.revision}),
        *fence,
    )?;
    *fence = committed;
    Ok(())
}

fn revision(session: &SharedSession) -> Result<u64, String> {
    Ok(
        execute_shared(session, "inspect_construction", json!({}))?["current_revision"]
            .as_u64()
            .unwrap_or(0),
    )
}

impl Panel {
    pub(crate) fn take_generated(&mut self) -> Option<GeneratedDocument> {
        self.generated.take()
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }
    pub(crate) fn toggle(&mut self) {
        self.open = !self.open;
    }

    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui) {
        let button = egui::Button::new(RichText::new("Patch Engine").strong()).selected(self.open);
        if ui
            .add(button)
            .on_hover_text("Compose a new patch from three words")
            .clicked()
        {
            self.toggle();
        }
    }

    fn ready(&self) -> bool {
        self.genre.is_some()
            && self.mood.is_some()
            && self.texture.is_some()
            && !self.building
            && self.pending.is_none()
            && self.armed.is_none()
    }

    /// Document title for the next build; the app names the fresh document with it.
    pub(crate) fn prepared_title(&self) -> Option<String> {
        self.armed
            .as_ref()
            .map(|a| format!("{} {} · {}", MOODS[a.m], genre_name(a.g), TEXTURES[a.t]))
    }

    /// The user cancelled the unsaved-changes prompt: forget the armed build.
    pub(crate) fn cancel_armed(&mut self) {
        if self.armed.take().is_some() {
            self.label = "Build cancelled; your patch is unchanged".into();
            self.started = None;
            if let Some(h) = self.history.first() {
                if h.4 == 0.0 {
                    self.history.remove(0);
                }
            }
        }
    }

    fn start(&mut self, session: &SharedSession) {
        let (Some(g), Some(m), Some(t)) = (self.genre, self.mood, self.texture) else {
            return;
        };
        if !self.ready() {
            return;
        }
        if !self.keep_seed || self.seed == 0 {
            self.seed = next_seed(&mut self.rng);
        }
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
        self.armed = Some(Armed {
            g,
            m,
            t,
            seed,
            mode,
            interval,
            budget,
            ctl: self.ctl,
        });
        self.prepare_sent = false;
        self.label = "Starting a new document…".into();
    }

    /// The app has installed a fresh, empty document for this build: compose and construct it.
    pub(crate) fn launch(&mut self, session: &SharedSession, fence: DocumentFence) {
        let Some(Armed {
            g,
            m,
            t,
            seed,
            mode,
            interval,
            budget,
            ctl,
        }) = self.armed.take()
        else {
            return;
        };
        self.label = "Composing…".into();
        self.just_launched = true;
        let session = session.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        std::thread::spawn(move || {
            let result = (|| -> Result<Built, String> {
                let mut fence = fence;
                let r =
                    Recipe::with_controls(genre_name(g), MOODS[m], TEXTURES[t], seed, budget, ctl)?;
                let title = format!("Patch Engine · {}", r.title);
                let steps = match mode {
                    Mode::Instant => {
                        for chunk in r.edits().chunks(256) {
                            apply_build_edits(&session, &mut fence, chunk)?;
                        }
                        return Ok(Built {
                            info: r.info(),
                            steps: vec![],
                            plan: false,
                            fence,
                        });
                    }
                    Mode::Reveal => {
                        let (modules, steps) = r.reveal();
                        for chunk in modules.chunks(256) {
                            apply_build_edits(&session, &mut fence, chunk)?;
                        }
                        steps
                    }
                    Mode::Showtime => r.showtime(),
                };
                start_build_construction(&session, fence, interval, &steps, &title)?;
                Ok(Built {
                    info: r.info(),
                    steps,
                    plan: true,
                    fence,
                })
            })();
            let _ = tx.send(result);
        });
    }

    fn resume(&mut self, session: &SharedSession) {
        let Some(steps) = self.remaining.take() else {
            return;
        };
        let result = revision(session).and_then(|rev| execute_shared(session, "start_construction",
            json!({"expected_revision": rev, "interval_ms": self.interval_ms, "steps": steps.clone(), "title": "Patch Engine · resume"})));
        match result {
            Ok(_) => {
                self.plan_steps = steps;
                self.building = true;
                self.error = None;
                self.started = Some(Instant::now());
                self.elapsed = None;
            }
            Err(e) => {
                self.error = Some(e);
                self.remaining = Some(steps);
            }
        }
    }

    /// Track the running construction plan.
    fn poll(&mut self, session: &SharedSession) {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(Ok(built)) => {
                    self.pending = None;
                    self.info = Some(built.info);
                    self.build_fence = Some(built.fence);
                    if built.plan {
                        self.total = built.steps.len();
                        self.plan_steps = built.steps;
                        self.building = true;
                        self.label = "Building…".into();
                    } else {
                        self.generated = GeneratedDocument::capture(session, built.fence);
                        self.finish("Built in one step");
                    }
                }
                Ok(Err(e)) => {
                    self.pending = None;
                    self.error = Some(e);
                    self.label.clear();
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if !self.building
            || self
                .last_poll
                .is_some_and(|t| t.elapsed() < Duration::from_millis(60))
        {
            return;
        }
        self.last_poll = Some(Instant::now());
        let Ok(s) = execute_shared(session, "inspect_construction", json!({})) else {
            return;
        };
        self.done = s["completed_steps"].as_u64().unwrap_or(0) as usize;
        self.total = s["total_steps"].as_u64().unwrap_or(self.total as u64) as usize;
        if let Some(l) = s["last_label"].as_str() {
            self.label = l.to_string();
        }
        match s["status"].as_str() {
            Some("completed") => {
                if let Some(mut fence) = self.build_fence.take() {
                    // Never derive a baseline from a newer user revision or layout.
                    if s["base_revision"].as_u64() == Some(fence.revision) {
                        if let Some(revision) = s["last_revision"].as_u64() {
                            fence.revision = revision;
                            self.generated = GeneratedDocument::capture(session, fence);
                        }
                    }
                }
                self.finish("Built");
            }
            Some("conflicted") => {
                self.building = false;
                self.remaining =
                    Some(self.plan_steps[self.done.min(self.plan_steps.len())..].to_vec());
                self.error = Some(format!("The rack was edited during the build, so it stopped after {}/{} steps. Resume finishes the rest.", self.done, self.total));
            }
            Some("cancelled") | Some("failed") => {
                self.building = false;
                self.label = format!("Build {}", s["status"].as_str().unwrap_or("stopped"));
            }
            _ => {}
        }
    }

    fn finish(&mut self, label: &str) {
        self.building = false;
        self.elapsed = self.started.map(|t| t.elapsed());
        self.label = label.into();
        self.done = self.total;
        if let (Some(h), Some(e)) = (self.history.first_mut(), self.elapsed) {
            h.4 = e.as_secs_f32();
        }
    }

    fn sentence(&self) -> LayoutJob {
        let mut job = LayoutJob::default();
        let font = FontId::new(22.0, self.face_family.clone());
        let plain = TextFormat {
            font_id: font.clone(),
            color: Color32::from_rgb(214, 211, 202),
            ..Default::default()
        };
        let chosen = TextFormat {
            font_id: font.clone(),
            color: AMBER,
            ..Default::default()
        };
        let blank = TextFormat {
            font_id: font,
            color: Color32::from_rgb(92, 95, 91),
            ..Default::default()
        };
        job.append("A ", 0.0, plain.clone());
        match self.mood {
            Some(m) => job.append(&MOODS[m].to_lowercase(), 0.0, chosen.clone()),
            None => job.append("mood", 0.0, blank.clone()),
        }
        job.append(" ", 0.0, plain.clone());
        match self.genre {
            Some(g) => job.append(genre_noun(genre_name(g)), 0.0, chosen.clone()),
            None => job.append("genre", 0.0, blank.clone()),
        }
        job.append(" ", 0.0, plain.clone());
        match self.texture {
            Some(t) => job.append(texture_phrase(TEXTURES[t]), 0.0, chosen),
            None => job.append("texture", 0.0, blank),
        }
        job.append(".", 0.0, plain);
        job
    }

    fn section(ui: &mut egui::Ui, title: &str, focused: bool) {
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::hover());
        let x = if focused {
            ui.painter()
                .circle_filled(egui::pos2(rect.left() + 3.0, rect.center().y), 2.5, AMBER);
            10.0
        } else {
            0.0
        };
        ui.painter().text(
            rect.left_center() + egui::vec2(x, 0.0),
            egui::Align2::LEFT_CENTER,
            title,
            face(ui, 14.0),
            if focused { INK } else { DIM },
        );
    }

    fn keyboard(&mut self, ctx: &egui::Context, session: &SharedSession) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let pressed = |k: egui::Key| ctx.input(|i| i.key_pressed(k) && i.modifiers.is_none());
        for (i, k) in GENRE_KEYS.iter().enumerate() {
            if pressed(*k) {
                self.genre = Some(i);
            }
        }
        for (i, k) in MOOD_KEYS.iter().enumerate() {
            if pressed(*k) {
                self.mood = Some(i);
            }
        }
        for (i, k) in TEXTURE_KEYS.iter().enumerate() {
            if pressed(*k) {
                self.texture = Some(i);
            }
        }
        if pressed(egui::Key::ArrowUp) {
            self.cursor = (self.cursor + 2) % 3;
        }
        if pressed(egui::Key::ArrowDown) {
            self.cursor = (self.cursor + 1) % 3;
        }
        let step = if pressed(egui::Key::ArrowRight) {
            Some(1)
        } else if pressed(egui::Key::ArrowLeft) {
            Some(7)
        } else {
            None
        };
        if let Some(step) = step {
            let slot = match self.cursor {
                0 => &mut self.genre,
                1 => &mut self.mood,
                _ => &mut self.texture,
            };
            // genres include the C418 mode after the genre words; moods and textures have eight words
            let n = if self.cursor == 0 { MINECRAFT + 1 } else { 8 };
            let step = if step == 7 { n - 1 } else { step };
            *slot = Some(slot.map_or(0, |i| (i + step) % n));
        }
        if pressed(egui::Key::Enter) {
            self.start(session);
        }
        if pressed(egui::Key::Slash) || pressed(egui::Key::Space) {
            self.surprise(session);
        }
        if pressed(egui::Key::Escape) {
            self.open = false;
        }
    }

    fn surprise(&mut self, session: &SharedSession) {
        self.genre = Some((next_seed(&mut self.rng) % (MINECRAFT as u64 + 1)) as usize);
        self.mood = Some((next_seed(&mut self.rng) % 8) as usize);
        self.texture = Some((next_seed(&mut self.rng) % 8) as usize);
        self.keep_seed = false;
        self.start(session);
    }

    fn run_demo(&mut self, ctx: &egui::Context, session: &SharedSession) {
        let Some(mut demo) = self.demo.take() else {
            return;
        };
        if !demo.began {
            demo.began = true;
            if let Some(size) = demo.size {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            }
            if let Some((g, m, t, seed)) = demo.words {
                (
                    self.genre,
                    self.mood,
                    self.texture,
                    self.seed,
                    self.keep_seed,
                    self.open,
                ) = (Some(g), Some(m), Some(t), seed, true, true);
            }
        }
        if demo.words.is_some()
            && demo.start.elapsed() > Duration::from_millis(1500)
            && self.history.is_empty()
        {
            self.start(session);
            demo.start = Instant::now();
        }
        let t = demo.start.elapsed().as_secs_f32();
        if demo.waiting.is_none() && !self.history.is_empty() {
            if let Some(i) = demo.captures.iter().position(|c| t >= c.1) {
                let (path, _) = demo.captures.remove(i);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                demo.waiting = Some(path);
            }
        }
        if let Some(path) = demo.waiting.clone() {
            let shot = ctx.input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Screenshot { image, .. } = e {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
            });
            if let Some(image) = shot {
                match image::save_buffer(
                    &path,
                    image.as_raw(),
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                ) {
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
            self.open = true; // small screens (Steam Deck): start with the Patch Engine open
        }
        let was_idle = !self.building && self.pending.is_none();
        self.run_demo(ctx, session);
        self.poll(session);
        if !self.open {
            return vec![];
        }
        if self.building || self.pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        self.keyboard(ctx, session);
        self.face_family = crate::panel_skin::heading_font(ctx, 1.0).family;
        let narrow = ctx.content_rect().width() < 1400.0;
        let mut close = false;
        egui::SidePanel::right("patch-engine-sheet")
            .resizable(false)
            .exact_width(if narrow { 372.0 } else { 404.0 })
            .frame(egui::Frame::new().fill(SHEET).inner_margin(egui::Margin { left: 14, right: 14, top: 12, bottom: 12 })
                .stroke(egui::Stroke::new(1.0_f32, LINE)))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
                let width = ui.available_width();
                // header
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Patch Engine").font(face(ui, 17.0)).color(INK));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
                        let c = if resp.hovered() { INK } else { DIM };
                        let k = r.shrink(6.0);
                        ui.painter().line_segment([k.left_top(), k.right_bottom()], egui::Stroke::new(1.2_f32, c));
                        ui.painter().line_segment([k.right_top(), k.left_bottom()], egui::Stroke::new(1.2_f32, c));
                        if resp.on_hover_text("Close · F1 or B").clicked() { close = true; }
                    });
                });
                // display
                let mut job = self.sentence();
                job.wrap.max_width = width - 24.0;
                let galley = ui.painter().layout_job(job);
                let secs = self.elapsed.or_else(|| self.started.filter(|_| self.building || self.pending.is_some()).map(|t| t.elapsed()));
                let busy = self.building || self.pending.is_some() || self.armed.is_some();
                let caption = match (&self.info, busy) {
                    (_, true) => format!("Building · {} · {:.2} s", self.label, secs.map_or(0.0, |d| d.as_secs_f32())),
                    (Some(info), false) => format!("{} BPM · {} · {}{}", info["bpm"], info["key"].as_str().unwrap_or(""),
                        info["chords"].as_array().into_iter().flatten().filter_map(|c| c.as_str()).collect::<Vec<_>>().join(" "), ""),
                    (None, false) => "Choose a genre, a mood and a texture.".into(),
                };
                let details = self.info.as_ref().filter(|_| self.details && !busy).map(|info| {
                    let notes: Vec<&str> = info["notes"].as_array().into_iter().flatten().filter_map(|n| n.as_str()).collect();
                    let text = format!("{}\n\n{} · {} modules · {} cables · seed {}", info["description"].as_str().unwrap_or(""), notes.join(" · "),
                                       info["modules"], info["cables"], info["seed"]);
                    ui.painter().layout(text, face(ui, 13.0), Color32::from_rgb(160, 162, 156), width - 24.0)
                });
                let cap = ui.painter().layout(caption, face(ui, 13.0), DIM, width - 110.0);
                let height = 12.0 + galley.size().y + 6.0 + cap.size().y + 10.0 + details.as_ref().map_or(0.0, |d| d.size().y + 10.0);
                let (screen, screen_resp) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
                {
                    let p = ui.painter();
                    p.rect(screen, 4.0_f32, WELL, egui::Stroke::new(1.0_f32, Color32::from_rgb(42, 45, 44)), egui::StrokeKind::Inside);
                    if busy && self.total > 0 {
                        let frac = self.done as f32 / self.total as f32;
                        p.rect_filled(egui::Rect::from_min_size(screen.left_bottom() - egui::vec2(0.0, 2.0), egui::vec2(screen.width() * frac, 2.0)), 0.0_f32, AMBER);
                    }
                    let mut y = screen.top() + 12.0;
                    let gh = galley.size().y;
                    p.galley(egui::pos2(screen.left() + 12.0, y), galley, INK);
                    y += gh + 6.0;
                    let ch = cap.size().y;
                    p.galley(egui::pos2(screen.left() + 12.0, y), cap, DIM);
                    if self.info.is_some() && !busy {
                        let more = format!("{}{}", secs.map_or(String::new(), |d| format!("{:.2} s   ", d.as_secs_f32())), if self.details { "less" } else { "more" });
                        p.text(egui::pos2(screen.right() - 12.0, y), egui::Align2::RIGHT_TOP, more, face(ui, 13.0),
                               if screen_resp.hovered() { AMBER } else { Color32::from_rgb(120, 112, 92) });
                    }
                    if let Some(d) = details { p.galley(egui::pos2(screen.left() + 12.0, y + ch + 10.0), d, INK); }
                }
                if screen_resp.clicked() && self.info.is_some() { self.details = !self.details; }
                // actions
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 0.0);
                    let ready = self.ready();
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(width * 0.46, 34.0), egui::Sense::click());
                    let fill = if busy { Color32::from_rgb(70, 61, 42) } else if !ready { CELL } else if resp.hovered() { Color32::from_rgb(238, 198, 122) } else { AMBER };
                    ui.painter().rect(r, 3.0_f32, fill, egui::Stroke::new(1.0_f32, if ready || busy { fill } else { LINE }), egui::StrokeKind::Inside);
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, if busy { "Building…" } else if self.history.is_empty() { "Build" } else { "Build again" },
                                      face(ui, 17.0), if ready { DARK_INK } else if busy { AMBER } else { DIM });
                    if resp.on_hover_text("Compose and build · Enter / A").clicked() && ready { self.start(session); }
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(width * 0.27, 34.0), egui::Sense::click());
                    ui.painter().rect(r, 3.0_f32, if resp.hovered() && !busy { RAISED } else { CELL }, egui::Stroke::new(1.0_f32, LINE), egui::StrokeKind::Inside);
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "Surprise", face(ui, 15.0), if busy { DIM } else { INK });
                    if resp.on_hover_text("Random words and a new seed · Space / X").clicked() && !busy { self.surprise(session); }
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::click());
                    let kept = self.keep_seed;
                    ui.painter().rect(r, 3.0_f32, if resp.hovered() { RAISED } else { CELL }, egui::Stroke::new(1.0_f32, if kept { Color32::from_rgb(92, 78, 52) } else { LINE }), egui::StrokeKind::Inside);
                    let label = if self.seed == 0 { "Seed —".to_string() } else { format!("Seed {}", self.seed) };
                    ui.painter().text(r.center() - egui::vec2(0.0, 5.0), egui::Align2::CENTER_CENTER, label, face(ui, 13.5), if kept { AMBER } else { INK });
                    ui.painter().text(r.center() + egui::vec2(0.0, 8.0), egui::Align2::CENTER_CENTER, if kept { "kept" } else { "new each build" }, face(ui, 11.0), DIM);
                    if resp.on_hover_text("Every build composes a new song. Click to keep this seed and rebuild the same one.").clicked() { self.keep_seed = !self.keep_seed; }
                });
                if let Some(e) = &self.error { ui.label(RichText::new(e).font(face(ui, 13.0)).color(Color32::from_rgb(222, 140, 100))); }
                if self.remaining.is_some() && ui.button("Resume build").clicked() { self.resume(session); }
                // word matrices
                egui::ScrollArea::vertical().id_salt("patch-engine-words").auto_shrink([false, true]).max_height(ui.available_height() - 96.0).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    let mut genres: Vec<&str> = GENRES.to_vec();
                    genres.push("C418");
                    Self::section(ui, "Genre", self.cursor == 0);
                    if let Some(i) = matrix(ui, &genres, self.genre, 3, Some(|w: &str| if w == "C418" { "Minecraft-style: soft piano, slow drifting harmony, lots of space (original music in the spirit of C418)" } else { "" })) {
                        self.genre = Some(i);
                    }
                    ui.add_space(4.0);
                    Self::section(ui, "Mood", self.cursor == 1);
                    if let Some(i) = matrix(ui, &MOODS, self.mood, 3, None) { self.mood = Some(i); }
                    ui.add_space(4.0);
                    Self::section(ui, "Texture", self.cursor == 2);
                    if let Some(i) = matrix(ui, &TEXTURES, self.texture, 3, Some(texture_text)) { self.texture = Some(i); }
                    ui.add_space(6.0);
                    Self::section(ui, "Controls", false);
                    let w = ui.available_width();
                    for (label, v, tip) in [
                        ("Energy", &mut self.ctl.energy, "Tempo and how many layers play"),
                        ("Complexity", &mut self.ctl.complexity, "Ghost notes, counter-lines and variation from pass to pass"),
                        ("Weirdness", &mut self.ctl.weirdness, "Collage forms, generative arps and rarer sounds"),
                        ("Space", &mut self.ctl.space, "Reverb and echo depth"),
                    ] {
                        ui.horizontal(|ui| {
                            ui.add_sized([86.0, 20.0], egui::Label::new(RichText::new(label).font(face(ui, 13.0)).color(if (*v - 0.5).abs() > 0.01 { INK } else { DIM })));
                            ui.spacing_mut().slider_width = (w - 100.0).max(60.0);
                            ui.add(egui::Slider::new(v, 0.0..=1.0).show_value(false)).on_hover_text(tip);
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.add_sized([86.0, 20.0], egui::Label::new(RichText::new("Length").font(face(ui, 13.0)).color(DIM)));
                        let lengths = [32u32, 64, 128];
                        let cur = lengths.iter().position(|&l| l == self.ctl.length).unwrap_or(1);
                        if let Some(i) = segmented(ui, &["32 bars", "64", "128"], cur, ui.available_width()) { self.ctl.length = lengths[i]; }
                    });
                    if self.ctl != Controls::default() && ui.add(egui::Label::new(RichText::new("Reset controls").font(face(ui, 12.5)).color(AMBER)).sense(egui::Sense::click())).clicked() {
                        self.ctl = Controls::default();
                    }
                    if self.history.len() > 1 {
                        ui.add_space(6.0);
                        Self::section(ui, "Recent", false);
                        let mut replay = None;
                        for (i, &(g, m, t, seed, secs)) in self.history.iter().enumerate().skip(1).take(4) {
                            let text = format!("{} {} · {} · seed {}{}", MOODS[m], genre_name(g), TEXTURES[t], seed, if secs > 0.0 { format!(" · {secs:.1} s") } else { String::new() });
                            let resp = ui.add(egui::Label::new(RichText::new(text).font(face(ui, 13.0)).color(DIM)).sense(egui::Sense::click()));
                            if resp.on_hover_text("Choose these words and seed").clicked() { replay = Some(i); }
                        }
                        if let Some(i) = replay {
                            let (g, m, t, seed, _) = self.history[i];
                            (self.genre, self.mood, self.texture, self.seed, self.keep_seed) = (Some(g), Some(m), Some(t), seed, true);
                        }
                    }
                });
                // build options, pinned to the bottom
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                    ui.horizontal(|ui| {
                        check(ui, "Autoplay", &mut self.autoplay, "Start playback when a build begins");
                        check(ui, "Light", &mut self.light, "Keep each song within one Steam Deck core: optional layers are left out when a song would be too heavy");
                    });
                    ui.horizontal(|ui| {
                        let modes = [Mode::Reveal, Mode::Showtime, Mode::Instant];
                        let cur = modes.iter().position(|&m| m == self.mode).unwrap_or(0);
                        let w = if self.mode == Mode::Instant { width } else { width * 0.6 };
                        if let Some(i) = segmented(ui, &["Reveal", "Showtime", "Instant"], cur, w) { self.mode = modes[i]; }
                        if self.mode != Mode::Instant {
                            let speeds = [100u32, 150, 250];
                            let cur = speeds.iter().position(|&v| v == self.interval_ms).unwrap_or(1);
                            if let Some(i) = segmented(ui, &["Fast", "Med", "Slow"], cur, ui.available_width()) { self.interval_ms = speeds[i]; }
                        }
                    });
                });
            });
        if close {
            self.open = false;
        }
        let started = std::mem::take(&mut self.just_launched)
            || (was_idle && (self.building || self.pending.is_some()));
        let mut requests = vec![];
        if self.armed.is_some() && !self.prepare_sent {
            requests.push(Request::Prepare);
            self.prepare_sent = true;
        }
        if started && self.autoplay {
            requests.push(Request::StartPlayback);
        }
        if started {
            requests.push(Request::ShowPatch);
        }
        requests
    }
}

#[cfg(test)]
mod patch_engine_build_lifecycle_tests {
    include!("patch_engine_build_lifecycle_tests.rs");
}
