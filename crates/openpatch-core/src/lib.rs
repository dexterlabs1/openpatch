//! Original modular DSP for OpenPatch. Version 1 is the unchanged mono DAG;
//! version 2 has named 0–16-channel ports and one sample of delay per cable,
//! including feedback loops. New sessions should use `Patch::new_v2()`.
//!
//! Compile on a control thread, then move the engine to the audio thread.
//! `process_block` performs no allocation, locking, or I/O. Unpatched inputs
//! are zero, except gain.cv defaults to 1, analog_vca.cv to 10 V, and
//! attenuverter.signal to 10 V. Signals are unclipped in normal use, with a
//! ±1,000,000 safety bound on arithmetic modules; WAV export clips to ±1.

/// Full notices for third-party algorithms included in this crate.
pub const THIRD_PARTY_LICENSES: &str = include_str!("../LICENSE-THIRD-PARTY");

use serde::{Deserialize, Serialize};
mod cable_order;
pub use cable_order::{
    assign_new_cable_orders, cable_endpoint_order, promote_cable_endpoints, CableEndpoint,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;
use std::fs::OpenOptions;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

mod live_parameters;
mod sequence_ownership;
pub use live_parameters::{
    runtime_parameter_descriptor, runtime_parameter_names, ParameterDescriptor, ParameterHandle,
};
mod adsr_eg;
mod analog;
mod score;
pub use score::{ScoreCurve, ScoreData, ScoreLane, ScorePoint, ScoreStep, ScoreTrack};
mod rack264_vcf;
#[doc(hidden)]
pub mod rackmcp_sequence;
#[doc(hidden)]
pub use rack264_vcf::{Rack264VcfProbe, Rack264VcfSnapshot};
mod rack264_vco;
mod stereo_output;
mod stock_delay;
mod stock_noise;
mod stock_vco;
#[doc(hidden)]
pub use rack264_vco::{Rack264VcoProbe, Rack264VcoSnapshot};
#[doc(hidden)]
pub use stock_noise::{StockNoiseDrawEvent, StockNoiseSnapshot};
#[doc(hidden)]
pub use stock_vco::{StockVcoKernelSnapshot, StockVcoProbe, StockVcoSnapshot};
mod host_ports;
mod native_float;
mod stock_mid_side;
mod stock_mix;
mod stock_octave;
mod stock_process;
mod stock_scope;
#[cfg(test)]
mod stock_scope_tests;
#[doc(hidden)]
pub use stock_scope::{StockScopeDiagnosticBits, StockScopePointBits};
mod stock_vca_mix;
#[doc(hidden)]
pub use stock_mix::StockMixSnapshot;
pub use stock_octave::StockOctaveSnapshot;
#[doc(hidden)]
pub use stock_process::{
    StockProcessEngineBits, StockProcessEngineSnapshot, StockProcessSnapshot,
    StockProcessSnapshotBits,
};
pub use stock_vca_mix::{StockVcaMixMeterSnapshot, StockVcaMixSnapshot};
mod stock_quantizer;
#[doc(hidden)]
pub use host_ports::{HostCableSnapshot, HostModuleSnapshot, HostPortLifecycle, HostPortSnapshot};
pub use stock_quantizer::rotate_stock_quantizer_mask;
#[doc(hidden)]
pub use stock_quantizer::{StockQuantizerProbe, StockQuantizerSnapshot};
mod stock_seq3;
pub use stock_seq3::initial_stock_seq3_state;
#[doc(hidden)]
pub use stock_seq3::{StockSeq3Probe, StockSeq3Snapshot};
mod stock_lfo;
#[doc(hidden)]
pub use stock_lfo::{StockLfoProbe, StockLfoSnapshot};
mod stock_vca;
mod stock_vcf;
mod stream_render;
mod studio_fx;
mod studio_mixer;
pub use stream_render::{render_wav_stream, RenderStats};
mod bypass;
mod colored_noise;
mod content;
pub use colored_noise::colored_noise_response_db;
pub use content::{
    validate_content, ModuleContent, DEFAULT_BLANK_WIDTH_HP, MAX_BLANK_WIDTH_HP, MAX_NOTES_BYTES,
    MIN_BLANK_WIDTH_HP, NOTES_WIDTH_HP,
};
mod gates;
pub mod ladder;
pub mod midi;
pub mod midi_cc;
pub mod midi_gate;
mod midi_learn;
pub use midi_cc::{initial_midi_cc_state, MidiCcLearnCompletion};
pub use midi_gate::initial_midi_gate_state;
pub use midi_learn::{MidiLearnCompletion, MidiLearnKind};
mod macro_voice;
mod merge;
mod mixing;
mod modal_resonator;
mod multi_lfo;
mod multi_vco;
mod split;
pub use modal_resonator::{modal_resonator_modes, ModalResonatorMode, MODAL_RESONATOR_MODES};
mod parameter_actions;
mod svf_resampler;
mod voice_tools;
pub use parameter_actions::ModuleParameterActions;
mod poly;
mod wave_assets;
mod wave_prepare;
mod wavetable_bank;
pub mod wavetable_file;
mod wavetable_minblep;
mod wavetable_osc;
pub use wave_prepare::{PreparedWaveBanks, WavePreparationCache, WavePreparationStats};
/// Original mathematically generated sine/triangle/saw/square table (4×1024).
/// Construction allocates; use only on a control thread.
pub fn default_wavetable_asset() -> std::sync::Arc<WaveAsset> {
    wave_prepare::basic_asset()
}
#[doc(hidden)]
pub mod wavetable_probe;
pub use wave_assets::{
    AssetId, WaveAsset, WaveAssets, MAX_HISTORY_ASSET_BYTES, MAX_PATCH_ASSETS,
    MAX_PATCH_ASSET_BYTES, MAX_PATCH_FILE_BYTES,
};
pub mod port_lights;
pub use port_lights::{PortDirection, PortLightState, ResolvedPortLight};
mod input_lifecycle;
#[doc(hidden)]
pub use input_lifecycle::{InputLifecycleCountError, InputLifecycleTransform};
mod port_lifecycle;
#[doc(hidden)]
pub use port_lifecycle::{
    OutputLifecycleCountError, OutputLifecycleEntry, OutputLifecycleTransform,
};
mod pulses;
mod push;
mod shift_register;
pub use shift_register::initial_shift_register_state;
mod random_modulation;
#[doc(hidden)]
pub use random_modulation::{RandomProbe, RandomSnapshot};
mod random_values;
pub use random_values::{initial_random_values_state, randomize_random_values_state};
mod scope;
mod scope_measurement;
pub use scope::{ScopePoint, ScopeSnapshot, StockScopeMetadata, SCOPE_BINS};
pub use scope_measurement::{ScopeExtrema, ScopeMeasurements};
mod saved_state;
mod sum;
mod viz;
pub use saved_state::SavedModuleState;
mod routing;
mod sequence;
mod signal;
pub use signal::{ModuleControlState, ModuleLightState, MAX_MODULE_LIGHTS};
mod switch;
mod utility_bank;
pub use midi::{parse_midi, MidiEvent, TimedMidiEvent};
pub use sequence::{
    validate_module_events, ModuleAction, ModuleHandle, ModuleTelemetry, TimedModuleEvent,
    MAX_MODULE_ACTIONS_PER_SAMPLE, MAX_MODULE_EVENTS, MODULE_ACTION_NAMES,
};
mod utilities;
pub use poly::{PortFrame, MAX_CHANNELS};

pub const BLOCK_SIZE: usize = 128;
pub const MAX_NODES: usize = 256;
pub const MAX_CABLES: usize = 1024;
const SIGNAL_LIMIT: f32 = 1_000_000.0;

fn default_version() -> u32 {
    1
}
fn default_output_port() -> String {
    "out".into()
}
fn is_default_output_port(value: &str) -> bool {
    value == "out"
}

fn default_rate() -> u32 {
    48_000
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    /// Immutable sample payloads, deduplicated by content digest. Names and wave
    /// interpretation belong to the consuming module, not to sample identity.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "wave_assets::deserialize_assets"
    )]
    pub assets: WaveAssets,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default = "default_rate")]
    pub sample_rate: u32,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub cables: Vec<Cable>,
    /// ID of the node to hear. `None` is a silent graph, useful while editing.
    #[serde(default)]
    pub output: Option<String>,
    /// Named output of the selected node; version 1 supports only `out`.
    #[serde(
        default = "default_output_port",
        skip_serializing_if = "is_default_output_port"
    )]
    pub output_port: String,
}

impl Default for Patch {
    fn default() -> Self {
        Self {
            assets: WaveAssets::new(),
            version: 1,
            sample_rate: 48_000,
            nodes: Vec::new(),
            cables: Vec::new(),
            output: None,
            output_port: default_output_port(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    /// Persisted annotation content; absent Notes/Blank content uses defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<ModuleContent>,
    /// Structured patterns and automation for score_sequencer only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<ScoreData>,
    /// Source-version RackMCP sequence and explicit native target bindings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rackmcp_sequence: Option<Box<rackmcp_sequence::SequenceState>>,
    pub id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub bypassed: bool,
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
}
fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cable {
    /// Independent visual plug ranks. Absent ranks sort as zero with stable
    /// cable-array ties; neither field participates in DSP routing identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_order: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_order: Option<u16>,
    /// Optional persisted RGB bytes. Pure presentation metadata: not part of
    /// route identity or DSP state adoption. Omitted colors use the UI palette.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    /// Stable identity in the cable namespace; older patches may omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub from: String,
    pub to: String,
    pub input: String,
    /// Named source output. Older JSON omitting this field selects `out`.
    #[serde(
        default = "default_output_port",
        skip_serializing_if = "is_default_output_port"
    )]
    pub output: String,
}

fn validate_cable_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
    {
        return Err("cable IDs must contain 1..=64 ASCII letters, digits, '_', '-', or '.'".into());
    }
    Ok(())
}

fn validate_cable_ids(cables: &[Cable]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for cable in cables {
        if let Some(id) = &cable.id {
            validate_cable_id(id)?;
            if !seen.insert(id) {
                return Err(format!("duplicate cable ID '{id}'"));
            }
        }
    }
    Ok(())
}

