//! Original bounded score playback. Immutable typed data is compiled on the
//! control thread; the audio thread advances fixed-capacity state only.
use crate::{parameter, Node, PortFrame};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const TRACKS: usize = 4;
const STEPS: usize = 64;
const VOICES: usize = 4;
const LANES: usize = 16;
const POINTS: usize = 32;
pub(crate) const INPUTS: [&str; 2] = ["clock", "reset"];
pub(crate) const OUTPUTS: [&str; 15] = [
    "pitch1",
    "pitch2",
    "pitch3",
    "pitch4",
    "gate1",
    "gate2",
    "gate3",
    "gate4",
    "velocity1",
    "velocity2",
    "velocity3",
    "velocity4",
    "automation",
    "clock",
    "reset",
];

fn one_u32() -> u32 {
    1
}
fn one_u8() -> u8 {
    1
}
fn four() -> u8 {
    4
}
fn sixty_four() -> u8 {
    64
}
fn one() -> f32 {
    1.0
}
fn half() -> f32 {
    0.5
}
fn bars() -> f32 {
    64.0
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScoreData {
    #[serde(default = "one_u32")]
    pub version: u32,
    #[serde(default = "one_u32")]
    pub seed: u32,
    #[serde(default = "sixty_four")]
    pub length: u8,
    #[serde(default = "four")]
    pub steps_per_beat: u8,
    #[serde(default)]
    pub tracks: Vec<ScoreTrack>,
    #[serde(default)]
    pub lanes: Vec<ScoreLane>,
}
impl Default for ScoreData {
    fn default() -> Self {
        Self {
            version: 1,
            seed: 1,
            length: 64,
            steps_per_beat: 4,
            tracks: vec![],
            lanes: vec![],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScoreTrack {
    #[serde(default = "one_u8")]
    pub voices: u8,
    #[serde(default)]
    pub glide_ms: f32,
    #[serde(default)]
    pub steps: Vec<ScoreStep>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScoreStep {
    pub step: u8,
    #[serde(default)]
    pub notes: Vec<f32>,
    #[serde(default = "one")]
    pub velocity: f32,
    #[serde(default = "half")]
    pub gate: f32,
    #[serde(default = "one")]
    pub probability: f32,
    #[serde(default = "one_u8")]
    pub ratchet: u8,
    #[serde(default)]
    pub slide: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScoreLane {
    #[serde(default = "bars")]
    pub length_bars: f32,
    #[serde(default = "yes")]
    pub looped: bool,
    #[serde(default)]
    pub points: Vec<ScorePoint>,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ScoreCurve {
    #[default]
    Linear,
    Hold,
    EaseIn,
    EaseOut,
    EaseInOut,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScorePoint {
    pub bar: f32,
    pub value: f32,
    #[serde(default)]
    pub curve: ScoreCurve,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Cell {
    on: bool,
    notes: [f32; VOICES],
    note_count: u8,
    velocity: f32,
    gate: f32,
    probability: f32,
    ratchet: u8,
    slide: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Track {
    voices: u8,
    glide_ms: f32,
    cells: [Cell; STEPS],
}
impl Default for Track {
    fn default() -> Self {
        Self {
            voices: 1,
            glide_ms: 0.0,
            cells: [Cell::default(); STEPS],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Lane {
    length: f32,
    looped: bool,
    count: u8,
    points: [ScorePoint; POINTS],
}
impl Default for Lane {
    fn default() -> Self {
        Self {
            length: 64.0,
            looped: true,
            count: 0,
            points: [ScorePoint::default(); POINTS],
        }
    }
}
#[derive(Debug, PartialEq)]
struct Compiled {
    seed: u32,
    length: u8,
    steps_per_beat: u8,
    tracks: [Track; TRACKS],
    lane_count: u8,
    lanes: [Lane; LANES],
}

fn finite(value: f32, min: f32, max: f32, field: &str) -> Result<(), String> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(format!("score {field} must be finite and in {min}..={max}"))
    }
}
impl Compiled {
    fn build(data: &ScoreData) -> Result<Self, String> {
        if data.version != 1 {
            return Err("score version must be 1".into());
        }
        if !(1..=64).contains(&data.length) {
            return Err("score length must be in 1..=64".into());
        }
        if !(1..=8).contains(&data.steps_per_beat) {
            return Err("score steps_per_beat must be in 1..=8".into());
        }
        if data.tracks.len() > TRACKS {
            return Err("score supports at most 4 tracks".into());
        }
        if data.lanes.len() > LANES {
            return Err("score supports at most 16 automation lanes".into());
        }
        let mut result = Self {
            seed: data.seed,
            length: data.length,
            steps_per_beat: data.steps_per_beat,
            tracks: [Track::default(); TRACKS],
            lane_count: data.lanes.len() as u8,
            lanes: [Lane::default(); LANES],
        };
        for (index, source) in data.tracks.iter().enumerate() {
            if !(1..=4).contains(&source.voices) {
                return Err(format!("score track {index} voices must be in 1..=4"));
            }
            finite(source.glide_ms, 0.0, 5000.0, "glide_ms")?;
            if source.steps.len() > STEPS {
                return Err(format!("score track {index} has more than 64 steps"));
            }
            let track = &mut result.tracks[index];
            track.voices = source.voices;
            track.glide_ms = source.glide_ms;
            for step in &source.steps {
                if step.step >= data.length {
                    return Err(format!(
                        "score track {index} step {} is outside length",
                        step.step
                    ));
                }
                let cell = &mut track.cells[usize::from(step.step)];
                if cell.on {
                    return Err(format!(
                        "score track {index} has duplicate step {}",
                        step.step
                    ));
                }
                if step.notes.len() > VOICES {
                    return Err("score step supports at most 4 notes".into());
                }
                for note in &step.notes {
                    finite(*note, -60.0, 60.0, "note")?;
                }
                finite(step.velocity, 0.0, 1.0, "velocity")?;
                finite(step.gate, 0.0, 1.0, "gate")?;
                finite(step.probability, 0.0, 1.0, "probability")?;
                if !(1..=8).contains(&step.ratchet) {
                    return Err("score ratchet must be in 1..=8".into());
                }
                *cell = Cell {
                    on: true,
                    notes: [0.0; VOICES],
                    note_count: step.notes.len() as u8,
                    velocity: step.velocity,
                    gate: step.gate,
                    probability: step.probability,
                    ratchet: step.ratchet,
                    slide: step.slide,
                };
                cell.notes[..step.notes.len()].copy_from_slice(&step.notes);
            }
        }
        for (index, source) in data.lanes.iter().enumerate() {
            finite(source.length_bars, 0.001, 4096.0, "lane length_bars")?;
            if source.points.len() > POINTS {
                return Err(format!("score lane {index} has more than 32 points"));
            }
            let lane = &mut result.lanes[index];
            lane.length = source.length_bars;
            lane.looped = source.looped;
            lane.count = source.points.len() as u8;
            let mut previous = 0.0;
            for (point_index, point) in source.points.iter().enumerate() {
                finite(point.bar, 0.0, source.length_bars, "point bar")?;
                finite(point.value, -16.0, 16.0, "point value")?;
                if point_index > 0 && point.bar < previous {
                    return Err(format!(
                        "score lane {index} points must be sorted; equal bars are permitted"
                    ));
                }
                previous = point.bar;
                lane.points[point_index] = *point;
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Trigger {
    high: bool,
}
impl Trigger {
    fn rising(&mut self, value: f32) -> bool {
        let value = if value.is_finite() { value } else { 0.0 };
        if self.high {
            if value <= 0.1 {
                self.high = false;
            }
            false
        } else if value >= 1.0 {
            self.high = true;
            true
        } else {
            false
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct VoiceState {
    active: bool,
    tied_out: bool,
    tied_in: bool,
    ratchets: u8,
    sub_length: f64,
    gate_length: f64,
    pitch: [f32; VOICES],
    target: [f32; VOICES],
    increment: [f32; VOICES],
    velocity: f32,
}
#[derive(Clone, Copy, Debug)]
struct Runtime {
    running: bool,
    started: bool,
    step: u8,
    loops: u64,
    elapsed: f64,
    step_length: f64,
    beats: f64,
    beats_per_step: f64,
    clock_age: f64,
    external_period: f64,
    last_gap: f64,
    external_edges: u8,
    clock_trigger: Trigger,
    reset_trigger: Trigger,
    clock_pulse: f64,
    reset_pulse: f64,
    pending_reset: bool,
    divider: u8,
    tracks: [VoiceState; TRACKS],
    automation: [f32; LANES],
}
impl Runtime {
    fn new(running: bool) -> Self {
        Self {
            running,
            started: false,
            step: 0,
            loops: 0,
            elapsed: 0.0,
            step_length: 1.0,
            beats: 0.0,
            beats_per_step: 0.25,
            clock_age: 0.0,
            external_period: 0.0,
            last_gap: 0.0,
            external_edges: 0,
            clock_trigger: Trigger::default(),
            reset_trigger: Trigger::default(),
            clock_pulse: 0.0,
            reset_pulse: 0.0,
            pending_reset: false,
            divider: 0,
            tracks: [VoiceState::default(); TRACKS],
            automation: [0.0; LANES],
        }
    }
}

#[derive(Debug)]
pub(crate) struct ScoreSequencer {
    data: Box<Compiled>,
    rate: f64,
    tempo: f32,
    swing: f32,
    initial_running: Option<f32>,
    state: Runtime,
}
impl ScoreSequencer {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for key in node.params.keys() {
            if !matches!(key.as_str(), "tempo" | "swing" | "running") {
                return Err(format!(
                    "node '{}': unknown score parameter '{key}'",
                    node.id
                ));
            }
        }
        let tempo = parameter(node, "tempo", 120.0, 20.0, 300.0)?;
        let swing = parameter(node, "swing", 50.0, 50.0, 75.0)?;
        let running = parameter(node, "running", 1.0, 0.0, 1.0)?;
        if running.fract() != 0.0 {
            return Err("score running must be 0 or 1".into());
        }
        let empty = ScoreData::default();
        let data = Box::new(Compiled::build(node.score.as_ref().unwrap_or(&empty))?);
        let mut state = Runtime::new(running == 1.0);
        for i in 0..usize::from(data.lane_count) {
            state.automation[i] = lane_value(&data.lanes[i], 0.0);
        }
        Ok(Self {
            data,
            rate: rate as f64,
            tempo,
            swing,
            initial_running: node.params.get("running").copied(),
            state,
        })
    }
    /// Configuration edits start fresh; equal configuration/rate adopts only
    /// Copy state, without allocation. Ordinary tempo/swing edits retain phase.
    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.rate != old.rate || self.data != old.data {
            return;
        }
        let configured_run = self.state.running;
        self.state = old.state;
        if self.initial_running != old.initial_running {
            self.state.running = configured_run;
        }
    }
    pub(crate) fn reset_state(&mut self) {
        self.state.pending_reset = true;
    }
    /// Rearm only detectors whose incoming route changed after adoption.
    /// A new clock's first edge estimates duration from tempo, not a period
    /// measured on its previous source. Musical position and held outputs stay
    /// intact until the new edge/reset is actually received. No allocation.
    pub(crate) fn reset_input_detectors(&mut self, clock_changed: bool, reset_changed: bool) {
        if clock_changed {
            self.state.clock_trigger = Trigger::default();
            self.state.clock_age = 0.0;
            self.state.external_period = 0.0;
            self.state.last_gap = 0.0;
            self.state.external_edges = 0;
        }
        if reset_changed {
            self.state.reset_trigger = Trigger::default();
        }
    }
    fn begin_step(&mut self, external: bool) {
        let base = self.rate * 60.0 / (f64::from(self.tempo) * f64::from(self.data.steps_per_beat));
        let state = &mut self.state;
        state.beats_per_step = 1.0 / f64::from(self.data.steps_per_beat);
        state.step_length = if external {
            if state.external_period > 0.0 {
                state.external_period
            } else {
                base
            }
        } else {
            base * 2.0
                * if state.step.is_multiple_of(2) {
                    f64::from(self.swing) / 100.0
                } else {
                    1.0 - f64::from(self.swing) / 100.0
                }
        };
        state.elapsed = 0.0;
        state.clock_pulse = self.rate * 0.001;
        for index in 0..TRACKS {
            let track = &self.data.tracks[index];
            let cell = &track.cells[usize::from(state.step)];
            let voice = &mut state.tracks[index];
            let tied_in = voice.active && voice.tied_out;
            let fire = cell.on
                && (cell.probability >= 1.0
                    || chance(self.data.seed, state.loops, state.step, index as u8)
                        < cell.probability);
            if !fire {
                voice.active = false;
                voice.tied_out = false;
                continue;
            }
            voice.active = true;
            voice.tied_in = tied_in;
            voice.tied_out = cell.slide;
            voice.ratchets = cell.ratchet;
            voice.sub_length = state.step_length / f64::from(cell.ratchet);
            voice.gate_length = f64::from(cell.gate).max(0.01) * voice.sub_length;
            voice.velocity = cell.velocity;
            for v in 0..usize::from(track.voices) {
                if cell.note_count == 0 {
                    continue;
                }
                let target = cell.notes[v % usize::from(cell.note_count)] / 12.0;
                voice.target[v] = target;
                let duration = f64::from(track.glide_ms) * self.rate / 1000.0;
                if tied_in && duration >= 1.0 {
                    voice.increment[v] =
                        ((f64::from(target) - f64::from(voice.pitch[v])) / duration) as f32;
                } else {
                    voice.pitch[v] = target;
                    voice.increment[v] = 0.0;
                }
            }
        }
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame], outputs: &mut [PortFrame]) {
        let clock = inputs.first().copied().unwrap_or_default();
        let reset = inputs.get(1).copied().unwrap_or_default();
        let external = clock.channels > 0;
        let edge = external && self.state.clock_trigger.rising(clock.channel(0));
        if !external {
            self.state.clock_trigger.rising(0.0);
        }
        if self.state.reset_trigger.rising(reset.channel(0)) {
            self.state.pending_reset = true;
        }
        self.state.clock_age += 1.0;
        if !external {
            self.state.external_edges = 0;
        }
        if edge {
            let gap = self.state.clock_age;
            self.state.clock_age = 0.0;
            self.state.external_edges = self.state.external_edges.saturating_add(1);
            if self.state.external_edges == 1 {
                self.state.external_period = 0.0;
            } else if self.state.external_period <= 0.0
                || gap <= 2.5 * self.state.external_period
                || (gap - self.state.last_gap).abs() <= 0.05 * gap
            {
                self.state.external_period = gap;
            }
            self.state.last_gap = gap;
        }
        if self.state.pending_reset {
            self.state.pending_reset = false;
            self.state.step = 0;
            self.state.loops = 0;
            self.state.beats = 0.0;
            self.state.elapsed = 0.0;
            self.state.started = false;
            self.state.reset_pulse = self.rate * 0.001;
            for track in &mut self.state.tracks {
                track.active = false;
            }
        }
        if !self.state.running {
            self.state.started = false;
            for track in &mut self.state.tracks {
                track.active = false;
            }
        } else {
            let mut begin = false;
            if !self.state.started {
                begin = !external || edge;
            } else {
                self.state.elapsed += 1.0;
                let advance = if external {
                    edge
                } else {
                    self.state.elapsed >= self.state.step_length
                };
                if advance {
                    self.state.beats += self.state.beats_per_step;
                    self.state.step += 1;
                    if self.state.step >= self.data.length {
                        self.state.step = 0;
                        self.state.loops = self.state.loops.wrapping_add(1);
                    }
                    begin = true;
                }
            }
            if begin {
                self.state.started = true;
                self.begin_step(external);
            }
        }
        for index in 0..TRACKS {
            let voice = &mut self.state.tracks[index];
            let count = self.data.tracks[index].voices;
            let mut pitch = PortFrame {
                channels: count,
                ..PortFrame::default()
            };
            let mut gates = PortFrame {
                channels: count,
                ..PortFrame::default()
            };
            let mut velocity = PortFrame {
                channels: count,
                ..PortFrame::default()
            };
            let gate = if voice.active {
                let sub = (self.state.elapsed / voice.sub_length) as u32;
                let sub = sub.min(u32::from(voice.ratchets) - 1);
                let within = self.state.elapsed - f64::from(sub) * voice.sub_length;
                let gap = if sub == 0 && voice.tied_in {
                    0.0
                } else {
                    self.rate * 0.001
                };
                (within >= gap && within < voice.gate_length + gap)
                    || (voice.tied_out && sub == u32::from(voice.ratchets) - 1 && within >= gap)
            } else {
                false
            };
            for v in 0..usize::from(count) {
                let increment = voice.increment[v];
                if increment != 0.0 {
                    voice.pitch[v] += increment;
                    if (increment > 0.0 && voice.pitch[v] >= voice.target[v])
                        || (increment < 0.0 && voice.pitch[v] <= voice.target[v])
                    {
                        voice.pitch[v] = voice.target[v];
                        voice.increment[v] = 0.0;
                    }
                }
                pitch.values[v] = voice.pitch[v];
                gates.values[v] = if gate { 10.0 } else { 0.0 };
                velocity.values[v] = voice.velocity * 10.0;
            }
            outputs[index] = pitch;
            outputs[index + 4] = gates;
            outputs[index + 8] = velocity;
        }
        self.state.divider += 1;
        if self.state.divider == 32 {
            self.state.divider = 0;
            if self.state.running {
                let fraction = if self.state.started {
                    (self.state.elapsed / self.state.step_length).min(1.0)
                } else {
                    0.0
                };
                let bars = (self.state.beats + fraction * self.state.beats_per_step) / 4.0;
                for i in 0..usize::from(self.data.lane_count) {
                    self.state.automation[i] = lane_value(&self.data.lanes[i], bars);
                }
            }
        }
        outputs[12] = PortFrame {
            channels: self.data.lane_count,
            values: self.state.automation,
        };
        outputs[13] = PortFrame::mono(if self.state.clock_pulse > 0.0 {
            10.0
        } else {
            0.0
        });
        outputs[14] = PortFrame::mono(if self.state.reset_pulse > 0.0 {
            10.0
        } else {
            0.0
        });
        self.state.clock_pulse = (self.state.clock_pulse - 1.0).max(0.0);
        self.state.reset_pulse = (self.state.reset_pulse - 1.0).max(0.0);
    }
}

fn lane_value(lane: &Lane, bars: f64) -> f32 {
    if lane.count == 0 {
        return 0.0;
    }
    let mut position = bars.max(0.0);
    if lane.looped {
        position %= f64::from(lane.length);
    }
    let points = &lane.points[..usize::from(lane.count)];
    if position <= f64::from(points[0].bar) {
        return points[0].value;
    }
    for pair in points.windows(2) {
        let left = pair[0];
        let right = pair[1];
        if position < f64::from(right.bar) {
            let span = f64::from(right.bar) - f64::from(left.bar);
            let t = if span > 0.0 {
                ((position - f64::from(left.bar)) / span) as f32
            } else {
                1.0
            };
            let t = t.clamp(0.0, 1.0);
            let shape = match right.curve {
                ScoreCurve::Linear => t,
                ScoreCurve::Hold => {
                    if t < 1.0 {
                        0.0
                    } else {
                        1.0
                    }
                }
                ScoreCurve::EaseIn => t * t * t,
                ScoreCurve::EaseOut => 1.0 - (1.0 - t).powi(3),
                ScoreCurve::EaseInOut => {
                    if t < 0.5 {
                        4.0 * t * t * t
                    } else {
                        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                    }
                }
            };
            return left.value + shape * (right.value - left.value);
        }
    }
    points.last().unwrap().value
}

// SplitMix64 is a standard integer mixer. Keyed draws, rather than mutable RNG
// advancement, make probabilities reproducible independently of audio blocks.
fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}
fn chance(seed: u32, loop_index: u64, step: u8, track: u8) -> f32 {
    let seed = mix(u64::from(seed));
    let pattern = mix(seed); // One pattern per original native score instance.
    let iteration = mix(pattern ^ loop_index);
    ((mix(iteration ^ (u64::from(step) << 8) ^ u64::from(track)) >> 40) & 0xFF_FFFF) as f32
        / 16_777_216.0
}

pub(crate) fn catalog() -> Value {
    json!({"kind":"score_sequencer","title":"SCORE","model":"functional","description":"Original bounded four-track 1–64-step score player with fixed polyphonic voices, seeded per-loop probability, ties, ratchets and 16-channel automation. Pitch semitones from C4 /12; gates 10 V; velocity ×10 V. External clock advances one step and estimates duration from the previous edge; internal clock has sample-rounded swing. Independent typed score data; no third-party plugin code.","inputs":INPUTS,"params":{
        "tempo":{"default":120,"min":20,"max":300,"unit":"BPM"},"swing":{"default":50,"min":50,"max":75,"unit":"%"},"running":{"default":1,"min":0,"max":1,"step":1}},
        "score_schema":{"version":1,"seed":"u32","length":"1..64","steps_per_beat":"1..8","tracks":{"max":4,"voices":"1..4","glide_ms":"0..5000","steps":{"max":64,"fields":{"step":"0..length-1; unique","notes":"0..4 semitone offsets -60..60","velocity":"0..1 default1","gate":"0..1 default0.5; minimum emitted fraction0.01","probability":"0..1 default1","ratchet":"1..8 default1","slide":"boolean defaultfalse"}}},"lanes":{"max":16,"length_bars":"0.001..4096 default64","looped":"boolean defaulttrue","points":{"max":32,"fields":{"bar":"sorted 0..length_bars; equal bars retain order","value":"finite -16..16 V","curve":["linear","hold","ease_in","ease_out","ease_in_out"]}}}},
        "automation_policy":"One polyphonic output with one channel per lane, sampled every32 engine samples; evaluate destination point's curve in saved value domain. Values hold while stopped. Split channels with poly_split. Initial lane values at bar0 are emitted immediately.",
        "state_adoption":"Equal compiled score/rate preserves transport, probabilities, held values and input detectors allocation-free. Changed score resets. Tempo/swing edits preserve transport; changed initial running value/presence overrides adopted Run.",
        "compatibility":"Independent native score playback, not .vcv plugin import or an assertion of sample-identical Rack DSP. Rebuild score data explicitly through MCP."})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(score: Value, params: Value) -> ScoreSequencer {
        let node: Node = serde_json::from_value(
            json!({"id":"score","kind":"score_sequencer","score":score,"params":params}),
        )
        .unwrap();
        ScoreSequencer::build(&node, 48000).unwrap()
    }
    fn sample(player: &mut ScoreSequencer, inputs: &[PortFrame]) -> [PortFrame; 15] {
        let mut output = [PortFrame::default(); 15];
        player.process(inputs, &mut output);
        output
    }
    fn silent() -> Value {
        json!({"length":64})
    }

    #[test]
    fn internal_swung_clock_keeps_saved_64_steps_and_sample_rounding() {
        let mut score = player(silent(), json!({"tempo":128,"swing":54.525020599365234}));
        let mut rises = vec![];
        let mut high = false;
        for frame in 0..23000 {
            let pulse = sample(&mut score, &[])[13].values[0] > 0.0;
            if pulse && !high {
                rises.push(frame);
            }
            high = pulse;
        }
        assert_eq!(rises, [0, 6135, 11251, 17386, 22502]);
        assert_eq!(score.state.step, 4);
        assert_eq!(score.data.length, 64);
    }

    #[test]
    fn probability_matches_reference_seed_and_varies_with_actual_loop_count() {
        let expected = [true, false, true, true, false, true];
        for (i, want) in expected.iter().enumerate() {
            let threshold = if i % 2 == 0 { 0.6 } else { 0.5 };
            assert_eq!(chance(128, 0, 54 + i as u8, 0) < threshold, *want);
        }
        let first: Vec<_> = (0..64).map(|step| chance(128, 0, step, 0)).collect();
        let second: Vec<_> = (0..64).map(|step| chance(128, 1, step, 0)).collect();
        assert_ne!(first, second);
        let mut score = player(
            json!({"length":1,"seed":128,"tracks":[{"steps":[{"step":0,"probability":0.5}]}]}),
            json!({"tempo":300}),
        );
        let mut previous = score.state.loops;
        let mut choices = vec![];
        for _ in 0..24000 {
            sample(&mut score, &[]);
            if choices.is_empty() || score.state.loops != previous {
                choices.push(score.state.tracks[0].active);
                previous = score.state.loops;
            }
        }
        assert!(choices.len() > 4);
        for (loop_index, choice) in choices.into_iter().enumerate() {
            assert_eq!(choice, chance(128, loop_index as u64, 0, 0) < 0.5);
        }
    }

    #[test]
    fn tied_notes_have_no_boundary_gap_hold_voices_velocity_and_pitch_at_rests() {
        let mut score = player(
            json!({"length":4,"tracks":[{"voices":2,"glide_ms":0,"steps":[
            {"step":0,"notes":[0,7],"slide":true,"velocity":0.8,"gate":1},
            {"step":1,"notes":[12,19],"velocity":0.6,"gate":0.5}]}]}),
            json!({"tempo":120}),
        );
        let mut previous = false;
        let mut changes = vec![];
        for frame in 0..13000 {
            let output = sample(&mut score, &[]);
            let high = output[4].values[0] > 0.0;
            if high != previous {
                changes.push((frame, high));
            }
            previous = high;
            assert_eq!(output[0].channels, 2);
            assert_eq!(output[4].channels, 2);
            if frame == 6000 {
                assert_eq!(output[0].values[0], 1.0);
                assert_eq!(output[0].values[1], 19.0 / 12.0);
                assert!(high);
            }
            if frame == 12000 {
                assert_eq!(output[0].values[0], 1.0);
                assert_eq!(output[8].values[0], 6.0);
                assert!(!high);
            }
            assert!(output[0].values[2..].iter().all(|&v| v == 0.0));
        }
        assert_eq!(changes, [(48, true), (9000, false)]);
    }

    #[test]
    fn ratchets_apply_even_subhits_with_one_millisecond_retrigger_gaps() {
        let mut score = player(
            json!({"length":2,"tracks":[{"steps":[{"step":0,"ratchet":4,"gate":0.5}]}]}),
            json!({"tempo":120}),
        );
        let mut high = false;
        let mut rises = vec![];
        for frame in 0..6000 {
            let next = sample(&mut score, &[])[4].values[0] > 0.0;
            if next && !high {
                rises.push(frame);
            }
            high = next;
        }
        assert_eq!(rises, [48, 1548, 3048, 4548]);
    }

    #[test]
    fn externally_clocked_slave_waits_and_uses_previous_swung_edge_interval() {
        let mut master = player(silent(), json!({"tempo":128,"swing":54.525020599365234}));
        let mut slave = player(
            json!({"length":64,"tracks":[{"steps":[{"step":0},{"step":1},{"step":2}]}]}),
            json!({"tempo":128,"swing":50}),
        );
        let mut wire = PortFrame::mono(0.0);
        let mut periods = vec![];
        let mut rises = vec![];
        let mut high = false;
        for frame in 0..12000 {
            let output = sample(&mut master, &[]);
            let secondary = sample(&mut slave, &[wire, PortFrame::default()]);
            if frame == 0 {
                assert!(!slave.state.started);
                assert_eq!(secondary[4].values[0], 0.0);
            }
            let pulse = secondary[13].values[0] > 0.0;
            if pulse && !high {
                rises.push(frame);
                periods.push(slave.state.step_length);
            }
            high = pulse;
            wire = output[13];
        }
        assert_eq!(rises, [1, 6136, 11252]);
        assert_eq!(periods, [5625.0, 6135.0, 5116.0]);
    }

    #[test]
    fn actual_engine_connected_zero_clock_waits_for_first_delayed_source_edge() {
        let patch: crate::Patch = serde_json::from_value(json!({
            "version":2,"sample_rate":48000,"nodes":[
                {"id":"a","kind":"score_sequencer","params":{"tempo":128}},
                {"id":"b","kind":"score_sequencer","params":{"tempo":128},"score":{
                    "tracks":[{"steps":[{"step":0,"notes":[-30],"gate":0.5}]}]}}
            ],"cables":[{"id":"clock","from":"a","output":"clock","to":"b","input":"clock"}],
            "output":"b","output_port":"clock"
        }))
        .unwrap();
        let mut engine = patch.compile().unwrap();
        let taps = [
            Some(engine.resolve_tap("b", "pitch1", 0).unwrap()),
            Some(engine.resolve_tap("b", "gate1", 0).unwrap()),
        ];
        let mut audio = [[0.0; 2]; 60];
        let mut observed = [[crate::TapSample::default(); 2]; 60];
        engine.process_audio_block_with_taps(&mut audio, &taps, &mut observed);
        assert_eq!(
            observed[0][0].value, 0.0,
            "no internal step before the connected clock publishes its width"
        );
        assert_eq!(
            observed[1][0].value, -2.5,
            "first clock edge plays step zero, not step one"
        );
        assert!(observed[..49].iter().all(|sample| sample[1].value == 0.0));
        assert_eq!(observed[49][1].value, 10.0);
        assert_eq!(audio[0][0], 0.0);
        assert_eq!(audio[1][0], 10.0);
    }

    #[test]
    fn changed_clock_detector_forgets_old_period_without_resetting_transport() {
        let mut score = player(silent(), json!({"tempo":128}));
        sample(&mut score, &[PortFrame::mono(10.0)]);
        for _ in 0..9 {
            sample(&mut score, &[PortFrame::mono(0.0)]);
        }
        sample(&mut score, &[PortFrame::mono(10.0)]);
        assert_eq!(score.state.step, 1);
        assert_eq!(score.state.external_period, 10.0);
        assert!(score.state.clock_trigger.high);
        score.reset_input_detectors(false, false);
        assert_eq!(score.state.external_period, 10.0);
        assert!(score.state.clock_trigger.high);
        score.reset_input_detectors(true, false);
        assert_eq!(score.state.step, 1);
        assert!(score.state.started);
        assert_eq!(score.state.external_period, 0.0);
        assert!(!score.state.clock_trigger.high);
        // The new source already high must be recognized as a fresh edge;
        // its duration uses 128 BPM, not the previous source's ten samples.
        sample(&mut score, &[PortFrame::mono(10.0)]);
        assert_eq!(score.state.step, 2);
        assert_eq!(score.state.step_length, 5625.0);
        assert_eq!(score.state.external_edges, 1);
    }

    #[test]
    fn changed_reset_detector_rearms_high_source_without_resetting_clock_estimate() {
        let mut score = player(silent(), json!({"tempo":128}));
        sample(&mut score, &[PortFrame::default(), PortFrame::mono(10.0)]);
        for _ in 0..65 {
            sample(&mut score, &[PortFrame::default(), PortFrame::mono(10.0)]);
        }
        assert_eq!(score.state.elapsed, 65.0);
        assert!(score.state.reset_trigger.high);
        score.state.external_period = 6135.0;
        score.reset_input_detectors(false, true);
        assert_eq!(score.state.elapsed, 65.0);
        assert_eq!(score.state.external_period, 6135.0);
        sample(&mut score, &[PortFrame::default(), PortFrame::mono(10.0)]);
        assert_eq!(score.state.elapsed, 0.0);
        assert_eq!(score.state.step, 0);
        assert_eq!(score.state.reset_pulse, 47.0);
    }

    #[test]
    fn automation_destination_curves_duplicate_bars_and_loop_boundary_are_preserved() {
        let lane = Lane {
            length: 2.0,
            looped: false,
            count: 4,
            points: {
                let mut p = [ScorePoint::default(); 32];
                p[0] = ScorePoint {
                    bar: 0.0,
                    value: 0.0,
                    curve: ScoreCurve::Linear,
                };
                p[1] = ScorePoint {
                    bar: 1.0,
                    value: 1.0,
                    curve: ScoreCurve::EaseIn,
                };
                p[2] = ScorePoint {
                    bar: 1.0,
                    value: 4.0,
                    curve: ScoreCurve::Hold,
                };
                p[3] = ScorePoint {
                    bar: 2.0,
                    value: 2.0,
                    curve: ScoreCurve::Hold,
                };
                p
            },
        };
        assert_eq!(lane_value(&lane, 0.5), 0.125);
        assert_eq!(lane_value(&lane, 1.0), 4.0);
        assert_eq!(lane_value(&lane, 1.5), 4.0);
        assert_eq!(lane_value(&lane, 2.0), 2.0);
        let looped = Lane {
            looped: true,
            ..lane
        };
        assert_eq!(lane_value(&looped, 2.0), 0.0);
        let mut score = player(
            json!({"lanes":[{"length_bars":64,"points":[{"bar":0,"value":0},{"bar":1,"value":16}]}]}),
            json!({"tempo":120}),
        );
        for frame in 0..64 {
            let output = sample(&mut score, &[]);
            assert_eq!(output[12].channels, 1);
            let expected = if frame < 31 {
                0.0
            } else if frame < 63 {
                16.0 * 31.0 / 96000.0
            } else {
                16.0 * 63.0 / 96000.0
            };
            assert!((output[12].values[0] - expected).abs() < 1e-7);
        }
    }

    #[test]
    fn equal_score_adoption_keeps_exact_state_while_config_edit_and_reset_restart() {
        let data = json!({"length":64,"seed":128,"tracks":[{"steps":[{"step":0,"notes":[18],"slide":true},{"step":1,"notes":[17]}]}]});
        let mut old = player(
            data.clone(),
            json!({"tempo":128,"swing":54.525020599365234}),
        );
        for _ in 0..379 {
            sample(&mut old, &[]);
        }
        let mut same = player(
            data.clone(),
            json!({"tempo":128,"swing":54.525020599365234}),
        );
        same.adopt(&old);
        for _ in 0..257 {
            assert_eq!(sample(&mut same, &[]), sample(&mut old, &[]));
        }
        let mut changed = player(json!({"length":32}), json!({"tempo":128}));
        changed.adopt(&old);
        assert!(!changed.state.started);
        assert_eq!(changed.state.elapsed, 0.0);
        same.reset_state();
        let reset = sample(&mut same, &[]);
        assert_eq!(same.state.step, 0);
        assert_eq!(same.state.loops, 0);
        assert_eq!(reset[14].values[0], 10.0);
        assert_eq!(reset[4].values[0], 0.0);
        let mut stopped = player(
            data,
            json!({"tempo":128,"swing":54.525020599365234,"running":0}),
        );
        stopped.adopt(&old);
        assert!(!stopped.state.running);
        assert_eq!(sample(&mut stopped, &[])[4].values[0], 0.0);
    }

    #[test]
    fn score_configuration_strict_bounds_unknown_fields_and_sorted_points() {
        let empty = ScoreData::default();
        Compiled::build(&empty).unwrap();
        let invalid = [
            json!({"version":2}),
            json!({"length":0}),
            json!({"length":65}),
            json!({"steps_per_beat":0}),
            json!({"tracks":[{"voices":0}]}),
            json!({"tracks":[{"voices":5}]}),
            json!({"tracks":[{"glide_ms":5001}]}),
            json!({"tracks":[{"steps":[{"step":64}]}]}),
            json!({"tracks":[{"steps":[{"step":0},{"step":0}]}]}),
            json!({"tracks":[{"steps":[{"step":0,"notes":[61]}]}]}),
            json!({"tracks":[{"steps":[{"step":0,"notes":[0,1,2,3,4]}]}]}),
            json!({"tracks":[{"steps":[{"step":0,"velocity":1.1}]}]}),
            json!({"tracks":[{"steps":[{"step":0,"probability":-0.1}]}]}),
            json!({"tracks":[{"steps":[{"step":0,"ratchet":9}]}]}),
            json!({"lanes":[{"length_bars":0}]}),
            json!({"lanes":[{"points":[{"bar":2,"value":0},{"bar":1,"value":0}]}]}),
            json!({"lanes":[{"points":[{"bar":65,"value":0}]}]}),
            json!({"lanes":[{"points":[{"bar":0,"value":17}]}]}),
        ];
        for value in invalid {
            let data: ScoreData = serde_json::from_value(value.clone()).unwrap();
            assert!(Compiled::build(&data).is_err(), "{value}");
        }
        for value in [
            json!({"bogus":1}),
            json!({"length":1.5}),
            json!({"tracks":[{"voices":1.5}]}),
            json!({"lanes":[{"points":[{"bar":0,"value":0,"curve":"unknown"}]}]}),
        ] {
            assert!(serde_json::from_value::<ScoreData>(value).is_err());
        }
        let mut data = ScoreData {
            tracks: vec![ScoreTrack {
                voices: 1,
                glide_ms: f32::NAN,
                steps: vec![],
            }],
            ..ScoreData::default()
        };
        assert!(Compiled::build(&data).is_err());
        data.tracks.clear();
        data.lanes = vec![ScoreLane {
            length_bars: 64.0,
            looped: true,
            points: vec![ScorePoint {
                bar: 0.0,
                value: f32::INFINITY,
                curve: ScoreCurve::Linear,
            }],
        }];
        assert!(Compiled::build(&data).is_err());
    }
}
