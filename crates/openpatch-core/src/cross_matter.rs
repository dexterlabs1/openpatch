//! Cross Matter: native host adapter for the pinned Warps (2014) modulator.
//! The kernel (cross-modulation algorithms, carrier oscillators, vocoder and
//! frequency shifter) is an MIT source adaptation in `kernel/`. Voltage
//! scaling, CV mapping, the 96 kHz rate bridge and block timing below are
//! explicit OpenPatch policies.
pub(crate) mod bridge;
mod kernel;

use crate::{parameter, ModuleLightState, Node, PortFrame};
use bridge::Bridge;
use kernel::{unwrap_pot, Modulator, BLOCK, KERNEL_SAMPLE_RATE};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 6] = [
    "carrier",
    "modulator",
    "level1_cv",
    "level2_cv",
    "algorithm_cv",
    "timbre_cv",
];
pub(crate) const OUTPUTS: [&str; 2] = ["out", "aux"];
const PARAMS: [&str; 6] = ["algorithm", "timbre", "level1", "level2", "carrier", "mode"];
/// Codec full scale: the Warps line inputs accept 16 Vpp.
const FULL_SCALE_VOLTS: f32 = 8.0;
/// Most kernel frames one host frame can produce (8 kHz host minimum).
const MAX_KERNEL_FRAMES_PER_HOST_FRAME: usize = 16;

pub(crate) fn catalog() -> Value {
    json!({"kind":"cross_matter","title":"CROSS MATTER","model":"functional","width_hp":22,
    "tags":["Custom","Oscillators","Filters"],
    "description":"Two-input cross-modulator from the pinned Warps source: crossfade, fold, analog and digital ring, XOR, comparator and vocoder with continuous algorithm morphing, a built-in carrier and a frequency-shifter mode.",
    "inputs":INPUTS,"params":{
        "algorithm":{"default":0.17,"min":0.0,"max":1.0,"description":"Algorithm knob. Sweeps crossfade, fold, analog ring, digital ring, XOR, comparator and vocoder with interpolation between neighbours. In frequency-shifter mode it sets the shift (internal carrier) or phase (external carrier)."},
        "timbre":{"default":0.5,"min":0.0,"max":1.0,"description":"Algorithm parameter: fold depth, ring drive, XOR mix, comparator mode, vocoder formant shift. In frequency-shifter mode it balances up and down shift."},
        "level1":{"default":0.5,"min":0.0,"max":1.0,"description":"Carrier level, or the built-in carrier's pitch when it is selected. In frequency-shifter mode, feedback."},
        "level2":{"default":0.7,"min":0.0,"max":1.0,"description":"Modulator level. In frequency-shifter mode, dry/wet."},
        "carrier":{"default":1.0,"min":0.0,"max":3.0,"step":1.0,"values":[0,1,2,3],"labels":["External","Sine","Triangle","Saw"],"description":"External carrier input, or the built-in carrier. The vocoder uses the matching saw, pulse or noise carrier."},
        "mode":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1],"labels":["Cross modulation","Frequency shifter"]}
    }})
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://github.com/pichenettes/eurorack/tree/f4cfa7697095a28d292bff758beb51deb49b5a86/warps");
    module["polyphony"]["version_2"] = json!("Channel zero only; one mono main and one mono aux output. Inputs accept up to sixteen channels and read channel zero; an absent input supplies zero volts.");
    module["polyphony"]["maximum_channels"] = json!(1);
    module["polyphony"]["primary_input"] = Value::Null;
    for output in module["output_metadata"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        output["maximum_channels"] = json!(1);
    }
    module["processing_policy"] = json!("The source kernel always runs at its native 96 kHz in 60-frame blocks with 16-bit codec frames. At other host rates, windowed-sinc resamplers (cutoff 0.45 of the lower rate) bridge to and from 96 kHz. Latency is constant: 105 host frames (2.2 ms) at 48 kHz, 63 frames at 96 kHz. Controls are sampled once per kernel block.");
    module["voltage_policy"] = json!("Audio inputs: +/-8 V maps to 16-bit full scale and clips beyond it. Main and aux outputs use the same scale; the source aux path is already half scale. Away from 96 kHz, band-limited resampling of saturated material can overshoot full scale by under 10%.");
    module["cv_policy"] = json!("Algorithm: source pot curve of the knob plus voltage/5, clamped to 0..1, then the source low-end bleed correction. Timbre: knob plus voltage/5, clamped. Levels: knob squared; a patched level CV multiplies it by voltage/5, clamped to 0..1. With the built-in carrier, level 1 sets pitch (MIDI 36..96) and a patched level 1 CV adds 1 V/oct. Frequency shifter: the algorithm knob and voltage/5 drive the shift and phase as in the source. Nonfinite volts read zero.");
    module["carrier_policy"] = json!("External: the carrier input is input 1 and aux carries both inputs after their level stages. Built-in: the carrier input phase/frequency-modulates the internal carrier and aux carries the raw carrier. Cross-modulation uses sine/triangle/saw; the vocoder uses saw/pulse/noise and both render through the transition.");
    module["initialization_policy"] = json!("Initialize and sample-rate changes create a freshly initialized kernel, empty resamplers and an empty block. Same-rate adoption copies the complete runtime state.");
    module["light_names"] = json!(["out", "aux"]);
    module["light_policy"] = json!("Two bipolar components from the actual delayed output volts: negative/positive magnitude divided by 8 and clamped to 0..1, independent of output consumers.");
}