/// Machine-readable module descriptions. Bounds depending on the sample rate
/// are checked by `validate`; input port names are also the cable `input` names.
pub fn catalog() -> Value {
    let mut result = json!({
        "version": 2,
        "patch_versions": [1, 2],
        "default_patch_version": 1,
        "wire_default_patch_version": 1,
        "new_session_patch_version": 2,
        "recommended_patch_version": 2,
        "timing": {"1":"same-sample DAG; cycles rejected", "2":"every cable delays values and channel count by exactly one sample; loops supported"},
        "sample_rate": {"min": 8000, "max": 192000, "default": 48000},
        "limits": {"nodes": MAX_NODES, "cables": MAX_CABLES, "block_size": BLOCK_SIZE},
        "unpatched_inputs": "zero except gain.cv defaults to 1, analog_vca.cv defaults to 10 V, and attenuverter.signal defaults to 10 V", "cycles": "version 1 rejected; version 2 delayed", "channels": {"1":1,"2_max":16},
        "modules": [
            {"kind":"oscillator", "description":"Sine oscillator; FM is additive Hz, instantaneous frequency clamped to 0..0.45 × sample rate", "inputs":["fm"], "params":{
                "frequency":{"default":220.0,"min":0.0,"max":20000.0,"unit":"Hz","sample_rate_max_ratio":0.45},
                "amplitude":{"default":0.2,"min":0.0,"max":1.0}}},
            {"kind":"lfo", "description":"Sine low-frequency oscillator with DC offset", "inputs":["fm"], "params":{
                "frequency":{"default":2.0,"min":0.0,"max":100.0,"unit":"Hz"},
                "amplitude":{"default":0.5,"min":0.0,"max":1.0},
                "offset":{"default":0.5,"min":-1.0,"max":1.0}}},
            {"kind":"constant", "inputs":[], "params":{"value":{"default":0.0,"min":-16.0,"max":16.0}}},
            {"kind":"gain", "description":"signal × gain; optional cv multiplies gain when connected", "inputs":["signal","cv"], "params":{"gain":{"default":1.0,"min":-16.0,"max":16.0}}},
            {"kind":"mixer", "description":"a + b", "inputs":["a","b"], "params":{}},
            {"kind":"lowpass", "description":"One-pole low-pass; coefficient computed at compile time", "inputs":["signal"], "params":{"cutoff":{"default":1000.0,"min":1.0,"max":20000.0,"unit":"Hz","sample_rate_max_ratio":0.45}}},
            {"kind":"output", "inputs":["signal"], "params":{}},
            {"kind":"analog_vco", "title":"VCO", "model":"idealized", "description":"Ideal capacitor-ramp VCO; ±5 V saw/pulse, polynomial BLEP. Pitch 1 V/oct; FM 1 Hz/V. Not a transistor-level oscillator.", "inputs":["pitch","fm"], "params":{
                "frequency":{"default":261.6256,"min":0.1,"max":18000.0,"unit":"Hz","sample_rate_max_ratio":0.4},
                "waveform":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1],"labels":["Saw","Pulse"]},
                "pulse_width":{"default":0.5,"min":0.05,"max":0.95,"unit":"ratio"}}},
            {"kind":"ladder_vcf", "title":"LADDER", "model":"circuit-derived", "description":"Matched-transistor ladder, four 10 nF stages at 25°C; nonlinear coupled feedback. Pole frequency is -12 dB at zero resonance. CV 1 V/oct.", "inputs":["signal","cutoff_cv"], "params":{
                "cutoff":{"default":1200.0,"min":20.0,"max":18000.0,"unit":"Hz","sample_rate_max_ratio":0.4},
                "resonance":{"default":0.3,"min":0.0,"max":1.1},
                "drive":{"default":1.0,"min":0.0,"max":4.0,"unit":"x"}}},
            {"kind":"analog_vca", "title":"OTA VCA", "model":"circuit-derived", "description":"Ideal matched differential-pair OTA, 25°C, 1:100 input divider. Linear current control: CV 0–10 V; unpatched CV 10 V.", "inputs":["signal","cv"], "params":{
                "gain":{"default":1.0,"min":0.0,"max":2.0,"unit":"x"}}},
            {"kind":"adsr", "title":"ADSR", "model":"circuit-derived", "description":"1 µF switched RC envelope, 0–10 V; 12 V attack target. A: time to 10 V; D/R: 99% settling times. Gate Schmitt thresholds 0.1/1 V.", "inputs":["gate","retrigger"], "params":{
                "attack":{"default":0.01,"min":0.001,"max":10.0,"unit":"s"},
                "decay":{"default":0.2,"min":0.001,"max":10.0,"unit":"s"},
                "sustain":{"default":0.5,"min":0.0,"max":1.0,"unit":"ratio"},
                "release":{"default":0.3,"min":0.001,"max":10.0,"unit":"s"}}},
            {"kind":"gate", "title":"GATE", "model":"idealized", "description":"Ideal 0/10 V clock gate; starts high. Sharp comparator edges for envelope testing.", "inputs":[], "params":{
                "frequency":{"default":2.0,"min":0.05,"max":100.0,"unit":"Hz"},
                "duty":{"default":0.5,"min":0.01,"max":0.99,"unit":"ratio"}}},
            {"kind":"audio_out", "title":"AUDIO", "model":"idealized", "description":"Voltage-to-audio interface: 10 V = digital full scale (1.0). Select this module as rack output for voltage patches.", "inputs":["signal"], "params":{}}
        ]
    });
    result["modules"]
        .as_array_mut()
        .unwrap()
        .extend(utilities::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .extend(poly::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .extend(routing::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(multi_lfo::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(multi_vco::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(macro_voice::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(studio_mixer::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(sequence::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(stock_seq3::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(stock_quantizer::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(stock_mid_side::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(stock_mix::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(stock_octave::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(stock_process::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(stock_scope::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(stock_vca_mix::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(pulses::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(push::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(adsr_eg::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(stock_vcf::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(stock_delay::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(stock_lfo::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(stock_vco::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(rack264_vco::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(rack264_vcf::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(stock_noise::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .extend(wavetable_osc::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(stock_vca::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(random_values::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(random_modulation::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(shift_register::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(scope::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(gates::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .extend(switch::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .extend(utility_bank::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .extend(signal::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .extend(mixing::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(score::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(rackmcp_sequence::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .extend(studio_fx::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .extend(voice_tools::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(modal_resonator::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .push(colored_noise::catalog());
    result["modules"]
        .as_array_mut()
        .unwrap()
        .extend(content::catalog());
    for module in result["modules"].as_array_mut().unwrap() {
        if module.get("model").is_none() {
            module["model"] = json!("legacy");
        }
        poly::annotate_catalog(module);
        bypass::annotate(module);
        if module["kind"].as_str() == Some("stock_process") {
            stock_process::annotate_bypass(module);
        }
        if module["kind"].as_str() == Some("stock_scope") {
            stock_scope::annotate_bypass(module);
        }
        parameter_actions::annotate_catalog(module);
        if module["kind"].as_str() == Some("stock_process") {
            module["params"]["slew"]["randomization"] = json!("linear");
        }
    }
    result["signal_conventions"] = json!({"reference_modules":"volts; audio_out converts 10 V to 1.0 digital","legacy_modules":"unchanged normalized utilities; no automatic voltage conversion","functional_modules":"volts; white_noise.level is RMS volts; colored_noise.level is a multiplier of normalized color voltages; no automatic antialiasing for discontinuous quantizer/sample_hold outputs","physical_modules":"volts; modal_resonator excite uses 1 N/V and out is a velocity pickup at 25 V*s/m before level", "pitch":"1 V/oct for analog_vco.pitch, ladder_vcf.cutoff_cv and modal_resonator.pitch"});
    result
}

#[derive(Clone, Debug)]
enum Dsp {
    Utility(utilities::Utility),
    Vco(analog::Vco),
    Ladder(Box<ladder::Ladder>),
    Vca(Box<analog::Vca>),
    Adsr(analog::Adsr),
    Gate(analog::Gate),
    AudioOut,
    Oscillator {
        phase: f64,
        frequency: f32,
        amplitude: f32,
        offset: f32,
    },
    Constant(f32),
    Gain(f32),
    Mixer,
    Lowpass {
        coefficient: f32,
        state: f32,
    },
    Output,
}

impl Dsp {
    fn copy_state_from(&mut self, old: &Self) {
        match (self, old) {
            (Self::Utility(current), Self::Utility(old)) => current.copy_state_from(old),
            (Self::Vco(current), Self::Vco(old)) => current.phase = old.phase,
            (Self::Gate(current), Self::Gate(old)) => current.phase = old.phase,
            (Self::Ladder(current), Self::Ladder(old)) => current.copy_state_from(old),
            (Self::Vca(current), Self::Vca(old)) => current.copy_state_from(old),
            (Self::Adsr(current), Self::Adsr(old)) => current.copy_state_from(old),
            (
                Self::Oscillator { phase, .. },
                Self::Oscillator {
                    phase: old_phase, ..
                },
            ) => *phase = *old_phase,
            (
                Self::Lowpass { state, .. },
                Self::Lowpass {
                    state: old_state, ..
                },
            ) => *state = *old_state,
            _ => {}
        }
    }
    fn solver_failures(&self) -> u64 {
        if let Self::Ladder(filter) = self {
            filter.diagnostics().solver_failures
        } else {
            0
        }
    }
    fn process_sample(
        &mut self,
        a: f32,
        b: f32,
        a_connected: bool,
        b_connected: bool,
        rate: f64,
    ) -> f32 {
        match self {
            Self::Utility(utility) => utility.process(a, b, a_connected),
            Self::Vco(vco) => vco.process(a, b),
            Self::Ladder(filter) => filter.process(a, b),
            Self::Vca(vca) => vca.process(a, if b_connected { b } else { 10.0 }),
            Self::Adsr(envelope) => envelope.process(a, b),
            Self::Gate(gate) => gate.process(),
            Self::AudioOut => a * 0.1,
            Self::Oscillator {
                phase,
                frequency,
                amplitude,
                offset,
            } => {
                let value = (*phase * TAU).sin() as f32 * *amplitude + *offset;
                let hz = (*frequency as f64 + a as f64).clamp(0.0, rate * 0.45);
                *phase += hz / rate;
                if *phase >= 1.0 {
                    *phase -= 1.0;
                }
                value
            }
            Self::Constant(value) => *value,
            Self::Gain(gain) => bounded(a * *gain * if b_connected { b } else { 1.0 }),
            Self::Mixer => bounded(a + b),
            Self::Lowpass { coefficient, state } => {
                *state += *coefficient * (a - *state);
                if state.abs() < 1e-20 {
                    *state = 0.0;
                }
                *state
            }
            Self::Output => a,
        }
    }

    fn port(&self, name: &str) -> Option<usize> {
        if let Self::Utility(utility) = self {
            return utility.port(name);
        }
        match (self, name) {
            (Self::Vco(_), "pitch") | (Self::Adsr(_), "gate") => Some(0),
            (Self::Vco(_), "fm") | (Self::Adsr(_), "retrigger") => Some(1),
            (Self::Ladder(_), "signal") | (Self::Vca(_), "signal") | (Self::AudioOut, "signal") => {
                Some(0)
            }
            (Self::Ladder(_), "cutoff_cv") | (Self::Vca(_), "cv") => Some(1),
            (Self::Oscillator { .. }, "fm") => Some(0),
            (Self::Gain(_), "signal")
            | (Self::Lowpass { .. }, "signal")
            | (Self::Output, "signal") => Some(0),
            (Self::Gain(_), "cv") => Some(1),
            (Self::Mixer, "a") => Some(0),
            (Self::Mixer, "b") => Some(1),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct CompiledNode {
    // Exact model identity distinguishes oscillator and LFO even though they
    // share an implementation. Built on the control thread with the graph.
    kind: String,
    bypassed: bool,
    bypass_input: Option<usize>,
    dsp: Dsp,
    inputs: [Option<usize>; 2],
}

/// Stateful compiled graph. Construct and replace engines outside the audio
/// callback; process any slice length without allocating intermediate buffers.
#[derive(Debug)]
struct LegacyEngine {
    sample_rate: f64,
    nodes: Vec<CompiledNode>,
    state_lookup: BTreeMap<String, usize>,
    buffers: Vec<[f32; BLOCK_SIZE]>,
    output: Option<usize>,
    port_lights: port_lights::Observer,
}

fn parameter(node: &Node, name: &str, default: f32, min: f32, max: f32) -> Result<f32, String> {
    let value = node.params.get(name).copied().unwrap_or(default);
    if !value.is_finite() || value < min || value > max {
        return Err(format!(
            "node '{}': parameter '{}' must be finite and in {min}..={max}",
            node.id, name
        ));
    }
    Ok(value)
}

fn build_dsp(node: &Node, rate: u32) -> Result<Dsp, String> {
    let allowed: &[&str] = match node.kind.as_str() {
        "analog_vco" => &["frequency", "waveform", "pulse_width"],
        "ladder_vcf" => &["cutoff", "resonance", "drive"],
        "analog_vca" => &["gain"],
        "adsr" => &["attack", "decay", "sustain", "release"],
        "gate" => &["frequency", "duty"],
        "audio_out" => &[],
        "oscillator" => &["frequency", "amplitude"],
        "lfo" => &["frequency", "amplitude", "offset"],
        "constant" => &["value"],
        "gain" => &["gain"],
        "mixer" | "output" => &[],
        "lowpass" => &["cutoff"],
        _ => utilities::allowed(&node.kind)
            .ok_or_else(|| format!("node '{}': unknown module kind '{}'", node.id, node.kind))?,
    };
    for name in node.params.keys() {
        if !allowed.contains(&name.as_str()) {
            return Err(format!("node '{}': unknown parameter '{}'", node.id, name));
        }
    }
    Ok(match node.kind.as_str() {
        "analog_vco" => {
            let waveform = parameter(node, "waveform", 0.0, 0.0, 1.0)?;
            if waveform.fract() != 0.0 {
                return Err("waveform must be 0 (saw) or 1 (pulse)".into());
            }
            Dsp::Vco(analog::Vco::new(
                rate as f64,
                parameter(
                    node,
                    "frequency",
                    261.6256,
                    0.1,
                    (rate as f32 * 0.4).min(18000.0),
                )? as f64,
                waveform,
                parameter(node, "pulse_width", 0.5, 0.05, 0.95)? as f64,
            ))
        }
        "ladder_vcf" => Dsp::Ladder(Box::new(ladder::Ladder::new(
            rate as f64,
            parameter(
                node,
                "cutoff",
                1200.0,
                20.0,
                (rate as f32 * 0.4).min(18000.0),
            )? as f64,
            parameter(node, "resonance", 0.3, 0.0, 1.1)? as f64,
            parameter(node, "drive", 1.0, 0.0, 4.0)? as f64,
        ))),
        "analog_vca" => Dsp::Vca(Box::new(analog::Vca::new(
            parameter(node, "gain", 1.0, 0.0, 2.0)? as f64,
        ))),
        "adsr" => Dsp::Adsr(analog::Adsr::new(
            rate as f64,
            parameter(node, "attack", 0.01, 0.001, 10.0)? as f64,
            parameter(node, "decay", 0.2, 0.001, 10.0)? as f64,
            parameter(node, "sustain", 0.5, 0.0, 1.0)? as f64,
            parameter(node, "release", 0.3, 0.001, 10.0)? as f64,
        )),
        "gate" => Dsp::Gate(analog::Gate::new(
            rate as f64,
            parameter(node, "frequency", 2.0, 0.05, 100.0)? as f64,
            parameter(node, "duty", 0.5, 0.01, 0.99)? as f64,
        )),
        "audio_out" => Dsp::AudioOut,
        "oscillator" | "lfo" => {
            let lfo = node.kind == "lfo";
            Dsp::Oscillator {
                phase: 0.0,
                frequency: parameter(
                    node,
                    "frequency",
                    if lfo { 2.0 } else { 220.0 },
                    0.0,
                    if lfo {
                        100.0
                    } else {
                        (rate as f32 * 0.45).min(20_000.0)
                    },
                )?,
                amplitude: parameter(node, "amplitude", if lfo { 0.5 } else { 0.2 }, 0.0, 1.0)?,
                offset: if lfo {
                    parameter(node, "offset", 0.5, -1.0, 1.0)?
                } else {
                    0.0
                },
            }
        }
        "constant" => Dsp::Constant(parameter(node, "value", 0.0, -16.0, 16.0)?),
        "gain" => Dsp::Gain(parameter(node, "gain", 1.0, -16.0, 16.0)?),
        "mixer" => Dsp::Mixer,
        "lowpass" => {
            let cutoff = parameter(
                node,
                "cutoff",
                1000.0,
                1.0,
                (rate as f32 * 0.45).min(20_000.0),
            )?;
            // exp_m1 avoids cancellation for low cutoffs at high sample rates.
            let coefficient = -(-TAU * cutoff as f64 / rate as f64).exp_m1() as f32;
            Dsp::Lowpass {
                coefficient,
                state: 0.0,
            }
        }
        "output" => Dsp::Output,
        kind if utilities::allowed(kind).is_some() => {
            Dsp::Utility(utilities::Utility::build(node)?)
        }
        _ => unreachable!(),
    })
}

impl Patch {
    /// Assign stable IDs without changing cable order or any existing identity.
    /// Explicit IDs are reserved first, so allocation is deterministic for a
    /// candidate patch and cannot collide with IDs later in the array.
    pub fn assign_missing_cable_ids(&mut self) -> Result<(), String> {
        let mut used = BTreeSet::new();
        for cable in &self.cables {
            if let Some(id) = &cable.id {
                validate_cable_id(id)?;
                if !used.insert(id.clone()) {
                    return Err(format!("duplicate cable ID '{id}'"));
                }
            }
        }
        let mut next = 1usize;
        for cable in &mut self.cables {
            if cable.id.is_none() {
                while used.contains(&format!("cable_{next}")) {
                    next += 1;
                }
                let id = format!("cable_{next}");
                used.insert(id.clone());
                cable.id = Some(id);
                next += 1;
            }
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        self.compile().map(|_| ())
    }

    /// Create an empty graph with one-sample cable timing and polyphonic ports.
    pub fn new_v2() -> Self {
        Self {
            version: 2,
            ..Self::default()
        }
    }

    pub fn compile(&self) -> Result<Engine, String> {
        let banks = self.prepare_wave_banks(&WavePreparationCache::default())?;
        self.compile_with_wave_banks(&banks)
    }

    /// Compile using control-thread prepared immutable wave banks. Callers that
    /// build revisions should retain a shared cache and prepare before commit.
    pub fn compile_with_wave_banks(&self, banks: &PreparedWaveBanks) -> Result<Engine, String> {
        banks.validate(self)?;
        wave_assets::validate_assets(&self.assets)?;
        if self.version != 2 && !self.assets.is_empty() {
            return Err("wavetable assets require patch version 2".into());
        }
        validate_cable_ids(&self.cables)?;
        for node in &self.nodes {
            validate_content(node)?;
            if self.version != 2 && (node.content.is_some() || content::is_annotation(&node.kind)) {
                return Err(format!(
                    "node {}: content and annotation modules require patch version 2",
                    node.id
                ));
            }
            if node.score.is_some() && (node.kind != "score_sequencer" || self.version != 2) {
                return Err(format!(
                    "node {}: score data requires version-2 score_sequencer",
                    node.id
                ));
            }
            if node.rackmcp_sequence.is_some()
                && (node.kind != "rackmcp_sequencer" || self.version != 2)
            {
                return Err(format!(
                    "node {}: RackMCP sequence data requires version-2 rackmcp_sequencer",
                    node.id
                ));
            }
        }
        let inner = match self.version {
            1 => EngineKind::Legacy(self.compile_legacy()?),
            2 => EngineKind::Poly(Box::new(poly::PolyEngine::compile_with_wave_banks(
                self, banks,
            )?)),
            version => {
                return Err(format!(
                    "unsupported patch version {version} (expected 1 or 2)"
                ))
            }
        };
        Ok(Engine {
            inner,
            identity: NEXT_ENGINE_ID.fetch_add(1, Ordering::Relaxed),
        })
    }

    fn compile_legacy(&self) -> Result<LegacyEngine, String> {
        if self.output_port != "out" || self.cables.iter().any(|c| c.output != "out") {
            return Err(
                "version 1 supports only source output 'out'; use version 2 for named outputs"
                    .into(),
            );
        }
        if self.nodes.iter().any(|node| poly::is_poly_kind(&node.kind)) {
            return Err("polyphonic utility modules require patch version 2".into());
        }
        if self.version != 1 {
            return Err(format!(
                "unsupported patch version {} (expected 1)",
                self.version
            ));
        }
        if !(8000..=192000).contains(&self.sample_rate) {
            return Err("sample_rate must be in 8000..=192000 Hz".into());
        }
        if self.nodes.len() > MAX_NODES {
            return Err(format!("patch exceeds {MAX_NODES} nodes"));
        }
        if self.cables.len() > MAX_CABLES {
            return Err(format!("patch exceeds {MAX_CABLES} cables"));
        }
        let mut ids = BTreeMap::new();
        let mut modules = Vec::with_capacity(self.nodes.len());
        for (i, node) in self.nodes.iter().enumerate() {
            if node.id.is_empty()
                || node.id.len() > 64
                || !node
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
            {
                return Err(
                    "node IDs must contain 1..=64 ASCII letters, digits, '_', '-', or '.'".into(),
                );
            }
            if ids.insert(node.id.as_str(), i).is_some() {
                return Err(format!("duplicate node ID '{}'", node.id));
            }
            modules.push(Some(build_dsp(node, self.sample_rate)?));
        }
        let output = self
            .output
            .as_deref()
            .map(|id| {
                ids.get(id)
                    .copied()
                    .ok_or_else(|| format!("output node '{id}' does not exist"))
            })
            .transpose()?;
        let count = self.nodes.len();
        let mut inputs = vec![[None; 2]; count];
        let mut outgoing = vec![Vec::new(); count];
        let mut indegree = vec![0usize; count];
        for cable in &self.cables {
            let from = *ids
                .get(cable.from.as_str())
                .ok_or_else(|| format!("cable source '{}' does not exist", cable.from))?;
            let to = *ids
                .get(cable.to.as_str())
                .ok_or_else(|| format!("cable destination '{}' does not exist", cable.to))?;
            let port = modules[to]
                .as_ref()
                .unwrap()
                .port(&cable.input)
                .ok_or_else(|| format!("node '{}': unknown input '{}'", cable.to, cable.input))?;
            if inputs[to][port].replace(from).is_some() {
                return Err(format!(
                    "node '{}': input '{}' already has a cable",
                    cable.to, cable.input
                ));
            }
            outgoing[from].push(to);
            indegree[to] += 1;
        }
        // Stable lexical ordering makes independent branches deterministic even
        // when a client rearranges the node list. Sources always precede sinks.
        let mut ready = BTreeSet::new();
        for (i, degree) in indegree.iter().enumerate() {
            if *degree == 0 {
                ready.insert((self.nodes[i].id.as_str(), i));
            }
        }
        let mut order = Vec::with_capacity(count);
        while let Some(&(id, index)) = ready.iter().next() {
            ready.remove(&(id, index));
            order.push(index);
            for &next in &outgoing[index] {
                indegree[next] -= 1;
                if indegree[next] == 0 {
                    ready.insert((self.nodes[next].id.as_str(), next));
                }
            }
        }
        if order.len() != count {
            return Err("patch contains a cycle; feedback is unsupported until an explicit delay module is available".into());
        }
        let mut remap = vec![0; count];
        for (new, &old) in order.iter().enumerate() {
            remap[old] = new;
        }
        let nodes: Vec<CompiledNode> = order
            .iter()
            .map(|&old| CompiledNode {
                kind: self.nodes[old].kind.clone(),
                bypassed: self.nodes[old].bypassed,
                bypass_input: bypass::routes(&self.nodes[old].kind)
                    .first()
                    .and_then(|(input, _)| modules[old].as_ref().unwrap().port(input)),
                dsp: modules[old].take().unwrap(),
                inputs: inputs[old].map(|source| source.map(|old_source| remap[old_source])),
            })
            .collect();
        let mut port_lights = port_lights::Observer::default();
        for (node, compiled) in nodes.iter().enumerate() {
            for (port, source) in compiled.inputs.iter().enumerate() {
                if let Some(source) = source {
                    port_lights.add(node, PortDirection::Input, port, *source);
                    port_lights.add(*source, PortDirection::Output, 0, *source);
                }
            }
        }
        Ok(LegacyEngine {
            port_lights,
            sample_rate: self.sample_rate as f64,
            nodes,
            state_lookup: ids
                .into_iter()
                .map(|(id, old)| (id.to_owned(), remap[old]))
                .collect(),
            buffers: vec![[0.0; BLOCK_SIZE]; count],
            output: output.map(|i| remap[i]),
        })
    }
}

fn bounded(value: f32) -> f32 {
    value.clamp(-SIGNAL_LIMIT, SIGNAL_LIMIT)
}

impl LegacyEngine {
    /// Aggregate bounded-solver failures without allocating or resetting state.
    pub fn solver_failures(&self) -> u64 {
        self.nodes.iter().fold(0_u64, |sum, node| {
            sum.saturating_add(match &node.dsp {
                Dsp::Ladder(filter) => filter.diagnostics().solver_failures,
                _ => 0,
            })
        })
    }
    /// Continue matching physical modules when replacing a compiled graph.
    /// IDs and exact model kinds must match; new parameters and wiring remain
    /// those of `self`. The previous engine is unchanged for crossfade rendering.
    /// Lookup tables are compiled beforehand: this does not allocate, deallocate,
    /// lock, or perform I/O, and is safe to call at an audio callback boundary.
    pub fn adopt_state_from_excluding(&mut self, previous: &Self, excluded: &[bool; MAX_NODES]) {
        for (id, &index) in &self.state_lookup {
            if excluded[index] {
                continue;
            }
            let Some(&previous_index) = previous.state_lookup.get(id) else {
                continue;
            };
            let node = &mut self.nodes[index];
            let old_node = &previous.nodes[previous_index];
            if node.kind != old_node.kind {
                continue;
            }
            match (&mut node.dsp, &old_node.dsp) {
                (Dsp::Utility(current), Dsp::Utility(old)) => current.copy_state_from(old),
                (Dsp::Vco(current), Dsp::Vco(old)) => current.phase = old.phase,
                (Dsp::Gate(current), Dsp::Gate(old)) => current.phase = old.phase,
                (Dsp::Ladder(current), Dsp::Ladder(old)) => current.copy_state_from(old),
                (Dsp::Vca(current), Dsp::Vca(old)) => current.copy_state_from(old),
                (Dsp::Adsr(current), Dsp::Adsr(old)) => current.copy_state_from(old),
                (
                    Dsp::Oscillator { phase, .. },
                    Dsp::Oscillator {
                        phase: old_phase, ..
                    },
                ) => {
                    *phase = *old_phase;
                }
                (
                    Dsp::Lowpass { state, .. },
                    Dsp::Lowpass {
                        state: old_state, ..
                    },
                ) => {
                    *state = *old_state;
                }
                _ => {}
            }
        }
    }

    fn adopt_port_lights(&mut self, old: &Self, excluded: &[bool; MAX_NODES]) {
        self.port_lights.adopt_clock(&old.port_lights);
        if !self.port_lights.can_adopt(&old.port_lights) {
            return;
        }
        for (id, &node) in &self.state_lookup {
            let Some(&old_node) = old.state_lookup.get(id) else {
                continue;
            };
            if excluded[node] || self.nodes[node].kind != old.nodes[old_node].kind {
                continue;
            }
            for direction in [PortDirection::Input, PortDirection::Output] {
                for port in 0..if direction == PortDirection::Input {
                    2
                } else {
                    1
                } {
                    let (Some(index), Some(old_index)) = (
                        self.port_lights.find(node, direction, port),
                        old.port_lights.find(old_node, direction, port),
                    ) else {
                        continue;
                    };
                    // A route includes both endpoint kinds and incarnations.
                    let unchanged = match direction {
                        PortDirection::Input => {
                            let source = self.nodes[node].inputs[port].unwrap();
                            let old_source = old.nodes[old_node].inputs[port].unwrap();
                            !excluded[source]
                                && self.nodes[source].kind == old.nodes[old_source].kind
                                && self.state_lookup.iter().any(|(id, &index)| {
                                    index == source && old.state_lookup.get(id) == Some(&old_source)
                                })
                        }
                        PortDirection::Output => {
                            let count = self
                                .nodes
                                .iter()
                                .flat_map(|n| n.inputs)
                                .filter(|s| *s == Some(node))
                                .count();
                            let old_count = old
                                .nodes
                                .iter()
                                .flat_map(|n| n.inputs)
                                .filter(|s| *s == Some(old_node))
                                .count();
                            count == old_count
                                && self.state_lookup.iter().all(|(id, &destination)| {
                                    self.nodes[destination].inputs.iter().enumerate().all(
                                        |(port, source)| {
                                            if *source != Some(node) {
                                                return true;
                                            }
                                            let Some(&old_destination) = old.state_lookup.get(id)
                                            else {
                                                return false;
                                            };
                                            !excluded[destination]
                                                && self.nodes[destination].kind
                                                    == old.nodes[old_destination].kind
                                                && old.nodes[old_destination].inputs[port]
                                                    == Some(old_node)
                                        },
                                    )
                                })
                        }
                    };
                    if unchanged {
                        self.port_lights.bindings[index].state =
                            old.port_lights.bindings[old_index].state;
                    }
                }
            }
        }
    }

    /// Render the next samples. This method does not allocate, lock, or do I/O.
    /// Chunk boundaries do not change DSP state or sample-accurate modulation.
    pub fn process_block(&mut self, output: &mut [f32]) {
        let zero = [0.0; BLOCK_SIZE];
        for chunk in output.chunks_mut(BLOCK_SIZE) {
            let length = chunk.len();
            for (index, node) in self.nodes.iter_mut().enumerate() {
                let (previous, current) = self.buffers.split_at_mut(index);
                let a = node.inputs[0]
                    .map(|source| &previous[source])
                    .unwrap_or(&zero);
                let b = node.inputs[1]
                    .map(|source| &previous[source])
                    .unwrap_or(&zero);
                let buffer = &mut current[0];
                if node.bypassed {
                    let source = match node.bypass_input {
                        Some(0) => a,
                        Some(1) => b,
                        _ => &zero,
                    };
                    buffer[..length].copy_from_slice(&source[..length]);
                    continue;
                }
                match &mut node.dsp {
                    Dsp::Utility(utility) => {
                        for sample in 0..length {
                            buffer[sample] =
                                utility.process(a[sample], b[sample], node.inputs[0].is_some());
                        }
                    }
                    Dsp::Vco(vco) => {
                        for sample in 0..length {
                            buffer[sample] = vco.process(a[sample], b[sample]);
                        }
                    }
                    Dsp::Ladder(filter) => {
                        for sample in 0..length {
                            buffer[sample] = filter.process(a[sample], b[sample]);
                        }
                    }
                    Dsp::Vca(vca) => {
                        for sample in 0..length {
                            buffer[sample] = vca.process(
                                a[sample],
                                if node.inputs[1].is_some() {
                                    b[sample]
                                } else {
                                    10.0
                                },
                            );
                        }
                    }
                    Dsp::Adsr(envelope) => {
                        for sample in 0..length {
                            buffer[sample] = envelope.process(a[sample], b[sample]);
                        }
                    }
                    Dsp::Gate(gate) => {
                        for sample in &mut buffer[..length] {
                            *sample = gate.process();
                        }
                    }
                    Dsp::AudioOut => {
                        for sample in 0..length {
                            buffer[sample] = a[sample] * 0.1;
                        }
                    }
                    Dsp::Oscillator {
                        phase,
                        frequency,
                        amplitude,
                        offset,
                    } => {
                        for sample in 0..length {
                            buffer[sample] = (*phase * TAU).sin() as f32 * *amplitude + *offset;
                            let hz = (*frequency as f64 + a[sample] as f64)
                                .clamp(0.0, self.sample_rate * 0.45);
                            *phase += hz / self.sample_rate;
                            if *phase >= 1.0 {
                                *phase -= 1.0;
                            }
                        }
                    }
                    Dsp::Constant(value) => buffer[..length].fill(*value),
                    Dsp::Gain(gain) => {
                        for sample in 0..length {
                            let cv = if node.inputs[1].is_some() {
                                b[sample]
                            } else {
                                1.0
                            };
                            buffer[sample] = bounded(a[sample] * *gain * cv);
                        }
                    }
                    Dsp::Mixer => {
                        for sample in 0..length {
                            buffer[sample] = bounded(a[sample] + b[sample]);
                        }
                    }
                    Dsp::Lowpass { coefficient, state } => {
                        for sample in 0..length {
                            *state += *coefficient * (a[sample] - *state);
                            // Avoid long subnormal tails on real-time CPUs.
                            if state.abs() < 1e-20 {
                                *state = 0.0;
                            }
                            buffer[sample] = *state;
                        }
                    }
                    Dsp::Output => buffer[..length].copy_from_slice(&a[..length]),
                }
            }
            if let Some(first) = self.port_lights.first_offset() {
                for binding in &mut self.port_lights.bindings {
                    for sample in (first..length).step_by(7) {
                        binding.state = Some(port_lights::step_port_light(
                            binding.state.unwrap_or_default(),
                            true,
                            1,
                            &self.buffers[binding.source][sample..sample + 1],
                            self.sample_rate as u32,
                        ));
                    }
                }
            }
            self.port_lights.advance(length);
            if let Some(index) = self.output {
                chunk.copy_from_slice(&self.buffers[index][..length]);
            } else {
                chunk.fill(0.0);
            }
        }
    }
}

/// Compiled versioned engine. Version 1 retains its original mono DAG scheduler;
/// version 2 reads previous edge frames and publishes every edge together.
#[derive(Debug)]
pub struct Engine {
    inner: EngineKind,
    identity: u64,
}
static NEXT_ENGINE_ID: AtomicU64 = AtomicU64::new(1);

/// Opaque cable history target bound to one compiled engine. Resolve again
/// after graph replacement; stale or foreign exclusions are ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CableHandle {
    generation: u64,
    cable: usize,
}

/// Opaque, allocation-free direct-lane tap bound to one compiled engine.
/// Re-resolve after compiling a replacement; a stale handle reads inactive.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedTap {
    owner: u64,
    index: usize,
    channel: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TapSample {
    pub value: f32,
    pub active: bool,
}

#[derive(Debug)]
enum EngineKind {
    Legacy(LegacyEngine),
    Poly(Box<poly::PolyEngine>),
}
impl Engine {
    fn port_light_observer(&self) -> &port_lights::Observer {
        match &self.inner {
            EngineKind::Legacy(engine) => &engine.port_lights,
            EngineKind::Poly(engine) => &engine.port_lights,
        }
    }
    fn port_light_observer_mut(&mut self) -> &mut port_lights::Observer {
        match &mut self.inner {
            EngineKind::Legacy(engine) => &mut engine.port_lights,
            EngineKind::Poly(engine) => &mut engine.port_lights,
        }
    }
    /// Opt in to passive signal observation. Default disabled. No allocation or
    /// destruction occurs on a toggle. Enabling clears visual history until the
    /// next acquisition; the global seven-sample clock advances while disabled.
    pub fn set_port_lights_enabled(&mut self, enabled: bool) {
        self.port_light_observer_mut().set_enabled(enabled);
    }
    /// Resolve any real named endpoint on the control thread, including valid
    /// unplugged ports. Handles belong only to this compiled engine.
    pub fn resolve_port_light(
        &self,
        id: &str,
        direction: PortDirection,
        port: &str,
    ) -> Result<ResolvedPortLight, String> {
        let index = match &self.inner {
            EngineKind::Legacy(engine) => {
                let &node = engine
                    .state_lookup
                    .get(id)
                    .ok_or_else(|| format!("module '{id}' does not exist"))?;
                let local = match direction {
                    PortDirection::Input => engine.nodes[node].dsp.port(port),
                    PortDirection::Output => (port == "out").then_some(0),
                }
                .ok_or_else(|| format!("module '{id}': unknown {direction:?} port '{port}'"))?;
                engine.port_lights.find(node, direction, local)
            }
            EngineKind::Poly(engine) => engine.resolve_port_light(id, direction, port)?,
        };
        Ok(ResolvedPortLight {
            owner: self.identity,
            index,
        })
    }
    /// Immutable O(1) read; no demand, taps, masks or signal frames are changed.
    /// Disabled, foreign and not-yet-acquired endpoints return None. A valid
    /// physically unplugged endpoint reports the default disconnected state.
    pub fn port_light(&self, handle: ResolvedPortLight) -> Option<PortLightState> {
        if handle.owner != self.identity {
            return None;
        }
        self.port_light_observer().get(handle.index)
    }
    /// Bounded allocation-free immutable batch read. Length mismatch returns
    /// false without writing; stale/foreign/unavailable endpoints write None.
    pub fn copy_port_lights(
        &self,
        handles: &[ResolvedPortLight],
        out: &mut [Option<PortLightState>],
    ) -> bool {
        if handles.len() != out.len() {
            return false;
        }
        for (handle, state) in handles.iter().zip(out) {
            *state = self.port_light(*handle);
        }
        true
    }
    /// Global sample index of the latest actual acquisition, if available.
    pub fn port_light_observation_frame(&self) -> Option<u64> {
        self.port_light_observer().last()
    }
    /// Count of processed samples across all rendering APIs, including disabled
    /// observation. Compatible same-version/rate adoption retains this clock.
    pub fn port_light_processed_frames(&self) -> u64 {
        self.port_light_observer().processed()
    }
    /// Resolve an explicit version-2 cable ID for state adoption exclusions.
    pub fn resolve_cable_state(&self, id: &str) -> Result<CableHandle, String> {
        let EngineKind::Poly(engine) = &self.inner else {
            return Err("version 1 has no delayed cable state".into());
        };
        Ok(CableHandle {
            generation: self.identity,
            cable: engine.resolve_cable_state(id)?,
        })
    }
    /// Resolve any module's state on the control thread, in either patch version.
    /// This handle is bound to this receiving engine; action support still
    /// depends on the resolved model and action kind.
    pub fn resolve_module_state(&self, id: &str) -> Result<ModuleHandle, String> {
        let node = match &self.inner {
            EngineKind::Legacy(engine) => *engine
                .state_lookup
                .get(id)
                .ok_or_else(|| format!("module '{id}' does not exist"))?,
            EngineKind::Poly(engine) => engine.resolve_module_state(id)?,
        };
        Ok(ModuleHandle {
            generation: self.identity,
            node,
        })
    }
    /// Inspect actual Stock Mix controls without consuming an output.
    #[doc(hidden)]
    pub fn stock_mix_snapshot(&self, handle: ModuleHandle) -> Option<StockMixSnapshot> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.stock_mix_snapshot(handle.node),
            _ => None,
        }
    }
    /// Read actual Stock Noise histories and draw order without adding a consumer.
    #[doc(hidden)]
    pub fn stock_noise_snapshot(&self, handle: ModuleHandle) -> Option<StockNoiseSnapshot> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.stock_noise_snapshot(handle.node),
            _ => None,
        }
    }
    /// Install a fixed controlled reference stream before the module's first
    /// draw. This diagnostic hook adds no module parameter or audio callback work.
    #[doc(hidden)]
    pub fn set_stock_noise_controlled_draws(
        &mut self,
        handle: ModuleHandle,
        normal_bits: &[u32],
        uniform_bits: &[u32],
    ) -> bool {
        self.set_stock_noise_controlled_draws_at(handle, normal_bits, uniform_bits, 0, 0)
    }
    /// Set a fresh instance's reference cursors to the external global draw
    /// position, used after source recreation. Arrays are copied off callback.
    #[doc(hidden)]
    pub fn set_stock_noise_controlled_draws_at(
        &mut self,
        handle: ModuleHandle,
        normal_bits: &[u32],
        uniform_bits: &[u32],
        normal_start: u64,
        uniform_start: u64,
    ) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.set_stock_noise_controlled_draws(
                handle.node,
                normal_bits,
                uniform_bits,
                normal_start,
                uniform_start,
            ),
            _ => false,
        }
    }
    /// Load actual legacy module data. This mutates runtime controls, not the
    /// source Patch; importers should persist the returned actual flag snapshot.
    #[doc(hidden)]
    pub fn load_stock_mix_legacy_data(&mut self, handle: ModuleHandle, value: &Value) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.load_stock_mix_legacy_data(handle.node, value),
            _ => false,
        }
    }
    /// Serialize the actual source-style Boolean data flags on a control thread.
    #[doc(hidden)]
    pub fn stock_mix_legacy_data(&self, handle: ModuleHandle) -> Option<Value> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.stock_mix_legacy_data(handle.node),
            _ => None,
        }
    }
    /// Inspect Stock Octave's actual raw parameter and channel-zero display state.
    #[doc(hidden)]
    pub fn stock_octave_snapshot(&self, handle: ModuleHandle) -> Option<StockOctaveSnapshot> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.stock_octave_snapshot(handle.node),
            _ => None,
        }
    }
    /// Lossless actual Stock Scope state for source trace qualification. This
    /// explicitly requested control-thread diagnostic allocates its point copy;
    /// the callback and display transport use the borrowed common snapshot.
    #[doc(hidden)]
    pub fn stock_scope_diagnostic_bits(
        &self,
        handle: ModuleHandle,
    ) -> Option<StockScopeDiagnosticBits> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.stock_scope_diagnostic_bits(handle.node),
            _ => None,
        }
    }
    /// Write current raw source Params inside the documented diagnostic finite
    /// domain, independently of the public quantity clamp/snap policy.
    #[doc(hidden)]
    pub fn set_stock_scope_parameter_raw(
        &mut self,
        handle: ModuleHandle,
        parameter: usize,
        value: f32,
    ) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => {
                engine.set_stock_scope_parameter_raw(handle.node, parameter, value)
            }
            _ => false,
        }
    }
    #[doc(hidden)]
    pub fn stock_scope_legacy_data(&self, handle: ModuleHandle) -> Option<Value> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.stock_scope_legacy_data(handle.node),
            _ => None,
        }
    }
    #[doc(hidden)]
    pub fn load_stock_scope_legacy_data(&mut self, handle: ModuleHandle, value: &Value) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.load_stock_scope_legacy_data(handle.node, value),
            _ => false,
        }
    }
    /// Accepted only for a single Stock Scope plus rate-independent raw sources
    /// and sinks. Rejected graphs/rates change nothing. No global adoption policy
    /// is relaxed for unrelated modules.
    #[doc(hidden)]
    pub fn stock_scope_sample_rate_event(&mut self, sample_rate: u32) -> bool {
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.stock_scope_sample_rate_event(sample_rate),
            _ => false,
        }
    }
    /// Copy all sixteen actual Process engines and controls without consuming
    /// any output. Inactive voices and disconnected port storage are retained.
    #[doc(hidden)]
    pub fn stock_process_snapshot(&self, handle: ModuleHandle) -> Option<StockProcessSnapshot> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.stock_process_snapshot(handle.node),
            _ => None,
        }
    }
    /// Lossless float bit pattern for independent SDK trace comparison.
    #[doc(hidden)]
    pub fn stock_process_snapshot_bits(
        &self,
        handle: ModuleHandle,
    ) -> Option<StockProcessSnapshotBits> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.stock_process_snapshot_bits(handle.node),
            _ => None,
        }
    }
    /// Write the actual raw GATE_PARAM of the compiled Process instance. The
    /// normal UI uses gate_press/gate_release actions; this is a diagnostic seam.
    #[doc(hidden)]
    pub fn set_stock_process_gate_value(&mut self, handle: ModuleHandle, value: f32) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.set_stock_process_gate_value(handle.node, value),
            _ => false,
        }
    }
    /// The source has no module-specific JSON data; parameter JSON is separate.
    #[doc(hidden)]
    pub fn stock_process_legacy_data(&self, handle: ModuleHandle) -> Option<Value> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.stock_process_legacy_data(handle.node),
            _ => None,
        }
    }
    /// Source dataFromJson has no Process-specific callback.
    #[doc(hidden)]
    pub fn load_stock_process_legacy_data(&mut self, handle: ModuleHandle, value: &Value) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.load_stock_process_legacy_data(handle.node, value),
            _ => false,
        }
    }
    /// Apply the source's legacy root-level octave migration to actual runtime state.
    #[doc(hidden)]
    pub fn load_stock_octave_legacy_data(&mut self, handle: ModuleHandle, value: &Value) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.load_stock_octave_legacy_data(handle.node, value),
            _ => false,
        }
    }
    /// Source dataToJson has no Octave-specific data and returns null.
    #[doc(hidden)]
    pub fn stock_octave_legacy_data(&self, handle: ModuleHandle) -> Option<Value> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.stock_octave_legacy_data(handle.node),
            _ => None,
        }
    }
    /// Inspect actual native controls, four meters, divider and module lamps.
    #[doc(hidden)]
    pub fn stock_vca_mix_snapshot(&self, handle: ModuleHandle) -> Option<StockVcaMixSnapshot> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.stock_vca_mix_snapshot(handle.node),
            _ => None,
        }
    }
    /// Read the compiled version-2 Engine rate and sample time without stepping.
    #[doc(hidden)]
    pub fn host_reference_timing(&self) -> Option<(u32, f32)> {
        match &self.inner {
            EngineKind::Poly(engine) => Some(engine.host_reference_timing()),
            _ => None,
        }
    }
    /// Apply a Rack-like sample-rate event to one live Stock Process host
    /// graph. Accepted graphs contain exactly one stock_process and otherwise
    /// only poly_constant sources and mult sinks. Rejected calls change nothing.
    /// This mutates actual engine timing in place and preserves all raw ports,
    /// cable frames, source controls, Process histories and observer frame.
    #[doc(hidden)]
    pub fn stock_process_sample_rate_event(&mut self, sample_rate: u32) -> bool {
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.stock_process_sample_rate_event(sample_rate),
            _ => false,
        }
    }
    /// Apply VCMixer's source-style chExp/mixExp JSON to actual runtime flags.
    #[doc(hidden)]
    pub fn load_stock_vca_mix_legacy_data(&mut self, handle: ModuleHandle, value: &Value) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.load_stock_vca_mix_legacy_data(handle.node, value),
            _ => false,
        }
    }
    /// Serialize actual native VCMixer Boolean data, without changing state.
    #[doc(hidden)]
    pub fn stock_vca_mix_legacy_data(&self, handle: ModuleHandle) -> Option<Value> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.stock_vca_mix_legacy_data(handle.node),
            _ => None,
        }
    }
    /// Set a desired poly_constant producer frame for native reference gating.
    /// Actual canonical host-port storage is affected only by normal processing.
    #[doc(hidden)]
    pub fn set_host_reference_source(&mut self, handle: ModuleHandle, frame: PortFrame) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.set_host_reference_source(handle.node, frame),
            _ => false,
        }
    }
    /// Read a desired native reference producer frame without consuming its output.
    #[doc(hidden)]
    pub fn get_host_reference_source(&self, handle: ModuleHandle) -> Option<PortFrame> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.get_host_reference_source(handle.node),
            _ => None,
        }
    }
    /// Inspect actual compiled cable endpoints on the control thread.
    #[doc(hidden)]
    pub fn host_cable_registry(&self) -> Option<Vec<HostCableSnapshot>> {
        match &self.inner {
            EngineKind::Poly(engine) => Some(engine.host_cable_registry()),
            _ => None,
        }
    }
    /// Passive host storage inspection. This never adds an output consumer or
    /// transfers a cable. Available for every version-2 graph.
    #[doc(hidden)]
    pub fn host_module_snapshot(&self, handle: ModuleHandle) -> Option<HostModuleSnapshot> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.host_module_snapshot(handle.node),
            _ => None,
        }
    }
    /// Enable a passive, process-time copy of one canonical host input. Set on
    /// the control thread before this graph is published to the audio thread.
    /// Disabled ports incur no callback copy, and this adds no cable consumer.
    #[doc(hidden)]
    pub fn enable_host_input_observer(&mut self, handle: ModuleHandle, port: usize) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.enable_host_input_observer(handle.node, port),
            _ => false,
        }
    }
    /// Last actual input value seen during a callback, including raw inactive
    /// lanes. No-sample graph edits retain this value for compatible nodes.
    #[doc(hidden)]
    pub fn observed_host_input(
        &self,
        handle: ModuleHandle,
        port: usize,
    ) -> Option<HostPortSnapshot> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.observed_host_input(handle.node, port),
            _ => None,
        }
    }
    /// Resolve lifecycle metadata on the control thread against this engine.
    #[doc(hidden)]
    pub fn host_port_layout(&self, handle: ModuleHandle) -> Option<(u8, u8, u16, u32, bool)> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.host_port_layout(handle.node),
            _ => None,
        }
    }
    #[doc(hidden)]
    pub fn resolve_host_port(
        &self,
        handle: ModuleHandle,
        direction: PortDirection,
        name: &str,
    ) -> Option<usize> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.resolve_host_port(handle.node, direction, name),
            _ => None,
        }
    }
    /// Apply a prepared ordered host delta once, after runtime adoption. All
    /// storage and endpoint indices have already been resolved off the callback.
    #[doc(hidden)]
    pub fn apply_host_port_lifecycle(
        &mut self,
        handle: ModuleHandle,
        delta: &HostPortLifecycle,
    ) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.apply_host_port_lifecycle(handle.node, delta),
            _ => false,
        }
    }
    /// Resolve runtime action/telemetry targets on the control thread. Handles
    /// belong to this compiled engine and must be re-resolved after replacement.
    pub fn resolve_module_event(&self, id: &str) -> Result<ModuleHandle, String> {
        let node = match &self.inner {
            EngineKind::Poly(engine) => engine.resolve_module_event(id)?,
            _ => return Err("module actions require patch version 2".into()),
        };
        Ok(ModuleHandle {
            generation: self.identity,
            node,
        })
    }
    /// Resolve a fixed-copy runtime meter on the control thread. Sequencers and
    /// sequential switches expose selected step and length. This does not grant
    /// action support: switches reject handle_module_event.
    pub fn resolve_module_telemetry(&self, id: &str) -> Result<ModuleHandle, String> {
        let node = match &self.inner {
            EngineKind::Poly(engine) => engine.resolve_module_telemetry(id)?,
            _ => return Err("module telemetry requires patch version 2".into()),
        };
        Ok(ModuleHandle {
            generation: self.identity,
            node,
        })
    }
    /// Resolve and validate a particular action before queueing or rendering.
    pub fn resolve_module_action(
        &self,
        id: &str,
        action: ModuleAction,
    ) -> Result<ModuleHandle, String> {
        let node = match &self.inner {
            EngineKind::Poly(e) => e.resolve_module_action(id, action)?,
            _ => return Err("module actions require patch version 2".into()),
        };
        Ok(ModuleHandle {
            generation: self.identity,
            node,
        })
    }
    /// Resolve runtime momentary controls, including while bypassed for release.
    pub fn resolve_module_control(&self, id: &str) -> Result<ModuleHandle, String> {
        let node = match &self.inner {
            EngineKind::Poly(e) => e.resolve_module_control(id)?,
            _ => return Err("module controls require patch version 2".into()),
        };
        Ok(ModuleHandle {
            generation: self.identity,
            node,
        })
    }
    pub fn module_control_state(&self, handle: ModuleHandle) -> Option<ModuleControlState> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(e) => e.module_control_state(handle.node),
            _ => None,
        }
    }
    /// Emergency release clears held/pending manual controls, not DSP history.
    pub fn release_module_controls(&mut self, handle: ModuleHandle) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(e) => e.release_module_controls(handle.node),
            _ => false,
        }
    }
    pub fn resolve_module_scope(&self, id: &str) -> Result<ModuleHandle, String> {
        let node = match &self.inner {
            EngineKind::Poly(e) => e.resolve_module_scope(id)?,
            _ => return Err("module scope requires patch version 2".into()),
        };
        Ok(ModuleHandle {
            generation: self.identity,
            node,
        })
    }
    pub fn module_scope(&self, handle: ModuleHandle) -> Option<&ScopeSnapshot> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(e) => e.module_scope(handle.node),
            _ => None,
        }
    }
    pub fn resolve_module_lights(&self, id: &str) -> Result<ModuleHandle, String> {
        let node = match &self.inner {
            EngineKind::Poly(e) => e.resolve_module_lights(id)?,
            _ => return Err("module lights require patch version 2".into()),
        };
        Ok(ModuleHandle {
            generation: self.identity,
            node,
        })
    }
    pub fn module_lights(&self, handle: ModuleHandle) -> Option<ModuleLightState> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(e) => e.module_lights(handle.node),
            _ => None,
        }
    }
    /// Queue an ordered action for the next sample, without allocation. Returns
    /// false for stale handles, unsupported actions or a full 64-action module queue. No action is lost
    /// silently: hosts must report/retry rejected events according to policy.
    pub fn handle_module_event(&mut self, handle: ModuleHandle, action: ModuleAction) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.handle_module_event(handle.node, action),
            _ => false,
        }
    }
    pub fn module_telemetry(&self, handle: ModuleHandle) -> Option<ModuleTelemetry> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.module_telemetry(handle.node),
            _ => None,
        }
    }
    /// Restore a newly created host module instance after compatible-ID state
    /// adoption skipped an intermediate removal/re-addition. Clears its outgoing
    /// and incoming delayed frames too. Compiled parameters and engine identity stay intact.
    pub fn reset_module_state(&mut self, handle: ModuleHandle) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => {
                let preserve_history = engine.stock_noise_preserves_reset_history(handle.node);
                let reset = engine.reset_module_state(handle.node);
                if reset && !preserve_history {
                    engine.port_lights.clear_node(handle.node);
                }
                reset
            }
            _ => false,
        }
    }
    /// Compare compiled source sequence data without allocating or resolving
    /// strings. Native routing metadata is independent of source data loads.
    pub fn rackmcp_sequence_data_matches(
        &self,
        handle: ModuleHandle,
        previous: &Self,
        previous_handle: ModuleHandle,
    ) -> bool {
        if handle.generation != self.identity || previous_handle.generation != previous.identity {
            return false;
        }
        match (&self.inner, &previous.inner) {
            (EngineKind::Poly(current), EngineKind::Poly(previous)) => {
                current.rackmcp_sequence_data_matches(handle.node, previous, previous_handle.node)
            }
            _ => false,
        }
    }

    /// Apply an explicit RackMCP data-load event to the receiving compiled
    /// sequence. Preserves transport and rearms selection/automation caches,
    /// including when loaded data equals the prior data, and atomically acquires its
    /// native targets. Hosts call this after
    /// compatible graph adoption, never for unrelated graph replacements.
    pub fn reload_rackmcp_sequence_data(&mut self, handle: ModuleHandle) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.reload_rackmcp_sequence_data(handle.node),
            _ => false,
        }
    }

    /// Acquire this sequence's compiled native targets without reloading data
    /// or clearing its acknowledged-write cache. Explicit host events call this
    /// in their persistent global event order after graph adoption.
    pub fn claim_rackmcp_sequence_targets(&mut self, handle: ModuleHandle) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.claim_rackmcp_sequence_targets(handle.node),
            _ => false,
        }
    }

    /// Apply a module's dedicated Initialize runtime policy after compatible
    /// state adoption. This leaves cable histories and unrelated state intact.
    pub fn initialize_module_state(&mut self, handle: ModuleHandle) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Legacy(engine) => {
                if handle.node >= engine.nodes.len() {
                    return false;
                }
                engine.port_lights.clear_node(handle.node);
                true
            }
            EngineKind::Poly(engine) => {
                let preserve_history = engine
                    .stock_vca_mix_preserves_initialize_history(handle.node)
                    || engine.stock_noise_preserves_reset_history(handle.node)
                    || engine.stock_process_preserves_initialize_history(handle.node)
                    || engine.stock_scope_preserves_initialize_history(handle.node);
                let initialized = engine.initialize_module_state(handle.node);
                if initialized && !preserve_history {
                    engine.port_lights.clear_node(handle.node);
                }
                initialized
            }
        }
    }
    /// Restore SEQ3's running latch without synthesizing a Run action or
    /// changing its phase, step, detectors, pulses, or queued actions.
    pub fn set_module_running(&mut self, handle: ModuleHandle, running: bool) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.set_module_running(handle.node, running),
            EngineKind::Legacy(_) => false,
        }
    }
    /// Read a module's persistent runtime latch, excluding physical controls.
    pub fn module_saved_state(&self, handle: ModuleHandle) -> Option<SavedModuleState> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(engine) => engine.module_saved_state(handle.node),
            EngineKind::Legacy(_) => None,
        }
    }
    /// Assign persistent runtime state without replaying a button event. The
    /// state variant must match the target kind, including while bypassed.
    pub fn set_module_saved_state(
        &mut self,
        handle: ModuleHandle,
        state: SavedModuleState,
    ) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(engine) => engine.set_module_saved_state(handle.node, state),
            EngineKind::Legacy(_) => false,
        }
    }
    /// Arm a zero-based cell on an active MIDI-CC instance. Serial zero and
    /// uncollected completions are rejected; no command storage is allocated.
    pub fn arm_midi_cc_learn(&mut self, handle: ModuleHandle, serial: u64, cell: u8) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(e) => {
                e.midi_cc_active(handle.node)
                    && e.midi_cc_mut(handle.node)
                        .is_some_and(|m| m.arm_learn(serial, cell))
            }
            _ => false,
        }
    }
    /// Cancel only the matching armed request; completed requests stay durable.
    pub fn cancel_midi_cc_learn(&mut self, handle: ModuleHandle, serial: u64) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(e) => e
                .midi_cc_mut(handle.node)
                .is_some_and(|m| m.cancel_learn(serial)),
            _ => false,
        }
    }
    /// Take the reserved completion, including after bypass or panic.
    pub fn take_midi_cc_learn_completion(
        &mut self,
        handle: ModuleHandle,
    ) -> Option<MidiCcLearnCompletion> {
        if handle.generation != self.identity {
            return None;
        }
        match &mut self.inner {
            EngineKind::Poly(e) => e.midi_cc_mut(handle.node)?.take_learn_completion(),
            _ => None,
        }
    }
    pub fn midi_cc_learning(&self, handle: ModuleHandle) -> Option<(u64, u8)> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(e) => e.midi_cc(handle.node)?.learning(),
            _ => None,
        }
    }
    /// Shared Learn commands validate both the engine handle and module kind.
    pub fn arm_midi_learn(
        &mut self,
        handle: ModuleHandle,
        kind: MidiLearnKind,
        serial: u64,
        cell: u8,
    ) -> bool {
        match kind {
            MidiLearnKind::Cc => self.arm_midi_cc_learn(handle, serial, cell),
            MidiLearnKind::Gate => self.arm_midi_gate_learn(handle, serial, cell),
        }
    }
    pub fn cancel_midi_learn(
        &mut self,
        handle: ModuleHandle,
        kind: MidiLearnKind,
        serial: u64,
    ) -> bool {
        match kind {
            MidiLearnKind::Cc => self.cancel_midi_cc_learn(handle, serial),
            MidiLearnKind::Gate => self.cancel_midi_gate_learn(handle, serial),
        }
    }
    pub fn take_midi_learn_completion(
        &mut self,
        handle: ModuleHandle,
        kind: MidiLearnKind,
    ) -> Option<MidiLearnCompletion> {
        match kind {
            MidiLearnKind::Cc => self.take_midi_cc_learn_completion(handle),
            MidiLearnKind::Gate => self.take_midi_gate_learn_completion(handle),
        }
    }
    pub fn midi_learning(&self, handle: ModuleHandle, kind: MidiLearnKind) -> Option<(u64, u8)> {
        match kind {
            MidiLearnKind::Cc => self.midi_cc_learning(handle),
            MidiLearnKind::Gate => self.midi_gate_learning(handle),
        }
    }
    pub fn arm_midi_gate_learn(&mut self, handle: ModuleHandle, serial: u64, cell: u8) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(e) => {
                e.midi_gate_active(handle.node)
                    && e.midi_gate_mut(handle.node)
                        .is_some_and(|m| m.arm_learn(serial, cell))
            }
            _ => false,
        }
    }
    pub fn cancel_midi_gate_learn(&mut self, handle: ModuleHandle, serial: u64) -> bool {
        if handle.generation != self.identity {
            return false;
        }
        match &mut self.inner {
            EngineKind::Poly(e) => e
                .midi_gate_mut(handle.node)
                .is_some_and(|m| m.cancel_learn(serial)),
            _ => false,
        }
    }
    pub fn take_midi_gate_learn_completion(
        &mut self,
        handle: ModuleHandle,
    ) -> Option<MidiLearnCompletion> {
        if handle.generation != self.identity {
            return None;
        }
        match &mut self.inner {
            EngineKind::Poly(e) => e.midi_gate_mut(handle.node)?.take_learn_completion(),
            _ => None,
        }
    }
    pub fn midi_gate_learning(&self, handle: ModuleHandle) -> Option<(u64, u8)> {
        if handle.generation != self.identity {
            return None;
        }
        match &self.inner {
            EngineKind::Poly(e) => e.midi_gate(handle.node)?.learning(),
            _ => None,
        }
    }
    /// Invalid direct events are ignored without allocating on the audio thread.
    /// Legacy graphs have no MIDI modules and ignore all events.
    pub fn handle_midi(&mut self, event: MidiEvent) {
        if event.is_valid() {
            if let EngineKind::Poly(engine) = &mut self.inner {
                engine.handle_midi(event);
            }
        }
    }
    /// Only an explicitly selected audio_stereo.out is a stereo audio sink.
    pub fn output_channels(&self) -> u16 {
        match &self.inner {
            EngineKind::Poly(engine) => engine.output_channels(),
            _ => 1,
        }
    }
    /// Render device frames. Mono outputs are duplicated to left and right;
    /// arbitrary two-voice CV ports retain the ordinary channel-zero adapter.
    pub fn process_audio_block(&mut self, output: &mut [[f32; 2]]) {
        match &mut self.inner {
            EngineKind::Poly(engine) => {
                let stereo = engine.output_channels() == 2;
                for sample in output {
                    let frame = engine.process_sample();
                    let left = frame.channel(0);
                    *sample = [left, if stereo { frame.channel(1) } else { left }];
                }
            }
            EngineKind::Legacy(engine) => {
                let mut mono = [0.0; BLOCK_SIZE];
                for chunk in output.chunks_mut(BLOCK_SIZE) {
                    engine.process_block(&mut mono[..chunk.len()]);
                    for (frame, sample) in chunk.iter_mut().zip(&mono) {
                        *frame = [*sample; 2];
                    }
                }
            }
        }
    }
    /// Resolve on the control thread. Channels are zero-based direct reads,
    /// never mono broadcasts; a valid but absent lane reports inactive.
    pub fn resolve_tap(&self, id: &str, port: &str, channel: usize) -> Result<ResolvedTap, String> {
        if channel >= MAX_CHANNELS {
            return Err("tap channel must be 0..15".into());
        }
        let index = match &self.inner {
            EngineKind::Legacy(engine) => {
                if port != "out" {
                    return Err("version 1 taps support only out".into());
                }
                *engine
                    .state_lookup
                    .get(id)
                    .ok_or_else(|| format!("tap node '{id}' does not exist"))?
            }
            EngineKind::Poly(engine) => engine.resolve_tap(id, port)?,
        };
        Ok(ResolvedTap {
            owner: self.identity,
            index,
            channel: channel as u8,
        })
    }

    /// Render unchanged device audio while sampling module outputs before
    /// host gain/crossfade. Storage is supplied by the caller; no allocation.
    /// Tap and audio slices must have equal length. Handles from another engine
    /// (including a recompiled graph) produce inactive samples.
    pub fn process_audio_block_with_taps<const N: usize>(
        &mut self,
        output: &mut [[f32; 2]],
        taps: &[Option<ResolvedTap>; N],
        samples: &mut [[TapSample; N]],
    ) {
        assert_eq!(output.len(), samples.len());
        let identity = self.identity;
        match &mut self.inner {
            EngineKind::Poly(engine) => {
                let stereo = engine.output_channels() == 2;
                engine.begin_taps(
                    &taps.map(|tap| tap.filter(|t| t.owner == identity).map(|t| t.index)),
                );
                for (audio, sample) in output.iter_mut().zip(samples) {
                    let frame = engine.process_sample();
                    let left = frame.channel(0);
                    *audio = [left, if stereo { frame.channel(1) } else { left }];
                    for (sample, tap) in sample.iter_mut().zip(taps) {
                        *sample = tap
                            .filter(|tap| tap.owner == identity)
                            .map_or(TapSample::default(), |tap| {
                                engine.tap_sample(tap.index, tap.channel)
                            });
                    }
                }
                engine.end_taps();
            }
            EngineKind::Legacy(engine) => {
                let mut mono = [0.0; BLOCK_SIZE];
                for (audio, samples) in output
                    .chunks_mut(BLOCK_SIZE)
                    .zip(samples.chunks_mut(BLOCK_SIZE))
                {
                    engine.process_block(&mut mono[..audio.len()]);
                    for (index, (audio, samples)) in audio.iter_mut().zip(samples).enumerate() {
                        *audio = [mono[index]; 2];
                        for (sample, tap) in samples.iter_mut().zip(taps) {
                            *sample = tap
                                .filter(|tap| tap.owner == identity && tap.channel == 0)
                                .and_then(|tap| engine.buffers.get(tap.index))
                                .map_or(TapSample::default(), |buffer| TapSample {
                                    value: buffer[index],
                                    active: true,
                                });
                        }
                    }
                }
            }
        }
    }
    pub fn solver_failures(&self) -> u64 {
        match &self.inner {
            EngineKind::Legacy(engine) => engine.solver_failures(),
            EngineKind::Poly(engine) => engine.solver_failures(),
        }
    }
    /// Copy compatible state without allocation. Version and sample rate must
    /// match. Version 2 also retains only edges with unchanged compatible endpoints.
    /// This independent copy can be large for effects. The desktop uses mutable
    /// `handoff_state_from` to transfer Delay buffers without copying history.
    pub fn adopt_state_from(&mut self, previous: &Self) {
        self.adopt_state_from_excluding_routes(previous, &[], &[]);
    }
    /// Adopt compatible state, transferring large Delay allocations in constant
    /// time. Returns true only when runtime ownership moved. In that case the
    /// donor must be retired without further processing; it owns the receiver's
    /// prepared spare buffers. No allocation, deallocation, locks or I/O.
    pub fn handoff_state_from(&mut self, previous: &mut Self) -> bool {
        self.handoff_state_from_excluding_routes(previous, &[], &[])
    }
    /// Destructive counterpart of `adopt_state_from_excluding_routes` with the
    /// same incarnation/cable exclusions. Receiving parameters remain in force.
    pub fn handoff_state_from_excluding_routes(
        &mut self,
        previous: &mut Self,
        excluded: &[ModuleHandle],
        excluded_cables: &[CableHandle],
    ) -> bool {
        let mut mask = [false; MAX_NODES];
        for handle in excluded {
            if handle.generation == self.identity {
                if let Some(slot) = mask.get_mut(handle.node) {
                    *slot = true;
                }
            }
        }
        let mut cable_mask = [false; MAX_CABLES];
        for handle in excluded_cables {
            if handle.generation == self.identity {
                if let Some(slot) = cable_mask.get_mut(handle.cable) {
                    *slot = true;
                }
            }
        }
        match (&mut self.inner, &mut previous.inner) {
            (EngineKind::Legacy(current), EngineKind::Legacy(old))
                if current.sample_rate == old.sample_rate =>
            {
                current.adopt_state_from_excluding(old, &mask);
                current.adopt_port_lights(old, &mask);
                false
            }
            (EngineKind::Poly(current), EngineKind::Poly(old)) => {
                current.handoff_state_from_excluding(old, &mask, &cable_mask)
            }
            (EngineKind::Legacy(current), _) => {
                current.port_lights.reset_epoch();
                false
            }
            (EngineKind::Poly(current), _) => {
                current.port_lights.reset_epoch();
                false
            }
        }
    }
    /// Copy compatible state except modules identified as fresh incarnations.
    /// Resolve exclusions against self; stale/foreign handles are ignored.
    /// Excluded nodes retain their own compiled state and all their incident
    /// version-2 cable histories stay empty. Other nodes/edges adopt normally.
    /// Fixed stack storage; no allocation, deallocation, locks or recompilation.
    pub fn adopt_state_from_excluding(&mut self, previous: &Self, excluded: &[ModuleHandle]) {
        self.adopt_state_from_excluding_routes(previous, excluded, &[]);
    }
    /// Adopt compatible state while starting selected modules and cable
    /// histories fresh. Resolve both handle kinds against this engine.
    /// Uses fixed stack masks and performs no allocation or locking.
    pub fn adopt_state_from_excluding_routes(
        &mut self,
        previous: &Self,
        excluded: &[ModuleHandle],
        excluded_cables: &[CableHandle],
    ) {
        let mut mask = [false; MAX_NODES];
        for handle in excluded {
            if handle.generation == self.identity {
                if let Some(slot) = mask.get_mut(handle.node) {
                    *slot = true;
                }
            }
        }
        let mut cable_mask = [false; MAX_CABLES];
        for handle in excluded_cables {
            if handle.generation == self.identity {
                if let Some(slot) = cable_mask.get_mut(handle.cable) {
                    *slot = true;
                }
            }
        }
        match (&mut self.inner, &previous.inner) {
            (EngineKind::Legacy(current), EngineKind::Legacy(old))
                if current.sample_rate == old.sample_rate =>
            {
                current.adopt_state_from_excluding(old, &mask);
                current.adopt_port_lights(old, &mask);
            }
            (EngineKind::Poly(current), EngineKind::Poly(old)) => {
                current.adopt_state_from_excluding(old, &mask, &cable_mask)
            }
            (EngineKind::Legacy(current), _) => current.port_lights.reset_epoch(),
            (EngineKind::Poly(current), _) => current.port_lights.reset_epoch(),
        }
    }
    /// Mono adapter: select channel zero; never sum polyphonic voices.
    pub fn process_block(&mut self, output: &mut [f32]) {
        match &mut self.inner {
            EngineKind::Legacy(engine) => engine.process_block(output),
            EngineKind::Poly(engine) => {
                for sample in output {
                    *sample = engine.process_sample().channel(0);
                }
            }
        }
    }
    /// Render full port frames. Unused values are zero. No allocation, locking,
    /// or I/O occurs, including the first callback and state adoption.
    pub fn process_poly_block(&mut self, output: &mut [PortFrame]) {
        match &mut self.inner {
            EngineKind::Poly(engine) => {
                for frame in output {
                    *frame = engine.process_sample();
                }
            }
            EngineKind::Legacy(engine) => {
                let mut mono = [0.0; BLOCK_SIZE];
                for chunk in output.chunks_mut(BLOCK_SIZE) {
                    engine.process_block(&mut mono[..chunk.len()]);
                    for (frame, sample) in chunk.iter_mut().zip(&mono) {
                        *frame = PortFrame::mono(*sample);
                    }
                }
            }
        }
    }
}

