//! Original Rust implementation of the bounded stock LFO behavioral contract.
//! Rounded exponential coefficients and scalar Horner evaluation follow the
//! numerical method already qualified in OpenPatch's wavetable oscillator.
//! No upstream implementation or panel artwork is incorporated here.
use crate::{parameter, ModuleLightState, Node, PortFrame, MAX_CHANNELS};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 4] = ["fm", "pwm", "reset", "clock"];
pub(crate) const OUTPUTS: [&str; 4] = ["sine", "triangle", "saw", "square"];

pub(crate) fn catalog() -> Value {
    json!({"kind":"stock_lfo","title":"STOCK LFO","model":"functional","description":"Four unfiltered stock-style CV waveforms with shared clock timing, lane reset and observed phase/mode lamps. Advance phase, apply reset, then emit. Defaults to unipolar; existing multi_lfo and lfo retain their contracts.","inputs":INPUTS,"params":{
        "rate":{"default":1.0,"min":-8.0,"max":10.0,"unit":"log2(Hz)","description":"Unpatched: 2^rate Hz; patched clock: measured frequency × 2^rate / 2"},
        "pulse_width":{"default":0.5,"min":0.01,"max":0.99,"unit":"ratio"},
        "fm_amount":{"default":0.0,"min":-1.0,"max":1.0,"description":"Signed exponential FM depth, one octave/V at +1; Randomize disabled"},
        "pwm_amount":{"default":0.0,"min":-1.0,"max":1.0,"description":"Width += PWM/10 × amount; Randomize disabled"},
        "offset":{"default":1.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1],"labels":["Bipolar","Unipolar"]},
        "invert":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1],"labels":["Off","On"]}
    }})
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/LFO.cpp");
    module["polyphony"]["version_2"] = json!("max(FM width,1); FM reads raw lanes without mono broadcast; PWM/reset mono broadcast and missing short-poly lanes read zero; clock is shared channel zero");
    module["polyphony"]["primary_input"] = json!("fm");
    module["polyphony"]["retired_voices"] = json!("hidden lanes in an active four-lane group continue processing; wholly inactive groups freeze and resume phase/reset state; inactive output storage is zero");
    module["frequency_policy"] = json!("clock_frequency / 2 × fifth-order exp2(rate + rawFM × amount); phase increment limited to half a cycle/sample; no through-zero frequency control; exponent bounded to [-127,128]");
    module["clock_policy"] = json!("shared f32 timer advances only with a nonzero-channel clock; scalar Schmitt starts uninitialized with thresholds0.1/2 V; initial high arms without firing; a low-to-high edge measures1/elapsed and resets timer even for rejected measurements; accept0.001..1000 Hz; disconnected sets clock frequency2 and freezes timer/detector");
    module["reset_policy"] = json!("lane detectors start high, thresholds0.1/2 V; advance, reset phase on a low-to-high edge, then emit; Initialize preserves detector state");
    module["light_names"] = json!(["phase_red", "phase_green", "phase_blue", "invert", "offset"]);
    module["light_policy"] = json!("five primary brightness components updated every16 processed frames: mono red/green phase ramp with immediate rise and30/s Euler release, or poly blue; then invert and offset; clock_frequency is actual shared finite positive clock rate, updated every processed frame; bypass freezes divider/lamps/timing");
    module["initialization_policy"] = json!("reset all phases, clock frequency to1 and timer to0; retain reset/clock detector histories, lamp values and divider phase; control defaults restored separately");
    module["state_adoption"] = json!("same identity/kind/sample rate adopts all bounded runtime phase, detector, timing and lamp state; receiving parameters win; source edits preserve detector histories; fresh instances and rate changes reset runtime");
    module["consumer_policy"] = json!("only cable, selected-output and temporary tap consumers activate an output; disconnected outputs report zero channels and zero host storage; their source raw stale padding is outside voltage comparison; phase/timing/lights continue regardless of consumers");
    module["arithmetic_policy"] = json!("f32 scalar operations, nearest-even waveform rounding and rounded fifth-order exponential; nonfinite inputs become zero, finite CV is preserved and the effective exponent is bounded to [-127,128]; unfiltered discontinuities intentionally alias; no SIMD sine bit-exactness, hardware or full host parity claim");
}

fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

// This generic polynomial method and its rounded coefficients are shared with
// the previously qualified original scalar wavetable implementation. Bounding
// the exponent keeps bit construction in the bounded source-defined SIMD
// region: exponent zero gives zero and exponent255 gives infinity, which the
// half-cycle phase cap handles. Extreme native CV saturates to this interval.
#[allow(clippy::excessive_precision)]
fn exponential(pitch: f32) -> f32 {
    let biased = pitch.clamp(-127.0, 128.0) + 127.0;
    let whole = biased as u32;
    let fraction = biased - whole as f32;
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
    f32::from_bits(whole << 23) * value
}

/// Read-only reference diagnostics of actual bounded production runtime.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct StockLfoSnapshot {
    pub phases: [f32; MAX_CHANNELS],
    pub reset_high: [bool; MAX_CHANNELS],
    pub clock_trigger: u8,
    pub clock_frequency: f32,
    pub clock_timer: f32,
    pub light_divider: u8,
    pub lights: [f32; 5],
}
impl Default for StockLfoSnapshot {
    fn default() -> Self {
        Self {
            phases: [0.0; MAX_CHANNELS],
            reset_high: [true; MAX_CHANNELS],
            clock_trigger: 2,
            clock_frequency: 1.0,
            clock_timer: 0.0,
            light_divider: 0,
            lights: [0.0; 5],
        }
    }
}

#[derive(Debug)]
pub(crate) struct StockLfo {
    sample_time: f32,
    rate: f32,
    fm_amount: f32,
    pulse_width: f32,
    pwm_amount: f32,
    offset: bool,
    invert: bool,
    runtime: StockLfoSnapshot,
}

impl StockLfo {
    pub(crate) fn build(node: &Node, sample_rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err("stock_lfo sample rate must be in8000..=192000 Hz".into());
        }
        for key in node.params.keys() {
            if ![
                "rate",
                "pulse_width",
                "fm_amount",
                "pwm_amount",
                "offset",
                "invert",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let boolean = |name, default| {
            let value = parameter(node, name, default, 0.0, 1.0)?;
            if value.fract() != 0.0 {
                return Err(format!(
                    "node '{}': parameter '{name}' must be an integer",
                    node.id
                ));
            }
            Ok(value != 0.0)
        };
        Ok(Self {
            sample_time: 1.0 / sample_rate as f32,
            rate: parameter(node, "rate", 1.0, -8.0, 10.0)?,
            pulse_width: parameter(node, "pulse_width", 0.5, 0.01, 0.99)?,
            fm_amount: parameter(node, "fm_amount", 0.0, -1.0, 1.0)?,
            pwm_amount: parameter(node, "pwm_amount", 0.0, -1.0, 1.0)?,
            offset: boolean("offset", 1.0)?,
            invert: boolean("invert", 0.0)?,
            runtime: StockLfoSnapshot::default(),
        })
    }

    pub(crate) fn adopt(&mut self, previous: &Self) {
        if self.sample_time == previous.sample_time {
            self.runtime = previous.runtime;
        }
    }

    pub(crate) fn initialize_state(&mut self) {
        self.runtime.phases.fill(0.0);
        self.runtime.clock_frequency = 1.0;
        self.runtime.clock_timer = 0.0;
    }

    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut state = ModuleLightState {
            clock_frequency: Some(self.runtime.clock_frequency),
            count: 5,
            ..ModuleLightState::default()
        };
        for (index, value) in self.runtime.lights.iter().enumerate() {
            state.brightness[index][0] = *value;
        }
        state
    }

