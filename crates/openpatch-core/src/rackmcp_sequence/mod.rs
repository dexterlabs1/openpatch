//! RackMCP 2.0.0 sequencing primitives and native module adapter.
//!
//! Adapted from Zachary Scheffler's RackMCP, GPL-3.0-or-later, revision
//! 285e84c5ff2d3ecef25890da07f578af6a0c5ca6. See LICENSE-THIRD-PARTY.
//! Compilation/JSON belong on the control thread. Playback borrows compiled
//! fixed-capacity state and neither allocates nor resolves parameter targets.
//! Capture and source import/provenance are separate host responsibilities.

mod adapter;
mod engine;

pub use adapter::{NativeTarget, SequenceState};
pub(crate) use adapter::{RackMcpSequencer, ResolvedTarget, INPUTS, OUTPUTS};

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub(crate) fn catalog() -> serde_json::Value {
    serde_json::json!({
        "kind": "rackmcp_sequencer", "title": "RACKMCP SEQUENCER", "model": "source-version",
        "description": "RackMCP 2.0.0 four-track sequencer with patterns, ratchets, slides, accents and parameter automation. Native delayed port scheduling; process-time automation follows native node order. Unsupported Rack plugins remain unresolved.",
        "inputs": INPUTS, "outputs": OUTPUTS,
        "params": {
            "bpm": {"default":120,"min":20,"max":300,"unit":"BPM"},
            "swing": {"default":50,"min":50,"max":75,"unit":"%"},
            "running": {"default":0,"min":0,"max":1,"step":1,"values":[0,1],"labels":["Off","On"]},
            "reset": {"default":0,"min":0,"max":1,"step":1,"values":[0,1],"labels":["Idle","Reset"]}
        },
        "rackmcp_sequence_schema": {
            "data": "strict RackMCP version-1 camelCase sequence data; 1–16 patterns, 4 tracks, 1–64 steps, 1–4 voices, 8 lanes of 32 points",
            "targets": "up to 8 explicit source_module_id/source_param_id to native module/parameter bindings; unresolved destinations do not receive writes"
        },
        "automation_policy": "Evaluate every 32 samples while running. Clamp normalized/raw values to destination bounds, apply destination snap, write on source 1e-7 delta. Cache changes only after a successful assignment; unchanged lanes permit manual overrides.",
        "source": {"plugin":"RackMCP","version":"2.0.0","revision":"285e84c5ff2d3ecef25890da07f578af6a0c5ca6","license":"GPL-3.0-or-later"}
    })
}

const LANES: usize = 8;
const POINTS: usize = 32;
fn one() -> u32 {
    1
}
fn four() -> u8 {
    4
}
fn one_voice() -> u8 {
    1
}
fn glide() -> f64 {
    80.0
}
fn velocity() -> f64 {
    0.8
}
fn half() -> f64 {
    0.5
}
fn full() -> f64 {
    1.0
}
fn yes() -> bool {
    true
}
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

