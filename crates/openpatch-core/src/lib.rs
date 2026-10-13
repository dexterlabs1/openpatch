//! Original mono modular DSP for OpenPatch. Graphs are directed and acyclic;
//! feedback needs a future explicit delay module and is rejected today.
//!
//! Compile on a control thread, then move the engine to the audio thread.
//! `process_block` performs no allocation, locking, or I/O. Unpatched inputs
//! are zero, except gain.cv defaults to 1. Signals are unclipped in normal use, with a ±1,000,000
//! safety bound on arithmetic modules; WAV export clips to ±1.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;
use std::fs::OpenOptions;
use std::io::{self, BufWriter, Write};
use std::path::Path;

pub const BLOCK_SIZE: usize = 128;
pub const MAX_NODES: usize = 256;
pub const MAX_CABLES: usize = 1024;
const SIGNAL_LIMIT: f32 = 1_000_000.0;

fn default_version() -> u32 {
    1
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
}

impl Default for Patch {
    fn default() -> Self {
        Self {
            version: 1,
            sample_rate: 48_000,
            nodes: Vec::new(),
            cables: Vec::new(),
            output: None,
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
}

/// Machine-readable module descriptions. Bounds depending on the sample rate
/// are checked by `validate`; input port names are also the cable `input` names.
pub fn catalog() -> Value {
    json!({
        "version": 1,
        "sample_rate": {"min": 8000, "max": 192000, "default": 48000},
        "limits": {"nodes": MAX_NODES, "cables": MAX_CABLES, "block_size": BLOCK_SIZE},
        "unpatched_inputs": "zero except gain.cv defaults to 1", "cycles": "rejected", "channels": 1,
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
            {"kind":"output", "inputs":["signal"], "params":{}}
        ]
    })
}

#[derive(Debug)]
enum Dsp {
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
    fn port(&self, name: &str) -> Option<usize> {
        match (self, name) {
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
    dsp: Dsp,
    inputs: [Option<usize>; 2],
}

/// Stateful compiled graph. Construct and replace engines outside the audio
/// callback; process any slice length without allocating intermediate buffers.
#[derive(Debug)]
pub struct Engine {
    sample_rate: f64,
    nodes: Vec<CompiledNode>,
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
        "oscillator" => &["frequency", "amplitude"],
        "lfo" => &["frequency", "amplitude", "offset"],
        "constant" => &["value"],
        "gain" => &["gain"],
        "mixer" | "output" => &[],
        "lowpass" => &["cutoff"],
        _ => {
            return Err(format!(
                "node '{}': unknown module kind '{}'",
                node.id, node.kind
            ))
        }
    };
    for name in node.params.keys() {
        if !allowed.contains(&name.as_str()) {
            return Err(format!("node '{}': unknown parameter '{}'", node.id, name));
        }
    }
    Ok(match node.kind.as_str() {
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
        _ => unreachable!(),
    })
}

impl Patch {
    pub fn validate(&self) -> Result<(), String> {
        self.compile().map(|_| ())
    }

    pub fn compile(&self) -> Result<Engine, String> {
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
                dsp: modules[old].take().unwrap(),
                inputs: inputs[old].map(|source| source.map(|old_source| remap[old_source])),
            })
            .collect();
        Ok(Engine {
            sample_rate: self.sample_rate as f64,
            nodes,
            buffers: vec![[0.0; BLOCK_SIZE]; count],
            output: output.map(|i| remap[i]),
        })
    }
}

fn bounded(value: f32) -> f32 {
    value.clamp(-SIGNAL_LIMIT, SIGNAL_LIMIT)
}

impl Engine {
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

/// Write a new mono PCM16 WAV. Existing files are never overwritten. NaN maps
/// to silence; infinities and samples outside ±1 are clipped during export.
pub fn write_wav(path: &Path, samples: &[f32], rate: u32) -> io::Result<()> {
    let invalid = |message| io::Error::new(io::ErrorKind::InvalidInput, message);
    let byte_rate = rate
        .checked_mul(2)
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
    writer.write_all(&1u16.to_le_bytes())?; // PCM
    writer.write_all(&1u16.to_le_bytes())?; // mono
    writer.write_all(&rate.to_le_bytes())?;
    writer.write_all(&byte_rate.to_le_bytes())?;
    writer.write_all(&2u16.to_le_bytes())?;
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
