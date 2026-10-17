//! Native host adapter for RackMCP 2.0.0 SequencerModule.
//!
//! Adapted from Zachary Scheffler's RackMCP, GPL-3.0-or-later, revision
//! 285e84c5ff2d3ecef25890da07f578af6a0c5ca6. See LICENSE-THIRD-PARTY.
//! Target resolution and hashing happen during preparation. Processing and
//! adoption retain receiver-owned data and touch fixed-capacity state only.

use super::{CompiledSequence, LaneMode, Pattern, Player, SequenceData, LANES};
use crate::{live_parameters::ParameterId, parameter, Node, PortFrame};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fmt};

pub(crate) const INPUTS: [&str; 2] = ["clock", "reset"];
pub(crate) const OUTPUTS: [&str; 18] = [
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
    "accent1",
    "accent2",
    "accent3",
    "accent4",
    "clock",
    "reset",
];

/// Fixed-capacity source module transport request. Requests of each queued kind
/// coalesce until the next source process sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RackMcpTransport {
    Play,
    Stop,
    Toggle,
    Reset,
    Select { pattern: u8, now: bool },
    Seek { bar: f64 },
}

/// Source status: controls/counts are current; position/clock publish every 64
/// processed samples. An unconsumed selection overlays the published pending index.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct RackMcpSequenceStatus {
    pub running: bool,
    pub bpm: f32,
    pub swing: f32,
    pub selected_pattern: u8,
    pub active_pattern: i32,
    pub pending_pattern: Option<u8>,
    pub step: i32,
    pub loop_count: i64,
    pub beat: f32,
    pub external_clock: bool,
    pub pattern_count: u8,
    pub lane_count: u8,
}

#[derive(Clone, Copy, Debug, Default)]
struct PublishedPosition {
    active: i32,
    pending: Option<u8>,
    step: i32,
    loops: i64,
    beat: f32,
    external: bool,
}

/// Native destination associated with a source lane's stable target pair.
/// Source-package provenance is checked by the document boundary.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NativeTarget {
    pub source_module_id: String,
    pub source_param_id: u16,
    pub module: String,
    pub parameter: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SequenceState {
    pub data: SequenceData,
    #[serde(default)]
    pub targets: Vec<NativeTarget>,
}

impl Default for SequenceState {
    fn default() -> Self {
        Self {
            data: SequenceData {
                version: 1,
                seed: 1,
                patterns: vec![Pattern {
                    name: "A".into(),
                    length: 16,
                    steps_per_beat: 4,
                    tracks: vec![],
                }],
                selected_pattern: None,
                lanes: vec![],
            },
            targets: vec![],
        }
    }
}

impl SequenceState {
    fn compile(&self) -> Result<CompiledSequence, String> {
        let compiled = self.data.compile()?;
        if self.targets.len() > LANES {
            return Err("sequence supports at most 8 native targets".into());
        }
        let mut seen = BTreeSet::new();
        for target in &self.targets {
            if !seen.insert((&target.source_module_id, target.source_param_id)) {
                return Err("native target source pairs must be unique".into());
            }
            if !self.data.lanes.iter().any(|lane| {
                lane.module_id == target.source_module_id && lane.param_id == target.source_param_id
            }) {
                return Err("native target must identify an existing sequence lane".into());
            }
            for (name, value) in [("module", &target.module), ("parameter", &target.parameter)] {
                if value.is_empty() || value.len() > 64 || value.contains('\0') {
                    return Err(format!(
                        "native target {name} must be 1..64 bytes without NUL"
                    ));
                }
            }
        }
        Ok(compiled)
    }