/// Effective source algorithm for an algorithm knob position at zero CV:
/// the Warps pot curve followed by its low-end bleed correction.
pub fn effective_algorithm(knob: f32) -> f32 {
    let algorithm = unwrap_pot(knob);
    if algorithm <= 0.125 {
        (algorithm * 1.08 - 0.01).clamp(0.0, 1.0)
    } else {
        algorithm
    }
}

fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

#[inline]
fn to_codec(sample: f32) -> i16 {
    (sample * 32768.0).round().clamp(-32768.0, 32767.0) as i16
}

#[derive(Debug)]
struct Runtime {
    modulator: Box<Modulator>,
    bridge: Bridge,
    cursor: usize,
    block_in: [[i16; 2]; BLOCK],
    block_out: [[i16; 2]; BLOCK],
    lights: [[f32; 2]; 2],
}

impl Runtime {
    fn new(sample_rate: u32) -> Self {
        Self {
            modulator: Box::new(Modulator::new(KERNEL_SAMPLE_RATE)),
            bridge: Bridge::new(sample_rate, KERNEL_SAMPLE_RATE as u32, BLOCK),
            cursor: 0,
            block_in: [[0; 2]; BLOCK],
            block_out: [[0; 2]; BLOCK],
            lights: [[0.0; 2]; 2],
        }
    }

    fn reset(&mut self) {
        self.modulator.reinitialize(KERNEL_SAMPLE_RATE);
        self.bridge.reset();
        self.cursor = 0;
        self.block_in = [[0; 2]; BLOCK];
        self.block_out = [[0; 2]; BLOCK];
        self.lights = [[0.0; 2]; 2];
    }

    fn copy_from(&mut self, other: &Self) {
        *self.modulator = *other.modulator;
        self.bridge.copy_state_from(&other.bridge);
        self.cursor = other.cursor;
        self.block_in = other.block_in;
        self.block_out = other.block_out;
        self.lights = other.lights;
    }
}

#[derive(Debug)]
pub(crate) struct CrossMatter {
    sample_rate: u32,
    algorithm: f32,
    timbre: f32,
    levels: [f32; 2],
    carrier: i32,
    frequency_shifter: bool,
    runtime: Runtime,
}

