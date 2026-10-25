//! String Field: native host adapter for the pinned Mutable Instruments Rings
//! resonator. The kernel (strummer, voice allocation, modal resonator,
//! strings, FM voice, reverb and limiter) is an MIT source adaptation in
//! `kernel/`. Voltage scaling, CV mapping (informed by `cv_scaler.cc`), jack
//! normalization and the 48 kHz rate bridge below are explicit OpenPatch
//! policies.
mod kernel;

use crate::cross_matter::bridge::Bridge;
use crate::{parameter, ModuleLightState, Node, PortFrame};
use kernel::{
    Patch, PerformanceState, ResonatorModel, StringFieldKernel, KERNEL_SAMPLE_RATE, MAX_BLOCK_SIZE,
    NUM_CHORDS,
};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 8] = [
    "in",
    "strum",
    "v_oct",
    "frequency_cv",
    "structure_cv",
    "brightness_cv",
    "damping_cv",
    "position_cv",
];
pub(crate) const OUTPUTS: [&str; 2] = ["odd", "even"];
const PARAMS: [&str; 12] = [
    "frequency",
    "structure",
    "brightness",
    "damping",
    "position",
    "frequency_amount",
    "structure_amount",
    "brightness_amount",
    "damping_amount",
    "position_amount",
    "polyphony",
    "model",
];
pub(crate) const MODEL_LABELS: [&str; 6] = [
    "Modal",
    "Sympathetic strings",
    "Inharmonic string",
    "FM voice",
    "Chord strings",
    "String and reverb",
];
pub(crate) const POLYPHONY_LABELS: [&str; 4] = ["1 voice", "2 voices", "3 voices", "4 voices"];
/// Codec full scale at the exciter input and the outputs: the source limiter
/// holds its output to 8 Vpp (0.8 of full scale), clipping softly towards
/// 10 Vpp, so full scale is 5 V.
const FULL_SCALE_VOLTS: f32 = 5.0;
/// The firmware's CV scaler limits structure below 1.0, where the stiffness
/// table would be read past its end.
const MAX_STRUCTURE: f32 = 0.9995;
/// Most kernel frames one host frame can produce (8 kHz host minimum).
const MAX_KERNEL_FRAMES_PER_HOST_FRAME: usize = 8;
const TRIGGER_HIGH: f32 = 1.0;
const TRIGGER_LOW: f32 = 0.1;
/// Strum lamp hold after a strum, in seconds.
const STRUM_LAMP_SECONDS: f32 = 0.06;