    fn clock(&mut self, input: &PortFrame) {
        if input.channels == 0 {
            self.runtime.clock_frequency = 2.0;
            return;
        }
        self.runtime.clock_timer += self.sample_time;
        let voltage = finite(input.channel(0));
        let rising = self.runtime.clock_trigger == 0 && voltage >= 2.0;
        if voltage >= 2.0 {
            self.runtime.clock_trigger = 1;
        } else if voltage <= 0.1 {
            self.runtime.clock_trigger = 0;
        }
        if rising {
            let measured = 1.0 / self.runtime.clock_timer;
            self.runtime.clock_timer = 0.0;
            if (0.001..=1000.0).contains(&measured) {
                self.runtime.clock_frequency = measured;
            }
        }
    }

    pub(crate) fn process(
        &mut self,
        inputs: &[PortFrame; 4],
        connected_mask: u16,
        outputs: &mut [PortFrame],
    ) {
        self.clock(&inputs[3]);
        let channels = inputs[0].channels.clamp(1, MAX_CHANNELS as u8);
        for (index, output) in outputs.iter_mut().enumerate() {
            *output = PortFrame {
                channels: if connected_mask & (1 << index) != 0 {
                    channels
                } else {
                    0
                },
                ..PortFrame::default()
            };
        }
        for lane in 0..usize::from(channels).div_ceil(4) * 4 {
            let pitch = self.rate + finite(inputs[0].values[lane]) * self.fm_amount;
            let frequency = self.runtime.clock_frequency / 2.0 * exponential(pitch);
            let width = (self.pulse_width
                + finite(inputs[1].voice(lane, 0.0)) / 10.0 * self.pwm_amount)
                .clamp(0.01, 0.99);
            let delta = (frequency * self.sample_time).min(0.5);
            let mut phase = self.runtime.phases[lane] + delta;
            phase -= phase.trunc();
            let reset = finite(inputs[2].voice(lane, 0.0));
            let was_high = self.runtime.reset_high[lane];
            if reset >= 2.0 {
                self.runtime.reset_high[lane] = true;
            } else if reset <= 0.1 {
                self.runtime.reset_high[lane] = false;
            }
            if !was_high && reset >= 2.0 {
                phase = 0.0;
            }
            self.runtime.phases[lane] = phase;
            if lane >= usize::from(channels) {
                continue;
            }
            for (index, output) in outputs.iter_mut().enumerate() {
                if connected_mask & (1 << index) == 0 {
                    continue;
                }
                let mut value = match index {
                    0 => {
                        let p = phase - if self.offset { 0.25 } else { 0.0 };
                        (std::f32::consts::TAU * p).sin()
                    }
                    1 => {
                        let p = phase + if self.offset { 0.0 } else { 0.25 };
                        4.0 * (p - p.round_ties_even()).abs() - 1.0
                    }
                    2 => {
                        let p = phase - if self.offset { 0.5 } else { 0.0 };
                        2.0 * (p - p.round_ties_even())
                    }
                    3 => {
                        if phase < width {
                            1.0
                        } else {
                            -1.0
                        }
                    }
                    _ => unreachable!(),
                };
                if self.invert {
                    value *= -1.0;
                }
                if self.offset {
                    value += 1.0;
                }
                output.values[lane] = 5.0 * value;
            }
        }
        self.runtime.light_divider += 1;
        if self.runtime.light_divider == 16 {
            self.runtime.light_divider = 0;
            if channels == 1 {
                let target = 1.0 - self.runtime.phases[0];
                let elapsed = self.sample_time * 16.0;
                for value in &mut self.runtime.lights[..2] {
                    if target < *value {
                        *value += (target - *value) * 30.0 * elapsed;
                    } else {
                        *value = target;
                    }
                }
                self.runtime.lights[2] = 0.0;
            } else {
                self.runtime.lights[0] = 0.0;
                self.runtime.lights[1] = 0.0;
                self.runtime.lights[2] = 1.0;
            }
            self.runtime.lights[3] = u8::from(self.invert) as f32;
            self.runtime.lights[4] = u8::from(self.offset) as f32;
        }
    }
}