/// Strict transaction data. Legacy Rack patch loading is deliberately a
/// different, permissive contract; retain its original JSON in SourcePackage.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SequenceData {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default = "one")]
    pub seed: u32,
    pub patterns: Vec<Pattern>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub selected_pattern: Option<String>,
    #[serde(default)]
    pub lanes: Vec<Lane>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pattern {
    pub name: String,
    pub length: u8,
    #[serde(default = "four")]
    pub steps_per_beat: u8,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Track {
    #[serde(default = "one_voice")]
    pub voices: u8,
    #[serde(default = "glide")]
    pub glide_ms: f64,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub step: u8,
    #[serde(default)]
    pub notes: Vec<f64>,
    #[serde(default = "velocity")]
    pub vel: f64,
    #[serde(default = "half")]
    pub gate: f64,
    #[serde(default = "full")]
    pub prob: f64,
    #[serde(default = "one_voice")]
    pub ratchet: u8,
    #[serde(default)]
    pub accent: bool,
    #[serde(default)]
    pub slide: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Curve {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    Hold,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LaneMode {
    #[default]
    Normalized,
    Raw,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Lane {
    pub module_id: String,
    pub param_id: u16,
    #[serde(default)]
    pub mode: LaneMode,
    #[serde(default = "yes", rename = "loop")]
    pub looping: bool,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub length_bars: Option<f64>,
    pub points: Vec<Point>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub bar: f64,
    pub value: f64,
    #[serde(default)]
    pub curve: Curve,
}

/// Immutable control-thread compilation. Source IDs are preserved until the
/// host resolves them to actual native targets; missing targets are not CVs.
pub struct CompiledSequence {
    data: Box<engine::Data>,
    names: Vec<String>,
    selected: usize,
    lanes: [Option<AutomationLane>; LANES],
}

#[derive(Clone, Copy, Default)]
struct CompiledPoint {
    bar: f32,
    value: f32,
    curve: Curve,
}

pub struct AutomationLane {
    pub module_id: i64,
    pub param_id: u16,
    pub mode: LaneMode,
    pub looping: bool,
    pub length_bars: f32,
    count: usize,
    points: [CompiledPoint; POINTS],
}

fn range(value: f64, lo: f64, hi: f64, field: &str) -> Result<(), String> {
    if value.is_finite() && value >= lo && value <= hi {
        Ok(())
    } else {
        Err(format!("{field} must be within {lo}..{hi}"))
    }
}

impl SequenceData {
    pub fn compile(&self) -> Result<CompiledSequence, String> {
        if self.version != 1 || !(1..=16).contains(&self.patterns.len()) || self.lanes.len() > LANES
        {
            return Err("sequence requires version 1, 1..16 patterns and at most 8 lanes".into());
        }
        let mut names = BTreeSet::new();
        // Compilation is intentionally off the callback. The eventual native
        // runtime owns/retires this allocation across graph adoption.
        let mut data = Box::new(engine::Data::default());
        data.seed = self.seed;
        data.pattern_count = self.patterns.len() as u8;
        for (pi, pattern) in self.patterns.iter().enumerate() {
            if pattern.name.is_empty()
                || pattern.name.len() > 32
                || pattern.name.contains('\0')
                || !names.insert(&pattern.name)
            {
                return Err(format!(
                    "patterns[{pi}].name must be unique, 1..32 bytes and contain no NUL"
                ));
            }
            if !(1..=64).contains(&pattern.length)
                || !(1..=8).contains(&pattern.steps_per_beat)
                || pattern.tracks.len() > 4
            {
                return Err(format!(
                    "patterns[{pi}] exceeds length, division or track bounds"
                ));
            }
            let compiled = &mut data.patterns[pi];
            compiled.length = pattern.length;
            compiled.steps_per_beat = pattern.steps_per_beat;
            for (ti, track) in pattern.tracks.iter().enumerate() {
                if !(1..=4).contains(&track.voices) || track.steps.len() > 64 {
                    return Err(format!("tracks[{ti}] exceeds voices or steps bounds"));
                }
                range(track.glide_ms, 0.0, 5000.0, "glideMs")?;
                let tr = &mut compiled.tracks[ti];
                tr.voices = track.voices;
                tr.glide_ms = track.glide_ms as f32;
                let mut seen = [false; 64];
                for step in &track.steps {
                    if step.step >= pattern.length
                        || seen[step.step as usize]
                        || step.notes.len() > 4
                        || !(1..=8).contains(&step.ratchet)
                    {
                        return Err(format!("tracks[{ti}].steps has an invalid/duplicate step, note count or ratchet"));
                    }
                    seen[step.step as usize] = true;
                    range(step.vel, 0.0, 1.0, "vel")?;
                    range(step.gate, 0.0, 1.0, "gate")?;
                    range(step.prob, 0.0, 1.0, "prob")?;
                    let mut st = engine::Step {
                        on: true,
                        note_count: step.notes.len() as u8,
                        vel: step.vel as f32,
                        gate: step.gate as f32,
                        prob: step.prob as f32,
                        ratchet: step.ratchet,
                        accent: step.accent,
                        slide: step.slide,
                        ..engine::Step::default()
                    };
                    for (index, note) in step.notes.iter().enumerate() {
                        range(*note, -60.0, 60.0, "notes")?;
                        st.notes[index] = *note as f32;
                    }
                    tr.steps[step.step as usize] = st;
                }
            }
        }
        let selected = match &self.selected_pattern {
            Some(name) => self
                .patterns
                .iter()
                .position(|p| &p.name == name)
                .ok_or("selectedPattern names no pattern")?,
            None => 0,
        };
        let mut targets = BTreeSet::new();
        let mut lanes = std::array::from_fn(|_| None);
        for (i, lane) in self.lanes.iter().enumerate() {
            let id = lane
                .module_id
                .parse::<i64>()
                .map_err(|_| "moduleId must be a canonical decimal ID")?;
            if id < 0 || id.to_string() != lane.module_id || !targets.insert((id, lane.param_id)) {
                return Err("lane target must be canonical, nonnegative and unique".into());
            }
            if !(1..=POINTS).contains(&lane.points.len()) {
                return Err("lane requires 1..32 points".into());
            }
            let mut points = [CompiledPoint::default(); POINTS];
            let mut last = -1.0;
            for (j, point) in lane.points.iter().enumerate() {
                range(point.bar, 0.0, 4096.0, "point.bar")?;
                if point.bar < last {
                    return Err("lane points must be nondecreasing".into());
                }
                last = point.bar;
                if !point.value.is_finite() || !(point.value as f32).is_finite() {
                    return Err("lane point.value must be finite and representable as f32".into());
                }
                if lane.mode == LaneMode::Normalized {
                    range(point.value, 0.0, 1.0, "point.value")?;
                }
                points[j] = CompiledPoint {
                    bar: point.bar as f32,
                    value: point.value as f32,
                    curve: point.curve,
                };
                if j > 0 && !(points[j].value - points[j - 1].value).is_finite() {
                    return Err("lane interpolation delta must be representable as f32".into());
                }
            }
            // The source derives this default from its already-rounded f32 point.
            let last_bar = points[lane.points.len() - 1].bar;
            let length = lane.length_bars.unwrap_or(if last_bar > 0.0 {
                f64::from(last_bar)
            } else {
                1.0
            });
            range(length, 1e-6, 4096.0, "lengthBars")?;
            lanes[i] = Some(AutomationLane {
                module_id: id,
                param_id: lane.param_id,
                mode: lane.mode,
                looping: lane.looping,
                length_bars: length as f32,
                count: lane.points.len(),
                points,
            });
        }
        Ok(CompiledSequence {
            data,
            selected,
            names: self.patterns.iter().map(|p| p.name.clone()).collect(),
            lanes,
        })
    }
}

impl CompiledSequence {
    pub fn pattern_names(&self) -> &[String] {
        &self.names
    }
    pub fn selected_pattern(&self) -> usize {
        self.selected
    }
    pub fn lanes(&self) -> &[Option<AutomationLane>; LANES] {
        &self.lanes
    }
}

impl Curve {
    fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::EaseIn => t * t * t,
            Self::EaseOut => {
                let u = 1.0 - t;
                1.0 - u * u * u
            }
            Self::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powf(3.0) / 2.0
                }
            }
            Self::Hold => {
                if t < 1.0 {
                    0.0
                } else {
                    1.0
                }
            }
        }
    }
}

