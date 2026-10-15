//! Original fixed-storage MIDI controller conversion and transient Learn state.
use crate::{parameter, MidiEvent, Node, PortFrame, SavedModuleState};
use serde_json::{json, Value};

pub const OUTPUTS: [&str; 16] = [
    "out1", "out2", "out3", "out4", "out5", "out6", "out7", "out8", "out9", "out10", "out11",
    "out12", "out13", "out14", "out15", "out16",
];
const DEFAULT_ASSIGNMENTS: [i16; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiCcLearnCompletion {
    pub serial: u64,
    pub cell: u8,
    pub before: [i16; 16],
    pub after: [i16; 16],
}

/// No heap storage or callback-time initialization is required.
#[derive(Clone, Debug)]
pub struct MidiCc {
    assignments: [i16; 16],
    // Keep graph baselines separate from live state: a controller movement or
    // learned mapping must survive recompilation of an unrelated graph edit.
    configured_assignments: [i16; 16],
    configured_values: [u8; 128],
    values: [[u8; 128]; 16],
    pending: [[u8; 32]; 16],
    filters: [[f32; 16]; 16],
    smooth: bool,
    mpe: bool,
    high_resolution: bool,
    channel: u8,
    sample_time: f32,
    learning: Option<(u64, u8)>,
    completion: Option<MidiCcLearnCompletion>,
}

fn normalize(mut assignments: [i16; 16]) -> [i16; 16] {
    for cell in 0..16 {
        if assignments[cell] >= 0 {
            for prior in 0..cell {
                if assignments[prior] == assignments[cell] {
                    assignments[prior] = -1;
                }
            }
        }
    }
    assignments
}
fn integer(node: &Node, name: &str, default: f32, min: f32, max: f32) -> Result<i16, String> {
    let value = parameter(node, name, default, min, max)?;
    if value.fract() != 0.0 {
        return Err(format!(
            "node '{}': parameter '{name}' must be an integer",
            node.id
        ));
    }
    Ok(value as i16)
}
pub fn initial_midi_cc_state(node: &Node) -> Result<SavedModuleState, String> {
    Ok(MidiCc::build(node, 48_000)?.saved_state())
}
impl MidiCc {
    pub fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for key in node.params.keys() {
            let mapping = key
                .strip_prefix("cc")
                .and_then(|s| s.parse::<usize>().ok())
                .is_some_and(|i| (1..=16).contains(&i) && *key == format!("cc{i}"));
            let value = key
                .strip_prefix("cc_value")
                .and_then(|s| s.parse::<usize>().ok())
                .is_some_and(|i| i < 128 && *key == format!("cc_value{i}"));
            if !mapping
                && !value
                && !["smooth", "mpe", "high_resolution", "channel"].contains(&key.as_str())
            {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let mut assignments = DEFAULT_ASSIGNMENTS;
        for (i, assignment) in assignments.iter_mut().enumerate() {
            *assignment = integer(node, &format!("cc{}", i + 1), (i + 1) as f32, -1.0, 127.0)?;
        }
        let assignments = normalize(assignments);
        let mut values = [[0; 128]; 16];
        for (i, value) in values[0].iter_mut().enumerate() {
            *value = integer(node, &format!("cc_value{i}"), 0.0, 0.0, 127.0)? as u8;
        }
        Ok(Self {
            assignments,
            configured_assignments: assignments,
            configured_values: values[0],
            values,
            pending: [[0; 32]; 16],
            filters: [[0.0; 16]; 16],
            smooth: integer(node, "smooth", 1.0, 0.0, 1.0)? != 0,
            mpe: integer(node, "mpe", 0.0, 0.0, 1.0)? != 0,
            high_resolution: integer(node, "high_resolution", 0.0, 0.0, 1.0)? != 0,
            channel: integer(node, "channel", 0.0, 0.0, 16.0)? as u8,
            sample_time: 1.0 / rate as f32,
            learning: None,
            completion: None,
        })
    }
    pub fn arm_learn(&mut self, serial: u64, cell: u8) -> bool {
        if serial == 0 || cell >= 16 || self.completion.is_some() {
            return false;
        }
        self.learning = Some((serial, cell));
        true
    }
    pub fn cancel_learn(&mut self, serial: u64) -> bool {
        if self.learning.is_some_and(|(active, _)| active == serial) {
            self.learning = None;
            true
        } else {
            false
        }
    }
    pub fn learning(&self) -> Option<(u64, u8)> {
        self.learning
    }
    pub fn take_learn_completion(&mut self) -> Option<MidiCcLearnCompletion> {
        self.completion.take()
    }
    /// Host panic/disconnect policy preserves committed values and filters.
    pub fn panic(&mut self) {
        self.learning = None;
        self.pending = [[0; 32]; 16];
    }
    fn cancel_configuration(&mut self) {
        self.learning = None;
        self.completion = None;
    }
    pub fn handle_midi(&mut self, event: MidiEvent) {
        if !event.is_valid() {
            return;
        }
        if event == MidiEvent::Panic {
            self.panic();
            return;
        }
        let MidiEvent::ControlChange {
            channel,
            controller,
            value,
        } = event
        else {
            return;
        };
        if self.channel != 0 && self.channel != channel + 1 {
            return;
        }
        let lane = if self.mpe { channel as usize } else { 0 };
        let cc = controller as usize;
        if value != self.values[lane][cc] {
            if let Some((serial, cell)) = self.learning.take() {
                let before = self.assignments;
                self.assignments[cell as usize] = -1;
                for assignment in &mut self.assignments {
                    if *assignment == cc as i16 {
                        *assignment = -1;
                    }
                }
                self.assignments[cell as usize] = cc as i16;
                self.completion = Some(MidiCcLearnCompletion {
                    serial,
                    cell,
                    before,
                    after: self.assignments,
                });
            }
        }
        if self.high_resolution && cc < 32 {
            self.pending[lane][cc] = value;
        } else {
            self.values[lane][cc] = value;
            if self.high_resolution && cc < 64 {
                self.values[lane][cc - 32] = self.pending[lane][cc - 32];
            }
        }
    }
    pub fn process(&mut self, outputs: &mut [PortFrame], connected_mask: u16) {
        for (cell, output) in outputs.iter_mut().take(16).enumerate() {
            *output = PortFrame::default();
            if connected_mask & (1 << cell) == 0 {
                continue;
            }
            output.channels = if self.mpe { 16 } else { 1 };
            let cc = self.assignments[cell];
            if cc < 0 {
                continue;
            }
            let cc = cc as usize;
            for lane in 0..usize::from(output.channels) {
                let mut number = self.values[lane][cc] as f32 * 128.0;
                if self.high_resolution && cc < 32 {
                    number += self.values[lane][cc + 32] as f32;
                }
                let target = (number / 16256.0).clamp(0.0, 1.0);
                let filter = &mut self.filters[cell][lane];
                if self.smooth && (target - *filter).abs() < 1.0 {
                    // Preserve the f32 time-constant arithmetic. When the
                    // Euler step rounds away, settle at its target instead
                    // of leaving a permanent quantization-sized residual.
                    let rate = 1.0_f32 / (1.0_f32 / 30.0);
                    let next = *filter + (target - *filter) * rate * self.sample_time;
                    *filter = if next == *filter { target } else { next };
                } else {
                    *filter = target;
                }
                output.values[lane] = *filter * 10.0;
            }
        }
    }
    pub fn saved_state(&self) -> SavedModuleState {
        SavedModuleState::MidiCc {
            assignments: self.assignments,
            values: self.values[0],
        }
    }
    pub fn set_saved_state(&mut self, state: SavedModuleState) -> bool {
        let (assignments, values) = match state {
            SavedModuleState::MidiCc {
                assignments,
                values,
            } => (assignments, Some(values)),
            SavedModuleState::MidiCcMappings(assignments) => (assignments, None),
            _ => return false,
        };
        if assignments.iter().any(|v| !(-1..=127).contains(v))
            || values.is_some_and(|values| values.iter().any(|v| *v > 127))
        {
            return false;
        }
        self.cancel_configuration();
        self.assignments = normalize(assignments);
        if let Some(values) = values {
            self.values[0] = values;
        }
        true
    }
    /// The caller guarantees identity, kind and sample rate compatibility.
    /// Only graph value fields that actually changed override controller motion.
    /// Explicit full restoration uses set_saved_state after this transfer.
    pub(crate) fn adopt(&mut self, old: &Self) {
        let changed = self.configured_assignments != old.configured_assignments
            || self.smooth != old.smooth
            || self.mpe != old.mpe
            || self.high_resolution != old.high_resolution
            || self.channel != old.channel;
        self.values = old.values;
        for i in 0..128 {
            if self.configured_values[i] != old.configured_values[i] {
                self.values[0][i] = self.configured_values[i];
            }
        }
        self.pending = old.pending;
        self.filters = old.filters;
        if self.configured_assignments == old.configured_assignments {
            self.assignments = old.assignments;
        }
        if !changed {
            self.learning = old.learning;
            self.completion = old.completion;
        }
    }
    pub(crate) fn initialize_state(&mut self) {
        self.assignments = DEFAULT_ASSIGNMENTS;
        self.smooth = true;
        self.mpe = false;
        self.high_resolution = false;
        self.channel = 0;
        self.values = [[0; 128]; 16];
        self.pending = [[0; 32]; 16];
        self.cancel_configuration();
    }
    pub(crate) fn reset_state(&mut self) {
        self.values = [[0; 128]; 16];
        self.values[0] = self.configured_values;
        self.pending = [[0; 32]; 16];
        self.filters = [[0.0; 16]; 16];
        self.assignments = self.configured_assignments;
        self.cancel_configuration();
    }
}

pub(crate) fn catalog() -> Value {
    let mut params = serde_json::Map::new();
    for i in 1..=16 {
        params.insert(
            format!("cc{i}"),
            json!({"default":i,"min":-1,"max":127,"step":1}),
        );
    }
    for i in 0..128 {
        params.insert(
            format!("cc_value{i}"),
            json!({"default":0,"min":0,"max":127,"step":1,"hidden":true}),
        );
    }
    for (name, default) in [("smooth", 1), ("mpe", 0), ("high_resolution", 0)] {
        params.insert(
            name.into(),
            json!({"default":default,"min":0,"max":1,"step":1}),
        );
    }
    params.insert("channel".into(),json!({"default":0,"min":0,"max":16,"step":1,"description":"0 omni; 1–16 selects wire channel 0–15"}));
    json!({"kind":"midi_cc","title":"MIDI-CC","model":"functional","description":"Sixteen assignable MIDI CC outputs, with smoothing, MPE and paired 14-bit controllers. Hardware MIDI endpoint selection is global.","inputs":[],"params":params})
}
pub(crate) fn annotate(module: &mut Value) {
    module["polyphony"]["version_2"] = json!("connected outputs have one lane, or sixteen wire-channel lanes with MPE; disconnected outputs have zero lanes and freeze filters");
    module["polyphony"]["retired_voices"] = json!("inactive lanes retain filter histories");
    module["midi_channel_numbering"] =
        json!("events 0–15; channel parameter 0 omni or 1–16 selection");
    module["midi_endpoint_scope"] = json!("global");
}