/// A 220 Hz tone with a smooth 2 Hz tremolo, routed through a low-pass filter.
pub fn demo_patch() -> Patch {
    let node = |id: &str, kind: &str, params: &[(&str, f32)]| Node {
        content: None,
        rackmcp_sequence: None,
        score: None,
        bypassed: false,
        id: id.into(),
        kind: kind.into(),
        params: params
            .iter()
            .map(|(key, value)| (key.to_string(), *value))
            .collect(),
    };
    let cable = |from: &str, to: &str, input: &str| Cable {
        input_order: None,
        output_order: None,
        color: None,
        id: None,
        from: from.into(),
        to: to.into(),
        input: input.into(),
        output: default_output_port(),
    };
    Patch {
        nodes: vec![
            node(
                "tone",
                "oscillator",
                &[("frequency", 220.0), ("amplitude", 0.3)],
            ),
            node(
                "tremolo",
                "lfo",
                &[("frequency", 2.0), ("amplitude", 0.35), ("offset", 0.65)],
            ),
            node("vca", "gain", &[]),
            node("filter", "lowpass", &[("cutoff", 1800.0)]),
            node("out", "output", &[]),
        ],
        cables: vec![
            cable("tone", "vca", "signal"),
            cable("tremolo", "vca", "cv"),
            cable("vca", "filter", "signal"),
            cable("filter", "out", "signal"),
        ],
        output: Some("out".into()),
        ..Patch::default()
    }
}

