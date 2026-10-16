//! Original state-machine realization of the pinned stock SEQ 3 contract.
//! Fundamental 10dd0160 defines the observable timing, mathematical constants,
//! and defaults; no upstream implementation or artwork is incorporated here.
//! Physical held buttons are distinct from coalesced host convenience events.
use crate::{
    parameter, ModuleAction, ModuleControlState, ModuleLightState, ModuleTelemetry, Node,
    PortFrame, SavedModuleState, MAX_MODULE_ACTIONS_PER_SAMPLE,
};
use serde_json::{json, Value};
pub(crate) const INPUTS: [&str; 5] = ["tempo", "clock", "run", "reset", "steps"];
pub(crate) const OUTPUTS: [&str; 16] = [
    "cv1", "cv2", "cv3", "trigger", "step1", "step2", "step3", "step4", "step5", "step6", "step7",
    "step8", "steps", "clock", "run", "reset",
];
const BUTTONS: u16 = (1 << 11) - 1;

pub(crate) fn catalog() -> Value {
    let mut result = json!({"kind":"stock_seq3","title":"STOCK SEQ 3","model":"functional","description":"Stock-style eight-step three-row sequencer with source-ordered f32 clock/reset timing, physical held buttons, durable gates/run/passthrough and actual lamps. Legacy seq3 remains independent.","inputs":INPUTS,"params":{
        "tempo":{"default":1.0,"min":-2.0,"max":4.0,"unit":"log2(Hz)","description":"60 × 2^tempo BPM; signed tempo CV adds octaves"},
        "tempo_amount":{"default":1.0,"min":0.0,"max":1.0},"steps":{"default":8.0,"min":1.0,"max":8.0,"step":1.0},"steps_amount":{"default":1.0,"min":0.0,"max":1.0},
        "running":{"default":1.0,"min":0.0,"max":1.0,"step":1.0,"hidden":true,"description":"Persistent current Run snapshot; live state is authoritative"},
        "passthrough":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"description":"Persistent clock passthrough setting"}
    }});
    let params = result["params"].as_object_mut().unwrap();
    for row in 1..=3 {
        for step in 1..=8 {
            params.insert(
                format!("row{row}_{step}"),
                json!({"default":0.0,"min":-10.0,"max":10.0,"unit":"V"}),
            );
        }
    }
    for step in 1..=8 {
        params.insert(format!("gate{step}"),json!({"default":1.0,"min":0.0,"max":1.0,"step":1.0,"hidden":true,"description":"Persistent gate-enable snapshot; physical button toggles the runtime flag"}));
    }
    result
}
pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"]=json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/SEQ3.cpp");
    module["runtime_actions"] = json!([
        "clock_press",
        "clock_release",
        "run_press",
        "run_release",
        "reset_press",
        "reset_release",
        "seq_gate1_press",
        "seq_gate1_release",
        "seq_gate2_press",
        "seq_gate2_release",
        "seq_gate3_press",
        "seq_gate3_release",
        "seq_gate4_press",
        "seq_gate4_release",
        "seq_gate5_press",
        "seq_gate5_release",
        "seq_gate6_press",
        "seq_gate6_release",
        "seq_gate7_press",
        "seq_gate7_release",
        "seq_gate8_press",
        "seq_gate8_release",
        "clock",
        "run",
        "reset"
    ]);
    module["polyphony"]["version_2"] = json!(
        "all16 outputs mono; inputs read channel zero only; no mono broadcast or polyphonic clocks"
    );
    module["polyphony"]["primary_input"] = Value::Null;
    for output in module["output_metadata"].as_array_mut().unwrap() {
        output["maximum_channels"] = json!(1);
    }
    module["tempo_policy"]=json!("f32 source-ordered fifth-order exp2 of raw tempo + CV × amount; internal phase freezes when stopped or external clock connected; reset guard defers internal wrap, rather than consuming it; source-domain finite arithmetic preserved, nonfinite CV becomes0 and outside-domain phase overflow safely saturates");
    module["trigger_policy"]=json!("one shared clock pulse on final selected-index change, including Reset from later steps; length-one clocks do not pulse; external edges respect reset pulse, manual Clock bypasses it; passthrough freezes clock pulse remaining and uses source detector/internal phase gate; no automatic wrap RESET");
    module["light_policy"]=json!("19 logical groups every frame: clock/run/reset,8 gate-enable groups,8 yellow selected-step/red out-of-length groups; smooth30/s release for clock/reset/selected-step");
    module["manual_policy"]=json!("held mask bits clock0,run1,reset2,gates3..10; physical Boolean detectors initially unknown, initial high arms without firing; clock button detector freezes stopped; Clock/Run/Reset convenience events coalesce once per action per sample and bypass button startup as an explicit host extension");
    module["initialization_policy"]=json!("restore eligible controls separately; source reset selects step1, enables all gates and disables passthrough; preserve running, phase, pulses, detectors, held levels and lamps");
    module["state_adoption"]=json!("same identity/kind/rate preserves full runtime including durable flags and held controls; receiving numerical controls win; routes do not rearm detectors; explicit full saved-state loads apply after adoption; queues are not copied");
}
fn boolean(node: &Node, name: &str, default: f32) -> Result<bool, String> {
    let value = parameter(node, name, default, 0.0, 1.0)?;
    if value.fract() != 0.0 {
        return Err(format!("node '{}': '{name}' must be an integer", node.id));
    }
    Ok(value != 0.0)
}
/// Validate and decode persistent snapshots without constructing a DSP processor.
pub fn initial_stock_seq3_state(node: &Node) -> Result<SavedModuleState, String> {
    let mut gates = 0;
    for i in 0..8 {
        if boolean(node, &format!("gate{}", i + 1), 1.0)? {
            gates |= 1 << i;
        }
    }
    Ok(SavedModuleState::StockSeq3 {
        running: boolean(node, "running", 1.0)?,
        gates,
        passthrough: boolean(node, "passthrough", 0.0)?,
    })
}
fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}
#[allow(clippy::excessive_precision)]
fn exponential(pitch: f32) -> f32 {
    let biased = pitch.clamp(-127.0, 128.0) + 127.0;
    let integer = biased as u32;
    let fraction = biased - integer as f32;
    let mut value = 0.001879100722_f32;
    for coefficient in [
        0.008991698010,
        0.055817908652,
        0.2401595990753,
        0.69315169353961,
        1.0,
    ] {
        value = value * fraction + coefficient;
    }
    f32::from_bits(integer << 23) * value
}
fn button(state: &mut u8, held: bool) -> bool {
    let rising = *state == 0 && held;
    *state = u8::from(held);
    rising
}
fn schmitt(state: &mut u8, value: f32) -> bool {
    let previous = *state;
    if value >= 2.0 {
        *state = 1;
    } else if value <= 0.1 {
        *state = 0;
    }
    previous == 0 && *state == 1
}
fn trigger(remaining: &mut f32) {
    if 0.001 > *remaining {
        *remaining = 0.001;
    }
}
fn pulse(remaining: &mut f32, time: f32) -> bool {
    let high = *remaining > 0.0;
    if high {
        *remaining -= time;
    }
    high
}
fn smooth(old: &mut f32, target: f32, time: f32) {
    if target < *old {
        *old += (target - *old) * 30.0 * time;
    } else {
        *old = target;
    }
}

