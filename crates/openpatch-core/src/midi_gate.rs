//! Original fixed-storage note-to-cell gates with sample-time minimum pulses.
use crate::{
    parameter, MidiEvent, MidiLearnCompletion, ModuleLightState, Node, PortFrame, SavedModuleState,
};
use serde_json::{json, Value};

pub const OUTPUTS: [&str; 16] = [
    "gate1", "gate2", "gate3", "gate4", "gate5", "gate6", "gate7", "gate8", "gate9", "gate10",
    "gate11", "gate12", "gate13", "gate14", "gate15", "gate16",
];
pub const DEFAULT_ASSIGNMENTS: [i16; 16] = [
    48, 49, 50, 51, 44, 45, 46, 47, 40, 41, 42, 43, 36, 37, 38, 39,
];

#[derive(Clone, Debug)]
pub struct MidiGate {
    assignments: [i16; 16],
    // A graph baseline distinguishes saved configuration from live Learn maps.
    configured_assignments: [i16; 16],
    gates: [[bool; 16]; 16],
    remaining: [[f32; 16]; 16],
    velocities: [[u8; 16]; 16],
    velocity: bool,
    mpe: bool,
    channel: u8,
    sample_time: f32,
    learning: Option<(u64, u8)>,
    completion: Option<MidiLearnCompletion>,
    lights: ModuleLightState,
}
fn normalize(mut assignments: [i16; 16]) -> [i16; 16] {
    for cell in 0..16 {
        if assignments[cell] >= 0 {
            for previous in 0..cell {
                if assignments[previous] == assignments[cell] {
                    assignments[previous] = -1;
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
pub fn initial_midi_gate_state(node: &Node) -> Result<SavedModuleState, String> {
    Ok(MidiGate::build(node, 48000)?.saved_state())
}
impl MidiGate {
    pub fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for key in node.params.keys() {
            let note = key
                .strip_prefix("note")
                .and_then(|s| s.parse::<usize>().ok())
                .is_some_and(|i| (1..=16).contains(&i) && *key == format!("note{i}"));
            if !note && !["velocity", "mpe", "channel"].contains(&key.as_str()) {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let mut assignments = DEFAULT_ASSIGNMENTS;
        for (i, note) in assignments.iter_mut().enumerate() {
            *note = integer(node, &format!("note{}", i + 1), *note as f32, -1.0, 127.0)?;
        }
        let assignments = normalize(assignments);
        Ok(Self {
            assignments,
            configured_assignments: assignments,
            gates: [[false; 16]; 16],
            remaining: [[0.0; 16]; 16],
            velocities: [[0; 16]; 16],
            velocity: integer(node, "velocity", 0.0, 0.0, 1.0)? != 0,
            mpe: integer(node, "mpe", 0.0, 0.0, 1.0)? != 0,
            channel: integer(node, "channel", 0.0, 0.0, 16.0)? as u8,
            sample_time: 1.0 / rate as f32,
            learning: None,
            completion: None,
            lights: ModuleLightState {
                count: 16,
                ..ModuleLightState::default()
            },
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
    pub fn take_learn_completion(&mut self) -> Option<MidiLearnCompletion> {
        self.completion.take()
    }
    /// Source gate-clearing behavior: velocities and Learn remain intact.
    pub fn panic(&mut self) {
        self.gates = [[false; 16]; 16];
        self.remaining = [[0.0; 16]; 16];
    }
    /// Host safety policy also cancels armed requests; a completion stays owned.
    pub fn host_panic(&mut self) {
        self.panic();
        self.learning = None;
        self.lights.brightness = [[0.0; 2]; crate::MAX_MODULE_LIGHTS];
    }
    pub fn handle_midi(&mut self, event: MidiEvent) {
        if !event.is_valid() {
            return;
        }
        if event == MidiEvent::Panic {
            self.host_panic();
            return;
        }
        let (channel, note, velocity) = match event {
            MidiEvent::NoteOn {
                channel,
                note,
                velocity,
            } => (channel, note, Some(velocity)),
            MidiEvent::NoteOff { channel, note, .. } => (channel, note, None),
            _ => return,
        };
        if self.channel != 0 && self.channel != channel + 1 {
            return;
        }
        let lane = if self.mpe { channel as usize } else { 0 };
        let positive = velocity.is_some_and(|v| v > 0);
        if positive {
            if let Some((serial, cell)) = self.learning.take() {
                let before = self.assignments;
                for assignment in &mut self.assignments {
                    if *assignment == note as i16 {
                        *assignment = -1;
                    }
                }
                self.assignments[cell as usize] = note as i16;
                self.completion = Some(MidiLearnCompletion {
                    serial,
                    cell,
                    before,
                    after: self.assignments,
                });
            }
        }
        for cell in 0..16 {
            if self.assignments[cell] != note as i16 {
                continue;
            }
            self.gates[cell][lane] = positive;
            if positive {
                self.remaining[cell][lane] = 0.001;
                self.velocities[cell][lane] = velocity.unwrap();
            }
        }
    }
    /// Every cell publishes and advances independently of cable connectivity.
    pub fn process(&mut self, outputs: &mut [PortFrame]) {
        let width = if self.mpe { 16 } else { 1 };
        for (cell, output) in outputs.iter_mut().take(16).enumerate() {
            *output = PortFrame {
                channels: width as u8,
                ..PortFrame::default()
            };
            let mut peak = 0.0_f32;
            for lane in 0..width {
                if self.gates[cell][lane] || self.remaining[cell][lane] > 0.0 {
                    let amplitude = if self.velocity {
                        self.velocities[cell][lane] as f32 / 127.0
                    } else {
                        1.0
                    };
                    output.values[lane] = amplitude * 10.0;
                    self.remaining[cell][lane] -= self.sample_time;
                    peak = peak.max(output.values[lane]);
                }
            }
            self.lights.brightness[cell] = [peak / 10.0, 0.0];
        }
    }
    pub fn saved_state(&self) -> SavedModuleState {
        SavedModuleState::MidiGateMappings(self.assignments)
    }
    pub fn set_saved_state(&mut self, state: SavedModuleState) -> bool {
        let SavedModuleState::MidiGateMappings(assignments) = state else {
            return false;
        };
        if assignments.iter().any(|n| !(-1..=127).contains(n)) {
            return false;
        }
        self.assignments = normalize(assignments);
        self.learning = None;
        true
    }
    pub fn lights(&self) -> ModuleLightState {
        self.lights
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        let maps_changed = self.configured_assignments != old.configured_assignments;
        let config_changed = maps_changed
            || self.velocity != old.velocity
            || self.mpe != old.mpe
            || self.channel != old.channel;
        self.gates = old.gates;
        self.remaining = old.remaining;
        self.velocities = old.velocities;
        self.lights = old.lights;
        if !maps_changed {
            self.assignments = old.assignments;
        }
        if !config_changed {
            self.learning = old.learning;
        }
        self.completion = old.completion;
    }
    pub(crate) fn initialize_state(&mut self) {
        self.assignments = DEFAULT_ASSIGNMENTS;
        self.velocity = false;
        self.mpe = false;
        self.channel = 0;
        self.host_panic();
    }
    pub(crate) fn reset_state(&mut self) {
        // Reset transient signal state without rewinding saved mapping edits.
        self.host_panic();
        self.velocities = [[0; 16]; 16];
    }
}
pub(crate) fn catalog() -> Value {
    let mut params = serde_json::Map::new();
    for (i, note) in DEFAULT_ASSIGNMENTS.iter().enumerate() {
        params.insert(
            format!("note{}", i + 1),
            json!({"default":note,"min":-1,"max":127,"step":1}),
        );
    }
    for name in ["velocity", "mpe"] {
        params.insert(name.into(), json!({"default":0,"min":0,"max":1,"step":1}));
    }
    params.insert("channel".into(),json!({"default":0,"min":0,"max":16,"step":1,"description":"0 omni; 1–16 selects wire channel 0–15"}));
    json!({"kind":"midi_gate","title":"MIDI → Gate","model":"functional","description":"Sixteen assignable note gates with minimum 1 ms pulses, optional velocity amplitude and MPE lanes. Cell signal histories survive remapping; Panic clears held gates. Hardware MIDI endpoint selection is global.","inputs":[],"params":params})
}
pub(crate) fn annotate(module: &mut Value) {
    module["runtime_actions"] = json!(["midi_panic"]);
    module["polyphony"]["version_2"]=json!("all sixteen outputs always publish one lane, or sixteen wire-channel lanes in MPE, including disconnected or unmapped cells");
    module["polyphony"]["retired_voices"] =
        json!("inactive MPE lanes freeze cell gates, timers and velocity");
    module["midi_endpoint_scope"] = json!("global");
    module["midi_channel_numbering"] =
        json!("events 0–15; channel parameter 0 omni or 1–16 selection");
    module["light_names"] = json!((1..=16).map(|i| format!("Gate {i}")).collect::<Vec<_>>());
    module["light_policy"] = json!({"source":"maximum published lane voltage per cell / 10 V","publication_frames":1,"colors":["green"],"count":16});
}
