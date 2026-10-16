//! Original Rust random-modulation implementation of a pinned behavioral contract.
//! No upstream program text or panel artwork is incorporated. The native seeded
//! generator is an OpenPatch extension; arbitrary upstream random sequences differ.
use crate::{parameter, ModuleLightState, Node, PortFrame, SIGNAL_LIMIT};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 6] = [
    "rate_cv",
    "shape_cv",
    "trigger",
    "external",
    "probability_cv",
    "spread_cv",
];
pub(crate) const OUTPUTS: [&str; 5] = ["stepped", "linear", "smooth", "exponential", "trigger"];
const PARAMETERS: [&str; 10] = [
    "rate",
    "probability",
    "spread",
    "shape",
    "rate_cv",
    "probability_cv",
    "spread_cv",
    "shape_cv",
    "unipolar",
    "seed",
];

pub(crate) fn catalog() -> Value {
    let mut params = serde_json::Map::new();
    for (index, name) in PARAMETERS.iter().enumerate() {
        let (default, min, max) = match index {
            0 => (1.0, 0.002_f32.log2(), 2000.0_f32.log2()),
            1..=3 => (1.0, 0.0, 1.0),
            4..=7 => (0.0, -1.0, 1.0),
            8 => (0.0, 0.0, 1.0),
            _ => (1.0, 1.0, 16777215.0),
        };
        params.insert(
            (*name).into(),
            json!({"default":default,"min":min,"max":max}),
        );
    }
    params["rate"]["display"] =
        json!({"scale":"exponential","base":2.0,"multiplier":1.0,"unit":"Hz"});
    for name in &PARAMETERS[1..8] {
        params[*name]["display"] = json!({"multiplier":100.0,"unit":"%"});
    }
    params["unipolar"]["step"] = json!(1);
    params["unipolar"]["values"] = json!([0, 1]);
    params["unipolar"]["labels"] = json!(["Bipolar", "Unipolar"]);
    params["seed"]["step"] = json!(1);
    json!({"kind":"random","title":"Random","model":"functional","inputs":INPUTS,"params":params,
        "description":"Clocked probabilistic random CV or external sample-and-hold with stepped, linear, smooth and exponential interpolation."})
}
pub(crate) fn annotate(module: &mut Value) {
    module["polyphony"]["version_1"] = json!("unsupported");
    module["polyphony"]["version_2"] = json!("four CV outputs use max(1, external channels); trigger output is mono; phase and clock shared across all lanes");
    module["polyphony"]["primary_input"] = json!("external");
    module["polyphony"]["auxiliary_inputs"] = json!("all controls and trigger read channel zero only; external uses raw lanes without broadcast");
    module["polyphony"]["retired_voices"] =
        json!("inactive held lanes remain untouched and resume on reactivation");
    module["output_metadata"][4]["maximum_channels"] = json!(1);
    for metadata in module["input_metadata"].as_array_mut().unwrap() {
        metadata["channel_policy"] = json!(if metadata["name"] == "external" {
            "raw active lanes; missing lanes zero; width drives four CV outputs"
        } else {
            "channel zero only; higher channels ignored"
        });
    }
    module["light_names"] = json!(["rate", "shape", "probability", "spread", "unipolar"]);
    module["light_policy"] = json!("five actual source-ordered brightness values; rate/probability are 3.0 event impulses with per-sample 30/s Euler release; shape/spread/unipolar show effective controls");
    module["state_adoption"] = json!("same identity/kind/rate preserves phase, clocks, timer, Schmitt, pulse, held lanes and lights; unchanged seed preserves native PRNG, changed seed replaces only PRNG");
    module["initialization_policy"] = json!("Initialize restores default parameters and preserves runtime histories, matching inherited source reset; a changed seed restarts only PRNG. Fresh identity or rate starts zero histories and uninitialized Schmitt");
    module["saved_state"] = json!("parameters and seed only; runtime voltages, phase, clock and PRNG progress are not serialized");
    module["random_policy"] = json!("native xorshift32 (13,17,5), seed XOR 0xA341316C, top 24 bits / 16777216; probability draws only below 1, voltage draws only on accepted triggers without external input; probability comparison accepts equality including exact zero draw at probability zero");
    module["safety_policy"] = json!("nonfinite input maps to zero; finite voltage clamps to ±1,000,000 V; rate exponent clamps to [-120,120]; shape/probability/spread clamp to [0,1]; sample rate must be 8000–192000 Hz inclusive; native zero-channel inputs normalize as disconnected");
    module["connectivity_policy"] = json!("all five outputs are calculated every frame; connecting an output does not restart interpolation");
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Random.cpp");
}
fn voltage(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(-SIGNAL_LIMIT, SIGNAL_LIMIT)
    } else {
        0.0
    }
}
fn raw(input: PortFrame) -> f32 {
    voltage(input.channel(0))
}
fn initial_rng(seed: u32) -> u32 {
    seed ^ 0xa341_316c
}
fn uniform(rng: &mut u32) -> f32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 17;
    *rng ^= *rng << 5;
    (*rng >> 8) as f32 / 16777216.0
}
// Independent scalar construction of the SDK's rounded fifth-order exp2
// approximation, with explicit safe behavior outside its exponent domain.
#[allow(clippy::excessive_precision)]
fn frequency(exponent: f32) -> f32 {
    let position = exponent.clamp(-120.0, 120.0) + 127.0;
    let octave = position as u32;
    let fraction = position - octave as f32;
    let coefficients = [
        1.0,
        0.69315169353961_f32,
        0.2401595990753,
        0.055817908652,
        0.008991698010,
        0.001879100722,
    ];
    let mut value = coefficients[5];
    for coefficient in coefficients[..5].iter().rev() {
        value = value * fraction + coefficient;
    }
    value * f32::from_bits(octave << 23)
}