    /// Strict native-state validation, performed off the audio thread.
    pub fn validate(&self) -> Result<(), String> {
        self.compile().map(|_| ())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResolvedTarget {
    pub node: usize,
    pub parameter: ParameterId,
    pub min: f32,
    pub max: f32,
    pub snap: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ParameterWrite {
    pub lane: usize,
    pub target: ResolvedTarget,
    pub value: f32,
}

/// Scalar Rack SDK SchmittTrigger, including its uninitialized startup state.
#[derive(Clone, Copy, Debug, Default)]
enum Schmitt {
    Low,
    High,
    #[default]
    Uninitialized,
}

impl Schmitt {
    fn process(&mut self, value: f32, low: f32) -> bool {
        match *self {
            Self::Low if value >= 1.0 => {
                *self = Self::High;
                return true;
            }
            Self::High if value <= low => *self = Self::Low,
            Self::Uninitialized if value >= 1.0 => *self = Self::High,
            Self::Uninitialized if value <= low => *self = Self::Low,
            _ => {}
        }
        false
    }
}

pub(crate) struct RackMcpSequencer {
    state: Box<SequenceState>,
    data: Box<CompiledSequence>,
    data_fingerprint: [u8; 32],
    content_fingerprint: [u8; 32],
    target_fingerprints: [Option<[u8; 32]>; LANES],
    source_target_indices: [Option<usize>; LANES],
    targets: [Option<ResolvedTarget>; LANES],
    player: Player,
    rate: f32,
    bpm: f32,
    swing: f32,
    running: bool,
    reset: f32,
    clock_trigger: Schmitt,
    reset_trigger: Schmitt,
    reset_button: Schmitt,
    lane_divider: u8,
    publish_divider: u8,
    published: PublishedPosition,
    selected: u8,
    requested_reset: bool,
    requested_seek: Option<f64>,
    requested_pattern: Option<(u8, bool)>,
    last_lane_value: [f32; LANES],
}

impl fmt::Debug for RackMcpSequencer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RackMcpSequencer")
            .field("rate", &self.rate)
            .field("bpm", &self.bpm)
            .field("running", &self.running)
            .field("position", &self.player.position())
            .finish_non_exhaustive()
    }
}

fn fingerprint(value: &impl Serialize) -> Result<[u8; 32], String> {
    let bytes = serde_json::to_vec(value).map_err(|e| format!("sequence fingerprint: {e}"))?;
    Ok(Sha256::digest(bytes).into())
}

impl RackMcpSequencer {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for key in node.params.keys() {
            if !matches!(key.as_str(), "bpm" | "swing" | "running" | "reset") {
                return Err(format!(
                    "node '{}': unknown RackMCP parameter '{key}'",
                    node.id
                ));
            }
        }
        let bpm = parameter(node, "bpm", 120.0, 20.0, 300.0)?;
        let swing = parameter(node, "swing", 50.0, 50.0, 75.0)?;
        let running = parameter(node, "running", 0.0, 0.0, 1.0)?;
        let reset = parameter(node, "reset", 0.0, 0.0, 1.0)?;
        if running.fract() != 0.0 || reset.fract() != 0.0 {
            return Err("RackMCP running and reset must be 0 or 1".into());
        }
        let state = node.rackmcp_sequence.clone().unwrap_or_default();
        let data = Box::new(state.compile()?);
        let data_fingerprint = fingerprint(&state.data)?;
        let content_fingerprint = super::content_fingerprint(&state.data)?;
        let mut target_fingerprints = [None; LANES];
        let mut source_target_indices = [None; LANES];
        for (lane_index, lane) in state.data.lanes.iter().enumerate() {
            if let Some((index, target)) = state.targets.iter().enumerate().find(|(_, target)| {
                target.source_module_id == lane.module_id && target.source_param_id == lane.param_id
            }) {
                source_target_indices[lane_index] = Some(index);
                target_fingerprints[lane_index] = Some(fingerprint(target)?);
            }
        }
        let mut player = Player::default();
        player.adopt_data(&data);
        let selected = data.selected_pattern() as u8;
        Ok(Self {
            state,
            data,
            data_fingerprint,
            content_fingerprint,
            target_fingerprints,
            source_target_indices,
            targets: [None; LANES],
            player,
            rate: rate as f32,
            bpm,
            swing,
            running: running > 0.5,
            reset,
            clock_trigger: Schmitt::default(),
            reset_trigger: Schmitt::default(),
            reset_button: Schmitt::default(),
            lane_divider: 0,
            publish_divider: 0,
            published: PublishedPosition::default(),
            selected,
            requested_reset: false,
            requested_seek: None,
            requested_pattern: None,
            last_lane_value: [f32::NAN; LANES],
        })
    }