/// A reference voice in volts, with a clock-gated RC envelope and OTA VCA.
pub fn analog_demo_patch() -> Patch {
    let node = |id: &str, kind: &str, params: &[(&str, f32)]| Node {
        content: None,
        rackmcp_sequence: None,
        score: None,
        bypassed: false,
        id: id.into(),
        kind: kind.into(),
        params: params
            .iter()
            .map(|(key, value)| (key.to_string(), *value))
            .collect(),
    };
    let cable = |from: &str, to: &str, input: &str| Cable {
        input_order: None,
        output_order: None,
        color: None,
        id: None,
        from: from.into(),
        to: to.into(),
        input: input.into(),
        output: default_output_port(),
    };
    Patch {
        nodes: vec![
            node("vco", "analog_vco", &[("frequency", 130.8128)]),
            node(
                "filter",
                "ladder_vcf",
                &[("cutoff", 1800.0), ("resonance", 0.45)],
            ),
            node("clock", "gate", &[("frequency", 1.5), ("duty", 0.45)]),
            node(
                "envelope",
                "adsr",
                &[
                    ("attack", 0.015),
                    ("decay", 0.18),
                    ("sustain", 0.4),
                    ("release", 0.25),
                ],
            ),
            node("vca", "analog_vca", &[]),
            node("out", "audio_out", &[]),
        ],
        cables: vec![
            cable("vco", "filter", "signal"),
            cable("filter", "vca", "signal"),
            cable("clock", "envelope", "gate"),
            cable("envelope", "vca", "cv"),
            cable("vca", "out", "signal"),
        ],
        output: Some("out".into()),
        ..Patch::default()
    }
}