/// Reference access to the actual production model, without graph cable delay.
#[doc(hidden)]
#[derive(Debug)]
pub struct StockLfoProbe(StockLfo);
impl StockLfoProbe {
    pub fn new(node: &Node, rate: u32) -> Result<Self, String> {
        StockLfo::build(node, rate).map(Self)
    }
    pub fn step(&mut self, inputs: &[PortFrame; 4], connected_mask: u16) -> [PortFrame; 4] {
        let mut outputs = [PortFrame::default(); 4];
        self.0.process(inputs, connected_mask, &mut outputs);
        outputs
    }
    pub fn snapshot(&self) -> StockLfoSnapshot {
        self.0.runtime
    }
    pub fn initialize(&mut self) {
        self.0.initialize_state();
    }
    pub fn adopt(&mut self, previous: &Self) {
        self.0.adopt(&previous.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(params: &[(&str, f32)]) -> StockLfo {
        StockLfo::build(
            &Node {
                id: "lfo".into(),
                kind: "stock_lfo".into(),
                params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
                content: None,
                rackmcp_sequence: None,
                score: None,
                bypassed: false,
            },
            8192,
        )
        .unwrap()
    }
    fn step(lfo: &mut StockLfo, inputs: &[PortFrame; 4], mask: u16) -> [PortFrame; 4] {
        let mut output = [PortFrame::default(); 4];
        lfo.process(inputs, mask, &mut output);
        output
    }
    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 0.00001, "{a} != {b}");
    }

    #[test]
    fn waveform_landmarks_modes_and_nearest_even_saw_boundary() {
        for offset in [0.0, 1.0] {
            for invert in [0.0, 1.0] {
                for (phase, bipolar, unipolar) in [
                    (0.0, [0.0, 0.0, 0.0, 5.0], [0.0, 0.0, 0.0, 10.0]),
                    (0.25, [5.0, 5.0, 2.5, 5.0], [5.0, 5.0, 2.5, 10.0]),
                    (0.5, [0.0, 0.0, 5.0, -5.0], [10.0, 10.0, 5.0, 0.0]),
                    (0.75, [-5.0, -5.0, -2.5, -5.0], [5.0, 5.0, 7.5, 0.0]),
                ] {
                    let mut lfo = model(&[("rate", 0.0), ("offset", offset), ("invert", invert)]);
                    let mut inputs = [PortFrame::default(); 4];
                    if phase == 0.0 {
                        lfo.runtime.reset_high[0] = false;
                        inputs[2] = PortFrame::mono(2.0);
                    } else {
                        lfo.runtime.phases[0] = phase - 1.0 / 8192.0;
                    }
                    let output = step(&mut lfo, &inputs, 15);
                    for (index, frame) in output.iter().enumerate() {
                        let reference = if offset == 0.0 {
                            bipolar[index]
                        } else {
                            unipolar[index]
                        };
                        let reference = if invert == 0.0 {
                            reference
                        } else if offset == 0.0 {
                            -reference
                        } else {
                            10.0 - reference
                        };
                        close(frame.values[0], reference);
                    }
                }
            }
        }
    }

    #[test]
    fn source_exponent_zero_and_infinity_have_defined_phase_hold_and_half_cycle_cap() {
        assert_eq!(exponential(-127.0).to_bits(), 0);
        assert_eq!(exponential(-126.5).to_bits(), 0);
        assert_eq!(exponential(0.0), 1.0);
        assert_eq!(exponential(1.0), 2.0);
        assert_eq!(exponential(128.0), f32::INFINITY);
        let mut lfo = model(&[("rate", 0.0), ("fm_amount", 1.0)]);
        lfo.runtime.phases[0] = 0.25;
        let mut inputs = [PortFrame::default(); 4];
        inputs[0] = PortFrame::mono(-127.0);
        let held = step(&mut lfo, &inputs, 15);
        assert_eq!(lfo.runtime.phases[0], 0.25);
        inputs[0].values[0] = -0.5;
        step(&mut lfo, &inputs, 15);
        assert!(
            lfo.runtime.phases[0] > 0.25,
            "negative exponential FM lowers a positive rate"
        );
        lfo.runtime.phases[0] = 0.25;
        inputs[0].values[0] = 128.0;
        let capped = step(&mut lfo, &inputs, 15);
        assert_eq!(lfo.runtime.phases[0], 0.75);
        assert!(held
            .iter()
            .chain(capped.iter())
            .all(|f| f.values.iter().all(|v| v.is_finite())));
    }