    /// Off-thread source-pair mapping; targets may appear in any saved order.
    pub(crate) fn source_targets(&self) -> [Option<&NativeTarget>; LANES] {
        self.source_target_indices
            .map(|index| index.map(|i| &self.state.targets[i]))
    }

    /// Bind prepared native destinations. Source lane caches survive target
    /// disappearance/recreation; only successful writes or explicit data loads
    /// change them. A target missing before its first write retains NaN.
    pub(crate) fn bind_targets(&mut self, targets: [Option<ResolvedTarget>; LANES]) {
        self.targets = targets;
    }

    /// Preserve source transport and detectors across graph replacement while
    /// retaining the receiving configuration, allocations and parameter values.
    pub(crate) fn adopt(&mut self, old: &Self) {
        self.adopt_preserving_selection(old, false);
    }

    pub(crate) fn adopt_preserving_selection(&mut self, old: &Self, preserve: bool) {
        self.player.adopt_runtime(&old.player);
        self.clock_trigger = old.clock_trigger;
        self.reset_trigger = old.reset_trigger;
        self.reset_button = old.reset_button;
        self.lane_divider = old.lane_divider;
        self.publish_divider = old.publish_divider;
        self.published = old.published;
        self.selected = old.selected;
        self.requested_reset = old.requested_reset;
        self.requested_seek = old.requested_seek;
        self.requested_pattern = old.requested_pattern;
        self.last_lane_value = old.last_lane_value;
        if !self.data_matches(old) && !(preserve && self.content_matches(old)) {
            self.reload_data();
        } else {
            for i in 0..LANES {
                if self.target_fingerprints[i] != old.target_fingerprints[i] {
                    self.last_lane_value[i] = f32::NAN;
                }
            }
        }
    }

    /// Binding changes acquire the receiving target set after donor ownership
    /// restoration, even when source data itself is unchanged.
    pub(crate) fn claims_match(&self, previous: &Self) -> bool {
        self.data_matches(previous) && self.target_fingerprints == previous.target_fingerprints
    }

    pub(crate) fn claims_match_preserving_selection(
        &self,
        previous: &Self,
        preserve: bool,
    ) -> bool {
        (self.data_matches(previous) || (preserve && self.content_matches(previous)))
            && self.target_fingerprints == previous.target_fingerprints
    }

    pub(crate) fn content_fingerprint(&self) -> [u8; 32] {
        self.content_fingerprint
    }

    /// Musical source content identity excludes only its persisted desired selection.
    pub(crate) fn content_matches(&self, previous: &Self) -> bool {
        self.content_fingerprint == previous.content_fingerprint
    }

    pub(crate) fn saved_state(&self) -> crate::SavedModuleState {
        let name = &self.data.pattern_names()[usize::from(self.selected)];
        crate::SavedModuleState::RackMcpSelection(
            crate::FixedPatternName::new(name).expect("compiled source name"),
        )
    }

    pub(crate) fn set_saved_selection(&mut self, name: crate::FixedPatternName) -> bool {
        let Some(index) = self
            .data
            .pattern_names()
            .iter()
            .position(|candidate| candidate == name.as_str())
        else {
            return false;
        };
        self.transport(RackMcpTransport::Select {
            pattern: index as u8,
            now: false,
        })
    }

    /// Source-data identity is independent of resolved native target routes.
    pub(crate) fn data_matches(&self, previous: &Self) -> bool {
        self.data_fingerprint == previous.data_fingerprint
    }

    /// Explicit source dataFromJson lifecycle, including an equal-data load.
    pub(crate) fn reload_data(&mut self) {
        self.selected = self.data.selected_pattern() as u8;
        self.player.adopt_data(&self.data);
        self.last_lane_value = [f32::NAN; LANES];
    }