/// Offline convenience renderer, bounded to 30 seconds per request.
pub fn render(patch: &Patch, seconds: f64) -> Result<Vec<f32>, String> {
    if !seconds.is_finite() || !(0.0..=30.0).contains(&seconds) {
        return Err("seconds must be finite and in 0..=30".into());
    }
    let mut engine = patch.compile()?;
    let count = (seconds * patch.sample_rate as f64).round() as usize;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(count)
        .map_err(|e| format!("cannot allocate render buffer: {e}"))?;
    samples.resize(count, 0.0);
    engine.process_block(&mut samples);
    Ok(samples)
}

/// Interleaved audio from an explicitly selected mono or stereo output.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderedAudio {
    pub samples: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
}
/// Maximum number of scheduled events in one offline request.
pub const MAX_MIDI_EVENTS: usize = 65_536;
/// Shared trace validation: nondecreasing absolute frames strictly inside the
/// render. Equal-frame events preserve their supplied order. No events are
/// valid in a zero-frame render.
pub fn validate_midi_events(events: &[TimedMidiEvent], total_frames: usize) -> Result<(), String> {
    if events.len() > MAX_MIDI_EVENTS {
        return Err(format!("MIDI trace exceeds {MAX_MIDI_EVENTS} events"));
    }
    let mut previous = 0;
    for (index, timed) in events.iter().enumerate() {
        timed
            .event
            .validate()
            .map_err(|e| format!("MIDI event {index}: {e}"))?;
        if timed.frame >= total_frames as u64 {
            return Err(format!("MIDI event {index}: frame is outside render"));
        }
        if index > 0 && timed.frame < previous {
            return Err(format!("MIDI event {index}: frames must be sorted"));
        }
        previous = timed.frame;
    }
    Ok(())
}
pub fn render_audio(patch: &Patch, seconds: f64) -> Result<RenderedAudio, String> {
    render_audio_with_events(patch, seconds, &[])
}
/// Apply events before their indicated sample, independent of block partitions.
pub fn render_audio_with_events(
    patch: &Patch,
    seconds: f64,
    events: &[TimedMidiEvent],
) -> Result<RenderedAudio, String> {
    render_audio_with_performance_events(patch, seconds, events, &[])
}
/// Offline render from reset with two ordered traces. MIDI is dispatched before
/// module actions at an equal frame; all queued module actions are applied in
/// supplied order before that sample's CV processing; for momentary controls
/// the final same-frame physical button state wins. Validate
/// both traces and resolve every module ID before rendering any samples.
pub fn render_audio_with_performance_events(
    patch: &Patch,
    seconds: f64,
    midi: &[TimedMidiEvent],
    modules: &[TimedModuleEvent],
) -> Result<RenderedAudio, String> {
    if !seconds.is_finite() || !(0.0..=30.0).contains(&seconds) {
        return Err("seconds must be finite and in 0..=30".into());
    }
    let mut engine = patch.compile()?;
    let count = (seconds * patch.sample_rate as f64).round() as usize;
    validate_midi_events(midi, count)?;
    validate_module_events(modules, count)?;
    let resolved: Vec<_> = modules
        .iter()
        .enumerate()
        .map(|(index, event)| {
            engine
                .resolve_module_action(&event.id, event.action)
                .map(|handle| (event.frame, handle, event.action))
                .map_err(|error| format!("module event {index}: {error}"))
        })
        .collect::<Result<_, _>>()?;
    let channels = engine.output_channels();
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(count * channels as usize)
        .map_err(|e| format!("cannot allocate render buffer: {e}"))?;
    let mut buffer = [[0.0; 2]; BLOCK_SIZE];
    let mut frame = 0;
    let mut next_midi = 0;
    let mut next_module = 0;
    while frame < count {
        while next_midi < midi.len() && midi[next_midi].frame == frame as u64 {
            engine.handle_midi(midi[next_midi].event);
            next_midi += 1;
        }
        while next_module < resolved.len() && resolved[next_module].0 == frame as u64 {
            if !engine.handle_module_event(resolved[next_module].1, resolved[next_module].2) {
                return Err("validated module action queue rejected an event".into());
            }
            next_module += 1;
        }
        let end = midi
            .get(next_midi)
            .map_or(count, |event| event.frame as usize)
            .min(
                resolved
                    .get(next_module)
                    .map_or(count, |event| event.0 as usize),
            );
        let length = (end - frame).min(BLOCK_SIZE);
        engine.process_audio_block(&mut buffer[..length]);
        for pair in &buffer[..length] {
            samples.extend_from_slice(&pair[..channels as usize]);
        }
        frame += length;
    }
    Ok(RenderedAudio {
        samples,
        channels,
        sample_rate: patch.sample_rate,
    })
}