    #[test]
    fn reset_starts_high_uses_two_volts_and_fires_after_advance() {
        let mut lfo = model(&[("rate", 0.0)]);
        let mut inputs = [PortFrame::default(); 4];
        inputs[2] = PortFrame::mono(10.0);
        step(&mut lfo, &inputs, 15);
        assert_eq!(lfo.runtime.phases[0], 1.0 / 8192.0);
        inputs[2].values[0] = 0.1;
        step(&mut lfo, &inputs, 15);
        assert!(!lfo.runtime.reset_high[0]);
        inputs[2].values[0] = 1.99;
        step(&mut lfo, &inputs, 15);
        assert_eq!(lfo.runtime.phases[0], 3.0 / 8192.0);
        inputs[2].values[0] = 2.0;
        let reset = step(&mut lfo, &inputs, 15);
        assert_eq!(lfo.runtime.phases[0], 0.0);
        close(reset[0].values[0], 0.0);
        step(&mut lfo, &inputs, 15);
        assert_eq!(lfo.runtime.phases[0], 1.0 / 8192.0);
    }

    #[test]
    fn shared_clock_startup_thresholds_rejection_and_disconnect_freeze() {
        let mut lfo = model(&[]);
        let mut inputs = [PortFrame::default(); 4];
        inputs[3] = PortFrame::mono(10.0);
        for _ in 0..20 {
            step(&mut lfo, &inputs, 0);
        }
        assert_eq!(lfo.runtime.clock_trigger, 1);
        assert_eq!(lfo.runtime.clock_frequency, 1.0);
        inputs[3].values[0] = 0.1;
        step(&mut lfo, &inputs, 0);
        inputs[3].values[0] = 1.99;
        step(&mut lfo, &inputs, 0);
        assert_eq!(lfo.runtime.clock_trigger, 0);
        let elapsed = lfo.runtime.clock_timer + lfo.sample_time;
        inputs[3].values[0] = 2.0;
        step(&mut lfo, &inputs, 0);
        assert_eq!(lfo.runtime.clock_frequency, 1.0 / elapsed);
        assert_eq!(lfo.runtime.clock_timer, 0.0);
        let accepted = lfo.runtime.clock_frequency;
        inputs[3].values[0] = 0.0;
        step(&mut lfo, &inputs, 0);
        inputs[3].values[0] = 2.0;
        step(&mut lfo, &inputs, 0);
        assert_eq!(
            lfo.runtime.clock_frequency, accepted,
            "too-fast edge retains prior valid rate"
        );
        assert_eq!(lfo.runtime.clock_timer, 0.0);
        inputs[3].values[0] = 0.0;
        step(&mut lfo, &inputs, 0);
        lfo.runtime.clock_timer = 1001.0;
        inputs[3].values[0] = 2.0;
        step(&mut lfo, &inputs, 0);
        assert_eq!(
            lfo.runtime.clock_frequency, accepted,
            "too-slow edge is rejected"
        );
        assert_eq!(lfo.runtime.clock_timer, 0.0);
        step(&mut lfo, &inputs, 0);
        let timer = lfo.runtime.clock_timer;
        let trigger = lfo.runtime.clock_trigger;
        inputs[3] = PortFrame::default();
        step(&mut lfo, &inputs, 0);
        assert_eq!(lfo.runtime.clock_frequency, 2.0);
        assert_eq!(lfo.runtime.clock_timer, timer);
        assert_eq!(lfo.runtime.clock_trigger, trigger);
        inputs[3] = PortFrame::mono(2.0);
        step(&mut lfo, &inputs, 0);
        assert_eq!(
            lfo.runtime.clock_frequency, 2.0,
            "reconnection does not invent an edge"
        );
    }