/// Finite snapshot of the actual production runtime, for reference harnesses.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct RandomSnapshot {
    pub last_voltage: [f32; 16],
    pub next_voltage: [f32; 16],
    pub phase: f32,
    pub clock_phase: f32,
    pub clock_frequency: f32,
    pub timer: f32,
    /// 0 low, 1 high, 2 uninitialized. Initial high suppresses the first edge.
    pub schmitt: u8,
    pub pulse_remaining: f32,
    /// Source order: rate, shape, probability, spread, unipolar.
    pub lights: [f32; 5],
    pub rng: u32,
}
impl RandomSnapshot {
    fn fresh(seed: u32) -> Self {
        Self {
            last_voltage: [0.0; 16],
            next_voltage: [0.0; 16],
            phase: 0.0,
            clock_phase: 0.0,
            clock_frequency: 0.0,
            timer: 0.0,
            schmitt: 2,
            pulse_remaining: 0.0,
            lights: [0.0; 5],
            rng: initial_rng(seed),
        }
    }
}
#[derive(Debug)]
pub(crate) struct Random {
    rate: u32,
    parameters: [f32; 10],
    state: RandomSnapshot,
}
impl Random {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&rate) {
            return Err("random sample rate must be in 8000..=192000 Hz".into());
        }
        if let Some(name) = node
            .params
            .keys()
            .find(|name| !PARAMETERS.contains(&name.as_str()))
        {
            return Err(format!("node '{}': unknown parameter '{name}'", node.id));
        }
        let mut parameters = [0.0; 10];
        for (i, name) in PARAMETERS.iter().enumerate() {
            let (default, min, max) = match i {
                0 => (1.0, 0.002_f32.log2(), 2000.0_f32.log2()),
                1..=3 => (1.0, 0.0, 1.0),
                4..=7 => (0.0, -1.0, 1.0),
                8 => (0.0, 0.0, 1.0),
                _ => (1.0, 1.0, 16777215.0),
            };
            parameters[i] = parameter(node, name, default, min, max)?;
            if i >= 8 && parameters[i].fract() != 0.0 {
                return Err(format!("node '{}': {name} must be integer", node.id));
            }
        }
        Ok(Self {
            rate,
            state: RandomSnapshot::fresh(parameters[9] as u32),
            parameters,
        })
    }
    pub(crate) fn reset_state(&mut self) {
        self.state = RandomSnapshot::fresh(self.parameters[9] as u32);
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.rate == old.rate {
            let rng = self.state.rng;
            self.state = old.state;
            if self.parameters[9] != old.parameters[9] {
                self.state.rng = rng;
            }
        }
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut result = ModuleLightState {
            count: 5,
            ..ModuleLightState::default()
        };
        for (target, source) in result.brightness.iter_mut().zip(self.state.lights) {
            target[0] = source;
        }
        result
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame; 6]) -> [PortFrame; 5] {
        let mut rng = self.state.rng;
        let result = self.process_with_uniform(inputs, &mut || uniform(&mut rng));
        self.state.rng = rng;
        result
    }
    fn process_with_uniform(
        &mut self,
        inputs: &[PortFrame; 6],
        draw: &mut impl FnMut() -> f32,
    ) -> [PortFrame; 5] {
        let p = self.parameters;
        let dt = 1.0 / self.rate as f32;
        let external = inputs[3].channels > 0;
        let channels = inputs[3].channels.clamp(1, 16);
        let spread = if external {
            0.0
        } else {
            (p[2] + raw(inputs[5]) / 10.0 * p[6]).clamp(0.0, 1.0)
        };
        let external_clock = inputs[2].channels > 0;
        let event;
        if external_clock {
            self.state.timer += dt;
            let trigger = raw(inputs[2]);
            event = self.state.schmitt == 0 && trigger >= 2.0;
            if trigger >= 2.0 {
                self.state.schmitt = 1;
            } else if trigger <= 0.1 {
                self.state.schmitt = 0;
            }
            if event {
                self.state.clock_frequency = 1.0 / self.state.timer;
                self.state.timer = 0.0;
            }
        } else {
            self.state.clock_frequency = frequency(p[0] + raw(inputs[0]) * p[4]);
            self.state.clock_phase += (self.state.clock_frequency * dt).min(0.5);
            event = self.state.clock_phase >= 1.0;
            if event {
                self.state.clock_phase -= 1.0;
            }
        }
        if event {
            self.state.lights[0] = 3.0;
            let probability = (p[1] + raw(inputs[4]) / 10.0 * p[5]).clamp(0.0, 1.0);
            if probability == 1.0 || draw() <= probability {
                for lane in 0..usize::from(channels) {
                    self.state.last_voltage[lane] = self.state.next_voltage[lane];
                }
                if external {
                    for lane in 0..usize::from(channels) {
                        self.state.next_voltage[lane] = voltage(inputs[3].channel(lane));
                    }
                } else {
                    let target = 10.0 * draw() - if p[8] == 0.0 { 5.0 } else { 0.0 };
                    let previous = self.state.next_voltage[0];
                    self.state.next_voltage[0] = previous + (target - previous) * spread;
                }
                self.state.phase = 0.0;
                self.state.pulse_remaining = self.state.pulse_remaining.max(0.001);
                self.state.lights[2] = 3.0;
            }
        }
        self.state.phase = (self.state.phase + (self.state.clock_frequency * dt).min(0.5)).min(1.0);
        let shape = (p[3] + raw(inputs[1]) / 10.0 * p[7]).clamp(0.0, 1.0);
        let steps = (f64::from(shape).powi(2) * 15.0 + 1.0).ceil() as f32;
        let progress = if 1.0 / shape < 1e6 {
            (self.state.phase * (1.0 / shape)).min(1.0)
        } else {
            1.0
        };
        // The smooth path uses a double-precision angle, as specified by its
        // mathematical cosine contract, before rounding back to f32.
        let cosine = (std::f64::consts::PI * f64::from(progress)).cos() as f32;
        let base = f64::from(shape).powi(8) as f32;
        let exponential = if base > 0.999 {
            self.state.phase
        } else if base > 1e-20 {
            (base.powf(self.state.phase) - 1.0) / (base - 1.0)
        } else {
            1.0
        };
        let weights = [
            (self.state.phase * steps).ceil() / steps,
            progress,
            (1.0 - cosine) / 2.0,
            exponential,
        ];
        let mut outputs = [PortFrame::default(); 5];
        for (output, weight) in outputs[..4].iter_mut().zip(weights) {
            output.channels = channels;
            for lane in 0..usize::from(channels) {
                let from = self.state.last_voltage[lane];
                output.values[lane] = from + (self.state.next_voltage[lane] - from) * weight;
            }
        }
        let pulse = self.state.pulse_remaining > 0.0;
        if pulse {
            self.state.pulse_remaining -= dt;
        }
        outputs[4] = PortFrame {
            channels: 1,
            values: [0.0; 16],
        };
        outputs[4].values[0] = if pulse { 10.0 } else { 0.0 };
        for index in [0, 2] {
            self.state.lights[index] += (0.0 - self.state.lights[index]) * 30.0 * dt;
        }
        self.state.lights[1] = shape;
        self.state.lights[3] = spread;
        self.state.lights[4] = p[8];
        outputs
    }
}

/// Reference-test access to actual production stepping, without graph delay.
#[doc(hidden)]
#[derive(Debug)]
pub struct RandomProbe(Random);
impl RandomProbe {
    pub fn new(node: &Node, rate: u32) -> Result<Self, String> {
        Random::build(node, rate).map(Self)
    }
    pub fn step(&mut self, inputs: &[PortFrame; 6]) -> [PortFrame; 5] {
        self.0.process(inputs)
    }
    /// Injected draws must be finite uniform values in [0,1). This bypasses only
    /// the native PRNG; all timing/interpolation/telemetry is production code.
    pub fn step_with_uniform(
        &mut self,
        inputs: &[PortFrame; 6],
        draw: &mut impl FnMut() -> f32,
    ) -> [PortFrame; 5] {
        self.0.process_with_uniform(inputs, draw)
    }
    pub fn snapshot(&self) -> RandomSnapshot {
        self.0.state
    }
    pub fn adopt(&mut self, previous: &Self) {
        self.0.adopt(&previous.0);
    }
    pub fn reset(&mut self) {
        self.0.reset_state();
    }
}