impl AutomationLane {
    /// Evaluate the source float curve at a finite transport position. This
    /// returns a lane value, not a parameter write; the host must resolve and
    /// clamp/snap the target and preserve its per-lane change cache.
    pub fn value_at(&self, bars: f64) -> f32 {
        let mut position = if bars.is_finite() { bars.max(0.0) } else { 0.0 };
        if self.looping {
            position %= f64::from(self.length_bars);
        }
        let points = &self.points[..self.count];
        if position <= f64::from(points[0].bar) {
            return points[0].value;
        }
        for pair in points.windows(2) {
            if position < f64::from(pair[1].bar) {
                let span = f64::from(pair[1].bar) - f64::from(pair[0].bar);
                let t = if span > 0.0 {
                    ((position - f64::from(pair[0].bar)) / span) as f32
                } else {
                    1.0
                };
                return pair[0].value + pair[1].curve.apply(t) * (pair[1].value - pair[0].value);
            }
        }
        points[points.len() - 1].value
    }
}

/// Fixed-capacity source transport. The host calls `adopt_data` when source
/// structured data is actually loaded/edited, not for unrelated parameter-only
/// graph adoption or every sample. Data adoption preserves playhead.
#[derive(Default)]
pub struct Player {
    engine: engine::Engine,
    loaded: bool,
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub voices: [usize; 4],
    pub pitch: [[f32; 4]; 4],
    pub gate: [f32; 4],
    pub velocity: [f32; 4],
    pub accent: [f32; 4],
    pub clock: bool,
    pub reset: bool,
}

impl Player {
    /// Preserve transport without transferring control-owned compiled data.
    pub(crate) fn adopt_runtime(&mut self, previous: &Self) {
        self.engine = previous.engine.clone();
        self.loaded = previous.loaded;
    }
    pub fn adopt_data(&mut self, data: &CompiledSequence) {
        if data.selected != self.engine.active_pattern() {
            self.engine.request_pattern(data.selected, !self.loaded);
        }
        self.loaded = true;
    }
    pub fn reset(&mut self) {
        self.engine.reset();
    }
    pub fn seek(&mut self, beats: f64) {
        self.engine.seek(beats);
    }
    pub fn request_pattern(&mut self, data: &CompiledSequence, index: usize, now: bool) -> bool {
        index < data.names.len() && self.engine.request_pattern(index, now)
    }
    pub fn position(&self) -> (usize, Option<usize>, usize, i64, f64) {
        (
            self.engine.active_pattern(),
            self.engine.pending_pattern(),
            self.engine.step_index(),
            self.engine.loop_count(),
            self.engine.beat_position(),
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        data: &CompiledSequence,
        sample_rate: f32,
        bpm: f32,
        swing: f32,
        running: bool,
        external_clock: bool,
        rising: bool,
    ) -> Frame {
        let out = self.engine.process(
            &data.data,
            sample_rate,
            bpm,
            swing,
            running,
            external_clock,
            rising,
        );
        Frame {
            voices: out.voices,
            pitch: out.pitch,
            gate: out.gate,
            velocity: out.vel,
            accent: out.accent,
            clock: out.clock,
            reset: out.reset,
        }
    }
}

#[cfg(test)]
mod tests;
