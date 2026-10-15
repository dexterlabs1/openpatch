//! Independently implemented stock digital Delay contract. The mono sinc engine
//! adapts BSD-2-Clause libsamplerate 0.1.9; see docs/licenses and STOCK_DELAY.md.
use crate::{parameter, Node, PortFrame, SIGNAL_LIMIT};
use serde_json::{json, Value};

#[path = "stock_delay/sinc.rs"]
mod sinc;
use sinc::MonoSinc;

pub(crate) const INPUTS: [&str; 6] = [
    "time_cv",
    "feedback_cv",
    "tone_cv",
    "mix_cv",
    "signal",
    "clock",
];
pub(crate) const OUTPUTS: [&str; 2] = ["mix", "wet"];
const NAMES: [&str; 8] = [
    "time",
    "feedback",
    "tone",
    "mix",
    "time_cv_amount",
    "feedback_cv_amount",
    "tone_cv_amount",
    "mix_cv_amount",
];
const HISTORY_SIZE: usize = 1 << 21;
const HISTORY_MASK: usize = HISTORY_SIZE - 1;

fn default_time() -> f32 {
    500.0_f32.log10() / 4.0
}

pub(crate) fn catalog() -> Value {
    let mut parameters = serde_json::Map::new();
    for (index, name) in NAMES.iter().enumerate() {
        parameters.insert(
            (*name).into(),
            json!({
                "default": if index == 0 { default_time() } else if index < 4 { 0.5 } else { 0.0 },
                "min": if index < 4 { 0.0 } else { -1.0 }, "max": 1.0,
                "randomization": if index >= 4 { "disabled" } else { "uniform" }
            }),
        );
    }
    parameters["time"]["display"] =
        json!({"scale":"exponential","base":10000.0,"multiplier":0.001,"unit":"s"});
    for name in &NAMES[1..] {
        parameters[*name]["display"] = json!({"scale":"linear","multiplier":if *name == "tone" { 200.0 } else { 100.0 },"offset":if *name == "tone" { -100.0 } else { 0.0 },"unit":"%"});
    }
    json!({"kind":"stock_delay","title":"Stock Delay","model":"functional",
        "inputs":INPUTS,"outputs":OUTPUTS,"params":parameters,
        "description":"Clock-synchronized mono delay with pitch-controlled sinc resampling, filtered feedback, signed CV depths and independent MIX/WET outputs. Audio input sums polyphonic channels."})
}

pub(crate) fn annotate(module: &mut Value) {
    module["polyphony"]["version_1"] = json!("unsupported");
    module["polyphony"]["version_2"] = json!("audio sums all active channels to mono; each CV and clock reads channel zero; both normal outputs are mono");
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["auxiliary_inputs"] =
        json!("each CV and clock reads channel zero; higher auxiliary channels are ignored");
    module["polyphony"]["retired_voices"] =
        json!("one continuous mono state independent of input width; bypass freezes it");
    module["bypass_policy"] = json!("raw audio input routes to both outputs with its original channel count, or mono zero for empty input; delay, SRC, filters and clock state freeze");
    module["state_adoption"] = json!("same identity/rate retains all history, resampling and clock state; desktop ownership handoff avoids the 8 MiB history copy; immutable Engine adoption makes an independent copy");
    module["initialization_policy"] = json!("Initialize restores controls without erasing the existing tail, matching the reference class's inherited reset behavior");
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Delay.cpp");
    module["resampler_reference"] = json!(
        "libsamplerate 0.1.9 SRC_SINC_FASTEST, mono continuous-input adaptation; BSD-2-Clause"
    );
    module["safety_policy"] = json!("nonfinite inputs become zero; each input is host-bounded; extreme pitch is bounded before exponent construction; undefined upstream before-buffer SRC reads use zero history");
}

#[derive(Debug, Clone, Copy, Default)]
struct Rc {
    input: f32,
    output: f32,
}
impl Rc {
    fn process(&mut self, input: f32, angular: f32) {
        let c = 2.0 / angular;
        let next = (input + self.input - self.output * (1.0 - c)) / (1.0 + c);
        self.input = input;
        self.output = next;
    }
}

