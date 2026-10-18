//! Grids pattern/clock kernels with explicit native 8 kHz input/voltage policies.
//! The complete raw source API is retained for the independent C++ oracle.
#[allow(dead_code)]
mod kernel;
mod resources;

use crate::{
    parameter, ModuleAction, ModuleControlState, ModuleLightState, Node, PortFrame,
    MAX_MODULE_ACTIONS_PER_SAMPLE,
};
use kernel::{Clock, Options, PatternGenerator, Settings};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 8] = [
    "clock", "reset", "x_cv", "y_cv", "chaos_cv", "fill1_cv", "fill2_cv", "fill3_cv",
];
pub(crate) const OUTPUTS: [&str; 8] = [
    "trig1", "trig2", "trig3", "aux1", "aux2", "aux3", "clock", "random",
];
const SOURCE_RATE: u32 = 8000;
const EDGE_CAPACITY: usize = 64;
const COMMAND_RESET: u8 = 1;
const COMMAND_RELEASE_TAP: u8 = 2;

pub(crate) fn catalog() -> Value {
    let mut result = json!({"kind":"rhythm_terrain","title":"RHYTHM TERRAIN","model":"functional","width_hp":48,
    "tags":["Custom","Clock","Sequencers","Modulation","Random"],"inputs":INPUTS,
    "description":"Three-lane source-derived drum-map and Euclidean rhythm generator, with accents, swing, gate/trigger modes, clock/reset and tapped tempo.",
    "params":{
        "tempo":{"default":120,"min":0,"max":480,"step":1,"unit":"BPM","description":"Integer source tempo. Below40 selects external CLOCK;40..480 selects internal. A valid tapped tempo overrides this control until released."},
        "mode":{"default":1,"min":0,"max":1,"step":1,"values":[0,1],"labels":["Euclidean","Drum map"]},
        "resolution":{"default":2,"min":0,"max":2,"step":1,"values":[0,1,2],"labels":["4 PPQN","8 PPQN","24 PPQN"]},
        "seed":{"default":1,"min":1,"max":65535,"step":1,"unit":"seed","description":"Exact nonzero16-bit source LFSR state. Changing seed starts a fresh instrument."}
    }});
    for (name, default) in [
        ("x", 0.5),
        ("y", 0.5),
        ("chaos", 0.0),
        ("fill1", 0.5),
        ("fill2", 0.5),
        ("fill3", 0.5),
    ] {
        result["params"][name] = json!({"default":default,"min":0,"max":1,"description":"Normalized configured control; matching CV adds voltage/5 before source8-bit rounding. In Euclidean mode X/Y/CHAOS select lengths1/2/3."});
    }
    for (name, default) in [
        ("gate_mode", 0),
        ("alternate", 0),
        ("swing", 0),
        ("tap_tempo", 1),
    ] {
        result["params"][name] = json!({"default":default,"min":0,"max":1,"step":1,"values":[0,1],"labels":["Off","On"]});
    }
    result
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://github.com/pichenettes/eurorack/tree/f4cfa7697095a28d292bff758beb51deb49b5a86/grids");
    module["polyphony"]["maximum_channels"] = json!(1);
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["version_2"] = json!("Eight mono outputs. Inputs read channel zero; CLOCK is used only below40 configured BPM and while tap tempo is unlocked. Internal clock ignores CLOCK voltage.");
    for output in module["output_metadata"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        output["maximum_channels"] = json!(1);
    }
    module["runtime_actions"] = json!(["tap_press", "tap_release", "release_tap", "reset"]);
    module["control_policy"] = json!("round(clamp(configured + finite(CV channel0)/5,0,1)*255). Source map interpolation,25 nodes, density thresholds, bar perturbation,16-bit LFSR, Euclidean resources, clock phase and byte wrap are retained. Euclidean lengths=(X/Y/CHAOS byte>>3)+1; density remains source8-bit. Swing disables drum perturbation and repurposes CHAOS; Euclidean ignores swing.");
    module["timing_policy"] = json!("Integer bridge to exactly8000 source scans/s, first scan after one source interval. Host8000..192000Hz, zero-order-held outputs. The physical20MHz AVR timer uses OCR78 and runs~8012.82Hz; native8000 is explicit. Each scan increments tap elapsed, applies direct button actions, runs source CLOCK then RESET ordering and TickClock, increments pulse counter, then publishes. Nominal1ms triggers occupy seven published source intervals (0.875ms), plus host sample quantization. Native cables add one host frame separately.");
    module["input_policy"] = json!("CLOCK/RESET use0.1V low/1V high Schmitt detectors, initially low. Combined simultaneous host changes are queued together with TAP state in64 fixed slots; one snapshot is consumed per source scan. This intentionally captures subscan pulses that physical polling could miss, stretching them in FIFO order. Sustainable combined transition bandwidth is8000 snapshots/s (a full two-edge clock cycle<=4000Hz); backlog latency<=64 scans. Overflow discards backlog and reconciles latest levels on the next scan, with a sticky warning lamp; it does not promise to retain over-bandwidth pulses. CV/parameters are sampled at the service scan, not historic edge time.");
    module["reset_policy"] = json!("Musical reset retains source outputs, pulse duration, RNG, perturbation, beat flags and cached swing while resetting step/pulse/Euclidean indices. External nonfactory RESET alone is transparent. Internal/locked RESET retriggers and resets clock phase; simultaneous clock/reset retains the source double-evaluation behavior. Native reset action follows the source reset-button path without Retrigger. Factory first-five-power-on EEPROM behavior is excluded. Resolution edits reset pattern indices and update clock increment, preserving phase.");
    module["tap_policy"] = json!("TAP/RESET is runtime-only: tap_press/release follows the saved tap_tempo mode. Valid direct taps use integer480000/elapsed8k_scans, accept30..480BPM, reset source clock and lock. Zero/expired/invalid intervals safely unlock. While locked, configured TEMPO changes remain saved but ineffective, even at30..39BPM. release_tap or disabling tap_tempo unlocks; configured tempo applies on that same defined control scan. This excludes physical switch debounce, options long-press, ADC/pot scheduling and overflow artifacts of the board timer arithmetic.");
    module["output_policy"] = json!("Ideal0/5V from all eight source state bits. First six are the physical lanes/auxiliary layout. Normal aux are three accents (Drum map) or three cycle starts (Euclidean). Alternate aux1=common accent/cycle,aux2=clock,aux3=bar/all-cycle reset, following executable code rather than the inconsistent source header. Extra clock/random ports explicitly expose bits6/7; they are native extensions, not additional hardware jacks. Gate mode and pulse clearing affect all bits.");
    module["light_names"] = json!([
        "trig1",
        "trig2",
        "trig3",
        "aux1",
        "aux2",
        "aux3",
        "clock",
        "random",
        "tap_locked",
        "input_overflow"
    ]);
    module["light_policy"] = json!("First8 first-component lights are actual source output bits;8 is actual tap lock,9 sticky input FIFO overflow. manual/controls.pressed report physical TAP held. clock_frequency is source internal quarter-note Hz=BPM/60, None in external mode; waveform_position is actual NEXT source step/32, not fabricated played-step data. No telemetry is implied before the first scan.");
    module["state_adoption"] = json!("Same ID/kind/rate/seed retain kernel, clock/tap authority, scan phase, input FIFO and hysteresis. Receiving parameters/CV apply at the next scan; receiving pending actions remain local. Bypass freezes runtime and emits mono zero; transition releases physical TAP. Initialize/seed/rate changes restart state. Source runtime is transient, configured controls/seed persist through presets/save/reopen. No callback allocations, deallocations, locks or filesystem work.");
}

