//! Grain Weather: native host adapter for the pinned Mutable Instruments
//! Clouds granular processor (original firmware). The kernel (recording
//! buffers, granular/stretch/looping/spectral players, effects and phase
//! vocoder) is an MIT source adaptation in `kernel/`. Voltage scaling, CV
//! mapping, the 32 kHz rate bridge and the block/main-loop cadence below are
//! explicit OpenPatch policies.
mod kernel;

use crate::cross_matter::bridge::Bridge;
use crate::{parameter, ModuleLightState, Node, PortFrame};
use kernel::{GranularProcessor, PlaybackMode, KERNEL_SAMPLE_RATE, MAX_BLOCK_SIZE};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 10] = [
    "left",
    "right",
    "trigger",
    "freeze",
    "position_cv",
    "size_cv",
    "pitch_cv",
    "density_cv",
    "texture_cv",
    "blend_cv",
];
pub(crate) const OUTPUTS: [&str; 2] = ["left", "right"];
const PARAMS: [&str; 12] = [
    "position", "size", "pitch", "density", "texture", "blend", "spread", "feedback", "reverb",
    "freeze", "mode", "quality",
];
pub(crate) const MODE_LABELS: [&str; 4] = ["Granular", "Stretch", "Looping delay", "Spectral"];
pub(crate) const QUALITY_LABELS: [&str; 4] =
    ["16-bit stereo", "16-bit mono", "8-bit stereo", "8-bit mono"];
/// Codec input full scale: +/-8 V is 16-bit full scale.
const INPUT_FULL_SCALE_VOLTS: f32 = 8.0;
/// The source output stage halves its signal before soft-limiting
/// (`SoftConvert`); doubling the codec scale undoes that halving. The source's
/// equal-power blend then leaves the fully dry path at -3 dB.
const OUTPUT_FULL_SCALE_VOLTS: f32 = 16.0;
/// Main-loop `Prepare` calls after every 32-frame block. The firmware runs it
/// continuously; eight always completes the largest WSOLA correlation search
/// and every phase-vocoder frame within the block that queued it.
pub(crate) const PREPARE_CALLS_PER_BLOCK: usize = 8;
/// Most kernel frames one host frame can produce (8 kHz host minimum).
const MAX_KERNEL_FRAMES_PER_HOST_FRAME: usize = 8;
const TRIGGER_HIGH: f32 = 1.0;
const TRIGGER_LOW: f32 = 0.1;