/// Write a new mono PCM16 WAV. Existing files are never overwritten.
pub fn write_wav(path: &Path, samples: &[f32], rate: u32) -> io::Result<()> {
    write_wav_interleaved(path, samples, rate, 1)
}
/// Write interleaved mono/stereo PCM16. NaN maps to silence; other values clip
/// to ±1. Header arithmetic and complete frame alignment are checked before
/// creating the file. Existing files are never overwritten.
pub fn write_wav_interleaved(
    path: &Path,
    samples: &[f32],
    rate: u32,
    channels: u16,
) -> io::Result<()> {
    let invalid = |message| io::Error::new(io::ErrorKind::InvalidInput, message);
    if !(1..=2).contains(&channels) || !samples.len().is_multiple_of(channels as usize) {
        return Err(invalid("WAV requires complete mono or stereo frames"));
    }
    let block_align = channels * 2;
    let byte_rate = rate
        .checked_mul(u32::from(block_align))
        .filter(|_| rate > 0)
        .ok_or_else(|| invalid("invalid WAV sample rate"))?;
    let data_size = samples
        .len()
        .checked_mul(2)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n <= u32::MAX - 36)
        .ok_or_else(|| invalid("samples exceed RIFF WAV size limit"))?;
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(b"RIFF")?;
    writer.write_all(&(data_size + 36).to_le_bytes())?;
    writer.write_all(b"WAVEfmt ")?;
    writer.write_all(&16u32.to_le_bytes())?;
    writer.write_all(&1u16.to_le_bytes())?;
    writer.write_all(&channels.to_le_bytes())?;
    writer.write_all(&rate.to_le_bytes())?;
    writer.write_all(&byte_rate.to_le_bytes())?;
    writer.write_all(&block_align.to_le_bytes())?;
    writer.write_all(&16u16.to_le_bytes())?;
    writer.write_all(b"data")?;
    writer.write_all(&data_size.to_le_bytes())?;
    for &sample in samples {
        let value = if sample.is_nan() {
            0.0
        } else {
            sample.clamp(-1.0, 1.0)
        };
        let pcm = if value < 0.0 {
            (value * 32768.0).round() as i16
        } else {
            (value * 32767.0).round() as i16
        };
        writer.write_all(&pcm.to_le_bytes())?;
    }
    writer.flush()
}

