//! Original functional eight-step, three-row sequencer and typed runtime actions.
//! These explicit timing policies do not claim sample-identical Rack behavior.
use crate::{parameter, Node, PortFrame};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const MAX_MODULE_EVENTS: usize = 65_536;
pub const MAX_MODULE_ACTIONS_PER_SAMPLE: usize = 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleAction {
    Clock,
    Run,
    Reset,
}
impl<'de> Deserialize<'de> for ModuleAction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Unit-enum derives also accept externally tagged objects. Our wire
        // contract is deliberately a string enum, including inside trace events.
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "clock" => Ok(Self::Clock),
            "run" => Ok(Self::Run),
            "reset" => Ok(Self::Reset),
            _ => Err(serde::de::Error::unknown_variant(
                &value,
                &["clock", "run", "reset"],
            )),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimedModuleEvent {
    pub frame: u64,
    pub id: String,
    pub action: ModuleAction,
}
/// An opaque target resolved for exactly one compiled Engine. Re-resolve after
/// graph replacement; host identity tokens may map separately to both engines
/// during a crossfade. Handles are Copy and callback-safe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleHandle {
    pub(crate) generation: u64,
    pub(crate) node: usize,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModuleTelemetry {
    /// One-based selected step, including a held step above a shrunken length.
    pub step: u8,
    pub running: bool,
    pub length: u8,
}
/// Validate ordering, bounds, IDs, and the per-node per-sample queue budget.
/// Resolve every ID against the compiled engine before scheduling a trace.
pub fn validate_module_events(
    events: &[TimedModuleEvent],
    total_frames: usize,
) -> Result<(), String> {
    if events.len() > MAX_MODULE_EVENTS {
        return Err(format!("module trace exceeds {MAX_MODULE_EVENTS} events"));
    }
    let mut previous = 0;
    let mut counts = BTreeMap::<&str, usize>::new();
    for (index, event) in events.iter().enumerate() {
        if event.frame >= total_frames as u64 {
            return Err(format!("module event {index}: frame is outside render"));
        }
        if index > 0 && event.frame < previous {
            return Err(format!("module event {index}: frames must be sorted"));
        }
        if event.id.is_empty()
            || event.id.len() > 64
            || !event
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
        {
            return Err(format!("module event {index}: invalid module ID"));
        }
        if event.frame != previous {
            counts.clear();
        }
        let count = counts.entry(&event.id).or_default();
        *count += 1;
        if *count > MAX_MODULE_ACTIONS_PER_SAMPLE {
            return Err(format!("module event {index}: exceeds {MAX_MODULE_ACTIONS_PER_SAMPLE} actions for one module/sample"));
        }
        previous = event.frame;
    }
    Ok(())
}
pub(crate) const INPUTS: [&str; 5] = ["tempo", "clock", "run", "reset", "steps"];
pub(crate) const OUTPUTS: [&str; 16] = [
    "cv1", "cv2", "cv3", "trigger", "step1", "step2", "step3", "step4", "step5", "step6", "step7",
    "step8", "steps", "clock", "run", "reset",
];
pub(crate) fn catalog() -> Value {
    let mut module = json!({"kind":"seq3","title":"SEQ 3","model":"functional","description":"Original eight-step, three-row voltage sequencer. All ports are mono, reading input channel one. CV holds the selected step while stopped; one-hot step gates remain active. Clock/manual ticks advance only while running and outside reset guard; every accepted tick pulses, including length one. Wrap does not emit RESET. Ideal switched CV; no de-clicking or saved runtime position.","inputs":INPUTS,"params":{
        "tempo":{"default":120.0,"min":15.0,"max":960.0,"unit":"BPM"},
        "tempo_amount":{"default":1.0,"min":0.0,"max":1.0},
        "steps":{"default":8.0,"min":1.0,"max":8.0,"step":1.0},
        "steps_amount":{"default":1.0,"min":0.0,"max":1.0},
        "passthrough":{"default":0.0,"min":0.0,"max":1.0,"step":1.0},
        "running":{"default":1.0,"min":0.0,"max":1.0,"step":1.0,"description":"Initial running state. Changed value or explicit-presence edits override adopted runtime state; otherwise runtime Run is retained."}
    }});
    let params = module["params"].as_object_mut().unwrap();
    for row in 1..=3 {
        for step in 1..=8 {
            params.insert(
                format!("row{row}_{step}"),
                json!({"default":0.0,"min":-10.0,"max":10.0,"unit":"V"}),
            );
        }
    }
    for step in 1..=8 {
        params.insert(
            format!("gate{step}"),
            json!({"default":1.0,"min":0.0,"max":1.0,"step":1.0}),
        );
    }
    module
}
pub(crate) fn annotate(module: &mut Value) {
    module["polyphony"]["version_2"]=json!("all outputs mono; every input reads only channel one; missing and zero-channel inputs are zero");
    module["polyphony"]["primary_input"] = Value::Null;
    for output in module["output_metadata"].as_array_mut().unwrap() {
        output["maximum_channels"] = json!(1);
    }
    module["tempo_policy"] = json!({"formula":"clamp(tempo/60 * 2^clamp(tempoCV*tempo_amount,-64,64), 0.000001, sample_rate/2)","clock_connected":"internal phase freezes when clock carries one or more channels","paused":"phase freezes; CV detectors continue tracking","sample_timing":"advance phase, consume threshold crossing and wrap, then emit outputs"});
    module["steps_policy"] = json!({"formula":"clamp(round(steps + stepsCV*steps_amount),1,8)","rounding":"half ties away from zero","output":"effective length minus one volts, 0–7 V","shrink":"hold current step/CV until the next accepted tick wraps to step one; no hidden event"});
    module["event_policy"] = json!({"actions":["clock","run","reset"],"capacity_per_module_per_sample":MAX_MODULE_ACTIONS_PER_SAMPLE,"ordering":"queued actions in supplied order, then CV run, reset, clock; MIDI is dispatched before module actions at equal frames","run":"toggle running and emit a 1 ms RUN pulse","clock":"accepted only while running and outside reset guard; manual and external clocks reset phase to zero","reset":"step one and phase zero, clear clock/trigger pulses, emit 1 ms RESET, guard clocks for 1 ms including reset sample","overlap":"logical actions are never coalesced; overlapping physical pulses naturally merge","guard":"detectors track and internal crossings wrap while guarded; no deferred clock catch-up"});
    module["trigger_policy"] = json!({"pulse":"1 ms at each accepted clock on an enabled selected step","passthrough":"10 V accepted external clock gate or internal 50% clock gate; manual Clock supplies 1 ms; suppressed while paused/guarded","step_outputs":"one-hot 10 V even while stopped","wrap_reset":"no RESET output pulse on wrap, including length one"});
    module["state_adoption"]=json!("same ID/kind/rate retains step, phase, running, pulses and guard with new parameters; changed initial-running value/presence overrides runtime running; run/reset/clock input detectors reset independently when source ID/output/kind changes; pending actions are per-engine and not adopted");
    module["functional_reference"] = json!("https://vcvrack.com/Free#vcv-seq-3");
    module["compatibility"]=json!("original functional contract, not a sample-identical Rack clone; manual clocks obey reset guard; every accepted length-one tick emits a pulse; wrap has no reset pulse");
}
#[derive(Clone, Copy, Debug, Default)]
struct Schmitt {
    high: bool,
}
impl Schmitt {
    fn rising(&mut self, value: f32) -> bool {
        let value = if value.is_finite() { value } else { 0.0 };
        if self.high {
            if value <= 0.1 {
                self.high = false;
            }
            false
        } else if value >= 2.0 {
            self.high = true;
            true
        } else {
            false
        }
    }
}
#[derive(Clone, Copy, Debug)]
struct State {
    step: u8,
    length: u8,
    running: bool,
    phase: f64,
    compensation: f64,
    clock: Schmitt,
    run: Schmitt,
    reset: Schmitt,
    external_gate: bool,
    clock_pulse: u32,
    trigger_pulse: u32,
    manual_pulse: u32,
    run_pulse: u32,
    reset_pulse: u32,
    guard: u32,
}
impl State {
    fn new(length: u8, running: bool) -> Self {
        Self {
            step: 0,
            length,
            running,
            phase: 0.0,
            compensation: 0.0,
            clock: Schmitt::default(),
            run: Schmitt::default(),
            reset: Schmitt::default(),
            external_gate: false,
            clock_pulse: 0,
            trigger_pulse: 0,
            manual_pulse: 0,
            run_pulse: 0,
            reset_pulse: 0,
            guard: 0,
        }
    }
    fn phase_zero(&mut self) {
        self.phase = 0.0;
        self.compensation = 0.0;
    }
}
#[derive(Debug)]
pub(crate) struct Seq3 {
    rate: f64,
    pulse: u32,
    tempo: f64,
    tempo_amount: f64,
    steps: u8,
    steps_amount: f64,
    passthrough: bool,
    initial_running: bool,
    explicit_running: bool,
    rows: [[f32; 8]; 3],
    gates: [bool; 8],
    state: State,
    actions: [ModuleAction; MAX_MODULE_ACTIONS_PER_SAMPLE],
    action_count: usize,
}
impl Seq3 {
    pub(crate) fn discard_actions(&mut self) {
        self.action_count = 0;
    }
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for key in node.params.keys() {
            let fixed = [
                "tempo",
                "tempo_amount",
                "steps",
                "steps_amount",
                "passthrough",
                "running",
            ]
            .contains(&key.as_str());
            let row = (1..=3).any(|row| (1..=8).any(|step| key == &format!("row{row}_{step}")));
            let gate = (1..=8).any(|step| key == &format!("gate{step}"));
            if !fixed && !row && !gate {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let integer = |name: &str, default, min, max| -> Result<f32, String> {
            let value = parameter(node, name, default, min, max)?;
            if value.fract() != 0.0 {
                return Err(format!(
                    "node '{}': parameter '{name}' must be an integer",
                    node.id
                ));
            }
            Ok(value)
        };
        let steps = integer("steps", 8.0, 1.0, 8.0)? as u8;
        let initial_running = integer("running", 1.0, 0.0, 1.0)? == 1.0;
        let mut rows = [[0.0; 8]; 3];
        let mut gates = [true; 8];
        for (row, values) in rows.iter_mut().enumerate() {
            for (step, value) in values.iter_mut().enumerate() {
                *value = parameter(
                    node,
                    &format!("row{}_{}", row + 1, step + 1),
                    0.0,
                    -10.0,
                    10.0,
                )?;
            }
        }
        for (step, gate) in gates.iter_mut().enumerate() {
            *gate = integer(&format!("gate{}", step + 1), 1.0, 0.0, 1.0)? == 1.0;
        }
        Ok(Self {
            rate: f64::from(rate),
            pulse: rate.div_ceil(1000),
            tempo: f64::from(parameter(node, "tempo", 120.0, 15.0, 960.0)?),
            tempo_amount: f64::from(parameter(node, "tempo_amount", 1.0, 0.0, 1.0)?),
            steps,
            steps_amount: f64::from(parameter(node, "steps_amount", 1.0, 0.0, 1.0)?),
            passthrough: integer("passthrough", 0.0, 0.0, 1.0)? == 1.0,
            initial_running,
            explicit_running: node.params.contains_key("running"),
            rows,
            gates,
            state: State::new(steps, initial_running),
            actions: [ModuleAction::Clock; MAX_MODULE_ACTIONS_PER_SAMPLE],
            action_count: 0,
        })
    }
    pub(crate) fn enqueue(&mut self, action: ModuleAction) -> bool {
        if self.action_count == self.actions.len() {
            return false;
        }
        self.actions[self.action_count] = action;
        self.action_count += 1;
        true
    }
    pub(crate) fn telemetry(&self) -> ModuleTelemetry {
        ModuleTelemetry {
            step: self.state.step + 1,
            running: self.state.running,
            length: self.state.length,
        }
    }
    pub(crate) fn reset_state(&mut self) {
        self.state = State::new(self.steps, self.initial_running);
        self.action_count = 0;
    }
    pub(crate) fn adopt(&mut self, old: &Self, unchanged: [bool; 3]) {
        self.state = old.state;
        // Pending actions belong to their receiving engine: retain this queue,
        // never duplicate the old engine's pending actions during adoption.
        if self.initial_running != old.initial_running
            || self.explicit_running != old.explicit_running
        {
            self.state.running = self.initial_running;
            if !self.state.running {
                self.state.external_gate = false;
            }
        }
        if !unchanged[0] {
            self.state.clock = Schmitt::default();
            self.state.external_gate = false;
        }
        if !unchanged[1] {
            self.state.run = Schmitt::default();
        }
        if !unchanged[2] {
            self.state.reset = Schmitt::default();
        }
    }
    fn run(&mut self) {
        self.state.running = !self.state.running;
        self.state.run_pulse = self.pulse;
        if !self.state.running {
            self.state.external_gate = false;
        }
    }
    fn reset(&mut self) {
        self.state.step = 0;
        self.state.phase_zero();
        self.state.external_gate = false;
        self.state.clock_pulse = 0;
        self.state.trigger_pulse = 0;
        self.state.manual_pulse = 0;
        self.state.reset_pulse = self.pulse;
        self.state.guard = self.pulse;
    }
    fn tick(&mut self, reset_phase: bool, manual: bool) -> bool {
        if !self.state.running || self.state.guard > 0 {
            return false;
        }
        self.state.step = if self.state.step + 1 >= self.state.length {
            0
        } else {
            self.state.step + 1
        };
        if reset_phase {
            self.state.phase_zero();
        }
        self.state.clock_pulse = self.pulse;
        self.state.trigger_pulse = if self.gates[self.state.step as usize] {
            self.pulse
        } else {
            0
        };
        if manual {
            self.state.manual_pulse = self.pulse;
        }
        true
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame; 5], outputs: &mut [PortFrame]) {
        let value = |input: usize| {
            let value = inputs[input].channel(0);
            if value.is_finite() {
                f64::from(value)
            } else {
                0.0
            }
        };
        self.state.length = (f64::from(self.steps) + value(4) * self.steps_amount)
            .round()
            .clamp(1.0, 8.0) as u8;
        for index in 0..self.action_count {
            match self.actions[index] {
                ModuleAction::Clock => {
                    self.tick(true, true);
                }
                ModuleAction::Run => self.run(),
                ModuleAction::Reset => self.reset(),
            }
        }
        self.action_count = 0;
        if self.state.run.rising(inputs[2].channel(0)) {
            self.run();
        }
        if self.state.reset.rising(inputs[3].channel(0)) {
            self.reset();
        }
        let external = inputs[1].channels > 0;
        if external {
            if self.state.clock.rising(inputs[1].channel(0)) && self.tick(true, false) {
                self.state.external_gate = true;
            }
            if !self.state.clock.high {
                self.state.external_gate = false;
            }
        } else {
            self.state.clock = Schmitt::default();
            self.state.external_gate = false;
            if self.state.running {
                let frequency = (self.tempo / 60.0
                    * (value(0) * self.tempo_amount).clamp(-64.0, 64.0).exp2())
                .clamp(1e-6, self.rate / 2.0);
                let increment = frequency / self.rate - self.state.compensation;
                let next = self.state.phase + increment;
                self.state.compensation = (next - self.state.phase) - increment;
                self.state.phase = next;
                if self.state.phase >= 1.0 {
                    self.state.phase -= 1.0;
                    self.tick(false, false);
                }
            }
        }
        let gate = self.gates[self.state.step as usize];
        let clock = if self.passthrough {
            self.state.running
                && self.state.guard == 0
                && (self.state.manual_pulse > 0
                    || if external {
                        self.state.external_gate
                    } else {
                        self.state.phase < 0.5
                    })
        } else {
            self.state.clock_pulse > 0
        };
        for (index, output) in outputs.iter_mut().enumerate() {
            *output = PortFrame::mono(match index {
                0..=2 => self.rows[index][self.state.step as usize],
                3 => {
                    if gate
                        && if self.passthrough {
                            clock
                        } else {
                            self.state.trigger_pulse > 0
                        }
                    {
                        10.0
                    } else {
                        0.0
                    }
                }
                4..=11 => {
                    if self.state.step as usize == index - 4 {
                        10.0
                    } else {
                        0.0
                    }
                }
                12 => f32::from(self.state.length - 1),
                13 => {
                    if clock {
                        10.0
                    } else {
                        0.0
                    }
                }
                14 => {
                    if self.state.run_pulse > 0 {
                        10.0
                    } else {
                        0.0
                    }
                }
                15 => {
                    if self.state.reset_pulse > 0 {
                        10.0
                    } else {
                        0.0
                    }
                }
                _ => unreachable!(),
            });
        }
        for counter in [
            &mut self.state.clock_pulse,
            &mut self.state.trigger_pulse,
            &mut self.state.manual_pulse,
            &mut self.state.run_pulse,
            &mut self.state.reset_pulse,
            &mut self.state.guard,
        ] {
            *counter = counter.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn seq(params: &[(&str, f32)]) -> Seq3 {
        Seq3::build(
            &Node {
                bypassed: false,
                id: "s".into(),
                kind: "seq3".into(),
                params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
            },
            8000,
        )
        .unwrap()
    }
    fn sample(s: &mut Seq3, inputs: &[PortFrame; 5]) -> [PortFrame; 16] {
        let mut outputs = [PortFrame::default(); 16];
        s.process(inputs, &mut outputs);
        outputs
    }
    #[test]
    fn cv_schmitt_thresholds_track_while_paused_and_reset_precedes_clock() {
        let mut s = seq(&[("running", 0.0)]);
        let mut i = [PortFrame::default(); 5];
        i[1] = PortFrame::mono(2.0);
        sample(&mut s, &i);
        assert_eq!(s.state.step, 0);
        assert!(s.state.clock.high);
        s.enqueue(ModuleAction::Run);
        sample(&mut s, &i);
        assert_eq!(s.state.step, 0); // Held-high ignored edge is not replayed.
        i[1] = PortFrame::mono(0.1001);
        sample(&mut s, &i);
        assert!(s.state.clock.high);
        i[1] = PortFrame::mono(0.1);
        sample(&mut s, &i);
        assert!(!s.state.clock.high);
        i[1] = PortFrame::mono(2.0);
        i[3] = PortFrame::mono(2.0);
        let out = sample(&mut s, &i);
        assert_eq!(s.state.step, 0);
        assert_eq!(out[15].values[0], 10.0);
        assert_eq!(out[13].values[0], 0.0);
        i[3] = PortFrame::mono(0.0);
        for _ in 0..8 {
            sample(&mut s, &i);
        }
        assert_eq!(s.state.step, 0); // Guarded edge consumed, not deferred.
        i[1] = PortFrame::mono(0.1);
        sample(&mut s, &i);
        i[1] = PortFrame::mono(2.0);
        sample(&mut s, &i);
        assert_eq!(s.state.step, 1);
    }
    #[test]
    fn guarded_internal_crossings_wrap_without_catch_up_and_cv_rate_is_finite_bounded() {
        let mut s = seq(&[]);
        let mut i = [PortFrame::default(); 5];
        i[0] = PortFrame::mono(f32::MAX);
        s.enqueue(ModuleAction::Reset);
        for _ in 0..8 {
            let out = sample(&mut s, &i);
            assert_eq!(s.state.step, 0);
            assert_eq!(out[13].values[0], 0.0);
            assert!(s.state.phase < 1.0);
        }
        sample(&mut s, &i);
        assert_eq!(s.state.step, 0);
        sample(&mut s, &i);
        assert_eq!(s.state.step, 1); // One new crossing, not four guarded crossings.
        let mut s = seq(&[]);
        i[0] = PortFrame::mono(-f32::MAX);
        sample(&mut s, &i);
        assert_eq!(s.state.phase, 1e-6 / 8000.0);
        let mut s = seq(&[]);
        for input in &mut i {
            *input = PortFrame::mono(f32::NAN);
        }
        let out = sample(&mut s, &i);
        assert!(out.iter().flat_map(|f| f.values).all(f32::is_finite));
        assert!(!s.state.clock.high);
    }
    #[test]
    fn passthrough_uses_accepted_external_gate_and_internal_half_cycle() {
        let mut s = seq(&[("passthrough", 1.0), ("gate2", 0.0)]);
        let mut i = [PortFrame::default(); 5];
        i[1] = PortFrame::mono(2.0);
        let out = sample(&mut s, &i);
        assert_eq!(out[13].values[0], 10.0);
        assert_eq!(out[3].values[0], 0.0); // Disabled step2 suppresses trigger, not clock.
        i[1] = PortFrame::mono(0.0);
        assert_eq!(sample(&mut s, &i)[13].values[0], 0.0);
        s.enqueue(ModuleAction::Reset);
        i[1] = PortFrame::mono(2.0);
        for _ in 0..9 {
            assert_eq!(sample(&mut s, &i)[13].values[0], 0.0);
        } // Ignored high gate does not emerge after guard.
        let mut s = seq(&[("passthrough", 1.0)]);
        i[1] = PortFrame::default();
        assert_eq!(sample(&mut s, &i)[13].values[0], 10.0);
        s.state.phase = 0.75;
        assert_eq!(sample(&mut s, &i)[13].values[0], 0.0);
    }
    #[test]
    fn independent_adoption_detectors_and_pending_actions_follow_compiled_engine_ownership() {
        let mut old = seq(&[]);
        old.state.step = 5;
        old.state.phase = 0.37;
        old.state.clock.high = true;
        old.state.run.high = true;
        old.state.reset.high = true;
        old.enqueue(ModuleAction::Clock);
        let mut new = seq(&[]);
        new.adopt(&old, [false, true, true]);
        assert_eq!(new.state.step, 5);
        assert_eq!(new.state.phase, 0.37);
        assert!(!new.state.clock.high);
        assert!(new.state.run.high && new.state.reset.high);
        assert_eq!(new.action_count, 0);
        new.adopt(&old, [true, false, false]);
        assert!(new.state.clock.high);
        assert!(!new.state.run.high && !new.state.reset.high);
        assert_eq!(new.action_count, 0);
        new.enqueue(ModuleAction::Run);
        new.adopt(&old, [true; 3]);
        assert_eq!(new.action_count, 1); // Preserve events already addressed to the new engine.
    }
    #[test]
    fn catalog_has_all_parameters_named_outputs_and_mono_metadata() {
        let catalog = crate::catalog();
        let entry = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == "seq3")
            .unwrap();
        assert_eq!(entry["params"].as_object().unwrap().len(), 38);
        assert_eq!(entry["outputs"], json!(OUTPUTS));
        assert_eq!(entry["inputs"], json!(INPUTS));
        assert_eq!(entry["min_patch_version"], 2);
        assert_eq!(entry["model"], "functional");
        assert!(entry["output_metadata"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["maximum_channels"] == 1 && p["units"] == "V"));
    }
}