pub(crate) fn catalog() -> Value {
    json!({"kind":"grain_weather","title":"GRAIN WEATHER","model":"functional","width_hp":26,
    "tags":["Custom","Effects","Granular","Sampler"],
    "description":"Stereo live-buffer texture processor from the pinned Clouds source: granular clouds, WSOLA pitch shifting and time stretching, a looping delay and a spectral phase vocoder, with freeze, feedback and reverb.",
    "inputs":INPUTS,"params":{
        "position":{"default":0.5,"min":0.0,"max":1.0,"description":"Where in the recorded buffer playback happens, from the newest audio (0) to the oldest (1). Spectral mode: which stored spectrum is written and played."},
        "size":{"default":0.5,"min":0.0,"max":1.0,"description":"Grain size, WSOLA window size, loop length or pitch-shifter window; spectral mode warps the frequency axis."},
        "pitch":{"default":0.0,"min":-24.0,"max":24.0,"unit":"semitones","description":"Transposition in semitones; the pitch CV adds 1 V/oct. The total is limited to +/-48 semitones."},
        "density":{"default":0.7,"min":0.0,"max":1.0,"description":"Granular: grain rate, random above the centre and regular below it, silent at the centre. Stretch and looping: diffusion. Spectral: spectrum refresh rate and phase randomization."},
        "texture":{"default":0.5,"min":0.0,"max":1.0,"description":"Granular: grain window from square to smooth, then diffusion. Stretch and looping: low-pass to high-pass filter. Spectral: magnitude quantization."},
        "blend":{"default":0.5,"min":0.0,"max":1.0,"description":"Dry/wet balance."},
        "spread":{"default":0.0,"min":0.0,"max":1.0,"description":"Stereo spread of randomly panned grains."},
        "feedback":{"default":0.0,"min":0.0,"max":1.0,"description":"Amount of output fed back into the recording buffer, high-passed and soft-limited."},
        "reverb":{"default":0.0,"min":0.0,"max":1.0,"description":"Reverb amount on the wet signal."},
        "freeze":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1],"labels":["Recording","Frozen"],"description":"Stop recording into the buffer. A high freeze gate also freezes."},
        "mode":{"default":0.0,"min":0.0,"max":3.0,"step":1.0,"values":[0,1,2,3],"labels":MODE_LABELS,"description":"Playback mode. Changing to or from spectral clears the buffer, as the source does."},
        "quality":{"default":0.0,"min":0.0,"max":3.0,"step":1.0,"values":[0,1,2,3],"labels":QUALITY_LABELS,"description":"Buffer format. 8-bit settings store mu-law audio at 16 kHz for a longer buffer. Changing it clears the buffer."}
    }})
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://github.com/pichenettes/eurorack/tree/f4cfa7697095a28d292bff758beb51deb49b5a86/clouds");
    module["polyphony"]["version_2"] = json!("Channel zero only; stereo left and right outputs. Inputs accept up to sixteen channels and read channel zero; an absent input supplies zero volts, and an unpatched right input follows the left.");
    module["polyphony"]["maximum_channels"] = json!(1);
    module["polyphony"]["primary_input"] = Value::Null;
    for output in module["output_metadata"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        output["maximum_channels"] = json!(1);
    }
    module["processing_policy"] = json!("The source kernel always runs at its native 32 kHz in 32-frame blocks with 16-bit codec frames; after each block it runs the firmware main-loop task eight times. At other host rates, windowed-sinc resamplers (cutoff 0.45 of the lower rate) bridge to and from 32 kHz. Latency is constant: 35 frames at 32 kHz, 159 host frames (3.3 ms) at 48 kHz. Controls are sampled once per kernel block. The recording buffer is preallocated: about 1 s stereo or 1.9 s mono at 16 bits, 4 s stereo or 7.4 s mono with the 8-bit settings.");
    module["voltage_policy"] = json!("Audio inputs: +/-8 V maps to 16-bit full scale and clips beyond it. Outputs: the source output stage halves and soft-limits its signal, so codec full scale is +/-16 V. The source equal-power blend passes the fully dry signal at -3 dB (0.71x) below the limiter.");
    module["cv_policy"] = json!("Position, size, density, texture and blend: knob plus voltage/5, clamped to 0..1. Pitch: knob semitones plus 12 per volt, limited to +/-48. Spread, feedback and reverb are knob only. Trigger: a rising edge (above 1 V after falling below 0.1 V) seeds a grain, restarts the stretch envelope or taps the delay time; a high trigger input is the spectral glitch gate. Freeze: the switch or a high freeze gate. Nonfinite volts read zero.");
    module["state_policy"] = json!("Saved patches and presets store the authored controls only, never buffer audio. Initialize and sample-rate changes start a freshly initialized kernel with an empty buffer; same-rate adoption copies the complete runtime, buffer included. Processing allocates nothing, including on mode and quality changes.");
    module["light_names"] = json!(["left", "right", "freeze"]);
    module["light_policy"] = json!("Left/right: bipolar components from the actual delayed output volts divided by 8 and clamped to 0..1. Freeze: on while the kernel is frozen. waveform_position reports the record head as a fraction of the buffer.");
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