#[derive(Debug)]
struct Runtime {
    history: Box<[f32]>,
    read: usize,
    length: usize,
    queued: [f32; 16],
    queue_read: usize,
    queue_length: usize,
    src: MonoSinc,
    last_wet: f32,
    lowpass: Rc,
    highpass: Rc,
    clock_frequency: f32,
    clock_timer: f32,
    clock_trigger: u8,
    clock_phase: f32,
    clock_light: f32,
}
impl Runtime {
    fn new() -> Self {
        Self {
            history: vec![0.0; HISTORY_SIZE].into_boxed_slice(),
            read: 0,
            length: 0,
            queued: [0.0; 16],
            queue_read: 0,
            queue_length: 0,
            src: MonoSinc::new(),
            last_wet: 0.0,
            lowpass: Rc::default(),
            highpass: Rc::default(),
            clock_frequency: 1.0,
            clock_timer: 0.0,
            clock_trigger: 2,
            clock_phase: 0.0,
            clock_light: 0.0,
        }
    }
    fn copy_state_from(&mut self, old: &Self) {
        self.history.copy_from_slice(&old.history);
        self.src.copy_state_from(&old.src);
        self.read = old.read;
        self.length = old.length;
        self.queued = old.queued;
        self.queue_read = old.queue_read;
        self.queue_length = old.queue_length;
        self.last_wet = old.last_wet;
        self.lowpass = old.lowpass;
        self.highpass = old.highpass;
        self.clock_frequency = old.clock_frequency;
        self.clock_timer = old.clock_timer;
        self.clock_trigger = old.clock_trigger;
        self.clock_phase = old.clock_phase;
        self.clock_light = old.clock_light;
    }
}