    #[test]
    fn raw_fm_auxiliary_broadcast_and_quartet_retirement() {
        let mut lfo = model(&[("rate", 0.0), ("fm_amount", 1.0)]);
        let mut inputs = [PortFrame::default(); 4];
        inputs[0] = PortFrame::mono(1.0);
        step(&mut lfo, &inputs, 15);
        assert_eq!(lfo.runtime.phases[0], 2.0 / 8192.0);
        assert_eq!(
            lfo.runtime.phases[1],
            1.0 / 8192.0,
            "hidden FM lanes do not broadcast"
        );
        inputs[0] = PortFrame {
            channels: 16,
            ..PortFrame::default()
        };
        inputs[2] = PortFrame::mono(0.0);
        step(&mut lfo, &inputs, 15);
        inputs[2] = PortFrame::mono(2.0);
        step(&mut lfo, &inputs, 15);
        assert_eq!(
            lfo.runtime.phases, [0.0; 16],
            "mono reset broadcasts to every processed lane"
        );
        inputs[2] = PortFrame::default();
        step(&mut lfo, &inputs, 15);
        let before = lfo.runtime.phases;
        inputs[0].channels = 1;
        for _ in 0..5 {
            step(&mut lfo, &inputs, 15);
        }
        for (lane, value) in before.iter().enumerate() {
            assert_eq!(
                lfo.runtime.phases[lane],
                *value + if lane < 4 { 5.0 / 8192.0 } else { 0.0 }
            );
        }
        inputs[0].channels = 16;
        step(&mut lfo, &inputs, 15);
        for (lane, value) in before.iter().enumerate() {
            assert_eq!(
                lfo.runtime.phases[lane],
                *value + if lane < 4 { 6.0 / 8192.0 } else { 1.0 / 8192.0 }
            );
        }
    }

    #[test]
    fn light_cadence_modes_half_cycle_cap_and_consumer_independence() {
        let mut lfo = model(&[("rate", 10.0), ("fm_amount", 1.0), ("invert", 1.0)]);
        let mut inputs = [PortFrame::default(); 4];
        inputs[0] = PortFrame::mono(16.0);
        for _ in 0..15 {
            assert_eq!(step(&mut lfo, &inputs, 0), [PortFrame::default(); 4]);
        }
        assert_eq!(lfo.runtime.phases[0], 0.5);
        assert_eq!(lfo.runtime.lights, [0.0; 5]);
        let output = step(&mut lfo, &inputs, 1);
        assert_eq!(lfo.runtime.phases[0], 0.0);
        assert_eq!(output[0].channels, 1);
        assert_eq!(output[1..], [PortFrame::default(); 3]);
        assert_eq!(lfo.runtime.lights, [1.0, 1.0, 0.0, 1.0, 1.0]);
        inputs[0].channels = 3;
        for _ in 0..16 {
            step(&mut lfo, &inputs, 15);
        }
        assert_eq!(lfo.runtime.lights, [0.0, 0.0, 1.0, 1.0, 1.0]);
        let lights = lfo.lights();
        assert_eq!(lights.clock_frequency, Some(2.0));
        assert_eq!(lights.count, 5);
        assert!(lights.brightness.iter().all(|v| v[1] == 0.0));
    }