#[derive(Debug)]
struct Runtime {
    processor: Box<GranularProcessor>,
    bridge: Bridge,
    cursor: usize,
    block_in: [[i16; 2]; MAX_BLOCK_SIZE],
    block_out: [[i16; 2]; MAX_BLOCK_SIZE],
    trigger: Schmitt,
    freeze_gate: Schmitt,
    trigger_pending: bool,
    lights: [[f32; 2]; 2],
}

impl Runtime {
    fn new(sample_rate: u32) -> Self {
        Self {
            processor: Box::new(GranularProcessor::new()),
            bridge: Bridge::new(sample_rate, KERNEL_SAMPLE_RATE as u32, MAX_BLOCK_SIZE),
            cursor: 0,
            block_in: [[0; 2]; MAX_BLOCK_SIZE],
            block_out: [[0; 2]; MAX_BLOCK_SIZE],
            trigger: Schmitt::default(),
            freeze_gate: Schmitt::default(),
            trigger_pending: false,
            lights: [[0.0; 2]; 2],
        }
    }

    fn reset(&mut self) {
        self.processor.reinitialize();
        self.bridge.reset();
        self.cursor = 0;
        self.block_in = [[0; 2]; MAX_BLOCK_SIZE];
        self.block_out = [[0; 2]; MAX_BLOCK_SIZE];
        self.trigger = Schmitt::default();
        self.freeze_gate = Schmitt::default();
        self.trigger_pending = false;
        self.lights = [[0.0; 2]; 2];
    }