#[derive(Debug)]
pub(crate) struct StockDelay {
    parameters: [f32; 8],
    rate: u32,
    sample_time: f32,
    runtime: Runtime,
}
fn read_parameters(node: &Node) -> Result<[f32; 8], String> {
    for name in node.params.keys() {
        if !NAMES.contains(&name.as_str()) {
            return Err(format!("node '{}': unknown parameter '{name}'", node.id));
        }
    }
    let mut values = [0.0; 8];
    for (index, name) in NAMES.iter().enumerate() {
        values[index] = parameter(
            node,
            name,
            if index == 0 {
                default_time()
            } else if index < 4 {
                0.5
            } else {
                0.0
            },
            if index < 4 { 0.0 } else { -1.0 },
            1.0,
        )?;
    }
    Ok(values)
}
fn voltage(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-SIGNAL_LIMIT, SIGNAL_LIMIT)
    } else {
        0.0
    }
}
fn channel_zero(inputs: &[PortFrame], index: usize) -> f32 {
    inputs
        .get(index)
        .map_or(0.0, |input| voltage(input.channel(0)))
}
// Same rounded scalar pitch approximation used by the pinned SDK. The caller
// bounds the exponent before integer conversion; upstream's extremes are undefined.
#[allow(clippy::excessive_precision)]
fn exp2_taylor5(pitch: f32) -> f32 {
    let shifted = pitch.clamp(-120.0, 120.0) + 127.0;
    let exponent = shifted as i32;
    let fraction = shifted - exponent as f32;
    let scale = f32::from_bits((exponent as u32) << 23);
    scale
        * (1.0
            + fraction
                * (0.69315169353961
                    + fraction
                        * (0.2401595990753
                            + fraction
                                * (0.055817908652
                                    + fraction * (0.008991698010 + fraction * 0.001879100722)))))
}
impl StockDelay {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        let parameters = read_parameters(node)?;
        if rate == 0 {
            return Err("stock_delay sample rate must be positive".into());
        }
        Ok(Self {
            parameters,
            rate,
            sample_time: 1.0 / rate as f32,
            runtime: Runtime::new(),
        })
    }
    // The standalone source-reference adapter drives per-frame control changes
    // through this validated path without allocating a new history buffer.
    #[allow(dead_code)]
    pub(crate) fn set_parameters(&mut self, node: &Node) -> Result<(), String> {
        let parameters = read_parameters(node)?;
        self.parameters = parameters;
        Ok(())
    }
    /// Destructive real-time transfer. The donor must not be processed afterward.
    pub(crate) fn adopt(&mut self, old: &mut Self) -> bool {
        if self.rate != old.rate {
            return false;
        }
        std::mem::swap(&mut self.runtime, &mut old.runtime);
        true
    }
    /// Independent immutable Engine snapshot, not the desktop real-time path.
    pub(crate) fn copy_state_from(&mut self, old: &Self) {
        if self.rate == old.rate {
            self.runtime.copy_state_from(&old.runtime);
        }
    }
    pub(crate) fn initialize_state(&mut self) {
        // The reference class inherits Module::onReset; it does not clear DSP.
    }
    /// A fresh host incarnation discards history without clearing the 8 MiB
    /// allocation. Only queued valid samples are ever read after this reset.
    pub(crate) fn reset_state(&mut self) {
        let state = &mut self.runtime;
        state.read = 0;
        state.length = 0;
        state.queue_read = 0;
        state.queue_length = 0;
        state.src.reset();
        state.last_wet = 0.0;
        state.lowpass = Rc::default();
        state.highpass = Rc::default();
        state.clock_frequency = 1.0;
        state.clock_timer = 0.0;
        state.clock_trigger = 2;
        state.clock_phase = 0.0;
        state.clock_light = 0.0;
    }
    pub(crate) fn clock_light(&self) -> f32 {
        self.runtime.clock_light.clamp(0.0, 1.0)
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame]) -> [PortFrame; 2] {
        let p = self.parameters;
        let state = &mut self.runtime;
        let dt = self.sample_time;
        if inputs.get(5).is_some_and(|input| input.channels != 0) {
            state.clock_timer += dt;
            let value = channel_zero(inputs, 5);
            let triggered = state.clock_trigger == 0 && value >= 2.0;
            if value >= 2.0 {
                state.clock_trigger = 1;
            } else if value <= 0.1 {
                state.clock_trigger = 0;
            }
            if triggered {
                let frequency = 1.0 / state.clock_timer;
                state.clock_timer = 0.0;
                if (0.001..=1000.0).contains(&frequency) {
                    state.clock_frequency = frequency;
                }
            }
        } else {
            state.clock_frequency = 2.0;
        }
        let input = inputs.get(4).map_or(0.0, |port| {
            port.values
                .iter()
                .take(usize::from(port.channels).min(16))
                .fold(0.0, |sum, value| sum + voltage(*value))
        });
        let feedback = (p[1] + channel_zero(inputs, 1) / 10.0 * p[5]).clamp(0.0, 1.0);
        let dry = input + state.last_wet * feedback;
        let pitch = 1000.0_f32.log2() - 10000.0_f32.log2() * p[0] + channel_zero(inputs, 0) * p[4];
        let frequency = (state.clock_frequency / 2.0 * exp2_taylor5(pitch)).min(f32::MAX);
        let index = (self.rate as f32 / frequency - 20.0).clamp(2.0, (HISTORY_SIZE - 1) as f32);
        if state.length < HISTORY_SIZE {
            state.history[(state.read + state.length) & HISTORY_MASK] = dry;
            state.length += 1;
        }
        if state.queue_length == 0 {
            let error = index - state.length as f32;
            // pow(float,float) returns float before the SDK stores it in double.
            let ratio = 4.0_f32.powf((error / 10000.0).clamp(-1.0, 1.0));
            let count = state.length.min(16);
            let mut prefix = [0.0; 16];
            for (i, value) in prefix.iter_mut().take(count).enumerate() {
                *value = state.history[(state.read + i) & HISTORY_MASK];
            }
            let (used, generated) =
                state
                    .src
                    .process(&prefix[..count], &mut state.queued, f64::from(ratio));
            state.read = (state.read + used) & HISTORY_MASK;
            state.length -= used;
            state.queue_read = 0;
            state.queue_length = generated;
        }
        let wet = if state.queue_length == 0 {
            0.0
        } else {
            let value = state.queued[state.queue_read];
            state.queue_read += 1;
            state.queue_length -= 1;
            value.clamp(-100.0, 100.0)
        };
        let tone = (p[2] + channel_zero(inputs, 2) / 10.0 * p[6]).clamp(0.0, 1.0);
        let color = 100.0_f32.powf(2.0 * tone - 1.0);
        let low = (20000.0 * color).clamp(20.0, 20000.0) / self.rate as f32;
        let angular = (2.0_f64 * std::f64::consts::PI * f64::from(low)) as f32;
        state.lowpass.process(wet, angular);
        // Deliberately use the source's angular setter for HP, not 2π conversion.
        let high = (20.0 * color).clamp(20.0, 20000.0) / self.rate as f32;
        state.highpass.process(state.lowpass.output, high);
        let wet = state.highpass.input - state.highpass.output;
        state.last_wet = voltage(wet);
        let mix = (p[3] + channel_zero(inputs, 3) / 10.0 * p[7]).clamp(0.0, 1.0);
        let mixed = input + (wet - input) * mix;
        let phase = state.clock_phase + frequency * dt;
        state.clock_phase = phase.min(f32::MAX);
        if state.clock_phase >= 1.0 {
            state.clock_phase -= 1.0;
            state.clock_light = 1.0;
        } else if state.clock_light > 0.0 {
            state.clock_light += -state.clock_light * 30.0 * dt;
        } else {
            state.clock_light = 0.0;
        }
        let mut output = [PortFrame::default(); 2];
        for (port, value) in output.iter_mut().zip([mixed, wet]) {
            port.channels = 1;
            port.values[0] = voltage(value);
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(mix: f32) -> Node {
        serde_json::from_value(json!({
            "id":"delay", "kind":"stock_delay", "params":{"mix":mix}
        }))
        .unwrap()
    }

    #[test]
    fn handoff_exchanges_history_allocations_and_retains_receiving_controls() {
        let mut donor = StockDelay::build(&node(0.8), 48000).unwrap();
        let mut receiver = StockDelay::build(&node(0.2), 48000).unwrap();
        donor.runtime.history[HISTORY_SIZE - 1] = 7.25;
        receiver.runtime.history[HISTORY_SIZE - 1] = -3.5;
        let donor_pointer = donor.runtime.history.as_ptr();
        let receiver_pointer = receiver.runtime.history.as_ptr();
        assert!(receiver.adopt(&mut donor));
        assert_eq!(receiver.runtime.history.as_ptr(), donor_pointer);
        assert_eq!(donor.runtime.history.as_ptr(), receiver_pointer);
        assert_eq!(receiver.runtime.history[HISTORY_SIZE - 1], 7.25);
        assert_eq!(donor.runtime.history[HISTORY_SIZE - 1], -3.5);
        assert_eq!(receiver.parameters[3], 0.2);
        assert_eq!(donor.parameters[3], 0.8);
        let mut other_rate = StockDelay::build(&node(0.4), 44100).unwrap();
        assert!(!other_rate.adopt(&mut receiver));
        assert_eq!(receiver.runtime.history.as_ptr(), donor_pointer);
    }

    #[test]
    fn fresh_reset_does_not_read_retired_history_and_preserves_allocations() {
        let mut used = StockDelay::build(&node(1.0), 48000).unwrap();
        let mut fresh = StockDelay::build(&node(1.0), 48000).unwrap();
        let pointer = used.runtime.history.as_ptr();
        let mut input = [PortFrame::default(); 6];
        input[4].channels = 1;
        input[4].values[0] = 8.0;
        for _ in 0..4099 {
            used.process(&input);
        }
        used.reset_state();
        assert_eq!(used.runtime.history.as_ptr(), pointer);
        input[4].values[0] = 0.0;
        for _ in 0..4099 {
            assert_eq!(used.process(&input), fresh.process(&input));
        }
    }
}