pub(crate) fn catalog() -> Value {
    json!({"kind":"string_field","title":"STRING FIELD","model":"functional","width_hp":26,
    "tags":["Custom","Oscillator","Physical modeling","Effects"],
    "description":"Polyphonic resonator from the pinned Rings source: a modal resonator, sympathetic and inharmonic Karplus-Strong strings, an FM voice, quantized chord strings and a string with reverb, played by an internal exciter or any audio input.",
    "inputs":INPUTS,"params":{
        "frequency":{"default":30.0,"min":0.0,"max":60.0,"unit":"semitones","description":"Coarse pitch above C0, in semitones. With V/OCT patched it moves in whole semitones; unpatched it is continuous."},
        "structure":{"default":0.5,"min":0.0,"max":1.0,"description":"Modal: inharmonicity of the partials. Sympathetic strings: intervals between strings. Inharmonic string and string with reverb: stiffness and dispersion. FM voice: frequency ratio. Chord strings: chord (eleven steps)."},
        "brightness":{"default":0.5,"min":0.0,"max":1.0,"description":"High-frequency content of the excitation and of the resonance. FM voice: modulation index."},
        "damping":{"default":0.5,"min":0.0,"max":1.0,"description":"Decay time; near the top the strings sustain indefinitely. FM voice: envelope decay. String with reverb: reverb time and amount."},
        "position":{"default":0.5,"min":0.0,"max":1.0,"description":"Excitation and pickup position along the string or plate. FM voice: feedback, negative below the centre. String with reverb: stereo mix."},
        "frequency_amount":{"default":0.0,"min":-1.0,"max":1.0,"description":"Attenuverter for the frequency CV: up to +/-48 semitones at 5 V, with a quartic response."},
        "structure_amount":{"default":0.0,"min":-1.0,"max":1.0,"description":"Attenuverter for the structure CV: at full scale 5 V sweeps the whole knob range."},
        "brightness_amount":{"default":0.0,"min":-1.0,"max":1.0,"description":"Attenuverter for the brightness CV: at full scale 5 V sweeps the whole knob range."},
        "damping_amount":{"default":0.0,"min":-1.0,"max":1.0,"description":"Attenuverter for the damping CV: at full scale 5 V sweeps the whole knob range."},
        "position_amount":{"default":0.0,"min":-1.0,"max":1.0,"description":"Attenuverter for the position CV: at full scale 5 V sweeps the whole knob range."},
        "polyphony":{"default":1.0,"min":1.0,"max":4.0,"step":1.0,"values":[1,2,3,4],"labels":POLYPHONY_LABELS,"description":"Number of voices; each strum moves to the next voice so earlier notes ring on. With more than one voice, odd and even voices go to separate outputs. Changing it restarts the resonators."},
        "model":{"default":0.0,"min":0.0,"max":5.0,"step":1.0,"values":[0,1,2,3,4,5],"labels":MODEL_LABELS,"description":"Resonator model. Changing it restarts the resonators."}
    }})
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://github.com/pichenettes/eurorack/tree/f4cfa7697095a28d292bff758beb51deb49b5a86/rings");
    module["polyphony"]["version_2"] = json!("Channel zero only; internal polyphony of up to four voices is a module setting. Inputs accept up to sixteen channels and read channel zero; an absent input supplies zero volts.");
    module["polyphony"]["maximum_channels"] = json!(1);
    module["polyphony"]["primary_input"] = Value::Null;
    for output in module["output_metadata"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        output["maximum_channels"] = json!(1);
    }
    module["processing_policy"] = json!("The source kernel always runs at its native 48 kHz in 24-frame blocks with 16-bit codec frames. At other host rates, windowed-sinc resamplers (cutoff 0.45 of the lower rate) bridge to and from 48 kHz. Latency is constant: 27 frames (0.56 ms) at 48 kHz. Controls are sampled once per kernel block. All string and reverb memory is preallocated.");
    module["voltage_policy"] = json!("Exciter input: +/-5 V maps to 16-bit full scale and clips beyond it, then passes the source noise gate. Outputs: codec full scale is +/-5 V; the source limiter keeps them near +/-4 V, soft-clipping towards 5 V. With only one output patched it carries the sum of both.");
    module["cv_policy"] = json!("Frequency knob: 0..60 semitones above MIDI note 12 (C0). V/OCT: 12 semitones per volt; while it is patched the knob is quantized to semitones with the source hysteresis. Frequency CV: volts x 9.6 semitones, smoothed as the source does, times the quartic attenuverter, limited to +/-48. Structure, brightness, damping and position: knob plus attenuverter x |attenuverter| x volts/5, clamped to 0..1 (structure to 0..0.9995). Chord strings: chord = round(10 x structure) with the source hysteresis. The total pitch is limited to MIDI notes -36..132. Strum: a rising edge (above 1 V after falling below 0.1 V). Nonfinite volts read zero.");
    module["normalization_policy"] = json!("As the hardware detects unpatched jacks: with IN unpatched the internal exciter plays; with STRUM unpatched the module strums on V/OCT changes above 0.4 semitone when V/OCT is patched, on detected input onsets when only IN is patched, and never when neither is; with V/OCT unpatched the note is the frequency knob alone.");
    module["state_policy"] = json!("Saved patches and presets store the authored controls only, never string or reverb memory. Initialize and sample-rate changes start a freshly initialized kernel; same-rate adoption copies the complete runtime. Processing allocates nothing, including on model and polyphony changes.");
    module["light_names"] = json!(["odd", "even", "strum"]);
    module["light_policy"] = json!("Odd/even: bipolar components from the actual delayed output volts divided by 5 and clamped to 0..1. Strum: lit for 60 ms after the kernel accepts a strum (external, note change or onset).");
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

/// The source's quadratic attenuverter law, on a -1..1 knob.
fn quadratic(amount: f32) -> f32 {
    amount * amount.abs()
}

/// The source's quartic attenuverter law, on a -1..1 knob.
fn quartic(amount: f32) -> f32 {
    let squared = amount * amount;
    (squared * squared).copysign(amount)
}

#[derive(Debug, Clone, Copy, Default)]
struct Schmitt {
    high: bool,
}

impl Schmitt {
    /// Returns true on a rising edge.
    fn process(&mut self, volts: f32) -> bool {
        if self.high {
            if volts <= TRIGGER_LOW {
                self.high = false;
            }
            false
        } else if volts >= TRIGGER_HIGH {
            self.high = true;
            true
        } else {
            false
        }
    }
}

/// Host-side state mirroring the firmware CV scaler's hysteresis and
/// smoothing.
#[derive(Debug, Clone, Copy, Default)]
struct Scaler {
    transpose: f32,
    chord: i32,
    fm_cv: f32,
}

#[derive(Debug)]
struct Runtime {
    kernel: Box<StringFieldKernel>,
    bridge: Bridge,
    cursor: usize,
    block_in: [i16; MAX_BLOCK_SIZE],
    block_out: [[i16; 2]; MAX_BLOCK_SIZE],
    strum: Schmitt,
    strum_pending: bool,
    scaler: Scaler,
    strum_lamp: u32,
    strums: u64,
    lights: [[f32; 2]; 2],
}

impl Runtime {
    fn new(sample_rate: u32) -> Self {
        Self {
            kernel: Box::new(StringFieldKernel::new()),
            bridge: Bridge::new(sample_rate, KERNEL_SAMPLE_RATE as u32, MAX_BLOCK_SIZE),
            cursor: 0,
            block_in: [0; MAX_BLOCK_SIZE],
            block_out: [[0; 2]; MAX_BLOCK_SIZE],
            strum: Schmitt::default(),
            strum_pending: false,
            scaler: Scaler::default(),
            strum_lamp: 0,
            strums: 0,
            lights: [[0.0; 2]; 2],
        }
    }

    fn reset(&mut self) {
        self.kernel.reinitialize();
        self.bridge.reset();
        self.cursor = 0;
        self.block_in = [0; MAX_BLOCK_SIZE];
        self.block_out = [[0; 2]; MAX_BLOCK_SIZE];
        self.strum = Schmitt::default();
        self.strum_pending = false;
        self.scaler = Scaler::default();
        self.strum_lamp = 0;
        self.strums = 0;
        self.lights = [[0.0; 2]; 2];
    }

    fn copy_from(&mut self, other: &Self) {
        self.kernel.copy_from(&other.kernel);
        self.bridge.copy_state_from(&other.bridge);
        self.cursor = other.cursor;
        self.block_in = other.block_in;
        self.block_out = other.block_out;
        self.strum = other.strum;
        self.strum_pending = other.strum_pending;
        self.scaler = other.scaler;
        self.strum_lamp = other.strum_lamp;
        self.strums = other.strums;
        self.lights = other.lights;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Controls {
    frequency: f32,
    structure: f32,
    brightness: f32,
    damping: f32,
    position: f32,
    frequency_amount: f32,
    structure_amount: f32,
    brightness_amount: f32,
    damping_amount: f32,
    position_amount: f32,
    polyphony: i32,
    model: i32,
}

#[derive(Debug)]
pub(crate) struct StringField {
    sample_rate: u32,
    controls: Controls,
    strum_lamp_frames: u32,
    runtime: Runtime,
}

impl StringField {
    pub(crate) fn build(node: &Node, sample_rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err("string_field sample rate must be in 8000..=192000 Hz".into());
        }
        for key in node.params.keys() {
            if !PARAMS.contains(&key.as_str()) {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let discrete = |name, default, minimum, maximum| -> Result<i32, String> {
            let value = parameter(node, name, default, minimum, maximum)?;
            if value.fract() != 0.0 {
                return Err(format!(
                    "node '{}': parameter '{name}' must be an integer",
                    node.id
                ));
            }
            Ok(value as i32)
        };
        Ok(Self {
            sample_rate,
            controls: Controls {
                frequency: parameter(node, "frequency", 30.0, 0.0, 60.0)?,
                structure: parameter(node, "structure", 0.5, 0.0, 1.0)?,
                brightness: parameter(node, "brightness", 0.5, 0.0, 1.0)?,
                damping: parameter(node, "damping", 0.5, 0.0, 1.0)?,
                position: parameter(node, "position", 0.5, 0.0, 1.0)?,
                frequency_amount: parameter(node, "frequency_amount", 0.0, -1.0, 1.0)?,
                structure_amount: parameter(node, "structure_amount", 0.0, -1.0, 1.0)?,
                brightness_amount: parameter(node, "brightness_amount", 0.0, -1.0, 1.0)?,
                damping_amount: parameter(node, "damping_amount", 0.0, -1.0, 1.0)?,
                position_amount: parameter(node, "position_amount", 0.0, -1.0, 1.0)?,
                polyphony: discrete("polyphony", 1.0, 1.0, 4.0)?,
                model: discrete("model", 0.0, 0.0, 5.0)?,
            },
            strum_lamp_frames: (STRUM_LAMP_SECONDS * sample_rate as f32) as u32,
            runtime: Runtime::new(sample_rate),
        })
    }

    /// Build straight into a box from a separate frame, so the module value
    /// does not enlarge the engine's (debug) `Model::build` stack frame.
    #[inline(never)]
    pub(crate) fn build_boxed(node: &Node, sample_rate: u32) -> Result<Box<Self>, String> {
        Self::build(node, sample_rate).map(Box::new)
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
            count: 3,
            ..ModuleLightState::default()
        };
        result.brightness[..2].copy_from_slice(&self.runtime.lights);
        let strum = if self.runtime.strum_lamp > 0 {
            1.0
        } else {
            0.0
        };
        result.brightness[2] = [0.0, strum];
        result
    }

    /// Gather the patched input frames and process one host frame. Kept out
    /// of the engine's per-sample dispatch so its port array does not enlarge
    /// that function's (debug) stack frame.
    #[inline(never)]
    pub(crate) fn process_edges(
        &mut self,
        frames: &[PortFrame],
        edges: &[Option<usize>],
        connected_mask: u16,
        outputs: &mut [PortFrame],
    ) {
        let mut inputs = [PortFrame::default(); 8];
        for (input, edge) in inputs.iter_mut().zip(edges) {
            if let Some(edge) = edge {
                *input = frames[*edge];
            }
        }
        self.process(&inputs, connected_mask, outputs);
    }

    pub(crate) fn process(
        &mut self,
        inputs: &[PortFrame; 8],
        connected_mask: u16,
        outputs: &mut [PortFrame],
    ) {
        let exciter = finite(inputs[0].channel(0));
        if self.runtime.strum.process(finite(inputs[1].channel(0))) {
            self.runtime.strum_pending = true;
        }
        let audio = [exciter / FULL_SCALE_VOLTS, 0.0];
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
            runtime.block_in[runtime.cursor] = to_codec(frame[0]);
            runtime.cursor += 1;
            if runtime.cursor == MAX_BLOCK_SIZE {
                runtime.cursor = 0;
                self.render_block(inputs);
            }
        }
        self.runtime.strum_lamp = self.runtime.strum_lamp.saturating_sub(1);
        let generated = self.runtime.bridge.pop();
        let odd = generated[0] * FULL_SCALE_VOLTS;
        let even = generated[1] * FULL_SCALE_VOLTS;
        let (odd, even) = match connected_mask & 0b11 {
            // A single patched output carries the sum of both.
            0b01 | 0b10 => (odd + even, odd + even),
            _ => (odd, even),
        };
        for (channel, output) in outputs.iter_mut().enumerate() {
            *output = PortFrame::default();
            if channel < 2 {
                let voltage = if channel == 0 { odd } else { even };
                if connected_mask & (1 << channel) != 0 {
                    *output = PortFrame::mono(voltage);
                }
                self.runtime.lights[channel] = [
                    (-voltage / FULL_SCALE_VOLTS).clamp(0.0, 1.0),
                    (voltage / FULL_SCALE_VOLTS).clamp(0.0, 1.0),
                ];
            }
        }
    }

    /// Performance state and patch for one kernel block, as the firmware CV
    /// scaler derives them from its pots, attenuverters and jacks.
    fn controls_for_block(&mut self, inputs: &[PortFrame; 8]) -> (PerformanceState, Patch) {
        let patched = |index: usize| inputs[index].channels != 0;
        let cv = |index: usize| finite(inputs[index].channel(0)).clamp(-10.0, 10.0);
        let c = self.controls;
        let scaler = &mut self.runtime.scaler;
        let modulated = |knob: f32, amount: f32, index: usize| {
            knob + quadratic(amount) * cv(index) / FULL_SCALE_VOLTS
        };
        let structure = modulated(c.structure, c.structure_amount, 4);
        let patch = Patch {
            structure: structure.clamp(0.0, MAX_STRUCTURE),
            brightness: modulated(c.brightness, c.brightness_amount, 5).clamp(0.0, 1.0),
            damping: modulated(c.damping, c.damping_amount, 6).clamp(0.0, 1.0),
            position: modulated(c.position, c.position_amount, 7).clamp(0.0, 1.0),
        };

        // Frequency CV: smoothed, then scaled by the attenuverter.
        let fm_target = cv(3) * (48.0 / FULL_SCALE_VOLTS);
        let error = fm_target - scaler.fm_cv;
        if error.abs() >= 0.8 {
            scaler.fm_cv = fm_target;
        } else {
            scaler.fm_cv += 0.02 * error;
        }
        let fm = (scaler.fm_cv * quartic(c.frequency_amount)).clamp(-48.0, 48.0);

        // Semitone quantization of the frequency knob with hysteresis.
        let transpose = c.frequency;
        let hysteresis = if transpose - scaler.transpose > 0.0 {
            -0.3
        } else {
            0.3
        };
        scaler.transpose = ((transpose + hysteresis + 0.5) as i32) as f32;

        let internal_note = !patched(2);
        let (tonic, note) = if internal_note {
            (12.0 + transpose, 0.0)
        } else {
            let tonic = 12.0 + scaler.transpose;
            // Keep the total pitch within MIDI -36..132, inside the
            // source's pitch tables and FM phase increments.
            let note = (12.0 * cv(2)).clamp(-36.0 - tonic - fm, 132.0 - tonic - fm);
            (tonic, note)
        };

        // Chord with hysteresis, from the unclamped structure.
        let chord = structure * (NUM_CHORDS - 1) as f32;
        let hysteresis = if chord - scaler.chord as f32 > 0.0 {
            -0.1
        } else {
            0.1
        };
        scaler.chord = ((chord + hysteresis + 0.5) as i32).clamp(0, NUM_CHORDS - 1);

        let performance_state = PerformanceState {
            strum: self.runtime.strum_pending,
            internal_exciter: !patched(0),
            internal_strum: !patched(1),
            internal_note,
            tonic,
            note,
            fm,
            chord: scaler.chord,
        };
        self.runtime.strum_pending = false;
        (performance_state, patch)
    }

    fn render_block(&mut self, inputs: &[PortFrame; 8]) {
        let (performance_state, patch) = self.controls_for_block(inputs);
        let runtime = &mut self.runtime;
        if self.controls.polyphony != runtime.kernel.polyphony() {
            runtime.kernel.set_polyphony(self.controls.polyphony);
        }
        runtime
            .kernel
            .set_model(ResonatorModel::from_index(self.controls.model));
        runtime.kernel.process(
            performance_state,
            &patch,
            &runtime.block_in,
            &mut runtime.block_out,
        );
        if runtime.kernel.last_strum() {
            runtime.strum_lamp = self.strum_lamp_frames;
            runtime.strums += 1;
        }
        for frame in runtime.block_out {
            runtime
                .bridge
                .push_output([frame[0] as f32 / 32768.0, frame[1] as f32 / 32768.0]);
        }
    }

    fn snapshot(&self) -> StringFieldSnapshot {
        let kernel = &self.runtime.kernel;
        StringFieldSnapshot {
            model: kernel.model() as u8,
            polyphony: kernel.polyphony() as u8,
            active_voice: kernel.active_voice() as u8,
            strums: self.runtime.strums,
            chord: self.runtime.scaler.chord as u8,
            guarded_reads: kernel.guarded_reads(),
        }
    }
}

/// Observable kernel state for native contract tests.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StringFieldSnapshot {
    pub model: u8,
    pub polyphony: u8,
    pub active_voice: u8,
    pub strums: u64,
    pub chord: u8,
    pub guarded_reads: u64,
}

/// Direct access to one module instance for native contract tests.
#[doc(hidden)]
#[derive(Debug)]
pub struct StringFieldProbe(StringField);
impl StringFieldProbe {
    pub fn new(node: &Node, rate: u32) -> Result<Self, String> {
        StringField::build(node, rate).map(Self)
    }
    pub fn step(&mut self, inputs: &[PortFrame; 8], connected_mask: u16) -> [PortFrame; 2] {
        let mut outputs = [PortFrame::default(); 2];
        self.0.process(inputs, connected_mask, &mut outputs);
        outputs
    }
    pub fn initialize(&mut self) {
        self.0.initialize_state();
    }
    pub fn snapshot(&self) -> StringFieldSnapshot {
        self.0.snapshot()
    }
    pub fn lights(&self) -> ModuleLightState {
        self.0.lights()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(params: &[(&str, f32)], rate: u32) -> StringField {
        StringField::build(
            &Node {
                id: "strings".into(),
                kind: "string_field".into(),
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

    #[test]
    fn latency_is_one_block_plus_three_frames_at_the_native_rate() {
        let field = model(&[], 48000);
        assert_eq!(field.runtime.bridge.latency(), 27.0);
    }

    #[test]
    fn strum_schmitt_needs_a_rearm_below_the_low_threshold() {
        let mut schmitt = Schmitt::default();
        assert!(schmitt.process(2.0));
        assert!(!schmitt.process(2.0));
        assert!(!schmitt.process(0.5));
        assert!(!schmitt.process(1.5));
        assert!(!schmitt.process(0.0));
        assert!(schmitt.process(1.0));
    }

    #[test]
    fn transpose_and_chord_hysteresis_follow_the_cv_scaler() {
        let mut field = model(&[("frequency", 30.4), ("structure", 0.44)], 48000);
        let mut inputs = [PortFrame::default(); 8];
        inputs[2] = PortFrame::mono(0.0);
        let (state, _) = field.controls_for_block(&inputs);
        assert_eq!((state.tonic, state.chord), (42.0, 4));
        // Small moves inside the hysteresis band keep the quantized values.
        field.controls.frequency = 30.6;
        field.controls.structure = 0.455;
        let (state, _) = field.controls_for_block(&inputs);
        assert_eq!((state.tonic, state.chord), (42.0, 4));
        field.controls.frequency = 30.9;
        field.controls.structure = 0.465;
        let (state, _) = field.controls_for_block(&inputs);
        assert_eq!((state.tonic, state.chord), (43.0, 5));
        // Without V/OCT the knob is continuous and the note is zero.
        let (state, _) = field.controls_for_block(&[PortFrame::default(); 8]);
        assert!((state.tonic - 42.9).abs() < 1e-5 && state.note == 0.0);
        assert!(state.internal_note && state.internal_exciter && state.internal_strum);
    }

    #[test]
    fn pitch_and_structure_stay_inside_the_source_tables() {
        let mut field = model(
            &[
                ("frequency", 60.0),
                ("structure", 1.0),
                ("frequency_amount", 1.0),
            ],
            48000,
        );
        let mut inputs = [PortFrame::default(); 8];
        inputs[2] = PortFrame::mono(10.0);
        inputs[3] = PortFrame::mono(10.0);
        inputs[4] = PortFrame::mono(10.0);
        let (state, patch) = field.controls_for_block(&inputs);
        assert_eq!(patch.structure, MAX_STRUCTURE);
        assert_eq!(state.fm, 48.0);
        assert!(state.tonic + state.note + state.fm <= 132.0);
        inputs[2] = PortFrame::mono(-10.0);
        inputs[3] = PortFrame::mono(-10.0);
        let (state, _) = field.controls_for_block(&inputs);
        assert!(state.tonic + state.note + state.fm >= -36.0);
    }
}
