//! Original functional wavetable oscillators. Preparation/cache ownership is
//! external; the audio path holds an immutable bank and bounded runtime arrays.
//! Source model contracts informed timing and controls, not copied DSP code.

use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};

use crate::{
    parameter,
    signal::ModuleLightState,
    wavetable_bank::{PreparedWaveBank, WavetableMode},
    wavetable_minblep::SYNC_BLEP,
    Node, PortFrame, MAX_CHANNELS, SIGNAL_LIMIT,
};

pub(crate) const VCO_INPUTS: [&str; 4] = ["fm", "sync", "position_cv", "pitch"];
pub(crate) const LFO_INPUTS: [&str; 4] = ["fm", "reset", "position_cv", "clock"];
pub(crate) const OUTPUTS: [&str; 1] = ["out"];
const C4: f32 = 261.6256;
const CORRECTION_FRAMES: usize = 32;
const VCO_PARAMETERS: [&str; 6] = [
    "tune",
    "position",
    "fm_amount",
    "position_cv_amount",
    "sync_mode",
    "fm_mode",
];
const LFO_PARAMETERS: [&str; 6] = [
    "rate",
    "position",
    "fm_amount",
    "position_cv_amount",
    "offset",
    "invert",
];

pub(crate) fn input_names(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "wavetable_vco" => Some(&VCO_INPUTS),
        "wavetable_lfo" => Some(&LFO_INPUTS),
        _ => None,
    }
}

pub(crate) fn output_names(kind: &str) -> Option<&'static [&'static str]> {
    input_names(kind).map(|_| OUTPUTS.as_slice())
}

pub(crate) fn catalog() -> [Value; 2] {
    let common = || {
        json!({
            "position":{"default":0.0,"min":0.0,"max":1.0,"unit":"ratio"},
            "fm_amount":{"default":0.0,"min":-1.0,"max":1.0,"unit":"ratio","randomize":false},
            "position_cv_amount":{"default":0.0,"min":-1.0,"max":1.0,"unit":"ratio","randomize":false}
        })
    };
    let mut audio = json!({"kind":"wavetable_vco","title":"WAVETABLE VCO","model":"functional","inputs":VCO_INPUTS,"outputs":OUTPUTS,"params":common(),"description":"Wavetable audio oscillator with eight-times dense harmonic-filtered tables, exponential or through-zero linear FM, and hard/soft sync. Original functional implementation; numerical parity remains subject to source-oracle testing."});
    audio["params"]["tune"] = json!({"default":0.0,"min":-75.0,"max":75.0,"unit":"semitones","description":"C4 × 2^(tune/12 + pitch CV), before FM"});
    audio["params"]["sync_mode"] =
        json!({"default":0.0,"min":0.0,"max":1.0,"step":1.0,"labels":["Hard","Soft"]});
    audio["params"]["fm_mode"] = json!({"default":0.0,"min":0.0,"max":1.0,"step":1.0,"labels":["Exponential","Through-zero linear"]});
    let mut modulation = json!({"kind":"wavetable_lfo","title":"WAVETABLE LFO","model":"functional","inputs":LFO_INPUTS,"outputs":OUTPUTS,"params":common(),"description":"Unfiltered wavetable modulation with linear point/wave interpolation, exponential FM, shared clock-rate measurement, reset, invert and offset. Advance phase, apply reset, then emit."});
    modulation["params"]["rate"] = json!({"default":1.0,"min":-8.0,"max":10.0,"unit":"log2(Hz)","description":"Unpatched: 2^rate Hz. Patched clock: clock frequency × 2^rate / 2"});
    modulation["params"]["offset"] =
        json!({"default":1.0,"min":0.0,"max":1.0,"step":1.0,"labels":["Bipolar","Unipolar"]});
    modulation["params"]["invert"] = json!({"default":0.0,"min":0.0,"max":1.0,"step":1.0});
    [audio, modulation]
}