    /// Control-side requests do not reload data, invalidate lane caches or claim targets.
    pub(crate) fn transport(&mut self, request: RackMcpTransport) -> bool {
        match request {
            RackMcpTransport::Play => self.running = true,
            RackMcpTransport::Stop => self.running = false,
            RackMcpTransport::Toggle => self.running = !self.running,
            RackMcpTransport::Reset => self.requested_reset = true,
            RackMcpTransport::Select { pattern, now } => {
                if usize::from(pattern) >= self.data.pattern_names().len() {
                    return false;
                }
                self.selected = pattern;
                self.requested_pattern = Some((pattern, now));
            }
            RackMcpTransport::Seek { bar } => {
                if !bar.is_finite() || !(0.0..=100000.0).contains(&bar) {
                    return false;
                }
                self.requested_seek = Some(bar);
            }
        }
        true
    }

    pub(crate) fn running(&self) -> f32 {
        if self.running {
            1.0
        } else {
            0.0
        }
    }

    pub(crate) fn set_running(&mut self, value: f32) -> bool {
        if value != 0.0 && value != 1.0 {
            return false;
        }
        self.running = value == 1.0;
        true
    }

    pub(crate) fn status(&self) -> RackMcpSequenceStatus {
        RackMcpSequenceStatus {
            running: self.running,
            bpm: self.bpm,
            swing: self.swing,
            selected_pattern: self.selected,
            active_pattern: self.published.active,
            pending_pattern: self
                .requested_pattern
                .map(|(index, _)| index)
                .or(self.published.pending),
            step: self.published.step,
            loop_count: self.published.loops,
            beat: self.published.beat,
            external_clock: self.published.external,
            pattern_count: self.data.pattern_names().len() as u8,
            lane_count: self.state.data.lanes.len() as u8,
        }
    }

    /// Commit a cache entry only after the native destination accepted it.
    pub(crate) fn acknowledge_write(&mut self, lane: usize, value: f32) {
        if let Some(cached) = self.last_lane_value.get_mut(lane) {
            *cached = value;
        }
    }

    #[cfg(test)]
    pub(crate) fn position(&self) -> (usize, Option<usize>, usize, i64, f64) {
        self.player.position()
    }

    /// The source module has no onReset override. A fresh constructor already
    /// initializes DSP state; generic host initialization must not synthesize
    /// a transport-reset event or its output pulse.
    pub(crate) fn initialize_state(&mut self) {}