impl CrossMatter {
    pub(crate) fn build(node: &Node, sample_rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err("cross_matter sample rate must be in 8000..=192000 Hz".into());
        }
        for key in node.params.keys() {
            if !PARAMS.contains(&key.as_str()) {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let discrete = |name, default, maximum| -> Result<f32, String> {
            let value = parameter(node, name, default, 0.0, maximum)?;
            if value.fract() != 0.0 {
                return Err(format!(
                    "node '{}': parameter '{name}' must be an integer",
                    node.id
                ));
            }
            Ok(value)
        };
        Ok(Self {
            sample_rate,
            algorithm: parameter(node, "algorithm", 0.17, 0.0, 1.0)?,
            timbre: parameter(node, "timbre", 0.5, 0.0, 1.0)?,
            levels: [
                parameter(node, "level1", 0.5, 0.0, 1.0)?,
                parameter(node, "level2", 0.7, 0.0, 1.0)?,
            ],
            carrier: discrete("carrier", 1.0, 3.0)? as i32,
            frequency_shifter: discrete("mode", 0.0, 1.0)? != 0.0,
            runtime: Runtime::new(sample_rate),
        })
    }

    pub(crate) fn adopt(&mut self, previous: &Self) {
        if self.sample_rate == previous.sample_rate {
            self.runtime.copy_from(&previous.runtime);
        }
    }

    pub(crate) fn initialize_state(&mut self) {
        self.runtime.reset();
    }

    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut result = ModuleLightState {
            count: 2,
            ..ModuleLightState::default()
        };
        result.brightness[..2].copy_from_slice(&self.runtime.lights);
        result
    }

    pub(crate) fn process(
        &mut self,
        inputs: &[PortFrame; 6],
        connected_mask: u16,
        outputs: &mut [PortFrame],
    ) {
        let audio = [
            finite(inputs[0].channel(0)) / FULL_SCALE_VOLTS,
            finite(inputs[1].channel(0)) / FULL_SCALE_VOLTS,
        ];
        let mut kernel_frames = [[0.0f32; 2]; MAX_KERNEL_FRAMES_PER_HOST_FRAME];
        let mut produced = 0;
        self.runtime.bridge.push_input(audio, |frame| {
            if produced < MAX_KERNEL_FRAMES_PER_HOST_FRAME {
                kernel_frames[produced] = frame;
                produced += 1;
            }
        });
        for frame in &kernel_frames[..produced] {
            let runtime = &mut self.runtime;
            runtime.block_in[runtime.cursor] = [to_codec(frame[0]), to_codec(frame[1])];
            runtime.cursor += 1;
            if runtime.cursor == BLOCK {
                runtime.cursor = 0;
                self.render_block(inputs);
            }
        }
        let generated = self.runtime.bridge.pop();
        for (channel, output) in outputs.iter_mut().enumerate() {
            *output = PortFrame::default();
            if channel < 2 {
                let voltage = generated[channel] * FULL_SCALE_VOLTS;
                if connected_mask & (1 << channel) != 0 {
                    *output = PortFrame::mono(voltage);
                }
                self.runtime.lights[channel] = [
                    (-voltage / 8.0).clamp(0.0, 1.0),
                    (voltage / 8.0).clamp(0.0, 1.0),
                ];
            }
        }
    }

    fn render_block(&mut self, inputs: &[PortFrame; 6]) {
        let cv = |index: usize| finite(inputs[index].channel(0)).clamp(-10.0, 10.0);
        let patched = |index: usize| inputs[index].channels != 0;
        let algorithm_cv = cv(4) / 5.0;
        let p = &mut self.runtime.modulator.parameters;
        for i in 0..2 {
            let pot = self.levels[i];
            p.channel_drive[i] = if patched(2 + i) {
                (pot * pot * cv(2 + i) / 5.0).clamp(0.0, 1.0)
            } else {
                pot * pot
            };
        }
        let mut algorithm = (unwrap_pot(self.algorithm) + algorithm_cv).clamp(0.0, 1.0);
        // Prevent wavefolder bleed caused by a slight offset (cv_scaler.cc).
        if algorithm <= 0.125 {
            algorithm = (algorithm * 1.08 - 0.01).clamp(0.0, 1.0);
        }
        p.modulation_algorithm = algorithm;
        p.modulation_parameter = (self.timbre + cv(5) / 5.0).clamp(0.0, 1.0);
        p.frequency_shift_pot = self.algorithm;
        p.frequency_shift_cv = algorithm_cv.clamp(-1.0, 1.0);
        p.phase_shift = (self.algorithm + algorithm_cv).clamp(0.0, 1.0);
        p.note = 60.0 * self.levels[0] + 36.0 + if patched(2) { 12.0 * cv(2) } else { 0.0 };
        p.carrier_shape = self.carrier;
        self.runtime.modulator.easter_egg = self.frequency_shifter;
        let runtime = &mut self.runtime;
        runtime
            .modulator
            .process(&runtime.block_in, &mut runtime.block_out);
        for frame in runtime.block_out {
            runtime
                .bridge
                .push_output([frame[0] as f32 / 32768.0, frame[1] as f32 / 32768.0]);
        }
    }
}