/// Exact read-only observable production runtime, in pinned source order.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct StockSeq3Snapshot {
    pub phase: f32,
    pub index: u8,
    pub length: u8,
    pub running: bool,
    pub gates: [bool; 8],
    pub passthrough: bool,
    pub button_triggers: [u8; 11],
    pub input_triggers: [u8; 3],
    pub pulse_remaining: [f32; 3],
    pub lights: [f32; 27],
    pub pressed_mask: u16,
}
impl Default for StockSeq3Snapshot {
    fn default() -> Self {
        Self {
            phase: 0.0,
            index: 0,
            length: 8,
            running: true,
            gates: [true; 8],
            passthrough: false,
            button_triggers: [2; 11],
            input_triggers: [2; 3],
            pulse_remaining: [0.0; 3],
            lights: [0.0; 27],
            pressed_mask: 0,
        }
    }
}
#[derive(Debug)]
pub(crate) struct StockSeq3 {
    sample_time: f32,
    tempo: f32,
    tempo_amount: f32,
    steps: f32,
    steps_amount: f32,
    rows: [[f32; 8]; 3],
    initial: SavedModuleState,
    state: StockSeq3Snapshot,
    actions: [ModuleAction; MAX_MODULE_ACTIONS_PER_SAMPLE],
    action_count: usize,
}
impl StockSeq3 {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&rate) {
            return Err("stock_seq3 sample rate must be8000..=192000 Hz".into());
        }
        for key in node.params.keys() {
            let known = [
                "tempo",
                "tempo_amount",
                "steps",
                "steps_amount",
                "running",
                "passthrough",
            ]
            .contains(&key.as_str())
                || (1..=8).any(|s| {
                    key == &format!("gate{s}") || (1..=3).any(|r| key == &format!("row{r}_{s}"))
                });
            if !known {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let steps = parameter(node, "steps", 8.0, 1.0, 8.0)?;
        if steps.fract() != 0.0 {
            return Err("stock_seq3 steps must be an integer".into());
        }
        let mut rows = [[0.0; 8]; 3];
        for (r, row) in rows.iter_mut().enumerate() {
            for (s, value) in row.iter_mut().enumerate() {
                *value = parameter(node, &format!("row{}_{}", r + 1, s + 1), 0.0, -10.0, 10.0)?;
            }
        }
        let initial = initial_stock_seq3_state(node)?;
        let mut model = Self {
            sample_time: 1.0 / rate as f32,
            tempo: parameter(node, "tempo", 1.0, -2.0, 4.0)?,
            tempo_amount: parameter(node, "tempo_amount", 1.0, 0.0, 1.0)?,
            steps,
            steps_amount: parameter(node, "steps_amount", 1.0, 0.0, 1.0)?,
            rows,
            initial,
            state: StockSeq3Snapshot::default(),
            actions: [ModuleAction::Clock; MAX_MODULE_ACTIONS_PER_SAMPLE],
            action_count: 0,
        };
        model.set_saved_state(initial);
        model.state.length = steps as u8;
        Ok(model)
    }
    pub(crate) fn saved_state(&self) -> SavedModuleState {
        let gates = self
            .state
            .gates
            .iter()
            .enumerate()
            .fold(0, |bits, (i, gate)| bits | (u8::from(*gate) << i));
        SavedModuleState::StockSeq3 {
            running: self.state.running,
            gates,
            passthrough: self.state.passthrough,
        }
    }
    pub(crate) fn set_saved_state(&mut self, saved: SavedModuleState) -> bool {
        let (running, gates, gate_mask, passthrough) = match saved {
            SavedModuleState::StockSeq3 {
                running,
                gates,
                passthrough,
            } => (Some(running), gates, 0xff, Some(passthrough)),
            SavedModuleState::StockSeq3Fields {
                running,
                gates,
                gate_mask,
                passthrough,
            } => (running, gates, gate_mask, passthrough),
            _ => return false,
        };
        if let Some(value) = running {
            self.state.running = value;
        }
        if let Some(value) = passthrough {
            self.state.passthrough = value;
        }
        for (i, gate) in self.state.gates.iter_mut().enumerate() {
            if gate_mask & (1 << i) != 0 {
                *gate = gates & (1 << i) != 0;
            }
        }
        true
    }
    pub(crate) fn set_running(&mut self, running: bool) {
        self.state.running = running;
    }
    pub(crate) fn initialize_state(&mut self) {
        self.state.index = 0;
        self.state.gates = [true; 8];
        self.state.passthrough = false;
    }
    pub(crate) fn reset_state(&mut self) {
        self.state = StockSeq3Snapshot::default();
        self.state.length = self.steps as u8;
        self.set_saved_state(self.initial);
        self.action_count = 0;
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.sample_time == old.sample_time {
            self.state = old.state;
        }
    }
    fn rotate(&mut self, delta: i32) {
        let offset = delta.rem_euclid(8) as usize;
        for row in &mut self.rows {
            row.rotate_right(offset);
        }
        self.state.gates.rotate_right(offset);
    }
    pub(crate) fn discard_actions(&mut self) {
        self.action_count = 0;
    }
    pub(crate) fn supports(action: ModuleAction) -> bool {
        matches!(
            action,
            ModuleAction::Clock
                | ModuleAction::Run
                | ModuleAction::Reset
                | ModuleAction::ClockPress
                | ModuleAction::ClockRelease
                | ModuleAction::RunPress
                | ModuleAction::RunRelease
                | ModuleAction::ResetPress
                | ModuleAction::ResetRelease
        ) || action.seq_gate_button().is_some()
    }
    pub(crate) fn enqueue(&mut self, action: ModuleAction) -> bool {
        if !Self::supports(action) || self.action_count == self.actions.len() {
            return false;
        }
        self.actions[self.action_count] = action;
        self.action_count += 1;
        true
    }
    pub(crate) fn release(&mut self) {
        self.state.pressed_mask = 0;
        self.action_count = 0;
    }
    pub(crate) fn release_action(&mut self, action: ModuleAction) -> bool {
        let bit = match action {
            ModuleAction::ClockRelease => Some(0),
            ModuleAction::RunRelease => Some(1),
            ModuleAction::ResetRelease => Some(2),
            _ => action
                .seq_gate_button()
                .and_then(|(i, press)| (!press).then_some(i + 3)),
        };
        if let Some(bit) = bit {
            self.state.pressed_mask &= !(1 << bit);
            true
        } else {
            false
        }
    }
    pub(crate) fn controls(&self) -> ModuleControlState {
        ModuleControlState {
            pressed: self.state.pressed_mask != 0,
            pressed_mask: self.state.pressed_mask,
        }
    }
    pub(crate) fn telemetry(&self) -> ModuleTelemetry {
        ModuleTelemetry {
            step: self.state.index + 1,
            length: self.state.length,
            running: self.state.running,
        }
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut lights = ModuleLightState {
            count: 19,
            ..Default::default()
        };
        for i in 0..11 {
            lights.brightness[i][0] = self.state.lights[i];
        }
        for i in 0..8 {
            lights.brightness[11 + i] =
                [self.state.lights[11 + 2 * i], self.state.lights[12 + 2 * i]];
        }
        lights
    }
    fn actions(&mut self) -> u8 {
        let mut convenience = 0;
        for i in 0..self.action_count {
            match self.actions[i] {
                ModuleAction::Clock => convenience |= 1,
                ModuleAction::Run => convenience |= 2,
                ModuleAction::Reset => convenience |= 4,
                ModuleAction::ClockPress => self.state.pressed_mask |= 1,
                ModuleAction::ClockRelease => self.state.pressed_mask &= !1,
                ModuleAction::RunPress => self.state.pressed_mask |= 2,
                ModuleAction::RunRelease => self.state.pressed_mask &= !2,
                ModuleAction::ResetPress => self.state.pressed_mask |= 4,
                ModuleAction::ResetRelease => self.state.pressed_mask &= !4,
                action => {
                    if let Some((button, press)) = action.seq_gate_button() {
                        if press {
                            self.state.pressed_mask |= 1 << (button + 3);
                        } else {
                            self.state.pressed_mask &= !(1 << (button + 3));
                        }
                    }
                }
            }
        }
        self.action_count = 0;
        convenience
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame; 5], outputs: &mut [PortFrame]) {
        let values = inputs.map(|port| finite(port.channel(0)));
        let one_shot = self.actions();
        let state = &mut self.state;
        let held = state.pressed_mask;
        let run_button = button(&mut state.button_triggers[1], held & 2 != 0);
        let run_input = schmitt(&mut state.input_triggers[1], values[2]);
        if run_button || run_input || one_shot & 2 != 0 {
            state.running = !state.running;
            trigger(&mut state.pulse_remaining[0]);
        }
        let run_gate = pulse(&mut state.pulse_remaining[0], self.sample_time);
        let old = state.index;
        let reset_button = button(&mut state.button_triggers[2], held & 4 != 0);
        let reset_input = schmitt(&mut state.input_triggers[2], values[3]);
        if reset_button || reset_input || one_shot & 4 != 0 {
            trigger(&mut state.pulse_remaining[2]);
            state.index = 0;
            state.phase = 0.0;
        }
        let reset_gate = pulse(&mut state.pulse_remaining[2], self.sample_time);
        let mut clock = false;
        let mut clock_gate = false;
        if state.running {
            let manual = button(&mut state.button_triggers[0], held & 1 != 0) || one_shot & 1 != 0;
            if inputs[1].channels != 0 {
                let rising = schmitt(&mut state.input_triggers[0], values[1]);
                clock = (rising && !reset_gate) || manual;
                clock_gate = state.input_triggers[0] == 1 || state.button_triggers[0] == 1;
                // One-shot Clock is an explicit host tap, with a one-frame gate.
                clock_gate |= one_shot & 1 != 0;
            } else {
                let pitch = self.tempo + values[0] * self.tempo_amount;
                let next = state.phase + exponential(pitch) * self.sample_time;
                state.phase = if next.is_finite() { next } else { f32::MAX };
                if state.phase >= 1.0 && !reset_gate {
                    clock = true;
                    state.phase -= state.phase.trunc();
                }
                if manual {
                    clock = true;
                    state.phase = 0.0;
                }
                clock_gate = state.phase < 0.5;
            }
        }
        let steps = self.steps + values[4] * self.steps_amount;
        state.length = steps.round().clamp(1.0, 8.0) as u8;
        if clock {
            state.index += 1;
            if state.index >= state.length {
                state.index = 0;
            }
        }
        if state.index != old {
            trigger(&mut state.pulse_remaining[1]);
        }
        if !state.passthrough {
            clock_gate = pulse(&mut state.pulse_remaining[1], self.sample_time);
        }
        for i in 0..8 {
            if button(
                &mut state.button_triggers[i + 3],
                held & (1 << (i + 3)) != 0,
            ) {
                state.gates[i] = !state.gates[i];
            }
            state.lights[3 + i] = f32::from(u8::from(state.gates[i]));
            smooth(
                &mut state.lights[11 + 2 * i],
                f32::from(u8::from(usize::from(state.index) == i)),
                self.sample_time,
            );
            state.lights[12 + 2 * i] = f32::from(u8::from(i >= usize::from(state.length)));
        }
        for (i, output) in outputs.iter_mut().enumerate() {
            *output = PortFrame::mono(match i {
                0..=2 => self.rows[i][usize::from(state.index)],
                3 => {
                    if clock_gate && state.gates[usize::from(state.index)] {
                        10.0
                    } else {
                        0.0
                    }
                }
                4..=11 => {
                    if usize::from(state.index) == i - 4 {
                        10.0
                    } else {
                        0.0
                    }
                }
                12 => f32::from(state.length - 1),
                13 => {
                    if clock_gate {
                        10.0
                    } else {
                        0.0
                    }
                }
                14 => {
                    if run_gate {
                        10.0
                    } else {
                        0.0
                    }
                }
                15 => {
                    if reset_gate {
                        10.0
                    } else {
                        0.0
                    }
                }
                _ => unreachable!(),
            });
        }
        smooth(
            &mut state.lights[0],
            f32::from(u8::from(clock_gate)),
            self.sample_time,
        );
        state.lights[1] = f32::from(u8::from(state.running));
        smooth(
            &mut state.lights[2],
            f32::from(u8::from(reset_gate)),
            self.sample_time,
        );
    }
}
/// Reference access to the actual production state machine, without graph delay.
#[doc(hidden)]
#[derive(Debug)]
pub struct StockSeq3Probe(StockSeq3);
impl StockSeq3Probe {
    pub fn new(node: &Node, rate: u32) -> Result<Self, String> {
        StockSeq3::build(node, rate).map(Self)
    }
    pub fn step(&mut self, inputs: &[PortFrame; 5], pressed_mask: u16) -> [PortFrame; 16] {
        self.0.state.pressed_mask = pressed_mask & BUTTONS;
        let mut outputs = [PortFrame::default(); 16];
        self.0.process(inputs, &mut outputs);
        outputs
    }
    pub fn snapshot(&self) -> StockSeq3Snapshot {
        self.0.state
    }
    pub fn initialize(&mut self) {
        self.0.initialize_state();
    }
    pub fn adopt(&mut self, old: &Self) {
        self.0.adopt(&old.0);
    }
    pub fn set_saved_state(&mut self, state: SavedModuleState) -> bool {
        self.0.set_saved_state(state)
    }
    pub fn enqueue(&mut self, action: ModuleAction) -> bool {
        self.0.enqueue(action)
    }
    pub fn rotate(&mut self, delta: i32) {
        self.0.rotate(delta);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(params: Value) -> StockSeq3 {
        StockSeq3::build(
            &serde_json::from_value(json!({"id":"s","kind":"stock_seq3","params":params})).unwrap(),
            8000,
        )
        .unwrap()
    }
    fn step(model: &mut StockSeq3, inputs: [PortFrame; 5]) -> [PortFrame; 16] {
        let mut out = [PortFrame::default(); 16];
        model.process(&inputs, &mut out);
        out
    }
    #[test]
    fn unknown_physical_detectors_suppress_startup_high_and_later_rearm() {
        let mut m = model(json!({}));
        m.state.pressed_mask = BUTTONS;
        let mut input = [PortFrame::mono(3.0); 5];
        input[1] = PortFrame::mono(0.0);
        let out = step(&mut m, input);
        assert!(m.state.running);
        assert_eq!(m.state.index, 0);
        assert_eq!(m.state.gates, [true; 8]);
        assert_eq!(out[14].values[0], 0.0);
        assert_eq!(m.state.button_triggers, [1; 11]);
        m.state.pressed_mask = 0;
        input[2] = PortFrame::mono(0.0);
        input[3] = PortFrame::mono(0.0);
        step(&mut m, input);
        m.state.pressed_mask = (1 << 3) | 1;
        step(&mut m, input);
        assert_eq!(m.state.index, 1);
        assert!(!m.state.gates[0]);
    }
    #[test]
    fn paused_clock_histories_and_physical_press_are_deferred_until_resume() {
        let mut m = model(json!({}));
        let mut input = [PortFrame::default(); 5];
        input[1] = PortFrame::mono(0.0);
        step(&mut m, input);
        m.set_running(false);
        m.state.pressed_mask = 1;
        input[1].values[0] = 3.0;
        step(&mut m, input);
        assert_eq!(m.state.button_triggers[0], 0);
        assert_eq!(m.state.input_triggers[0], 0);
        assert_eq!(m.state.index, 0);
        m.enqueue(ModuleAction::Run);
        step(&mut m, input);
        assert_eq!(m.state.index, 1, "CV and button clocks coalesce at resume");
        assert_eq!(m.state.button_triggers[0], 1);
        assert_eq!(m.state.input_triggers[0], 1);
    }
    #[test]
    fn reset_index_change_pulses_clock_and_manual_clock_bypasses_reset_guard() {
        let mut m = model(json!({}));
        m.state.index = 4;
        let mut input = [PortFrame::default(); 5];
        input[1] = PortFrame::mono(0.0);
        m.enqueue(ModuleAction::Reset);
        let out = step(&mut m, input);
        assert_eq!(m.state.index, 0);
        assert_eq!(out[13].values[0], 10.0);
        assert_eq!(out[15].values[0], 10.0);
        m.enqueue(ModuleAction::Clock);
        step(&mut m, input);
        assert_eq!(m.state.index, 1, "manual Clock ignores a live reset pulse");
        input[1].values[0] = 3.0;
        step(&mut m, input);
        assert_eq!(
            m.state.index, 1,
            "external Clock is guarded but its detector still tracks"
        );
        assert_eq!(m.state.input_triggers[0], 1);
    }
    #[test]
    fn length_one_clock_and_simultaneous_reset_clock_use_final_index_change() {
        let mut m = model(json!({"steps":1}));
        m.enqueue(ModuleAction::Clock);
        let out = step(&mut m, [PortFrame::default(); 5]);
        assert_eq!(m.state.index, 0);
        assert_eq!(out[13].values[0], 0.0);
        assert_eq!(out[3].values[0], 0.0);
        let mut m = model(json!({}));
        m.state.index = 1;
        m.enqueue(ModuleAction::Reset);
        m.enqueue(ModuleAction::Clock);
        let out = step(&mut m, [PortFrame::default(); 5]);
        assert_eq!(m.state.index, 1);
        assert_eq!(
            out[13].values[0], 0.0,
            "reset+clock can return to the same index"
        );
        assert_eq!(out[15].values[0], 10.0);
    }
    #[test]
    fn guarded_internal_wrap_is_deferred_and_external_clock_keeps_phase() {
        let mut m = model(json!({}));
        m.state.phase = 0.99;
        m.state.pulse_remaining[2] = 0.00025;
        let mut input = [PortFrame::default(); 5];
        input[0] = PortFrame::mono(10.0);
        step(&mut m, input);
        assert!(m.state.phase > 1.0);
        assert_eq!(m.state.index, 0);
        step(&mut m, input);
        assert!(m.state.phase > 1.0);
        step(&mut m, input);
        assert_eq!(m.state.index, 1);
        assert!((0.0..1.0).contains(&m.state.phase));
        let phase = m.state.phase;
        input[1] = PortFrame::mono(0.0);
        step(&mut m, input);
        input[1].values[0] = 3.0;
        step(&mut m, input);
        assert_eq!(m.state.phase, phase);
    }
    #[test]
    fn passthrough_freezes_pending_clock_pulse_even_stopped_and_resumes_it() {
        let mut m = model(json!({"running":0,"passthrough":1}));
        m.state.pulse_remaining[1] = 0.0005;
        let before = m.state.pulse_remaining[1];
        for _ in 0..16 {
            assert_eq!(step(&mut m, [PortFrame::default(); 5])[13].values[0], 0.0);
        }
        assert_eq!(m.state.pulse_remaining[1], before);
        m.state.passthrough = false;
        let out = step(&mut m, [PortFrame::default(); 5]);
        assert_eq!(out[13].values[0], 10.0);
        assert!(m.state.pulse_remaining[1] < before);
    }
    #[test]
    fn initialize_and_adoption_preserve_all_other_histories_and_receiving_rows() {
        let mut old = model(json!({"row1_1":1}));
        old.state.phase = 0.7;
        old.state.index = 5;
        old.state.running = false;
        old.state.gates = [false; 8];
        old.state.passthrough = true;
        old.state.pulse_remaining = [0.0004, 0.0005, 0.0006];
        old.state.pressed_mask = 19;
        old.state.lights[4] = 0.3;
        let before = old.state;
        let mut receiving = model(json!({"row1_1":7,"running":1}));
        receiving.adopt(&old);
        assert_eq!(receiving.state, before);
        receiving.initialize_state();
        let after = receiving.state;
        assert_eq!(after.index, 0);
        assert_eq!(after.gates, [true; 8]);
        assert!(!after.passthrough);
        assert!(!after.running);
        assert_eq!(after.phase, before.phase);
        assert_eq!(after.button_triggers, before.button_triggers);
        assert_eq!(after.input_triggers, before.input_triggers);
        assert_eq!(after.pulse_remaining, before.pulse_remaining);
        assert_eq!(after.lights, before.lights);
        assert_eq!(after.pressed_mask, before.pressed_mask);
        assert_eq!(
            step(&mut receiving, [PortFrame::default(); 5])[0].values[0],
            7.0
        );
    }
    #[test]
    fn durable_flags_rotation_and_nineteen_light_groups_are_actual_state() {
        let mut m = model(json!({"steps":2,"row1_1":1,"row1_8":8}));
        m.set_saved_state(SavedModuleState::StockSeq3 {
            running: false,
            gates: 1,
            passthrough: true,
        });
        m.rotate(1);
        assert_eq!(m.rows[0][0], 8.0);
        assert_eq!(
            m.saved_state(),
            SavedModuleState::StockSeq3 {
                running: false,
                gates: 2,
                passthrough: true
            }
        );
        step(&mut m, [PortFrame::default(); 5]);
        let lights = m.lights();
        assert_eq!(lights.count, 19);
        assert_eq!(lights.brightness[4][0], 1.0);
        assert_eq!(lights.brightness[11], [1.0, 0.0]);
        assert_eq!(lights.brightness[13][1], 1.0);
        assert_eq!(lights.brightness[18][1], 1.0);
        let mut params = std::collections::BTreeMap::new();
        m.saved_state().write_params(&mut params);
        assert_eq!(params["gate2"], 1.0);
        assert_eq!(params["gate1"], 0.0);
        assert_eq!(params["passthrough"], 1.0);
    }
    #[test]
    fn convenience_actions_coalesce_and_queue_and_wire_bounds_are_strict() {
        let mut m = model(json!({}));
        for _ in 0..MAX_MODULE_ACTIONS_PER_SAMPLE {
            assert!(m.enqueue(ModuleAction::Clock));
        }
        assert!(!m.enqueue(ModuleAction::Clock));
        step(&mut m, [PortFrame::default(); 5]);
        assert_eq!(m.state.index, 1);
        assert!(!m.enqueue(ModuleAction::SeqGatePress(0)));
        assert!(!m.enqueue(ModuleAction::SeqGateRelease(9)));
        for action in [
            ModuleAction::ClockPress,
            ModuleAction::ClockRelease,
            ModuleAction::RunPress,
            ModuleAction::RunRelease,
            ModuleAction::SeqGatePress(1),
            ModuleAction::SeqGateRelease(8),
        ] {
            let wire = serde_json::to_value(action).unwrap();
            assert_eq!(
                serde_json::from_value::<ModuleAction>(wire).unwrap(),
                action
            );
        }
    }
    #[test]
    fn finite_extreme_and_nonfinite_cv_never_poison_runtime() {
        let mut m = model(json!({"passthrough":1}));
        for value in [
            f32::MAX,
            -f32::MAX,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            -127.0,
            128.0,
        ] {
            let input = [PortFrame::mono(value); 5];
            for _ in 0..20 {
                let out = step(&mut m, input);
                assert!(out
                    .iter()
                    .all(|o| o.channels == 1 && o.values.iter().all(|v| v.is_finite())));
                assert!(m.state.phase.is_finite());
                assert!(m
                    .state
                    .pulse_remaining
                    .iter()
                    .chain(m.state.lights.iter())
                    .all(|v| v.is_finite()));
            }
        }
    }
}