    #[test]
    fn initialize_preserves_detector_lamp_and_divider_state_and_adoption_keeps_new_controls() {
        let mut old = model(&[("rate", 0.0)]);
        let mut inputs = [PortFrame::default(); 4];
        inputs[3] = PortFrame::mono(0.0);
        for _ in 0..19 {
            step(&mut old, &inputs, 15);
        }
        let observed = old.runtime;
        let mut current = model(&[("rate", 1.0), ("offset", 0.0)]);
        current.adopt(&old);
        assert_eq!(current.runtime, observed);
        assert_eq!(current.rate, 1.0);
        assert!(!current.offset);
        current.initialize_state();
        assert_eq!(current.runtime.phases, [0.0; 16]);
        assert_eq!(current.runtime.clock_timer, 0.0);
        assert_eq!(current.runtime.clock_frequency, 1.0);
        assert_eq!(current.runtime.clock_trigger, observed.clock_trigger);
        assert_eq!(current.runtime.reset_high, observed.reset_high);
        assert_eq!(current.runtime.lights, observed.lights);
        assert_eq!(current.runtime.light_divider, observed.light_divider);
    }
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    fn lfo() -> StockLfo {
        StockLfo::build(
            &Node {
                id: "lfo".into(),
                kind: "stock_lfo".into(),
                params: [
                    ("rate".into(), 0.0),
                    ("fm_amount".into(), 1.0),
                    ("pwm_amount".into(), 1.0),
                ]
                .into(),
                content: None,
                rackmcp_sequence: None,
                score: None,
                bypassed: false,
            },
            8192,
        )
        .unwrap()
    }
    #[test]
    fn huge_finite_pwm_is_scaled_before_width_clamping_and_hidden_fm_reads_raw_storage() {
        let mut model = lfo();
        model.pwm_amount = 1e-8;
        model.runtime.phases.fill(0.75);
        let mut inputs = [PortFrame::default(); 4];
        inputs[1] = PortFrame::mono(1e10);
        let mut outputs = [PortFrame::default(); 4];
        model.process(&inputs, 15, &mut outputs);
        assert_eq!(
            outputs[3].values[0], 10.0,
            "large PWM times tiny depth reaches the upper width bound"
        );
        model.runtime.phases.fill(0.25);
        inputs[1].values[0] = -1e10;
        model.process(&inputs, 15, &mut outputs);
        assert_eq!(
            outputs[3].values[0], 0.0,
            "negative large PWM reaches the lower width bound"
        );
        model.runtime.phases.fill(0.0);
        inputs[0] = PortFrame::mono(0.0);
        inputs[0].values[1] = 2.0;
        model.process(&inputs, 15, &mut outputs);
        assert_eq!(model.runtime.phases[0], 1.0 / 8192.0);
        assert_eq!(
            model.runtime.phases[1],
            4.0 / 8192.0,
            "hidden FM slot is a raw read, not broadcast or masked"
        );
        inputs[0].values[0] = f32::MAX;
        inputs[0].values[1] = -f32::MAX;
        model.process(&inputs, 15, &mut outputs);
        assert!(model
            .runtime
            .phases
            .iter()
            .all(|phase| phase.is_finite() && *phase >= 0.0 && *phase < 1.0));
        assert!(outputs
            .iter()
            .all(|frame| frame.values.iter().all(|value| value.is_finite())));
    }

    #[test]
    fn pwm_broadcast_short_poly_clamps_and_native_nonfinite_safety_are_explicit() {
        let mut model = lfo();
        model.runtime.phases.fill(0.25);
        let mut inputs = [PortFrame::default(); 4];
        inputs[0] = PortFrame {
            channels: 3,
            ..PortFrame::default()
        };
        inputs[1] = PortFrame::mono(10.0);
        let mut outputs = [PortFrame::default(); 4];
        model.process(&inputs, 15, &mut outputs);
        assert_eq!(&outputs[3].values[..3], &[10.0, 10.0, 10.0]);
        inputs[1].channels = 2;
        inputs[1].values[1] = -10.0;
        model.process(&inputs, 15, &mut outputs);
        assert_eq!(&outputs[3].values[..3], &[10.0, 0.0, 10.0]);
        inputs[0].values[0] = f32::MAX;
        inputs[0].values[1] = -f32::MAX;
        inputs[0].values[2] = f32::NAN;
        inputs[1].values[0] = f32::INFINITY;
        inputs[1].values[1] = f32::NEG_INFINITY;
        inputs[2] = PortFrame::mono(f32::NAN);
        inputs[3] = PortFrame::mono(f32::INFINITY);
        model.process(&inputs, 15, &mut outputs);
        assert!(outputs
            .iter()
            .all(|frame| frame.values.iter().all(|v| v.is_finite())));
        assert!(model
            .runtime
            .phases
            .iter()
            .all(|p| p.is_finite() && *p >= 0.0 && *p < 1.0));
        assert!(model.lights().clock_frequency.unwrap() > 0.0);
    }
}