#[derive(Clone, Copy, Debug)]
struct Params {
    seed: u16,
    tempo: u16,
    knobs: [f32; 6],
    options: Options,
}
impl Params {
    fn build(node: &Node) -> Result<Self, String> {
        const NAMES: [&str; 14] = [
            "seed",
            "tempo",
            "x",
            "y",
            "chaos",
            "fill1",
            "fill2",
            "fill3",
            "mode",
            "resolution",
            "gate_mode",
            "alternate",
            "swing",
            "tap_tempo",
        ];
        for key in node.params.keys() {
            if !NAMES.contains(&key.as_str()) {
                return Err(format!("Rhythm Terrain unknown parameter '{key}'"));
            }
        }
        let integer = |name, default, maximum| {
            let value = parameter(node, name, default, 0.0, maximum)?;
            if value.fract() != 0.0 {
                return Err(format!("Rhythm Terrain {name} must be an integer"));
            }
            Ok(value as u16)
        };
        let seed = integer("seed", 1.0, 65535.0)?;
        if seed == 0 {
            return Err("Rhythm Terrain seed must be nonzero".into());
        }
        let mut knobs = [0.0; 6];
        for (i, name) in ["x", "y", "chaos", "fill1", "fill2", "fill3"]
            .iter()
            .enumerate()
        {
            knobs[i] = parameter(node, name, if i == 2 { 0.0 } else { 0.5 }, 0.0, 1.0)?;
        }
        Ok(Self {
            seed,
            tempo: integer("tempo", 120.0, 480.0)?,
            knobs,
            options: Options {
                output_mode: integer("mode", 1.0, 1.0)? as u8,
                clock_resolution: integer("resolution", 2.0, 2.0)? as u8,
                output_clock: integer("alternate", 0.0, 1.0)? != 0,
                tap_tempo: integer("tap_tempo", 1.0, 1.0)? != 0,
                gate_mode: integer("gate_mode", 0.0, 1.0)? != 0,
                swing: integer("swing", 0.0, 1.0)? != 0,
            },
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct InputEvent {
    levels: u8,
    commands: u8,
}
#[derive(Clone, Debug)]
struct Runtime {
    pattern: PatternGenerator,
    clock: Clock,
    phase: u32,
    host_levels: u8,
    captured_levels: u8,
    delivered_levels: u8,
    events: [InputEvent; EDGE_CAPACITY],
    head: usize,
    len: usize,
    overflow: bool,
    tap_elapsed: u32,
    swing: i8,
    scanned: bool,
}
impl Runtime {
    fn new(seed: u16) -> Self {
        Self {
            pattern: PatternGenerator::new(u32::from(seed)),
            clock: Clock::new(),
            phase: 0,
            host_levels: 0,
            captured_levels: 0,
            delivered_levels: 0,
            events: [InputEvent::default(); EDGE_CAPACITY],
            head: 0,
            len: 0,
            overflow: false,
            tap_elapsed: 0,
            swing: 0,
            scanned: false,
        }
    }
    fn capture(&mut self, commands: u8) {
        if self.host_levels == self.captured_levels && commands == 0 {
            return;
        }
        if self.len == EDGE_CAPACITY {
            self.head = 0;
            self.len = 0;
            self.overflow = true;
        }
        self.events[(self.head + self.len) % EDGE_CAPACITY] = InputEvent {
            levels: self.host_levels,
            commands,
        };
        self.len += 1;
        self.captured_levels = self.host_levels;
    }
    fn pop(&mut self) -> InputEvent {
        if self.len == 0 {
            return InputEvent {
                levels: self.delivered_levels,
                commands: 0,
            };
        }
        let event = self.events[self.head];
        self.head = (self.head + 1) % EDGE_CAPACITY;
        self.len -= 1;
        event
    }
}

#[derive(Debug)]
pub(crate) struct RhythmTerrain {
    sample_rate: u32,
    params: Params,
    runtime: Runtime,
    actions: [ModuleAction; MAX_MODULE_ACTIONS_PER_SAMPLE],
    action_count: usize,
}
fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}
fn schmitt(previous: bool, value: f32) -> bool {
    let value = finite(value);
    if value >= 1.0 {
        true
    } else if value <= 0.1 {
        false
    } else {
        previous
    }
}
impl RhythmTerrain {
    pub(crate) fn build(node: &Node, sample_rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err("Rhythm Terrain sample rate outside8000..192000".into());
        }
        let params = Params::build(node)?;
        Ok(Self {
            sample_rate,
            params,
            runtime: Runtime::new(params.seed),
            actions: [ModuleAction::Reset; MAX_MODULE_ACTIONS_PER_SAMPLE],
            action_count: 0,
        })
    }
    pub(crate) fn supports(action: ModuleAction) -> bool {
        matches!(
            action,
            ModuleAction::TapPress
                | ModuleAction::TapRelease
                | ModuleAction::ReleaseTap
                | ModuleAction::Reset
        )
    }
    pub(crate) fn enqueue(&mut self, action: ModuleAction) -> bool {
        if !Self::supports(action) || self.action_count == self.actions.len() {
            return false;
        }
        self.actions[self.action_count] = action;
        self.action_count += 1;
        true
    }
    pub(crate) fn controls(&self) -> ModuleControlState {
        let pressed = self.runtime.host_levels & 4 != 0;
        ModuleControlState {
            pressed,
            pressed_mask: u16::from(pressed),
        }
    }
    pub(crate) fn release(&mut self) {
        self.action_count = 0;
        self.runtime.host_levels &= !4;
        self.runtime.captured_levels &= !4;
        self.runtime.delivered_levels &= !4;
        for event in &mut self.runtime.events {
            event.levels &= !4;
            event.commands = 0;
        }
    }
    pub(crate) fn initialize_state(&mut self) {
        self.runtime = Runtime::new(self.params.seed);
        self.action_count = 0;
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.sample_rate == old.sample_rate && self.params.seed == old.params.seed {
            self.runtime.clone_from(&old.runtime);
        }
    }
    fn internal(&self) -> bool {
        self.runtime.clock.bpm() >= 40 || self.runtime.clock.locked()
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut result = ModuleLightState {
            count: 10,
            manual: self.controls().pressed,
            waveform_position: self
                .runtime
                .scanned
                .then_some(f32::from(self.runtime.pattern.step()) / 32.0),
            clock_frequency: (self.runtime.scanned && self.internal())
                .then_some(f32::from(self.runtime.clock.bpm()) / 60.0),
            ..Default::default()
        };
        for i in 0..8 {
            result.brightness[i] = [f32::from(self.runtime.pattern.state() & (1 << i) != 0), 0.0];
        }
        result.brightness[8] = [f32::from(self.runtime.clock.locked()), 0.0];
        result.brightness[9] = [f32::from(self.runtime.overflow), 0.0];
        result
    }
    fn button_reset(&mut self) {
        self.runtime.pattern.reset();
        if self.internal() {
            self.runtime.clock.reset();
        }
    }
    fn scan(&mut self, inputs: &[PortFrame; 8]) {
        self.runtime.tap_elapsed = self.runtime.tap_elapsed.saturating_add(1).min(16001);
        let event = self.runtime.pop();
        let previous = self.runtime.delivered_levels;
        let old_options = self.runtime.pattern.options;
        if old_options.tap_tempo && !self.params.options.tap_tempo {
            self.runtime.clock.unlock();
        }
        self.runtime.pattern.options = self.params.options;
        if old_options.clock_resolution != self.params.options.clock_resolution {
            self.runtime.pattern.reset();
            self.runtime.clock.update(
                self.runtime.clock.bpm(),
                self.params.options.clock_resolution,
            );
        }
        let mut bytes = [0; 6];
        for (i, value) in bytes.iter_mut().enumerate() {
            *value = ((self.params.knobs[i] + finite(inputs[i + 2].channel(0)) / 5.0)
                .clamp(0.0, 1.0)
                * 255.0)
                .round() as u8;
        }
        self.runtime.pattern.settings = Settings {
            options: [bytes[0], bytes[1], bytes[2]],
            density: [bytes[3], bytes[4], bytes[5]],
        };
        if !self.runtime.clock.locked() {
            self.runtime
                .clock
                .update(self.params.tempo, self.params.options.clock_resolution);
        }
        if event.commands & COMMAND_RELEASE_TAP != 0 {
            self.runtime.clock.unlock();
        }
        if event.commands & COMMAND_RESET != 0 {
            self.button_reset();
        }
        if event.levels & 4 != 0 && previous & 4 == 0 {
            if self.params.options.tap_tempo {
                let bpm = 480000 / self.runtime.tap_elapsed;
                if (30..=480).contains(&bpm) {
                    self.runtime
                        .clock
                        .update(bpm as u16, self.params.options.clock_resolution);
                    self.runtime.clock.reset();
                    self.runtime.clock.lock();
                } else {
                    self.runtime.clock.unlock();
                }
                self.runtime.tap_elapsed = 0;
            } else {
                self.button_reset();
            }
        }
        if !self.runtime.clock.locked() {
            self.runtime
                .clock
                .update(self.params.tempo, self.params.options.clock_resolution);
        }
        let increment = [6, 3, 1][usize::from(self.params.options.clock_resolution)];
        let mut ticks = 0;
        if !self.internal() {
            if event.levels & 1 != 0 && previous & 1 == 0 {
                ticks = increment;
            }
            if event.levels & 1 == 0 && previous & 1 != 0 {
                self.runtime.pattern.clock_falling_edge();
            }
        } else {
            self.runtime.clock.tick();
            self.runtime.clock.wrap(self.runtime.swing);
            if self.runtime.clock.raising_edge() {
                ticks = increment;
            }
            if self.runtime.clock.past_falling_edge() {
                self.runtime.pattern.clock_falling_edge();
            }
        }
        if event.levels & 2 != 0 && previous & 2 == 0 {
            self.runtime.pattern.reset();
            if self.internal() {
                self.runtime.pattern.retrigger();
                self.runtime.clock.reset();
            }
        }
        self.runtime.delivered_levels = event.levels;
        if ticks != 0 {
            self.runtime.swing = self.runtime.pattern.swing_amount();
            self.runtime.pattern.tick_clock(ticks);
        }
        self.runtime.pattern.increment_pulse_counter();
        self.runtime.scanned = true;
    }
    pub(crate) fn process(
        &mut self,
        inputs: &[PortFrame; 8],
        demand: u16,
        outputs: &mut [PortFrame],
    ) {
        for (i, input) in inputs.iter().enumerate().take(2) {
            let high = schmitt(self.runtime.host_levels & (1 << i) != 0, input.channel(0));
            self.runtime.host_levels =
                (self.runtime.host_levels & !(1 << i)) | ((u8::from(high)) << i);
        }
        for i in 0..self.action_count {
            let mut commands = 0;
            match self.actions[i] {
                ModuleAction::TapPress => self.runtime.host_levels |= 4,
                ModuleAction::TapRelease => self.runtime.host_levels &= !4,
                ModuleAction::Reset => commands |= COMMAND_RESET,
                ModuleAction::ReleaseTap => commands |= COMMAND_RELEASE_TAP,
                _ => {}
            }
            self.runtime.capture(commands);
        }
        self.action_count = 0;
        self.runtime.capture(0);
        self.runtime.phase += SOURCE_RATE;
        if self.runtime.phase >= self.sample_rate {
            self.runtime.phase -= self.sample_rate;
            self.scan(inputs);
        }
        let state = self.runtime.pattern.state();
        for (i, output) in outputs.iter_mut().enumerate() {
            *output = if demand & (1 << i) != 0 {
                PortFrame::mono(if state & (1 << i) != 0 { 5.0 } else { 0.0 })
            } else {
                PortFrame::default()
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(params: Value) -> Node {
        serde_json::from_value(json!({"id":"terrain","kind":"rhythm_terrain","params":params}))
            .unwrap()
    }
    fn model(params: Value, rate: u32) -> RhythmTerrain {
        RhythmTerrain::build(&node(params), rate).unwrap()
    }
    fn tick(m: &mut RhythmTerrain, clock: f32, reset: f32) -> [f32; 8] {
        let mut inputs = [PortFrame::default(); 8];
        inputs[0] = PortFrame::mono(clock);
        inputs[1] = PortFrame::mono(reset);
        let mut outputs = [PortFrame::default(); 8];
        m.process(&inputs, 255, &mut outputs);
        assert!(outputs.iter().all(|p| p.channels == 1));
        outputs.map(|p| p.channel(0))
    }
    #[test]
    fn nominal_trigger_has_seven_published_scans_and_gate_follows_the_falling_edge() {
        let mut m = model(json!({"tempo":0}), 8000);
        assert_eq!(tick(&mut m, 5.0, 0.0)[6], 5.0);
        for _ in 1..7 {
            assert_eq!(tick(&mut m, 5.0, 0.0)[6], 5.0);
        }
        assert_eq!(tick(&mut m, 5.0, 0.0)[6], 0.0);
        for _ in 0..270 {
            assert_eq!(tick(&mut m, 5.0, 0.0)[6], 0.0);
        }
        let mut gated = model(json!({"tempo":0,"gate_mode":1}), 8000);
        for _ in 0..270 {
            assert_eq!(tick(&mut gated, 5.0, 0.0)[6], 5.0);
        }
        assert_eq!(tick(&mut gated, 0.1, 0.0), [0.0; 8]);
    }
    #[test]
    fn external_reset_retains_source_output_rng_perturbation_and_counter_progress() {
        let mut m = model(json!({"tempo":0,"chaos":1}), 8000);
        tick(&mut m, 5.0, 0.0);
        let before = m.runtime.pattern.snapshot();
        tick(&mut m, 5.0, 5.0);
        let after = m.runtime.pattern.snapshot();
        assert_eq!(after.state, before.state);
        assert_eq!(after.rng_state, before.rng_state);
        assert_eq!(after.part_perturbation, before.part_perturbation);
        assert_eq!(
            after.pulse_duration_counter,
            before.pulse_duration_counter + 1
        );
        assert_eq!(
            (after.step, after.pulse, after.euclidean_step),
            (0, 0, [0; 3])
        );
    }
    #[test]
    fn internal_reset_coincident_with_clock_retains_both_source_evaluations() {
        let mut m = model(json!({"tempo":480,"chaos":1}), 8000);
        tick(&mut m, 0.0, 0.0);
        for _ in 0..100 {
            let mut next = m.runtime.clock.clone();
            next.tick();
            next.wrap(m.runtime.swing);
            if next.raising_edge() {
                break;
            }
            tick(&mut m, 0.0, 0.0);
        }
        let mut next = m.runtime.clock.clone();
        next.tick();
        next.wrap(m.runtime.swing);
        assert!(next.raising_edge());
        let mut expected = m.runtime.pattern.clone();
        expected.reset();
        expected.retrigger();
        expected.tick_clock(1);
        expected.increment_pulse_counter();
        tick(&mut m, 0.0, 5.0);
        assert_eq!(m.runtime.pattern.snapshot(), expected.snapshot());
        assert_eq!(m.runtime.clock.snapshot().phase, 0);
    }
    #[test]
    fn tap_lock_at_thirty_bpm_survives_configured_edits_and_explicit_release_applies_them() {
        let mut m = model(json!({"tempo":0}), 8000);
        assert!(m.enqueue(ModuleAction::TapPress));
        tick(&mut m, 0.0, 0.0);
        assert!(!m.runtime.clock.locked());
        m.enqueue(ModuleAction::TapRelease);
        tick(&mut m, 0.0, 0.0);
        for _ in 1..15999 {
            tick(&mut m, 0.0, 0.0);
        }
        m.enqueue(ModuleAction::TapPress);
        tick(&mut m, 0.0, 0.0);
        assert_eq!(m.runtime.clock.bpm(), 30);
        assert!(m.runtime.clock.locked());
        assert!(m.internal());
        assert_eq!(m.lights().clock_frequency, Some(0.5));
        let mut edited = model(json!({"tempo":240}), 8000);
        edited.adopt(&m);
        tick(&mut edited, 0.0, 0.0);
        assert_eq!(edited.params.tempo, 240);
        assert_eq!(edited.runtime.clock.bpm(), 30);
        edited.enqueue(ModuleAction::ReleaseTap);
        tick(&mut edited, 0.0, 0.0);
        assert!(!edited.runtime.clock.locked());
        assert_eq!(edited.runtime.clock.bpm(), 240);
        assert!(edited.controls().pressed);
        edited.enqueue(ModuleAction::TapRelease);
        tick(&mut edited, 0.0, 0.0);
        assert!(!edited.controls().pressed);
    }
    #[test]
    fn invalid_taps_and_disabling_tap_mode_unlock_without_changing_the_saved_tempo() {
        let mut m = model(json!({"tempo":140}), 8000);
        m.runtime.clock.update(75, 2);
        m.runtime.clock.lock();
        m.runtime.tap_elapsed = 0;
        m.enqueue(ModuleAction::TapPress);
        tick(&mut m, 0.0, 0.0);
        assert!(!m.runtime.clock.locked());
        assert_eq!(m.runtime.clock.bpm(), 140);
        assert_eq!(m.params.tempo, 140);
        m.runtime.clock.update(75, 2);
        m.runtime.clock.lock();
        let mut edited = model(json!({"tempo":0,"tap_tempo":0}), 8000);
        edited.adopt(&m);
        tick(&mut edited, 0.0, 0.0);
        assert!(!edited.runtime.clock.locked());
        assert!(!edited.internal());
        assert_eq!(edited.lights().clock_frequency, None);
    }
    #[test]
    fn combined_edge_fifo_preserves_subscan_pulses_and_overflow_reconciles_latest_low() {
        let mut m = model(json!({"tempo":0,"gate_mode":1}), 192000);
        tick(&mut m, 5.0, 5.0);
        tick(&mut m, 0.0, 0.0);
        assert_eq!(m.runtime.len, 2);
        let mut seen = false;
        for _ in 0..50 {
            seen |= tick(&mut m, 0.0, 0.0)[6] == 5.0;
        }
        assert!(seen);
        assert_eq!(m.runtime.delivered_levels & 3, 0);
        for frame in 0..200 {
            tick(&mut m, if frame % 2 == 0 { 5.0 } else { 0.0 }, 0.0);
        }
        assert!(m.runtime.overflow);
        for _ in 0..2000 {
            tick(&mut m, 0.0, 0.0);
        }
        assert_eq!(m.runtime.pattern.state(), 0);
        assert_eq!(m.runtime.delivered_levels & 3, 0);
        assert_eq!(m.lights().brightness[9], [1.0, 0.0]);
        m.initialize_state();
        assert!(!m.runtime.overflow);
    }
    #[test]
    fn adoption_initialization_and_button_cleanup_keep_pending_actions_local() {
        let mut old = model(json!({"tempo":0}), 44100);
        old.enqueue(ModuleAction::TapPress);
        let mut receiver = model(json!({"tempo":0}), 44100);
        receiver.adopt(&old);
        assert_eq!(receiver.action_count, 0);
        assert!(!receiver.controls().pressed);
        for _ in 0..6 {
            tick(&mut old, 5.0, 0.0);
        }
        receiver.enqueue(ModuleAction::TapRelease);
        receiver.adopt(&old);
        tick(&mut receiver, 5.0, 0.0);
        assert!(!receiver.controls().pressed);
        receiver.release();
        assert!(receiver
            .runtime
            .events
            .iter()
            .all(|e| e.levels & 4 == 0 && e.commands == 0));
        let mut reseeded = model(json!({"seed":33}), 44100);
        reseeded.adopt(&old);
        assert!(!reseeded.runtime.scanned);
        assert_eq!(reseeded.runtime.pattern.rng_state(), 33);
        let mut rate = model(json!({}), 48000);
        rate.adopt(&old);
        assert!(!rate.runtime.scanned);
        receiver.initialize_state();
        assert!(!receiver.runtime.scanned);
        assert_eq!(receiver.action_count, 0);
    }
    #[test]
    fn source_bytes_euclidean_lengths_and_resolutions_are_validated_and_quantized() {
        for p in [
            json!({"seed":0}),
            json!({"seed":65536}),
            json!({"tempo":120.5}),
            json!({"resolution":3}),
            json!({"mode":0.5}),
            json!({"swing":0.1}),
            json!({"unknown":0}),
        ] {
            assert!(RhythmTerrain::build(&node(p), 48000).is_err());
        }
        let mut m = model(json!({"tempo":0,"mode":0,"x":1,"y":0,"chaos":0.5}), 8000);
        tick(&mut m, 5.0, 0.0);
        assert_eq!(m.runtime.pattern.settings.options, [255, 0, 128]);
        assert_eq!(
            m.runtime.pattern.settings.options.map(|v| (v >> 3) + 1),
            [32, 1, 17]
        );
        let mut changed = model(json!({"tempo":0,"resolution":0,"mode":0}), 8000);
        changed.adopt(&m);
        tick(&mut changed, 5.0, 0.0);
        let snapshot = changed.runtime.pattern.snapshot();
        assert_eq!(
            (snapshot.step, snapshot.pulse, snapshot.euclidean_step),
            (0, 0, [0; 3])
        );
        let mut inputs = [PortFrame::mono(f32::INFINITY); 8];
        inputs[2] = PortFrame::mono(f32::NAN);
        let mut outputs = [PortFrame::default(); 8];
        changed.process(&inputs, 255, &mut outputs);
        assert!(outputs.iter().all(|o| matches!(o.channel(0), 0.0 | 5.0)));
    }
}