#[cfg(test)]
mod scope_tap_tests {
    use super::*;
    #[test]
    fn named_raw_taps_leave_audible_output_unchanged_and_absent_lanes_inactive() {
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"audio","kind":"constant","params":{"value":0.25}},
                {"id":"poly","kind":"poly_constant","params":{"channels":2,"v1":3,"v2":-4}},
                {"id":"split","kind":"poly_split","params":{}}],
            "cables":[{"from":"poly","to":"split","input":"signal"}],"output":"audio"
        }))
        .unwrap();
        let mut engine = patch.compile().unwrap();
        let taps = [
            Some(engine.resolve_tap("split", "out2", 0).unwrap()),
            Some(engine.resolve_tap("poly", "out", 2).unwrap()),
        ];
        let mut audio = [[0.0; 2]; 259];
        let mut samples = [[TapSample::default(); 2]; 259];
        engine.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
        let mut reference = [[0.0; 2]; 259];
        patch.compile().unwrap().process_audio_block(&mut reference);
        assert_eq!(audio, reference);
        assert_eq!(audio, [[0.25; 2]; 259]);
        assert_eq!(
            samples[0][0],
            TapSample {
                value: 0.0,
                active: true
            }
        );
        assert!(samples[1..].iter().all(|s| s[0]
            == TapSample {
                value: -4.0,
                active: true
            }));
        assert!(samples.iter().all(|s| s[1] == TapSample::default()));
        assert!(engine.resolve_tap("missing", "out", 0).is_err());
        assert!(engine.resolve_tap("split", "out17", 0).is_err());
        assert!(engine.resolve_tap("poly", "out", 16).is_err());
        let mut replacement = patch.compile().unwrap();
        replacement.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
        assert!(
            samples.iter().flatten().all(|s| *s == TapSample::default()),
            "old engine handles must never alias a replacement graph"
        );
    }
    #[test]
    fn legacy_taps_sample_each_node_before_output_selection_and_support_arbitrary_lengths() {
        let patch: Patch = serde_json::from_value(json!({
            "nodes":[{"id":"dc","kind":"constant","params":{"value":2}},
                {"id":"gain","kind":"gain","params":{"gain":0.5}}],
            "cables":[{"from":"dc","to":"gain","input":"signal"}],"output":"gain"
        }))
        .unwrap();
        let mut engine = patch.compile().unwrap();
        let taps = [
            Some(engine.resolve_tap("dc", "out", 0).unwrap()),
            Some(engine.resolve_tap("gain", "out", 1).unwrap()),
        ];
        let mut output = [[0.0; 2]; 259];
        let mut samples = [[TapSample::default(); 2]; 259];
        engine.process_audio_block_with_taps(&mut output, &taps, &mut samples);
        assert_eq!(output, [[1.0; 2]; 259]);
        assert!(samples.iter().all(|s| s[0]
            == TapSample {
                value: 2.0,
                active: true
            }
            && s[1] == TapSample::default()));
        assert!(engine.resolve_tap("dc", "out2", 0).is_err());
    }
    #[test]
    fn invalid_state_exclusion_index_is_ignored_without_mask_aliasing() {
        let patch:Patch=serde_json::from_value(json!({"nodes":[{"id":"osc","kind":"oscillator","params":{"frequency":7}}],"output":"osc"})).unwrap();
        let mut old = patch.compile().unwrap();
        old.process_block(&mut [0.0; 123]);
        let mut expected = patch.compile().unwrap();
        expected.adopt_state_from(&old);
        let mut actual = patch.compile().unwrap();
        let invalid = ModuleHandle {
            generation: actual.identity,
            node: MAX_NODES + 7,
        };
        actual.adopt_state_from_excluding(&old, &[invalid]);
        let mut a = [0.0; 31];
        let mut b = [0.0; 31];
        actual.process_block(&mut a);
        expected.process_block(&mut b);
        assert_eq!(a, b);
    }
}
