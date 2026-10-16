//! Original polyphonic edge, flip-flop, timed gate and transition-delay utility.
//! History storage is fixed at compile time; render/adoption never allocate.
use crate::{
    parameter, ModuleAction, ModuleControlState, ModuleLightState, Node, PortFrame, MAX_CHANNELS,
    MAX_MODULE_ACTIONS_PER_SAMPLE, MAX_MODULE_LIGHTS,
};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 3] = ["length", "in", "reset"];
pub(crate) const OUTPUTS: [&str; 6] = ["rise", "fall", "flip", "flop", "gate", "delay"];
const HISTORY_CAPACITY: usize = 1023;
const REBASE_THRESHOLD: u64 = 1 << 63;
const REBASE_TO: u64 = 1 << 62;

pub(crate) fn catalog() -> Value {
    json!({"kind":"gates","title":"GATES","model":"functional","inputs":INPUTS,"params":{"length":{"default":0.1,"min":0.001,"max":10.0,"unit":"s"}},"description":"Polyphonic rising/falling triggers, complementary flip/flop, retriggerable timed gate and variable gate delay. IN controls width; LENGTH CV scales seconds exponentially. RESET clears flip/flop, timed gate and delay history after evaluating the current IN edge, preserving edge triggers and IN state."})
}
pub(crate) fn annotate(module: &mut Value) {
    module["runtime_actions"] = json!(["reset_press", "reset_release"]);
    module["polyphony"]["version_2"] = json!("max(IN channel count,1); IN and RESET read channel-local values without mono broadcast; mono LENGTH broadcasts, short poly LENGTH lanes zero-pad");
    module["polyphony"]["primary_input"] = json!("in");
    module["polyphony"]["retired_voices"] = json!("inactive lane IN/reset detectors, pulses, gate age, flip/flop and retained transitions freeze; module sample clock continues, so delay history timestamps age while inactive");
    module["manual_control"] = json!({"buttons":["reset"],"saved":false,"capacity_per_sample":MAX_MODULE_ACTIONS_PER_SAMPLE,"ordering":"ordered physical actions capture every rising RESET press even if released in the same sample; evaluate current IN edge first, then CV/manual RESET on each active lane","release_safety":"matching release or emergency release clears held/pending physical controls without changing DSP histories"});
    module["length_policy"] = json!({"formula":"f64(validated f32 length) * 2^clamp(finite LENGTH CV,-32,32) seconds; nonfinite CV becomes zero","gate":"on IN rise set integer age to zero; increment age before testing age >= duration * sample_rate; expiry stays off until next IN rise; current LENGTH applies every active sample","delay":"latest retained transition whose integer sample age >= duration * sample_rate; query before inserting current transition; even a sub-sample duration has at least one sample of delay","rounding":"no repeated floating time accumulation; actual f32 parameter promoted to f64; no reference float/polynomial bit-equivalence claim"});
    module["edge_policy"] = json!({"in":"starts LOW; rise >=2 V, fall <=0.1 V, middle holds; initial HIGH is a real rise; nonfinite becomes zero","reset":"unknown initial Schmitt state; first HIGH arms without reset; low <=0.1 V then high >=2 V fires; CV is lane-local; physical rising press resets all active lanes","order":"IN transition/toggle/retrigger, then RESET clears flop/gate/history, then outputs/delay lookup, then insert current IN transition if any","triggers":"mono/poly 10 V edge pulses for ceil(sample_rate/1000) active samples including edge sample; reset does not cancel RISE/FALL pulses","flip_flop":"FLOP toggles on each IN rise; FLIP is complement; RESET forces FLOP low"});
    module["delay_history"] = json!({"capacity_per_voice":HISTORY_CAPACITY,"voices":MAX_CHANNELS,"entry_bytes":std::mem::size_of::<Transition>(),"allocated_transition_bytes":std::mem::size_of::<Transition>()*HISTORY_CAPACITY*MAX_CHANNELS,"storage":"fixed circular buffers allocated at compile; binary timestamp lookup never consumes history; oldest transition evicted at capacity","connected_policy":"always record DELAY history, including when output unconnected; explicit OpenPatch policy differing from connection-dependent reference recording","reset":"clear before query, then insert same-sample IN transition afterward","clock":"u64 module sample clock; before 2^63 rebase to 2^62 and saturating-subtract offset from timestamps; collapsed old entries remain ordered and older than maximum supported duration"});
    module["light_names"] = json!(OUTPUTS);
    module["light_policy"] = json!({"pairs":"mono index zero, poly index one; any active lane lights each output","divider":32,"brightness":"immediate upward changes, otherwise decay by value += (target-value)*30*32/sample_rate","manual":"current physical RESET held state; no seventh output pair"});
    module["initialization_policy"] = json!("parameter Initialize restores LENGTH and host releases physical controls; all detector, flop, pulse, gate and delay histories remain; only a fresh instance/reset_module_state clears them");
    module["state_adoption"] = json!("same ID/kind/sample rate preserves all lane histories, inactive lanes, detectors, sample clock and lights with new length; receiver pending actions stay local and old pending actions are not adopted; bypass transitions release physical controls; bypass silences outputs and freezes state; rate/kind changes start fresh");
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Gates.cpp");
    module["compatibility"] = json!("original Rust functional implementation; seconds parameter, bounded CV, integer sample timing, fixed nonconsuming always-recorded history and intentional first-sample manual actions are explicit OpenPatch policies; no GPL source copied or sample-identical claim");
}
fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct Schmitt(u8); // unknown, low, high
impl Schmitt {
    fn rising(&mut self, value: f32) -> bool {
        let value = finite(value);
        let rising = self.0 == 1 && value >= 2.0;
        if value >= 2.0 {
            self.0 = 2;
        } else if value <= 0.1 {
            self.0 = 1;
        }
        rising
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct Transition {
    frame: u64,
    high: bool,
}
#[derive(Debug)]
struct History {
    entries: Box<[Transition]>,
    head: usize,
    len: usize,
}
impl History {
    fn new() -> Self {
        Self {
            entries: vec![Transition::default(); HISTORY_CAPACITY].into_boxed_slice(),
            head: 0,
            len: 0,
        }
    }
    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }
    fn at(&self, index: usize) -> Transition {
        self.entries[(self.head + index) % HISTORY_CAPACITY]
    }
    fn push(&mut self, event: Transition) {
        if self.len == HISTORY_CAPACITY {
            self.entries[self.head] = event;
            self.head = (self.head + 1) % HISTORY_CAPACITY;
        } else {
            self.entries[(self.head + self.len) % HISTORY_CAPACITY] = event;
            self.len += 1;
        }
    }
    fn query(&self, now: u64, duration_samples: f64) -> bool {
        let (mut low, mut high) = (0, self.len);
        while low < high {
            let middle = low + (high - low) / 2;
            if now.saturating_sub(self.at(middle).frame) as f64 >= duration_samples {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low > 0 && self.at(low - 1).high
    }
    fn rebase(&mut self, offset: u64) {
        for index in 0..self.len {
            let event = &mut self.entries[(self.head + index) % HISTORY_CAPACITY];
            event.frame = event.frame.saturating_sub(offset);
        }
    }
    fn adopt(&mut self, old: &Self) {
        // Only retained slots are observable. Copy at most two slices, keeping
        // the old physical head so the next push overwrites an evicted slot
        // before it becomes visible; empty dormant lanes need no buffer copy.
        let first = old.len.min(HISTORY_CAPACITY - old.head);
        self.entries[old.head..old.head + first]
            .copy_from_slice(&old.entries[old.head..old.head + first]);
        let second = old.len - first;
        self.entries[..second].copy_from_slice(&old.entries[..second]);
        self.head = old.head;
        self.len = old.len;
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct VoiceState {
    high: bool,
    reset: Schmitt,
    rise: u32,
    fall: u32,
    flop: bool,
    gate_age: Option<u64>,
}
#[derive(Debug)]
struct Voice {
    state: VoiceState,
    history: History,
}
impl Voice {
    fn new() -> Self {
        Self {
            state: VoiceState::default(),
            history: History::new(),
        }
    }
    fn process(
        &mut self,
        input: f32,
        reset: f32,
        manual_reset: bool,
        duration_samples: f64,
        now: u64,
        pulse_samples: u32,
    ) -> [bool; 6] {
        let input = finite(input);
        let state = &mut self.state;
        let changed = if state.high && input <= 0.1 {
            state.high = false;
            state.fall = pulse_samples;
            true
        } else if !state.high && input >= 2.0 {
            state.high = true;
            state.rise = pulse_samples;
            state.flop = !state.flop;
            state.gate_age = Some(0);
            true
        } else {
            false
        };
        // Evaluate the CV detector even when a manual reset is present.
        let reset = state.reset.rising(reset) || manual_reset;
        if reset {
            state.flop = false;
            state.gate_age = None;
            self.history.clear();
        }
        if let Some(age) = &mut state.gate_age {
            *age = age.saturating_add(1);
            if *age as f64 >= duration_samples {
                state.gate_age = None;
            }
        }
        let output = [
            state.rise > 0,
            state.fall > 0,
            !state.flop,
            state.flop,
            state.gate_age.is_some(),
            self.history.query(now, duration_samples),
        ];
        if changed {
            self.history.push(Transition {
                frame: now,
                high: state.high,
            });
        }
        state.rise = state.rise.saturating_sub(1);
        state.fall = state.fall.saturating_sub(1);
        output
    }
}
#[derive(Debug)]
pub(crate) struct Gates {
    length: f64,
    rate: f64,
    pulse_samples: u32,
    frame: u64,
    voices: [Voice; MAX_CHANNELS],
    pressed: bool,
    actions: [ModuleAction; MAX_MODULE_ACTIONS_PER_SAMPLE],
    count: usize,
    light_tick: u8,
    lights: [[f64; 2]; MAX_MODULE_LIGHTS],
}
impl Gates {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for name in node.params.keys() {
            if name != "length" {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        let length = f64::from(parameter(node, "length", 0.1, 0.001, 10.0)?);
        Ok(Self {
            length,
            rate: f64::from(rate),
            pulse_samples: rate.div_ceil(1000),
            frame: 0,
            voices: std::array::from_fn(|_| Voice::new()),
            pressed: false,
            actions: [ModuleAction::ResetRelease; MAX_MODULE_ACTIONS_PER_SAMPLE],
            count: 0,
            light_tick: 0,
            lights: [[0.0; 2]; MAX_MODULE_LIGHTS],
        })
    }
    pub(crate) fn supports(action: ModuleAction) -> bool {
        matches!(
            action,
            ModuleAction::ResetPress | ModuleAction::ResetRelease
        )
    }
    pub(crate) fn enqueue(&mut self, action: ModuleAction) -> bool {
        if !Self::supports(action) || self.count == self.actions.len() {
            return false;
        }
        self.actions[self.count] = action;
        self.count += 1;
        true
    }
    pub(crate) fn release(&mut self) {
        self.pressed = false;
        self.count = 0;
    }
    pub(crate) fn controls(&self) -> ModuleControlState {
        ModuleControlState {
            pressed: self.pressed,
            pressed_mask: u16::from(self.pressed),
        }
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        ModuleLightState {
            input_channels: None,
            output_channels: None,
            automatic_channels: None,
            clock_frequency: None,
            playing_notes: None,
            waveform_position: None,
            count: 6,
            brightness: self.lights.map(|pair| pair.map(|value| value as f32)),
            manual: self.pressed,
        }
    }
    pub(crate) fn reset_state(&mut self) {
        self.release();
        self.frame = 0;
        self.light_tick = 0;
        self.lights.fill([0.0; 2]);
        for voice in &mut self.voices {
            voice.state = VoiceState::default();
            voice.history.clear();
        }
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        self.frame = old.frame;
        self.pressed = old.pressed;
        self.light_tick = old.light_tick;
        self.lights = old.lights;
        for (voice, previous) in self.voices.iter_mut().zip(&old.voices) {
            voice.state = previous.state;
            voice.history.adopt(&previous.history);
        }
    }
    fn rebase_if_needed(&mut self) {
        if self.frame >= REBASE_THRESHOLD {
            let offset = self.frame - REBASE_TO;
            for voice in &mut self.voices {
                voice.history.rebase(offset);
            }
            self.frame = REBASE_TO;
        }
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame; 3], outputs: &mut [PortFrame]) {
        self.rebase_if_needed();
        let mut manual_reset = false;
        for action in &self.actions[..self.count] {
            let press = *action == ModuleAction::ResetPress;
            manual_reset |= press && !self.pressed;
            self.pressed = press;
        }
        self.count = 0;
        let width = usize::from(inputs[1].channels.max(1)).min(MAX_CHANNELS);
        for output in outputs.iter_mut() {
            *output = PortFrame {
                channels: width as u8,
                ..PortFrame::default()
            };
        }
        let mut active = [false; 6];
        for (lane, voice) in self.voices[..width].iter_mut().enumerate() {
            let cv = finite(inputs[0].voice(lane, 0.0)).clamp(-32.0, 32.0);
            let duration_samples = self.length * 2.0_f64.powf(f64::from(cv)) * self.rate;
            let values = voice.process(
                inputs[1].channel(lane),
                inputs[2].channel(lane),
                manual_reset,
                duration_samples,
                self.frame,
                self.pulse_samples,
            );
            for (index, value) in values.into_iter().enumerate() {
                outputs[index].values[lane] = if value { 10.0 } else { 0.0 };
                active[index] |= value;
            }
        }
        self.light_tick += 1;
        if self.light_tick == 32 {
            self.light_tick = 0;
            let decay = 30.0 * 32.0 / self.rate;
            for (index, pair) in self.lights[..6].iter_mut().enumerate() {
                for (poly, value) in pair.iter_mut().enumerate() {
                    let target = if active[index] && ((width > 1) == (poly == 1)) {
                        1.0
                    } else {
                        0.0
                    };
                    *value = if target >= *value {
                        target
                    } else {
                        (*value + (target - *value) * decay).max(0.0)
                    };
                }
            }
        }
        self.frame += 1; // rebasing above keeps this addition strictly bounded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(rate: u32, length: f32) -> Gates {
        Gates::build(
            &Node {
                content: None,
                score: None,
                id: "g".into(),
                kind: "gates".into(),
                bypassed: false,
                params: [("length".into(), length)].into(),
            },
            rate,
        )
        .unwrap()
    }
    fn frame(values: &[f32]) -> PortFrame {
        let mut frame = PortFrame {
            channels: values.len() as u8,
            ..PortFrame::default()
        };
        frame.values[..values.len()].copy_from_slice(values);
        frame
    }
    fn sample(
        g: &mut Gates,
        input: PortFrame,
        length: PortFrame,
        reset: PortFrame,
    ) -> [PortFrame; 6] {
        let mut out = [PortFrame::default(); 6];
        g.process(&[length, input, reset], &mut out);
        assert!(out.iter().all(|p| p.values[usize::from(p.channels)..]
            .iter()
            .all(|v| *v == 0.0)));
        out
    }
    fn mono(g: &mut Gates, input: f32, length: f32, reset: f32) -> [f32; 6] {
        sample(
            g,
            PortFrame::mono(input),
            PortFrame::mono(length),
            PortFrame::mono(reset),
        )
        .map(|p| p.values[0])
    }
    #[test]
    fn compile_allocates_fixed_history_capacity_and_known_storage_size() {
        let g = model(48_000, 0.1);
        let bytes = g
            .voices
            .iter()
            .map(|voice| voice.history.entries.len() * std::mem::size_of::<Transition>())
            .sum::<usize>();
        assert!(g
            .voices
            .iter()
            .all(|voice| voice.history.entries.len() == HISTORY_CAPACITY));
        assert_eq!(
            bytes,
            MAX_CHANNELS * HISTORY_CAPACITY * std::mem::size_of::<Transition>()
        );
        #[cfg(target_pointer_width = "64")]
        assert_eq!(bytes, 261_888);
    }
    #[test]
    fn binary_history_queries_rewind_without_consuming_and_choose_last_equal_timestamp() {
        let mut history = History::new();
        for (frame, high) in [(10, true), (20, false), (30, true), (40, false)] {
            history.push(Transition { frame, high });
        }
        for _ in 0..5 {
            for (duration, expected) in [
                (10.0, false),
                (11.0, true),
                (30.0, false),
                (31.0, true),
                (41.0, false),
            ] {
                assert_eq!(history.query(50, duration), expected);
            }
        }
        assert_eq!(history.len, 4);
        history.push(Transition {
            frame: 40,
            high: true,
        });
        assert!(history.query(50, 10.0));
    }
    #[test]
    fn bounded_history_eviction_and_wrapped_adoption_match_independent_scan() {
        let mut history = History::new();
        let mut events = Vec::new();
        for frame in 0..2400 {
            let event = Transition {
                frame,
                high: frame % 3 == 0,
            };
            history.push(event);
            events.push(event);
        }
        assert_eq!(history.len, HISTORY_CAPACITY);
        assert!(history.head > 0);
        let mut adopted = History::new();
        adopted.adopt(&history);
        for duration in [1.0, 2.0, 2.5, 100.0, 1023.0, 1024.0, 5000.0] {
            let expected = events[events.len() - HISTORY_CAPACITY..]
                .iter()
                .rev()
                .find(|event| 2400.0 - event.frame as f64 >= duration)
                .is_some_and(|event| event.high);
            assert_eq!(history.query(2400, duration), expected);
            assert_eq!(adopted.query(2400, duration), expected);
        }
        history.push(Transition {
            frame: 2400,
            high: true,
        });
        adopted.push(Transition {
            frame: 2400,
            high: true,
        });
        assert_eq!(history.head, adopted.head);
        for i in 0..HISTORY_CAPACITY {
            assert_eq!(history.at(i).frame, adopted.at(i).frame);
            assert_eq!(history.at(i).high, adopted.at(i).high);
        }
        // Adopting empty history after full history must hide every stale slot.
        adopted.adopt(&History::new());
        assert!(!adopted.query(u64::MAX, 1.0));
        adopted.push(Transition {
            frame: 4,
            high: true,
        });
        assert!(adopted.query(5, 1.0));
    }
    #[test]
    fn initial_in_high_is_a_rise_reset_initial_high_only_arms_and_edge_precedes_reset() {
        let mut g = model(48_000, 0.001);
        let first = mono(&mut g, 2.0, 0.0, 2.0);
        assert_eq!(first, [10.0, 0.0, 0.0, 10.0, 10.0, 0.0]);
        mono(&mut g, 0.1, 0.0, 0.1);
        let reset = mono(&mut g, 2.0, 0.0, 2.0);
        assert_eq!(reset, [10.0, 10.0, 10.0, 0.0, 0.0, 0.0]);
        assert_eq!(g.voices[0].history.len, 1);
        assert!(g.voices[0].history.at(0).high);
        // The same-frame rise survives reset and is inserted after its clear.
        assert_eq!(mono(&mut g, 2.0, -32.0, 2.0)[5], 10.0);
        g.enqueue(ModuleAction::ResetPress);
        g.enqueue(ModuleAction::ResetRelease);
        let reset = mono(&mut g, 2.0, 0.0, 2.0);
        assert_eq!(reset[0], 10.0);
        assert_eq!(reset[2], 10.0);
        assert_eq!(reset[3], 0.0);
        assert_eq!(reset[4], 0.0);
        assert_eq!(reset[5], 0.0);
        assert_eq!(g.voices[0].history.len, 0);
        assert!(!g.controls().pressed);
    }
    #[test]
    fn pulse_and_gate_boundaries_and_delay_query_timing_at_three_rates() {
        for rate in [44_100, 48_000, 96_000] {
            let mut g = model(rate, 0.001);
            let duration = f64::from(0.001_f32) * f64::from(rate);
            let gate_high = duration.ceil() as usize - 1;
            let delayed = duration.ceil() as usize;
            let pulse = rate.div_ceil(1000) as usize;
            for index in 0..delayed + 3 {
                let out = mono(&mut g, 2.0, 0.0, 0.0);
                assert_eq!(
                    out[0],
                    if index < pulse { 10.0 } else { 0.0 },
                    "{rate} rise at {index}"
                );
                assert_eq!(
                    out[4],
                    if index < gate_high { 10.0 } else { 0.0 },
                    "{rate} gate at {index}"
                );
                assert_eq!(
                    out[5],
                    if index >= delayed { 10.0 } else { 0.0 },
                    "{rate} delay at {index}"
                );
            }
            for index in 0..pulse + 1 {
                assert_eq!(
                    mono(&mut g, 0.1, 0.0, 0.0)[1],
                    if index < pulse { 10.0 } else { 0.0 }
                );
            }
        }
        // Exactly representable length/rate: rise age becomes one immediately.
        let mut g = model(8192, 1.0 / 512.0);
        for index in 0..17 {
            assert_eq!(
                mono(&mut g, 2.0, 0.0, 0.0)[4],
                if index < 15 { 10.0 } else { 0.0 }
            );
        }
    }
    #[test]
    fn variable_length_queries_can_rewind_and_expired_gates_do_not_revive() {
        let mut g = model(8000, 0.001);
        for _ in 0..10 {
            mono(&mut g, 2.0, 0.0, 0.0);
        }
        for _ in 0..10 {
            mono(&mut g, 0.0, 0.0, 0.0);
        }
        assert_eq!(mono(&mut g, 0.0, 0.0, 0.0)[5], 0.0); // target after fall
        assert_eq!(mono(&mut g, 0.0, 1.0, 0.0)[5], 10.0); // length increase rewinds before fall
        assert_eq!(mono(&mut g, 0.0, 2.0, 0.0)[5], 0.0); // before first event
        assert_eq!(mono(&mut g, 2.0, -32.0, 0.0)[4], 0.0); // sub-sample gate expires on rise
        assert_eq!(mono(&mut g, 2.0, 32.0, 0.0)[4], 0.0); // extending cannot revive expired gate
        let mut a = model(48_000, 0.1);
        let mut b = model(48_000, 0.1);
        for (raw, bounded) in [
            (f32::NAN, 0.0),
            (f32::INFINITY, 0.0),
            (f32::NEG_INFINITY, 0.0),
            (f32::MAX, 32.0),
            (-f32::MAX, -32.0),
        ] {
            a.reset_state();
            b.reset_state();
            assert_eq!(mono(&mut a, 2.0, raw, 0.0), mono(&mut b, 2.0, bounded, 0.0));
        }
    }
    #[test]
    fn poly_width_length_broadcast_reset_lane_local_and_dormant_state_retention() {
        let mut g = model(48_000, 0.001);
        let zeros = frame(&[0.0; 16]);
        let highs = frame(&[2.0; 16]);
        sample(&mut g, zeros, PortFrame::mono(1.0), PortFrame::mono(0.0));
        let rising = sample(&mut g, highs, PortFrame::mono(1.0), PortFrame::mono(0.0));
        assert!(rising.iter().all(|p| p.channels == 16));
        assert!(rising[3].values.iter().all(|v| *v == 10.0));
        let reset = sample(&mut g, highs, PortFrame::mono(1.0), PortFrame::mono(2.0));
        assert_eq!(reset[3].values[0], 0.0);
        assert!(reset[3].values[1..].iter().all(|v| *v == 10.0));
        assert_eq!(g.voices[15].state.gate_age, Some(2));
        for _ in 0..120 {
            mono(&mut g, 0.0, 0.0, 0.0);
        }
        assert_eq!(g.voices[15].state.gate_age, Some(2));
        assert_eq!(g.voices[15].state.rise, 46);
        let resumed = sample(&mut g, highs, PortFrame::mono(1.0), PortFrame::mono(0.0));
        assert_eq!(resumed[0].values[15], 10.0);
        assert_eq!(resumed[4].values[15], 10.0);
        assert_eq!(resumed[5].values[15], 10.0);
        assert_eq!(g.voices[15].state.gate_age, Some(3));
        // Two-channel LENGTH does not broadcast into lane three.
        let mut g = model(8000, 0.001);
        sample(
            &mut g,
            frame(&[2.0; 3]),
            frame(&[-32.0, -32.0]),
            PortFrame::default(),
        );
        assert_eq!(g.voices[0].state.gate_age, None);
        assert_eq!(g.voices[1].state.gate_age, None);
        assert_eq!(g.voices[2].state.gate_age, Some(1));
    }
    #[test]
    fn bounded_ordered_manual_reset_repeated_press_release_and_receiver_queue() {
        let mut g = model(48_000, 0.1);
        mono(&mut g, 2.0, 0.0, 0.0);
        assert!(g.voices[0].state.flop);
        for _ in 0..32 {
            assert!(g.enqueue(ModuleAction::ResetPress));
            assert!(g.enqueue(ModuleAction::ResetRelease));
        }
        assert!(!g.enqueue(ModuleAction::ResetPress));
        assert!(!g.enqueue(ModuleAction::Reset));
        mono(&mut g, 2.0, 0.0, 0.0);
        assert!(!g.voices[0].state.flop);
        assert!(!g.controls().pressed);
        g.enqueue(ModuleAction::ResetPress);
        mono(&mut g, 0.0, 0.0, 0.0);
        g.enqueue(ModuleAction::ResetPress);
        mono(&mut g, 2.0, 0.0, 0.0);
        assert!(g.voices[0].state.flop); // repeated held press cannot reset new rise
        assert_eq!(g.controls().pressed_mask, 1);
        g.enqueue(ModuleAction::ResetRelease);
        g.enqueue(ModuleAction::ResetPress);
        g.release();
        assert!(g.voices[0].state.flop);
        assert_eq!(g.voices[0].history.len, 2);
        assert!(!g.controls().pressed);
        assert_eq!(g.count, 0);
        let mut adopted = model(48_000, 0.1);
        adopted.enqueue(ModuleAction::ResetRelease);
        g.enqueue(ModuleAction::ResetPress);
        adopted.adopt(&g);
        mono(&mut adopted, 2.0, 0.0, 0.0);
        assert!(adopted.voices[0].state.flop);
        assert!(!adopted.controls().pressed);
    }
    #[test]
    fn rebase_preserves_old_baseline_new_transition_and_saturates_gate_age() {
        let mut g = model(192_000, 10.0);
        g.frame = REBASE_THRESHOLD;
        g.voices[0].history.push(Transition {
            frame: 0,
            high: true,
        });
        g.voices[0].history.push(Transition {
            frame: REBASE_THRESHOLD - 3,
            high: false,
        });
        assert!(g.voices[0].history.query(g.frame, 4.0));
        assert!(!g.voices[0].history.query(g.frame, 3.0));
        g.rebase_if_needed();
        assert_eq!(g.frame, REBASE_TO);
        assert!(g.voices[0].history.query(g.frame, 4.0));
        assert!(!g.voices[0].history.query(g.frame, 3.0));
        assert!(g.voices[0]
            .history
            .query(g.frame, 10.0 * 2.0_f64.powi(32) * 192_000.0));
        g.frame = u64::MAX;
        g.voices[0].state.gate_age = Some(u64::MAX);
        mono(&mut g, 0.0, 32.0, 0.0);
        assert_eq!(g.frame, REBASE_TO + 1);
        assert_eq!(g.voices[0].state.gate_age, None);
    }
    #[test]
    fn light_pairs_follow_output_width_and_manual_release_preserves_dsp() {
        let mut g = model(48_000, 0.1);
        g.enqueue(ModuleAction::ResetPress);
        mono(&mut g, 0.0, 0.0, 0.0);
        for _ in 1..32 {
            mono(&mut g, 0.0, 0.0, 0.0);
        }
        assert_eq!(g.lights().count, 6);
        assert_eq!(g.lights().brightness[2], [1.0, 0.0]);
        assert!(g.lights().manual);
        g.release();
        assert!(!g.lights().manual);
        assert_eq!(g.lights().brightness[2], [1.0, 0.0]);
        for _ in 0..32 {
            sample(
                &mut g,
                frame(&[0.0; 16]),
                PortFrame::default(),
                PortFrame::default(),
            );
        }
        assert_eq!(g.lights().brightness[2][1], 1.0);
        assert!(g.lights().brightness[2][0] > 0.0 && g.lights().brightness[2][0] < 1.0);
        assert!(g.lights().brightness[6..]
            .iter()
            .all(|pair| *pair == [0.0; 2]));
    }
}