    fn copy_from(&mut self, other: &Self) {
        self.processor.copy_from(&other.processor);
        self.bridge.copy_state_from(&other.bridge);
        self.cursor = other.cursor;
        self.block_in = other.block_in;
        self.block_out = other.block_out;
        self.trigger = other.trigger;
        self.freeze_gate = other.freeze_gate;
        self.trigger_pending = other.trigger_pending;
        self.lights = other.lights;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Controls {
    position: f32,
    size: f32,
    pitch: f32,
    density: f32,
    texture: f32,
    blend: f32,
    spread: f32,
    feedback: f32,
    reverb: f32,
    freeze: bool,
    mode: i32,
    quality: i32,
}

#[derive(Debug)]
pub(crate) struct GrainWeather {
    sample_rate: u32,
    controls: Controls,
    runtime: Runtime,
}

impl GrainWeather {
    pub(crate) fn build(node: &Node, sample_rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err("grain_weather sample rate must be in 8000..=192000 Hz".into());
        }
        for key in node.params.keys() {
            if !PARAMS.contains(&key.as_str()) {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let discrete = |name, maximum| -> Result<f32, String> {
            let value = parameter(node, name, 0.0, 0.0, maximum)?;
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
            controls: Controls {
                position: parameter(node, "position", 0.5, 0.0, 1.0)?,
                size: parameter(node, "size", 0.5, 0.0, 1.0)?,
                pitch: parameter(node, "pitch", 0.0, -24.0, 24.0)?,
                density: parameter(node, "density", 0.7, 0.0, 1.0)?,
                texture: parameter(node, "texture", 0.5, 0.0, 1.0)?,
                blend: parameter(node, "blend", 0.5, 0.0, 1.0)?,
                spread: parameter(node, "spread", 0.0, 0.0, 1.0)?,
                feedback: parameter(node, "feedback", 0.0, 0.0, 1.0)?,
                reverb: parameter(node, "reverb", 0.0, 0.0, 1.0)?,
                freeze: discrete("freeze", 1.0)? != 0.0,
                mode: discrete("mode", 3.0)? as i32,
                quality: discrete("quality", 3.0)? as i32,
            },
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
            count: 3,
            ..ModuleLightState::default()
        };
        result.brightness[..2].copy_from_slice(&self.runtime.lights);
        let frozen = if self.runtime.processor.parameters.freeze {
            1.0
        } else {
            0.0
        };
        result.brightness[2] = [0.0, frozen];
        result.waveform_position = Some(self.runtime.processor.record_position());
        result
    }

    pub(crate) fn process(
        &mut self,
        inputs: &[PortFrame; 10],
        connected_mask: u16,
        outputs: &mut [PortFrame],
    ) {
        let left = finite(inputs[0].channel(0));
        // The right input is normalled to the left.
        let right = if inputs[1].channels != 0 {
            finite(inputs[1].channel(0))
        } else {
            left
        };
        if self.runtime.trigger.process(finite(inputs[2].channel(0))) {
            self.runtime.trigger_pending = true;
        }
        self.runtime
            .freeze_gate
            .process(finite(inputs[3].channel(0)));
        let audio = [
            left / INPUT_FULL_SCALE_VOLTS,
            right / INPUT_FULL_SCALE_VOLTS,
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
            if runtime.cursor == MAX_BLOCK_SIZE {
                runtime.cursor = 0;
                self.render_block(inputs);
            }
        }
        let generated = self.runtime.bridge.pop();
        for (channel, output) in outputs.iter_mut().enumerate() {
            *output = PortFrame::default();
            if channel < 2 {
                let voltage = generated[channel] * OUTPUT_FULL_SCALE_VOLTS;
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

    fn render_block(&mut self, inputs: &[PortFrame; 10]) {
        let cv = |index: usize| finite(inputs[index].channel(0)).clamp(-10.0, 10.0);
        let unit = |knob: f32, index: usize| (knob + cv(index) / 5.0).clamp(0.0, 1.0);
        let c = self.controls;
        let runtime = &mut self.runtime;
        let p = &mut runtime.processor.parameters;
        p.position = unit(c.position, 4);
        p.size = unit(c.size, 5);
        p.pitch = (c.pitch + 12.0 * cv(6)).clamp(-48.0, 48.0);
        p.density = unit(c.density, 7);
        p.texture = unit(c.texture, 8);
        p.dry_wet = unit(c.blend, 9);
        p.stereo_spread = c.spread;
        p.feedback = c.feedback;
        p.reverb = c.reverb;
        p.freeze = c.freeze || runtime.freeze_gate.high;
        p.trigger = runtime.trigger_pending;
        p.gate = runtime.trigger.high;
        runtime.trigger_pending = false;
        runtime
            .processor
            .set_playback_mode(PlaybackMode::from_index(c.mode));
        runtime.processor.set_quality(c.quality);
        runtime
            .processor
            .process(&runtime.block_in, &mut runtime.block_out);
        for _ in 0..PREPARE_CALLS_PER_BLOCK {
            runtime.processor.prepare();
        }
        for frame in runtime.block_out {
            runtime
                .bridge
                .push_output([frame[0] as f32 / 32768.0, frame[1] as f32 / 32768.0]);
        }
    }

    fn snapshot(&self) -> GrainWeatherSnapshot {
        let processor = &self.runtime.processor;
        GrainWeatherSnapshot {
            mode: processor.playback_mode() as u8,
            quality: processor.quality() as u8,
            frozen: processor.parameters.freeze,
            active_grains: processor.active_grains(),
            record_position: processor.record_position(),
            buffer_seconds: processor.buffer_seconds(),
            guarded_reads: processor.guarded_reads(),
        }
    }
}

/// Observable kernel state for native contract tests.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrainWeatherSnapshot {
    pub mode: u8,
    pub quality: u8,
    pub frozen: bool,
    pub active_grains: usize,
    pub record_position: f32,
    pub buffer_seconds: f32,
    pub guarded_reads: u64,
}

/// Direct access to one module instance for native contract tests.
#[doc(hidden)]
#[derive(Debug)]
pub struct GrainWeatherProbe(GrainWeather);
impl GrainWeatherProbe {
    pub fn new(node: &Node, rate: u32) -> Result<Self, String> {
        GrainWeather::build(node, rate).map(Self)
    }
    pub fn step(&mut self, inputs: &[PortFrame; 10], connected_mask: u16) -> [PortFrame; 2] {
        let mut outputs = [PortFrame::default(); 2];
        self.0.process(inputs, connected_mask, &mut outputs);
        outputs
    }
    pub fn initialize(&mut self) {
        self.0.initialize_state();
    }
    pub fn snapshot(&self) -> GrainWeatherSnapshot {
        self.0.snapshot()
    }
    pub fn lights(&self) -> ModuleLightState {
        self.0.lights()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(params: &[(&str, f32)], rate: u32) -> GrainWeather {
        GrainWeather::build(
            &Node {
                id: "grain".into(),
                kind: "grain_weather".into(),
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

    fn run(weather: &mut GrainWeather, frequency: f32, rate: u32, count: usize) -> Vec<[f32; 2]> {
        (0..count)
            .map(|n| {
                let mut inputs = [PortFrame::default(); 10];
                inputs[0] = PortFrame::mono(
                    4.0 * (std::f32::consts::TAU * frequency * n as f32 / rate as f32).sin(),
                );
                let mut outputs = [PortFrame::default(); 2];
                weather.process(&inputs, 0b11, &mut outputs);
                [outputs[0].channel(0), outputs[1].channel(0)]
            })
            .collect()
    }

    #[test]
    fn dry_path_gain_and_latency_are_constant_at_every_host_rate() {
        for rate in [32000, 44100, 48000, 96000] {
            let mut weather = model(&[("blend", 0.0)], rate);
            let latency = weather.runtime.bridge.latency().round() as usize;
            let out = run(&mut weather, 200.0, rate, rate as usize / 2);
            // Skip the first kernel block (silent while buffers are laid out).
            let tail = &out[rate as usize / 4..];
            let peak = tail.iter().map(|f| f[0].abs()).fold(0.0, f32::max);
            // Equal-power dry gain 0.707 of a 4 V (0.5 full-scale) input,
            // halved and soft-limited: SoftLimit(0.177) x 16 V.
            assert!((peak - 2.81).abs() < 0.06, "rate {rate} peak {peak}");
            assert!(tail.iter().all(|f| (f[0] - f[1]).abs() < 1e-4));
            let expected = match rate {
                32000 => 35,
                48000 => 159,
                _ => latency,
            };
            assert_eq!(latency, expected, "rate {rate}");
        }
    }

    #[test]
    fn rate_bridge_passband_is_flat_through_the_dry_path() {
        // The dry path returns the input codec words through the source
        // output stage, so it measures the 32 kHz bridge round trip.
        for rate in [44100, 48000] {
            let level = |frequency: f32| {
                let mut weather = model(&[("blend", 0.0)], rate);
                let out: Vec<f32> = (0..rate as usize)
                    .map(|n| {
                        let mut inputs = [PortFrame::default(); 10];
                        inputs[0] = PortFrame::mono(
                            0.5 * (std::f32::consts::TAU * frequency * n as f32 / rate as f32).sin(),
                        );
                        let mut outputs = [PortFrame::default(); 2];
                        weather.process(&inputs, 0b11, &mut outputs);
                        outputs[0].channel(0)
                    })
                    .collect();
                out[rate as usize / 2..].iter().fold(0.0f32, |m, x| m.max(x.abs()))
            };
            let reference = level(1000.0);
            for frequency in [100.0, 5000.0, 10000.0, 13000.0] {
                let db = 20.0 * (level(frequency) / reference).log10();
                assert!(db.abs() < 0.1, "rate {rate} {frequency} Hz: {db} dB");
            }
        }
    }

    #[test]
    fn trigger_schmitt_needs_a_rearm_below_the_low_threshold() {
        let mut schmitt = Schmitt::default();
        assert!(schmitt.process(2.0));
        assert!(!schmitt.process(2.0));
        assert!(!schmitt.process(0.5));
        assert!(!schmitt.process(1.5));
        assert!(!schmitt.process(0.0));
        assert!(schmitt.process(1.0));
    }
}