    #[allow(clippy::manual_clamp)] // Preserve Rack's fmin/fmax NaN behavior.
    pub(crate) fn process(
        &mut self,
        inputs: &[PortFrame; 2],
        clock_connected: bool,
    ) -> ([PortFrame; 18], [Option<ParameterWrite>; LANES]) {
        // Source uses bitwise OR: both detectors always consume their sample.
        let reset_input = self.reset_trigger.process(inputs[1].channel(0), 0.1);
        let reset_button = self.reset_button.process(self.reset, 0.0);
        if reset_input | reset_button {
            self.player.reset();
        }
        if self.requested_reset {
            self.requested_reset = false;
            self.player.reset();
        }
        if let Some(bar) = self.requested_seek.take() {
            self.player.seek(bar * 4.0);
        }
        if let Some((pattern, now)) = self.requested_pattern.take() {
            self.player.queue_pattern(usize::from(pattern), now);
        }
        let rising = clock_connected && self.clock_trigger.process(inputs[0].channel(0), 0.1);
        let frame = self.player.process(
            &self.data,
            self.rate,
            self.bpm,
            self.swing,
            self.running,
            clock_connected,
            rising,
        );
        let mut outputs = [PortFrame::default(); 18];
        for track in 0..4 {
            let voices = frame.voices[track];
            for offset in [0, 4, 8, 12] {
                outputs[offset + track].channels = voices as u8;
            }
            for voice in 0..voices {
                outputs[track].values[voice] = frame.pitch[track][voice];
                outputs[4 + track].values[voice] = frame.gate[track];
                outputs[8 + track].values[voice] = frame.velocity[track];
                outputs[12 + track].values[voice] = frame.accent[track];
            }
        }
        outputs[16] = PortFrame::mono(if frame.clock { 10.0 } else { 0.0 });
        outputs[17] = PortFrame::mono(if frame.reset { 10.0 } else { 0.0 });

        let mut writes = [None; LANES];
        self.lane_divider += 1;
        if self.lane_divider >= 32 {
            self.lane_divider = 0;
            if self.running {
                let bars = self.player.position().4 / 4.0;
                for (i, write) in writes.iter_mut().enumerate() {
                    let (Some(lane), Some(target)) = (&self.data.lanes()[i], self.targets[i])
                    else {
                        continue;
                    };
                    let mut value = lane.value_at(bars);
                    if lane.mode == LaneMode::Normalized {
                        value = target.min + value * (target.max - target.min);
                    }
                    // Rack clampSafe swaps reversed bounds, then fmin/fmax;
                    // snap follows clamping and is not clamped a second time.
                    let (lo, hi) = if target.min <= target.max {
                        (target.min, target.max)
                    } else {
                        (target.max, target.min)
                    };
                    value = value.min(hi).max(lo);
                    if target.snap {
                        value = value.round();
                    }
                    let delta = (value - self.last_lane_value[i]).abs();
                    if delta.is_nan() || delta >= 1e-7_f32 {
                        *write = Some(ParameterWrite {
                            lane: i,
                            target,
                            value,
                        });
                    }
                }
            }
        }
        self.publish_divider += 1;
        if self.publish_divider >= 64 {
            self.publish_divider = 0;
            let (active, pending, step, loops, beat) = self.player.position();
            self.published = PublishedPosition {
                active: active as i32,
                pending: pending.map(|index| index as u8),
                step: step as i32,
                loops,
                beat: beat as f32,
                external: clock_connected,
            };
        }
        (outputs, writes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(state: SequenceState, params: serde_json::Value) -> Node {
        serde_json::from_value(json!({
            "id": "sequence", "kind": "rackmcp_sequencer",
            "rackmcp_sequence": state, "params": params
        }))
        .unwrap()
    }

    fn sequencer(state: SequenceState, params: serde_json::Value) -> RackMcpSequencer {
        RackMcpSequencer::build(&node(state, params), 1000).unwrap()
    }

    fn lane_state(mode: &str, value: f32) -> SequenceState {
        serde_json::from_value(json!({
            "data": {
                "patterns": [{"name": "A", "length": 16}],
                "lanes": [{"moduleId": "42", "paramId": 0, "mode": mode,
                    "points": [{"bar": 0, "value": value}]}]
            },
            "targets": [{"source_module_id": "42", "source_param_id": 0,
                "module": "vca", "parameter": "level"}]
        }))
        .unwrap()
    }

    fn target() -> ResolvedTarget {
        ResolvedTarget {
            node: 1,
            parameter: ParameterId::StockVcaLevel,
            min: 0.0,
            max: 1.0,
            snap: false,
        }
    }

    fn bind(seq: &mut RackMcpSequencer, target: ResolvedTarget) {
        let mut targets = [None; LANES];
        targets[0] = Some(target);
        seq.bind_targets(targets);
    }

    fn tick(seq: &mut RackMcpSequencer) -> ([PortFrame; 18], [Option<ParameterWrite>; LANES]) {
        seq.process(&[PortFrame::default(); 2], false)
    }

    fn cycle(seq: &mut RackMcpSequencer) -> Option<ParameterWrite> {
        let mut result = None;
        for _ in 0..32 {
            if let Some(write) = tick(seq).1[0] {
                assert!(result.is_none());
                result = Some(write);
            }
        }
        result
    }

    #[test]
    fn silent_default_is_stopped_and_initialize_does_not_emit_reset() {
        let state = SequenceState::default();
        state.validate().unwrap();
        assert_eq!(
            (
                state.data.patterns[0].name.as_str(),
                state.data.patterns[0].length
            ),
            ("A", 16)
        );
        let mut seq = sequencer(state, json!({}));
        seq.initialize_state();
        for _ in 0..64 {
            let (outputs, writes) = tick(&mut seq);
            assert!(outputs
                .iter()
                .all(|p| p.channels == 1 && p.values == [0.0; 16]));
            assert!(writes.iter().all(Option::is_none));
        }
        assert_eq!(seq.position(), (0, None, 0, 0, 0.0));
    }

    #[test]
    fn polyphonic_groups_repeat_gate_velocity_accent_for_every_voice() {
        let state: SequenceState = serde_json::from_value(json!({"data": {
            "patterns": [{"name": "A", "length": 16, "tracks": [{
                "voices": 4, "steps": [{"step": 0, "notes": [12, 24],
                    "vel": 0.7, "accent": true}]
            }]}]
        }}))
        .unwrap();
        let mut seq = sequencer(state, json!({"running": 1}));
        let first = tick(&mut seq).0;
        assert_eq!(first[16], PortFrame::mono(10.0));
        let outputs = tick(&mut seq).0;
        assert_eq!(&outputs[0].values[..4], &[1.0, 2.0, 1.0, 2.0]);
        for (port, value) in [(4, 10.0), (8, 7.0), (12, 10.0)] {
            assert_eq!(outputs[port].channels, 4);
            assert_eq!(&outputs[port].values[..4], &[value; 4]);
            assert_eq!(&outputs[port].values[4..], &[0.0; 12]);
        }
        assert_eq!(outputs[0].channels, 4);
        assert_eq!(outputs[16].channels, 1);
        assert_eq!(outputs[17], PortFrame::mono(0.0));
    }

    #[test]
    fn divider_advances_stopped_and_cache_commits_only_successful_writes() {
        let state = lane_state("normalized", 0.25);
        let mut stopped = sequencer(state.clone(), json!({}));
        bind(&mut stopped, target());
        for _ in 0..31 {
            assert!(tick(&mut stopped).1[0].is_none());
        }
        let mut playing = sequencer(state, json!({"running": 1}));
        bind(&mut playing, target());
        playing.adopt(&stopped);
        let write = tick(&mut playing).1[0].unwrap();
        assert_eq!((write.lane, write.value), (0, 0.25));
        // A rejected/missing destination write is retried at the next division.
        assert_eq!(cycle(&mut playing).unwrap().value, 0.25);
        playing.acknowledge_write(write.lane, write.value);
        // An external manual override can persist: unchanged source lane data
        // does not query the destination or resend the cached value.
        let mut destination_value = 0.9;
        if let Some(write) = cycle(&mut playing) {
            destination_value = write.value;
        }
        assert_eq!(destination_value, 0.9);
        playing.acknowledge_write(0, 0.25 + 5e-8);
        assert!(cycle(&mut playing).is_none());
        playing.acknowledge_write(0, 0.25 + 2e-7);
        assert!(cycle(&mut playing).is_some());
        playing.reload_data();
        assert_eq!(cycle(&mut playing).unwrap().value, 0.25);
    }

    #[test]
    fn source_pair_binding_is_independent_of_target_order_and_strict() {
        let mut state = lane_state("normalized", 0.2);
        let mut lane = state.data.lanes[0].clone();
        lane.module_id = "99".into();
        state.data.lanes.push(lane);
        state.targets.insert(
            0,
            NativeTarget {
                source_module_id: "99".into(),
                source_param_id: 0,
                module: "filter".into(),
                parameter: "frequency".into(),
            },
        );
        let seq = sequencer(state.clone(), json!({}));
        let targets = seq.source_targets();
        assert_eq!(targets[0].unwrap().module, "vca");
        assert_eq!(targets[1].unwrap().module, "filter");
        state.targets[0].source_module_id = "100".into();
        assert!(state.validate().is_err());
        state.targets[0] = state.targets[1].clone();
        assert!(state.validate().is_err());
        state.targets.remove(0);
        state.targets[0].parameter = "bad\0parameter".into();
        assert!(state.validate().is_err());
        assert!(serde_json::from_value::<SequenceState>(json!({
            "data": SequenceState::default().data, "unknown": 1
        }))
        .is_err());
    }

    #[test]
    fn mapping_then_clamp_then_snap_preserves_source_float_order() {
        let mut normalized = sequencer(lane_state("normalized", 0.25), json!({"running": 1}));
        bind(
            &mut normalized,
            ResolvedTarget {
                min: 10.0,
                max: -2.0,
                ..target()
            },
        );
        assert_eq!(cycle(&mut normalized).unwrap().value, 7.0);

        let mut raw = sequencer(lane_state("raw", 4.0), json!({"running": 1}));
        bind(
            &mut raw,
            ResolvedTarget {
                min: 0.2,
                max: 0.6,
                snap: true,
                ..target()
            },
        );
        // round(.6)=1: source intentionally does not clamp after snapping.
        assert_eq!(cycle(&mut raw).unwrap().value, 1.0);
    }

    #[test]
    fn sdk_schmitt_startup_disconnect_and_reset_button_thresholds() {
        let mut seq = sequencer(SequenceState::default(), json!({"running": 1, "reset": 1}));
        let high = [PortFrame::mono(10.0), PortFrame::mono(10.0)];
        // A high initial input/button arms HIGH without generating any event.
        let initial = seq.process(&high, true).0;
        assert_eq!(initial[16].channel(0), 0.0);
        assert_eq!(initial[17].channel(0), 0.0);
        seq.process(&[PortFrame::mono(0.1), PortFrame::mono(0.1)], true);
        let edge = seq.process(&high, true).0;
        assert_eq!(edge[16].channel(0), 10.0);
        assert_eq!(edge[17].channel(0), 10.0);
        // A disconnected clock does not feed low into the Schmitt detector.
        seq.process(&[PortFrame::default(), PortFrame::mono(10.0)], false);
        let reconnected = seq.process(&high, true).0;
        assert_eq!(reconnected[16].channel(0), 0.0);
        assert_eq!(reconnected[17].channel(0), 0.0);
        assert_eq!(seq.position().2, 0);
        seq.process(&[PortFrame::mono(0.1), PortFrame::mono(10.0)], true);
        seq.process(&high, true);
        assert_eq!(seq.position().2, 1);

        let mut button = Schmitt::default();
        assert!(!button.process(0.0, 0.0));
        assert!(button.process(1.0, 0.0));
        assert!(!button.process(0.1, 0.0));
        assert!(!button.process(1.0, 0.0));
        assert!(!button.process(0.0, 0.0));
        assert!(button.process(1.0, 0.0));
    }

    #[test]
    fn graph_adoption_preserves_position_and_cache_while_data_load_rearms() {
        let state = lane_state("normalized", 0.4);
        let mut old = sequencer(state.clone(), json!({"running": 1}));
        bind(&mut old, target());
        let write = cycle(&mut old).unwrap();
        old.acknowledge_write(0, write.value);
        for _ in 0..140 {
            tick(&mut old);
        }
        let position = old.position();
        assert!(position.2 > 0);
        let mut next = sequencer(state.clone(), json!({"running": 1, "bpm": 90}));
        bind(&mut next, target());
        let receiving_store = &*next.data as *const CompiledSequence;
        next.adopt(&old);
        assert_eq!(&*next.data as *const CompiledSequence, receiving_store);
        assert_eq!(next.position(), position);
        assert!(cycle(&mut next).is_none());
        next.reload_data();
        assert_eq!(cycle(&mut next).unwrap().value, 0.4);

        let mut changed = state;
        changed.data.patterns[0].length = 1;
        let mut replacement = sequencer(changed, json!({"running": 1}));
        bind(&mut replacement, target());
        replacement.adopt(&old);
        assert_eq!(replacement.position(), position);
        // Source repairs out-of-range steps on the first process sample.
        tick(&mut replacement);
        assert_eq!(replacement.position().2, 0);
        assert_eq!(cycle(&mut replacement).unwrap().value, 0.4);
    }
}