pub(crate) fn annotate(module: &mut Value) {
    let audio = match module["kind"].as_str() {
        Some("wavetable_vco") => true,
        Some("wavetable_lfo") => false,
        _ => return,
    };
    module["polyphony"]["version_2"] = json!(if audio {
        "max(pitch,FM,1); pitch/FM/sync/position mono broadcast, short-poly missing lanes zero; sync and position do not create voices"
    } else {
        "max(FM,1); FM reads raw lanes without mono broadcast; reset/position mono broadcast, short-poly missing lanes zero; clock is shared channel zero"
    });
    module["polyphony"]["retired_voices"] = json!("four-lane groups process through their hidden padding; wholly inactive groups freeze and resume phases, sync/reset detectors and correction queues; inactive output slots are zero");
    module["functional_reference"] = json!(format!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/{}.cpp", if audio { "WTVCO" } else { "WTLFO" }));
    module["arithmetic_policy"] = json!("f32 scalar operations and rounded SDK fifth-order exp2 coefficients, exponent bounded to [-120,120]; nonfinite input becomes zero, input/output bounded to ±1,000,000; no SIMD/FFT bit-exactness claim");
    module["position_policy"] = json!("clamp(position + positionCV × position_cv_amount / 10, 0, 1); crossfade complete waves; normalized CV-influenced position is observable; partial final wave is retained but not played");
    module["light_names"] = json!(if audio {
        [
            "phase_negative",
            "phase_positive",
            "polyphony",
            "soft_sync",
            "linear_fm",
        ]
    } else {
        ["phase_red", "phase_green", "polyphony", "offset", "invert"]
    });
    module["light_policy"] = json!("publish every 16 samples; source signed brightness with immediate rise and 30/s decay; mono phase lights or poly blue, then two mode lights; bypass freezes divider and light state");
    module["state_adoption"] = json!("same sample rate and model copies bounded runtime only; receiving parameters and immutable bank stay in place; source changes preserve detector histories");
    module["suspended_table_policy"] = json!("loading or invalid table emits zero at the connected voice width; phases, reset/sync detectors and correction queues freeze; LFO clock measurement and light divider continue");
    module["initialize_policy"] = json!(if audio {
        "whole-module initialize resets directions to +1 while retaining phases, sync histories and BLEP tails; caller separately initializes table and parameters"
    } else {
        "whole-module initialize zeros phases/timer and sets clock frequency to 1, preserving reset/clock detector histories and lights; caller separately initializes table and parameters"
    });
    if audio {
        module["frequency_policy"] = json!({"exponential":"C4 × exp2(tune/12 + pitch + FM × amount)","linear":"C4 × exp2(tune/12 + pitch) + C4 × FM × amount","safety_extension":"signed frequency magnitude clamps to Nyquist, filtering uses absolute frequency, zero uses highest bank level, phase wraps by floor; these are defined OpenPatch through-zero policies where source negative/zero octave conversion is unsafe"});
        module["sync_policy"] = json!("strict fractional crossing 0<t<=1 from prior/current sync voltage, current >=0; detector freezes when sync unplugged; soft flips direction after this sample's phase advance; hard changes phase and inserts a bounded 32-frame correction from the generated 16x minimum-phase step table; current base sample remains the pre-reset lookup");
    } else {
        module["clock_policy"] = json!("shared f32 elapsed timer advances only when clock connected; scalar Schmitt starts uninitialized, thresholds 0.1/2 V; first high arms without edge; low-to-high measures 1/elapsed including first acquisition since construction; timer resets at every rising edge even if measured rate outside 0.001..1000 Hz; disconnected clock sets frequency2 and freezes timer/detector");
        module["reset_policy"] = json!("SIMD-style Schmitt starts high; rescale reset from0.1..2 V to0..1 before comparison; first high never triggers until a low was observed; active padded groups advance then reset phase to0 before lookup; initialize preserves detectors");
        module["frequency_policy"] = json!("clock_frequency / 2 × exp2(rate + rawFM × amount), capped at1024 Hz; invert waveform, add1 when offset on, then multiply by5 V");
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct WavetableState {
    pub(crate) phases: [f32; MAX_CHANNELS],
    pub(crate) directions: [f32; MAX_CHANNELS],
    pub(crate) last_position: f32,
    pub(crate) clock_frequency: f32,
    pub(crate) timer: f32,
    pub(crate) channels: u8,
    pub(crate) lightdivider: u8,
    pub(crate) lights: [f32; 5],
    pub(crate) last_sync_values: [f32; MAX_CHANNELS],
    pub(crate) reset_high: [bool; MAX_CHANNELS],
    /// 0 low, 1 high, 2 uninitialized.
    pub(crate) clock_trigger: u8,
    pub(crate) correction_positions: [u8; 4],
}

#[derive(Clone, Copy, Debug)]
struct Runtime {
    state: WavetableState,
    corrections: [[f32; CORRECTION_FRAMES]; MAX_CHANNELS],
}

impl Runtime {
    fn new() -> Self {
        Self {
            state: WavetableState {
                phases: [0.0; MAX_CHANNELS],
                directions: [1.0; MAX_CHANNELS],
                last_position: 0.0,
                clock_frequency: 1.0,
                timer: 0.0,
                channels: 0,
                lightdivider: 0,
                lights: [0.0; 5],
                last_sync_values: [0.0; MAX_CHANNELS],
                reset_high: [true; MAX_CHANNELS],
                clock_trigger: 2,
                correction_positions: [0; 4],
            },
            corrections: [[0.0; CORRECTION_FRAMES]; MAX_CHANNELS],
        }
    }
}

#[derive(Debug)]
pub(crate) struct WavetableOscillator {
    rate: u32,
    sample_time: f32,
    mode: WavetableMode,
    parameters: [f32; 6],
    bank: Arc<PreparedWaveBank>,
    runtime: Runtime,
}

fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-SIGNAL_LIMIT, SIGNAL_LIMIT)
    } else {
        0.0
    }
}