/// Direct access to one module instance for native contract tests.
#[doc(hidden)]
#[derive(Debug)]
pub struct CrossMatterProbe(CrossMatter);
impl CrossMatterProbe {
    pub fn new(node: &Node, rate: u32) -> Result<Self, String> {
        CrossMatter::build(node, rate).map(Self)
    }
    pub fn step(&mut self, inputs: &[PortFrame; 6], connected_mask: u16) -> [PortFrame; 2] {
        let mut outputs = [PortFrame::default(); 2];
        self.0.process(inputs, connected_mask, &mut outputs);
        outputs
    }
    pub fn initialize(&mut self) {
        self.0.initialize_state();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(params: &[(&str, f32)], rate: u32) -> CrossMatter {
        CrossMatter::build(
            &Node {
                id: "cross".into(),
                kind: "cross_matter".into(),
                params: params
                    .iter()
                    .map(|(key, value)| ((*key).into(), *value))
                    .collect(),
                content: None,
                rackmcp_sequence: None,
                score: None,
                bypassed: false,
            },
            rate,
        )
        .unwrap()
    }

    fn sine(frequency: f32, rate: u32, n: usize) -> f32 {
        5.0 * (std::f32::consts::TAU * frequency * n as f32 / rate as f32).sin()
    }

    fn run(
        cross: &mut CrossMatter,
        carrier: f32,
        modulator: f32,
        rate: u32,
        count: usize,
    ) -> Vec<[f32; 2]> {
        (0..count)
            .map(|n| {
                let mut inputs = [PortFrame::default(); 6];
                if carrier != 0.0 {
                    inputs[0] = PortFrame::mono(sine(carrier, rate, n));
                }
                if modulator != 0.0 {
                    inputs[1] = PortFrame::mono(sine(modulator, rate, n));
                }
                let mut outputs = [PortFrame::default(); 2];
                cross.process(&inputs, 0b11, &mut outputs);
                [outputs[0].channel(0), outputs[1].channel(0)]
            })
            .collect()
    }

    #[test]
    fn host_rate_bridge_has_constant_latency_and_unity_crossfade() {
        // Algorithm 0, timbre 1: the source crossfade passes the modulator
        // (input 2) after its level stage.
        for rate in [44100, 48000, 96000] {
            let mut cross = model(
                &[
                    ("algorithm", 0.0),
                    ("timbre", 1.0),
                    ("carrier", 0.0),
                    ("level2", 0.7),
                ],
                rate,
            );
            let latency = cross.runtime.bridge.latency().round() as usize;
            let out = run(&mut cross, 0.0, 440.0, rate, rate as usize / 4);
            assert!(
                out[..latency.saturating_sub(40)]
                    .iter()
                    .all(|f| f[0].abs() < 1e-3),
                "rate {rate}"
            );
            let tail = &out[rate as usize / 8..];
            let peak = tail.iter().map(|f| f[0].abs()).fold(0.0, f32::max);
            assert!(peak > 2.0 && peak < 8.0, "rate {rate} peak {peak}");
        }
    }

    #[test]
    fn internal_carrier_pitch_follows_level1_at_every_host_rate() {
        for rate in [44100, 48000, 96000] {
            let mut cross = model(
                &[
                    ("algorithm", 0.0),
                    ("timbre", 1.0),
                    ("carrier", 1.0),
                    ("level1", 0.5),
                ],
                rate,
            );
            // Aux carries the raw built-in carrier: MIDI 66 = 369.99 Hz.
            let out = run(&mut cross, 0.0, 0.0, rate, rate as usize);
            let tail: Vec<f32> = out[rate as usize / 2..].iter().map(|f| f[1]).collect();
            let crossings = tail
                .windows(2)
                .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
                .count();
            let hz = crossings as f32 * 2.0;
            assert!((hz - 370.0).abs() <= 4.0, "rate {rate}: {hz} Hz");
        }
    }
}
