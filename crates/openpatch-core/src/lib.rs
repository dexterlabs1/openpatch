//! Original modular DSP for OpenPatch. Version 1 is the unchanged mono DAG;
//! version 2 has named 0–16-channel ports and one sample of delay per cable,
//! including feedback loops. New sessions should use `Patch::new_v2()`.
//!
//! Compile on a control thread, then move the engine to the audio thread.
//! `process_block` performs no allocation, locking, or I/O. Unpatched inputs
//! are zero, except gain.cv defaults to 1, analog_vca.cv to 10 V, and
//! attenuverter.signal to 10 V. Signals are unclipped in normal use, with a
//! ±1,000,000 safety bound on arithmetic modules; WAV export clips to ±1.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;
use std::fs::OpenOptions;
use std::io::{self, BufWriter, Write};
use std::path::Path;

mod analog;
pub mod ladder;
pub mod midi;
mod poly;
mod routing;
pub use midi::{parse_midi, MidiEvent, TimedMidiEvent};
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
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cable {
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
                "waveform":{"default":0.0,"min":0.0,"max":1.0,"step":1.0},
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
    for module in result["modules"].as_array_mut().unwrap() {
        if module.get("model").is_none() {
            module["model"] = json!("legacy");
        }
        poly::annotate_catalog(module);
    }
    result["signal_conventions"] = json!({"reference_modules":"volts; audio_out converts 10 V to 1.0 digital","legacy_modules":"unchanged normalized utilities; no automatic voltage conversion","functional_modules":"volts; seeded noise level is RMS volts; no automatic antialiasing for discontinuous quantizer/sample_hold outputs","pitch":"1 V/oct for analog_vco.pitch and ladder_vcf.cutoff_cv"});
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
        let inner = match self.version {
            1 => EngineKind::Legacy(self.compile_legacy()?),
            2 => EngineKind::Poly(poly::PolyEngine::compile(self)?),
            version => {
                return Err(format!(
                    "unsupported patch version {version} (expected 1 or 2)"
                ))
            }
        };
        Ok(Engine { inner })
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
        let nodes = order
            .iter()
            .map(|&old| CompiledNode {
                kind: self.nodes[old].kind.clone(),
                dsp: modules[old].take().unwrap(),
                inputs: inputs[old].map(|source| source.map(|old_source| remap[old_source])),
            })
            .collect();
        Ok(LegacyEngine {
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
    pub fn adopt_state_from(&mut self, previous: &Self) {
        for (id, &index) in &self.state_lookup {
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
}
#[derive(Debug)]
enum EngineKind {
    Legacy(LegacyEngine),
    Poly(poly::PolyEngine),
}
impl Engine {
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
    pub fn solver_failures(&self) -> u64 {
        match &self.inner {
            EngineKind::Legacy(engine) => engine.solver_failures(),
            EngineKind::Poly(engine) => engine.solver_failures(),
        }
    }
    /// Copy compatible state without allocation. Version and sample rate must
    /// match. Version 2 also retains only edges with unchanged compatible endpoints.
    pub fn adopt_state_from(&mut self, previous: &Self) {
        match (&mut self.inner, &previous.inner) {
            (EngineKind::Legacy(current), EngineKind::Legacy(old))
                if current.sample_rate == old.sample_rate =>
            {
                current.adopt_state_from(old)
            }
            (EngineKind::Poly(current), EngineKind::Poly(old)) => current.adopt_state_from(old),
            _ => {}
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
        id: id.into(),
        kind: kind.into(),
        params: params
            .iter()
            .map(|(key, value)| (key.to_string(), *value))
            .collect(),
    };
    let cable = |from: &str, to: &str, input: &str| Cable {
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
        id: id.into(),
        kind: kind.into(),
        params: params
            .iter()
            .map(|(key, value)| (key.to_string(), *value))
            .collect(),
    };
    let cable = |from: &str, to: &str, input: &str| Cable {
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
    if !seconds.is_finite() || !(0.0..=30.0).contains(&seconds) {
        return Err("seconds must be finite and in 0..=30".into());
    }
    let mut engine = patch.compile()?;
    let count = (seconds * patch.sample_rate as f64).round() as usize;
    validate_midi_events(events, count)?;
    let channels = engine.output_channels();
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(count * channels as usize)
        .map_err(|e| format!("cannot allocate render buffer: {e}"))?;
    let mut buffer = [[0.0; 2]; BLOCK_SIZE];
    let mut frame = 0;
    let mut next = 0;
    while frame < count {
        while next < events.len() && events[next].frame == frame as u64 {
            engine.handle_midi(events[next].event);
            next += 1;
        }
        let end = if next < events.len() {
            count.min(events[next].frame as usize)
        } else {
            count
        };
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