fn poly(input: &PortFrame, channel: usize) -> f32 {
    finite(input.voice(channel, 0.0))
}

fn raw(input: &PortFrame, channel: usize) -> f32 {
    finite(input.channel(channel))
}

fn wrap(phase: f32) -> f32 {
    phase - phase.floor()
}

// Rounded constants and Horner order verified against the installed SDK's
// dsp/approx.hpp. Bounding the exponent defines behavior outside its safe range.
#[allow(clippy::excessive_precision)]
fn exp2_taylor5(pitch: f32) -> f32 {
    let biased = pitch.clamp(-120.0, 120.0) + 127.0;
    let integer = biased as u32;
    let x = biased - integer as f32;
    let mut polynomial = 0.001879100722_f32;
    for coefficient in [
        0.008991698010,
        0.055817908652,
        0.2401595990753,
        0.69315169353961,
        1.0,
    ] {
        polynomial = polynomial * x + coefficient;
    }
    f32::from_bits(integer << 23) * polynomial
}

impl WavetableOscillator {
    pub(crate) fn build(
        node: &Node,
        rate: u32,
        bank: Arc<PreparedWaveBank>,
    ) -> Result<Self, String> {
        if rate == 0 {
            return Err("Wavetable oscillator sample rate must be positive".into());
        }
        let mode = match node.kind.as_str() {
            "wavetable_vco" => WavetableMode::Audio,
            "wavetable_lfo" => WavetableMode::Modulation,
            _ => return Err("Unknown wavetable oscillator model".into()),
        };
        if bank.mode() != mode {
            return Err("Wavetable oscillator received a bank prepared for another model".into());
        }
        if bank.wave_count() == 0
            || bank.wave_points() < 2
            || (mode == WavetableMode::Audio
                && (bank.wave_points() < 32 || !bank.wave_points().is_power_of_two()))
        {
            return Err("Wavetable oscillator received an invalid bank shape".into());
        }
        let names = if mode == WavetableMode::Audio {
            VCO_PARAMETERS
        } else {
            LFO_PARAMETERS
        };
        for name in node.params.keys() {
            if !names.contains(&name.as_str()) {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        let mut parameters = [0.0; 6];
        parameters[0] = if mode == WavetableMode::Audio {
            parameter(node, names[0], 0.0, -75.0, 75.0)?
        } else {
            parameter(node, names[0], 1.0, -8.0, 10.0)?
        };
        parameters[1] = parameter(node, names[1], 0.0, 0.0, 1.0)?;
        parameters[2] = parameter(node, names[2], 0.0, -1.0, 1.0)?;
        parameters[3] = parameter(node, names[3], 0.0, -1.0, 1.0)?;
        for index in 4..6 {
            parameters[index] = parameter(
                node,
                names[index],
                if index == 4 && mode == WavetableMode::Modulation {
                    1.0
                } else {
                    0.0
                },
                0.0,
                1.0,
            )?;
            if parameters[index].fract() != 0.0 {
                return Err(format!("Parameter '{}' must be zero or one", names[index]));
            }
        }
        Ok(Self {
            rate,
            sample_time: 1.0 / rate as f32,
            mode,
            parameters,
            bank,
            runtime: Runtime::new(),
        })
    }

    pub(crate) fn adopt_from(&mut self, old: &Self) -> bool {
        if self.rate != old.rate || self.mode != old.mode {
            return false;
        }
        self.runtime = old.runtime;
        true
    }

    pub(crate) fn reset_state(&mut self) {
        self.runtime = Runtime::new();
    }

    pub(crate) fn initialize_state(&mut self) {
        if self.mode == WavetableMode::Audio {
            self.runtime.state.directions = [1.0; MAX_CHANNELS];
        } else {
            self.runtime.state.phases = [0.0; MAX_CHANNELS];
            self.runtime.state.clock_frequency = 1.0;
            self.runtime.state.timer = 0.0;
        }
    }

    pub(crate) fn snapshot(&self) -> WavetableState {
        self.runtime.state
    }

    pub(crate) fn module_lights(&self) -> ModuleLightState {
        let mut lights = ModuleLightState {
            count: 5,
            ..ModuleLightState::default()
        };
        for (output, value) in lights.brightness.iter_mut().zip(self.runtime.state.lights) {
            output[0] = value;
        }
        lights.waveform_position = Some(if self.bank.wave_count() > 1 {
            self.runtime.state.last_position / (self.bank.wave_count() - 1) as f32
        } else {
            0.0
        });
        lights
    }

    fn clock(&mut self, input: &PortFrame) {
        let state = &mut self.runtime.state;
        if input.channels == 0 {
            state.clock_frequency = 2.0;
            return;
        }
        state.timer += self.sample_time;
        let voltage = raw(input, 0);
        let rising = state.clock_trigger == 0 && voltage >= 2.0;
        if voltage >= 2.0 {
            state.clock_trigger = 1;
        } else if voltage <= 0.1 {
            state.clock_trigger = 0;
        }
        if rising {
            let frequency = 1.0 / state.timer;
            state.timer = 0.0;
            if (0.001..=1000.0).contains(&frequency) {
                state.clock_frequency = frequency;
            }
        }
    }

    fn insert_correction(&mut self, lane: usize, crossing: f32, magnitude: f32) {
        let group = lane / 4;
        let head = usize::from(self.runtime.state.correction_positions[group]);
        // A crossing in (0,1] happened at that fraction of the current interval.
        // The sampled step is measured from its event time, then delayed into
        // the fixed ring. Its endpoint at sample512 is explicitly available.
        let delay = 1.0 - crossing;
        // A vanishingly small crossing can round this delay up to one frame.
        // Such an event is outside the step kernel's open previous-frame edge.
        if !(0.0..1.0).contains(&delay) {
            return;
        }
        for future in 0..CORRECTION_FRAMES {
            let coordinate = (future as f32 + delay) * 16.0;
            let index = (coordinate as usize).min(511);
            let fraction = coordinate - index as f32;
            let step = SYNC_BLEP[index] + (SYNC_BLEP[index + 1] - SYNC_BLEP[index]) * fraction;
            let slot = (head + future) % CORRECTION_FRAMES;
            let correction = magnitude * (step - 1.0);
            self.runtime.corrections[lane][slot] =
                finite(self.runtime.corrections[lane][slot] + correction);
        }
    }

    pub(crate) fn process(&mut self, inputs: &[PortFrame; 4]) -> PortFrame {
        self.process_available(inputs, true)
    }

    /// Reference lifecycle support: a loading or invalid table silences visible
    /// outputs and freezes wave processing, while clock and light bookkeeping
    /// still follows the connected ports. Retain the last immutable valid bank.
    pub(crate) fn process_suspended(&mut self, inputs: &[PortFrame; 4]) -> PortFrame {
        self.process_available(inputs, false)
    }

    fn process_available(&mut self, inputs: &[PortFrame; 4], bank_available: bool) -> PortFrame {
        let audio = self.mode == WavetableMode::Audio;
        if !audio {
            self.clock(&inputs[3]);
        }
        let channels = if audio {
            inputs[0].channels.max(inputs[3].channels)
        } else {
            inputs[0].channels
        }
        .clamp(1, MAX_CHANNELS as u8);
        self.runtime.state.channels = channels;
        let mut output = PortFrame {
            channels,
            ..PortFrame::default()
        };
        if !bank_available {
            self.update_lights(audio);
            return output;
        }
        for group in 0..usize::from(channels).div_ceil(4) {
            let base = group * 4;
            let count = (usize::from(channels) - base).min(4);
            let mut values = [0.0; 4];
            for (within, value) in values.iter_mut().enumerate() {
                let lane = base + within;
                let fm = if audio {
                    poly(&inputs[0], lane)
                } else {
                    raw(&inputs[0], lane)
                };
                let frequency = if audio {
                    let pitch = self.parameters[0] / 12.0 + poly(&inputs[3], lane);
                    let frequency = if self.parameters[5] == 0.0 {
                        C4 * exp2_taylor5(pitch + fm * self.parameters[2])
                    } else {
                        C4 * exp2_taylor5(pitch) + C4 * fm * self.parameters[2]
                    };
                    frequency.clamp(-(self.rate as f32 / 2.0), self.rate as f32 / 2.0)
                } else {
                    (self.runtime.state.clock_frequency / 2.0
                        * exp2_taylor5(self.parameters[0] + fm * self.parameters[2]))
                    .min(1024.0)
                };
                let octave = if frequency == 0.0 {
                    f32::INFINITY
                } else {
                    (self.rate as f32 / 2.0 / frequency.abs()).log2()
                };
                if audio && self.parameters[4] == 0.0 {
                    self.runtime.state.directions[lane] = 1.0;
                }
                let delta = frequency
                    * self.sample_time
                    * if audio {
                        self.runtime.state.directions[lane]
                    } else {
                        1.0
                    };
                self.runtime.state.phases[lane] = wrap(self.runtime.state.phases[lane] + delta);
                if !audio {
                    let scaled = (poly(&inputs[1], lane) - 0.1) / (2.0 - 0.1);
                    let previous = self.runtime.state.reset_high[lane];
                    if scaled >= 1.0 {
                        self.runtime.state.reset_high[lane] = true;
                    } else if scaled <= 0.0 {
                        self.runtime.state.reset_high[lane] = false;
                    }
                    if !previous && scaled >= 1.0 {
                        self.runtime.state.phases[lane] = 0.0;
                    }
                }
                let position = (self.parameters[1]
                    + poly(&inputs[2], lane) * self.parameters[3] / 10.0)
                    .clamp(0.0, 1.0);
                if lane == 0 {
                    self.runtime.state.last_position =
                        position * (self.bank.wave_count() - 1) as f32;
                }
                if within < count {
                    *value =
                        self.bank
                            .lookup_source(self.runtime.state.phases[lane], position, octave);
                }
                if audio && inputs[1].channels != 0 {
                    let previous = self.runtime.state.last_sync_values[lane];
                    let current = poly(&inputs[1], lane);
                    let crossing = -previous / (current - previous);
                    self.runtime.state.last_sync_values[lane] = current;
                    if crossing > 0.0 && crossing <= 1.0 && current >= 0.0 {
                        if self.parameters[4] != 0.0 {
                            self.runtime.state.directions[lane] =
                                -self.runtime.state.directions[lane];
                        } else {
                            self.runtime.state.phases[lane] = wrap((1.0 - crossing) * delta);
                            if within < count {
                                let after = self.bank.lookup_source(
                                    self.runtime.state.phases[lane],
                                    position,
                                    octave,
                                );
                                self.insert_correction(lane, crossing, finite(after - *value));
                            }
                        }
                    }
                }
            }
            if audio {
                let head = usize::from(self.runtime.state.correction_positions[group]);
                for (within, value) in values.iter_mut().enumerate() {
                    *value += self.runtime.corrections[base + within][head];
                    self.runtime.corrections[base + within][head] = 0.0;
                }
                self.runtime.state.correction_positions[group] =
                    ((head + 1) % CORRECTION_FRAMES) as u8;
            }
            for (within, &sample) in values.iter().enumerate().take(count) {
                let mut value = sample;
                if !audio {
                    if self.parameters[5] != 0.0 {
                        value *= -1.0;
                    }
                    if self.parameters[4] != 0.0 {
                        value += 1.0;
                    }
                }
                output.values[base + within] = finite(value * 5.0);
            }
        }
        self.update_lights(audio);
        output
    }

    fn update_lights(&mut self, audio: bool) {
        let state = &mut self.runtime.state;
        state.lightdivider += 1;
        if state.lightdivider < 16 {
            return;
        }
        state.lightdivider = 0;
        if state.channels == 1 {
            let value = if audio {
                (std::f64::consts::TAU * f64::from(state.phases[0])).sin() as f32
            } else {
                1.0 - state.phases[0]
            };
            let dt = self.sample_time * 16.0;
            for (index, target) in [if audio { -value } else { value }, value]
                .into_iter()
                .enumerate()
            {
                if target < state.lights[index] {
                    state.lights[index] += (target - state.lights[index]) * 30.0 * dt;
                } else {
                    state.lights[index] = target;
                }
            }
            state.lights[2] = 0.0;
        } else {
            state.lights[..3].copy_from_slice(&[0.0, 0.0, 1.0]);
        }
        state.lights[3] = self.parameters[4];
        state.lights[4] = self.parameters[5];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(kind: &str, params: Value) -> Node {
        serde_json::from_value(json!({"id":"wt","kind":kind,"params":params})).unwrap()
    }
    fn build(kind: &str, params: Value, rate: u32) -> WavetableOscillator {
        let mode = if kind == "wavetable_vco" {
            WavetableMode::Audio
        } else {
            WavetableMode::Modulation
        };
        let samples: Arc<[f32]> = (0..64)
            .map(|i| (std::f32::consts::TAU * (i % 32) as f32 / 32.0).sin())
            .collect::<Vec<_>>()
            .into();
        let bank = Arc::new(PreparedWaveBank::prepare(samples, 32, mode).unwrap());
        WavetableOscillator::build(&node(kind, params), rate, bank).unwrap()
    }

    #[test]
    fn parameters_defaults_and_switch_validation_are_exact() {
        assert_eq!(
            build("wavetable_vco", json!({}), 48000).parameters,
            [0.0; 6]
        );
        assert_eq!(
            build("wavetable_lfo", json!({}), 48000).parameters,
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
        );
        let bank = build("wavetable_vco", json!({}), 48000).bank;
        for params in [
            json!({"tune":76}),
            json!({"sync_mode":0.5}),
            json!({"fm_mode":-1}),
            json!({"unknown":0}),
        ] {
            assert!(WavetableOscillator::build(
                &node("wavetable_vco", params),
                48000,
                Arc::clone(&bank)
            )
            .is_err());
        }
        assert!(WavetableOscillator::build(&node("wavetable_vco", json!({})), 0, bank).is_err());
        assert_eq!(exp2_taylor5(0.0), 1.0);
        assert_eq!(exp2_taylor5(1.0), 2.0);
    }

    #[test]
    fn rejects_banks_prepared_for_the_other_model() {
        let audio = build("wavetable_vco", json!({}), 48000).bank;
        let modulation = build("wavetable_lfo", json!({}), 48000).bank;
        assert!(
            WavetableOscillator::build(&node("wavetable_lfo", json!({})), 48000, audio).is_err()
        );
        assert!(
            WavetableOscillator::build(&node("wavetable_vco", json!({})), 48000, modulation)
                .is_err()
        );
    }

    #[test]
    fn voice_policy_padding_and_whole_group_freeze() {
        for kind in ["wavetable_vco", "wavetable_lfo"] {
            let mut osc = build(kind, json!({}), 48000);
            let mut inputs = [PortFrame::default(); 4];
            inputs[0].channels = 5;
            assert_eq!(osc.process(&inputs).channels, 5);
            let state = osc.snapshot();
            assert!(state.phases[..8].iter().all(|x| *x > 0.0));
            assert_eq!(&state.phases[8..], &[0.0; 8]);
            inputs[0].channels = 1;
            let output = osc.process(&inputs);
            assert_eq!(&output.values[1..], &[0.0; 15]);
            assert_eq!(&osc.snapshot().phases[4..8], &state.phases[4..8]);
            assert!(osc.snapshot().phases[3] > state.phases[3]);
            inputs[3].channels = 16;
            assert_eq!(
                osc.process(&inputs).channels,
                if kind == "wavetable_vco" { 16 } else { 1 }
            );
        }
    }

    #[test]
    fn lfo_fm_is_raw_while_audio_fm_mono_broadcasts_hidden_lanes() {
        let mut inputs = [PortFrame::default(); 4];
        inputs[0] = PortFrame::mono(1.0);
        let mut lfo = build("wavetable_lfo", json!({"fm_amount":1.0}), 48000);
        lfo.process(&inputs);
        assert_eq!(lfo.snapshot().phases[0], lfo.snapshot().phases[1] * 2.0);
        let mut vco = build("wavetable_vco", json!({"fm_amount":1.0}), 48000);
        vco.process(&inputs);
        assert_eq!(vco.snapshot().phases[0], vco.snapshot().phases[1]);
    }

    #[test]
    fn clock_initial_high_arms_then_measures_elapsed_and_disconnect_freezes() {
        let mut osc = build("wavetable_lfo", json!({}), 1000);
        let mut inputs = [PortFrame::default(); 4];
        inputs[3] = PortFrame::mono(10.0);
        osc.process(&inputs);
        assert_eq!(osc.snapshot().clock_frequency, 1.0);
        assert_eq!(osc.snapshot().clock_trigger, 1);
        inputs[3].values[0] = 0.0;
        osc.process(&inputs);
        inputs[3].values[0] = 10.0;
        osc.process(&inputs);
        assert_eq!(osc.snapshot().clock_frequency, 1.0 / (3.0 * 0.001_f32));
        assert_eq!(osc.snapshot().timer, 0.0);
        inputs[3].channels = 0;
        osc.process(&inputs);
        assert_eq!(osc.snapshot().clock_frequency, 2.0);
        assert_eq!(osc.snapshot().timer, 0.0);
        assert_eq!(osc.snapshot().clock_trigger, 1);
        inputs[3].channels = 1;
        osc.process(&inputs);
        assert_eq!(osc.snapshot().clock_frequency, 2.0);
    }

    #[test]
    fn clock_rejects_an_out_of_range_edge_but_starts_a_new_interval() {
        let mut osc = build("wavetable_lfo", json!({}), 48000);
        let mut inputs = [PortFrame::default(); 4];
        inputs[3] = PortFrame::mono(0.0);
        osc.process(&inputs);
        inputs[3].values[0] = 2.0;
        osc.process(&inputs);
        assert_eq!(osc.snapshot().clock_frequency, 1.0);
        assert_eq!(osc.snapshot().timer, 0.0);
        inputs[3].values[0] = 0.1;
        for _ in 0..48 {
            osc.process(&inputs);
        }
        let elapsed = osc.snapshot().timer + osc.sample_time;
        inputs[3].values[0] = 2.0;
        osc.process(&inputs);
        assert_eq!(osc.snapshot().clock_frequency, 1.0 / elapsed);
        assert_eq!(osc.snapshot().timer, 0.0);
    }

    #[test]
    fn reset_starts_high_and_initialize_preserves_detectors() {
        let mut osc = build("wavetable_lfo", json!({}), 48000);
        let mut inputs = [PortFrame::default(); 4];
        inputs[1] = PortFrame::mono(10.0);
        osc.process(&inputs);
        assert!(osc.snapshot().phases[0] > 0.0);
        inputs[1].values[0] = 0.0;
        osc.process(&inputs);
        inputs[1].values[0] = 10.0;
        assert_eq!(osc.process(&inputs).values[0], 5.0);
        assert_eq!(osc.snapshot().phases[0], 0.0);
        osc.initialize_state();
        assert!(osc.snapshot().reset_high[0]);
        osc.process(&inputs);
        assert!(osc.snapshot().phases[0] > 0.0);
        osc.reset_state();
        assert_eq!(osc.snapshot().clock_trigger, 2);
        assert_eq!(osc.snapshot().channels, 0);
    }

    #[test]
    fn soft_sync_strict_crossing_and_hard_mode_direction_reset() {
        let mut osc = build("wavetable_vco", json!({"sync_mode":1}), 48000);
        let mut inputs = [PortFrame::default(); 4];
        inputs[1] = PortFrame::mono(10.0);
        osc.process(&inputs);
        assert_eq!(osc.snapshot().directions[0], 1.0);
        inputs[1].values[0] = -10.0;
        osc.process(&inputs);
        inputs[1].values[0] = 0.0;
        osc.process(&inputs);
        assert_eq!(osc.snapshot().directions[0], -1.0);
        inputs[1].channels = 0;
        osc.process(&inputs);
        assert_eq!(osc.snapshot().last_sync_values[0], 0.0);
        osc.parameters[4] = 0.0;
        osc.process(&inputs);
        assert_eq!(osc.snapshot().directions, [1.0; 16]);
    }

    #[test]
    fn hard_sync_queue_freezes_by_group_and_initialize_retains_tail() {
        let mut osc = build("wavetable_vco", json!({}), 48000);
        let mut inputs = [PortFrame::default(); 4];
        inputs[0].channels = 5;
        inputs[1] = PortFrame::mono(-1.0);
        osc.process(&inputs);
        inputs[1].values[0] = 1.0;
        osc.process(&inputs);
        assert!(osc.runtime.corrections[4].iter().any(|x| *x != 0.0));
        let before = osc.runtime;
        osc.initialize_state();
        assert_eq!(osc.runtime.corrections, before.corrections);
        assert_eq!(osc.snapshot().phases, before.state.phases);
        inputs[0].channels = 1;
        for _ in 0..5 {
            osc.process(&inputs);
        }
        assert_eq!(osc.runtime.corrections[4], before.corrections[4]);
        assert_eq!(
            osc.snapshot().correction_positions[1],
            before.state.correction_positions[1]
        );
        inputs[0].channels = 5;
        osc.process(&inputs);
        assert_ne!(
            osc.snapshot().correction_positions[1],
            before.state.correction_positions[1]
        );
    }

    #[test]
    fn hard_sync_uses_the_advanced_wave_sample_and_fractional_reset() {
        let mut osc = build("wavetable_vco", json!({}), 48000);
        let mut inputs = [PortFrame::default(); 4];
        inputs[1] = PortFrame::mono(-1.0);
        osc.process(&inputs);
        osc.runtime.state.phases[0] = 0.2;
        let delta = C4 * osc.sample_time;
        let before = osc.bank.lookup(0.2 + delta, 0.0, 6.0);
        let reset_phase = 0.75 * delta;
        let after = osc.bank.lookup(reset_phase, 0.0, 6.0);
        inputs[1].values[0] = 3.0;
        let output = osc.process(&inputs);
        assert_eq!(osc.snapshot().phases[0], reset_phase);
        // Crossing at one quarter of this interval leaves a 0.75-frame
        // kernel delay. The source emits the old sample plus correction.
        let expected = (before + (after - before) * (SYNC_BLEP[12] - 1.0)) * 5.0;
        assert!((output.values[0] - expected).abs() < 1e-6);
        assert!(osc.runtime.corrections[0].iter().any(|x| *x != 0.0));
    }

    #[test]
    fn suspended_table_freezes_wave_state_but_updates_clock_width_and_lights() {
        for kind in ["wavetable_vco", "wavetable_lfo"] {
            let mut osc = build(kind, json!({}), 48000);
            let mut inputs = [PortFrame::default(); 4];
            osc.process(&inputs);
            osc.runtime.corrections[0][3] = 0.25;
            let before = osc.runtime;
            inputs[0].channels = 5;
            inputs[1] = PortFrame::mono(10.0);
            inputs[3] = PortFrame::mono(0.0);
            for _ in 0..15 {
                let output = osc.process_suspended(&inputs);
                assert_eq!(output.channels, 5);
                assert_eq!(output.values, [0.0; MAX_CHANNELS]);
            }
            let state = osc.snapshot();
            assert_eq!(state.phases, before.state.phases);
            assert_eq!(state.directions, before.state.directions);
            assert_eq!(state.last_sync_values, before.state.last_sync_values);
            assert_eq!(state.reset_high, before.state.reset_high);
            assert_eq!(state.last_position, before.state.last_position);
            assert_eq!(
                state.correction_positions,
                before.state.correction_positions
            );
            assert_eq!(osc.runtime.corrections, before.corrections);
            assert_eq!(state.lightdivider, 0);
            assert_eq!(&state.lights[..3], &[0.0, 0.0, 1.0]);
            if kind == "wavetable_lfo" {
                assert!(state.timer > 0.0);
                assert_eq!(state.clock_trigger, 0);
            }
        }
    }

    #[test]
    fn state_adoption_copies_runtime_but_keeps_receiving_bank_and_parameters() {
        let mut donor = build("wavetable_vco", json!({}), 48000);
        donor.process(&[PortFrame::default(); 4]);
        donor.runtime.corrections[0][8] = 0.25;
        let mut receiver = build("wavetable_vco", json!({"tune":12}), 48000);
        let bank = Arc::clone(&receiver.bank);
        assert!(receiver.adopt_from(&donor));
        assert_eq!(receiver.snapshot(), donor.snapshot());
        assert_eq!(receiver.runtime.corrections, donor.runtime.corrections);
        assert!(Arc::ptr_eq(&receiver.bank, &bank));
        assert_eq!(receiver.parameters[0], 12.0);
        assert!(!build("wavetable_vco", json!({}), 44100).adopt_from(&donor));
        assert!(!build("wavetable_lfo", json!({}), 48000).adopt_from(&donor));
    }

    #[test]
    fn through_zero_frequency_wraps_and_zero_holds_without_nonfinite_output() {
        let mut osc = build("wavetable_vco", json!({"fm_mode":1,"fm_amount":1}), 48000);
        let mut inputs = [PortFrame::default(); 4];
        inputs[0] = PortFrame::mono(-1.0);
        osc.process(&inputs);
        assert_eq!(osc.snapshot().phases[0], 0.0);
        inputs[0].values[0] = -2.0;
        osc.process(&inputs);
        assert!(osc.snapshot().phases[0] > 0.99);
        for value in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::MAX,
            -f32::MAX,
        ] {
            inputs[0].values[0] = value;
            let output = osc.process(&inputs);
            assert!(output
                .values
                .iter()
                .all(|x| x.is_finite() && x.abs() <= SIGNAL_LIMIT));
        }
    }

    #[test]
    fn through_zero_preserves_phase_and_uses_magnitude_for_filtering_and_sync() {
        let mut forward = build("wavetable_vco", json!({"fm_mode":1,"fm_amount":1}), 48000);
        let mut backward = build("wavetable_vco", json!({"fm_mode":1,"fm_amount":1}), 48000);
        forward.runtime.state.phases[0] = 0.25;
        backward.runtime.state.phases[0] = 0.25;
        let mut inputs = [PortFrame::default(); 4];
        inputs[0] = PortFrame::mono(1.0);
        forward.process(&inputs);
        inputs[0].values[0] = -3.0;
        let output = backward.process(&inputs);
        let phase = backward.snapshot().phases[0];
        assert!((forward.snapshot().phases[0] + phase - 0.5).abs() < 1e-7);
        let octave = (24000.0 / (2.0 * C4)).log2();
        assert_eq!(
            output.values[0],
            backward.bank.lookup(phase, 0.0, octave) * 5.0
        );
        inputs[0].values[0] = -1.0;
        inputs[1] = PortFrame::mono(-1.0);
        backward.process(&inputs);
        assert_eq!(backward.snapshot().phases[0], phase);
        inputs[1].values[0] = 1.0;
        assert!(backward.process(&inputs).values[0].is_finite());
        assert_eq!(backward.snapshot().phases[0], 0.0);
        assert!(backward.runtime.corrections[0].iter().any(|x| *x != 0.0));
        inputs[0].values[0] = 0.0;
        backward.process(&inputs);
        assert_eq!(backward.snapshot().phases[0], C4 * backward.sample_time);
    }

    #[test]
    fn table_position_cv_offset_invert_and_lights_follow_the_controls() {
        let samples: Arc<[f32]> = [vec![-0.5; 32], vec![0.5; 32]].concat().into();
        let bank =
            Arc::new(PreparedWaveBank::prepare(samples, 32, WavetableMode::Modulation).unwrap());
        let mut osc = WavetableOscillator::build(
            &node(
                "wavetable_lfo",
                json!({"position":0.25,"position_cv_amount":0.5,"invert":1,"offset":1}),
            ),
            48000,
            bank,
        )
        .unwrap();
        let mut inputs = [PortFrame::default(); 4];
        inputs[2] = PortFrame::mono(10.0);
        for _ in 0..15 {
            assert_eq!(osc.process(&inputs).values[0], 3.75);
        }
        assert_eq!(osc.snapshot().lights, [0.0; 5]);
        osc.process(&inputs);
        assert_eq!(osc.snapshot().last_position, 0.75);
        assert_eq!(osc.module_lights().waveform_position, Some(0.75));
        assert_eq!(osc.snapshot().lightdivider, 0);
        assert_eq!(osc.snapshot().lights[3..], [1.0, 1.0]);
        assert!(osc.snapshot().lights[0] > 0.0);
        assert_eq!(osc.snapshot().lights[0], osc.snapshot().lights[1]);
    }
}
