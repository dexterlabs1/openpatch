//! Version-2 reference scheduler: all modules read last sample's cable frames,
//! then all edges publish current output frames. Strings, routing and voice
//! storage are resolved/allocated only during compilation.
use crate::midi::{MidiCv, OUTPUTS as MIDI_OUTPUTS};
use crate::routing::{self, Routing};
use crate::{bounded, build_dsp, parameter, Dsp, MidiEvent, Node, Patch, MAX_CABLES, MAX_NODES};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const MAX_CHANNELS: usize = 16;
const MAX_PORTS: usize = 16;

/// A fixed-capacity port value. Engine-produced frames have 0..=16 channels and
/// zero in inactive slots. Zero channels means disconnected for normaling.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PortFrame {
    pub channels: u8,
    pub values: [f32; MAX_CHANNELS],
}
impl PortFrame {
    pub fn mono(value: f32) -> Self {
        let mut frame = Self {
            channels: 1,
            ..Self::default()
        };
        frame.values[0] = value;
        frame
    }
    /// Read an existing channel without broadcast (the mono output adapter).
    pub fn channel(&self, channel: usize) -> f32 {
        if channel < usize::from(self.channels).min(MAX_CHANNELS) {
            self.values[channel]
        } else {
            0.0
        }
    }
    pub(crate) fn voice(&self, voice: usize, normal: f32) -> f32 {
        match self.channels {
            0 => normal,
            1 => self.values[0],
            _ => self.channel(voice),
        }
    }
}

pub(crate) fn is_poly_kind(kind: &str) -> bool {
    routing::input_names(kind).is_some()
        || matches!(
            kind,
            "poly_constant" | "poly_split" | "poly_merge" | "poly_sum" | "midi_cv" | "audio_stereo"
        )
}
fn numbered_port(name: &str, prefix: &str) -> Option<usize> {
    let number = name.strip_prefix(prefix)?.parse::<usize>().ok()?;
    if (1..=16).contains(&number) && name == format!("{prefix}{number}") {
        Some(number - 1)
    } else {
        None
    }
}
fn input_port(kind: &str, template: Option<&Dsp>, name: &str) -> Option<usize> {
    if let Some(ports) = routing::input_names(kind) {
        return ports.iter().position(|port| *port == name);
    }
    match kind {
        "poly_constant" | "midi_cv" => None,
        "audio_stereo" => match name {
            "left" => Some(0),
            "right" => Some(1),
            _ => None,
        },
        "poly_split" | "poly_sum" if name == "signal" => Some(0),
        "poly_merge" => numbered_port(name, "in"),
        _ => template.and_then(|dsp| dsp.port(name)),
    }
}
fn output_port(kind: &str, name: &str) -> Option<usize> {
    if let Some(ports) = routing::output_names(kind) {
        return ports.iter().position(|port| *port == name);
    }
    if kind == "midi_cv" {
        MIDI_OUTPUTS.iter().position(|port| *port == name)
    } else if kind == "poly_split" {
        numbered_port(name, "out")
    } else if name == "out" {
        Some(0)
    } else {
        None
    }
}
fn integer(node: &Node, name: &str, default: f32) -> Result<u8, String> {
    let value = parameter(node, name, default, 0.0, 16.0)?;
    if value.fract() != 0.0 {
        return Err(format!(
            "node '{}': parameter '{name}' must be an integer",
            node.id
        ));
    }
    Ok(value as u8)
}

pub(crate) fn catalog() -> [Value; 6] {
    let values: serde_json::Map<String, Value> = (1..=16)
        .map(|i| {
            (
                format!("v{i}"),
                json!({"default":i as f32,"min":-16.0,"max":16.0,"unit":"V"}),
            )
        })
        .collect();
    let mut constants = json!({"kind":"poly_constant","title":"POLY CONST","model":"functional","description":"0–16 independent constant voltage channels; inactive values are zero.","inputs":[],"params":{"channels":{"default":3.0,"min":0.0,"max":16.0,"step":1.0}}});
    constants["params"].as_object_mut().unwrap().extend(values);
    [
        json!({"kind":"midi_cv","title":"MIDI-CV","model":"functional","description":"MIDI notes to fixed polyphonic voltages; pitch 1 V/oct relative to C4; gates 10 V. Channel pressure aftertouch (poly pressure ignored), separate ±5 V pitchwheel, CC1 modwheel. Sustain CC64, all-sound-off CC120, reset controllers CC121, all-notes-off CC123. Triggers last 1 ms. Duplicate notes are counted; one note-off releases one pressed voice. Mono uses last-held-note priority. No MIDI events means silence.","inputs":[],"params":{"channels":{"default":4.0,"min":1.0,"max":16.0,"step":1.0},"channel":{"default":0.0,"min":0.0,"max":16.0,"step":1.0,"description":"0 omni; 1–16 selects wire channel 0–15"},"allocation":{"default":0.0,"min":0.0,"max":2.0,"step":1.0,"description":"0 rotate free then next steal; 1 reuse previous note/channel else lowest free then lane 1 steal; 2 lowest free then lane 1 steal, compact releases"},"clock_div":{"default":24.0,"min":1.0,"max":96.0,"step":1.0}}}),
        json!({"kind":"audio_stereo","title":"STEREO AUDIO","model":"functional","description":"Unity-sum polyphony independently on left/right, divide voltages by 10, multiply gain. Right normalizes to left only when disconnected or zero-channel. Selecting out explicitly renders stereo.","inputs":["left","right"],"params":{"gain":{"default":1.0,"min":0.0,"max":2.0}}}),
        constants,
        json!({"kind":"poly_split","title":"SPLIT","model":"functional","description":"Split channels 1–16 into named mono outputs out1..out16. Missing channels produce mono zero.","inputs":["signal"],"params":{}}),
        json!({"kind":"poly_merge","title":"MERGE","model":"functional","description":"Take channel zero of in1..in16 into one polyphonic output. channels=0 auto uses the highest nonzero-channel input; holes are preserved as zero. Override 1–16 forces channel count.","inputs":(1..=16).map(|i|format!("in{i}")).collect::<Vec<_>>(),"params":{"channels":{"default":0.0,"min":0.0,"max":16.0,"step":1.0}}}),
        json!({"kind":"poly_sum","title":"SUM","model":"functional","description":"Unity sum all active signal channels into one mono output. Empty signal produces mono zero; arithmetic bound ±1,000,000 V.","inputs":["signal"],"params":{}}),
    ]
}

#[derive(Clone, Copy, Debug)]
enum VoicePolicy {
    Generator,
    Primary(usize),
    Maximum,
}
fn voice_policy(kind: &str) -> VoicePolicy {
    match kind {
        "constant" | "gate" | "white_noise" => VoicePolicy::Generator,
        "sample_hold" => VoicePolicy::Primary(1),
        "mixer" | "crossfade" => VoicePolicy::Maximum,
        _ => VoicePolicy::Primary(0),
    }
}
pub(crate) fn annotate_catalog(module: &mut Value) {
    let kind = module["kind"].as_str().unwrap().to_owned();
    let outputs = if let Some(names) = routing::output_names(&kind) {
        names.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()
    } else if kind == "midi_cv" {
        MIDI_OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "poly_split" {
        (1..=16).map(|i| format!("out{i}")).collect::<Vec<_>>()
    } else {
        vec!["out".into()]
    };
    module["outputs"] = json!(outputs);
    module["min_patch_version"] = json!(if is_poly_kind(&kind) { 2 } else { 1 });
    let policy = match kind.as_str() {
        "midi_cv" => "fixed configured 1–16 channels, including release tails and idle frames",
        "audio_stereo" => "explicit stereo: two normalized digital channels; each input independently unity sums polyphony",
        "poly_constant" => "explicit channels parameter, 0–16",
        "poly_split" => "each output is mono; missing source channel is zero",
        "poly_merge" => {
            "auto highest occupied input or explicit count; channel zero of each input; holes zero"
        }
        "poly_sum" => "mono unity sum of active channels",
        _ => match voice_policy(&kind) {
            VoicePolicy::Generator => "one generated voice",
            VoicePolicy::Primary(1) => "trigger input sets voices, minimum one",
            VoicePolicy::Primary(_) => "first declared input sets voices, minimum one",
            VoicePolicy::Maximum => "maximum input channel count, minimum one",
        },
    };
    module["polyphony"] = json!({"version_1":"mono", "version_2":policy, "maximum_channels":16,"auxiliary_inputs":"mono broadcasts; missing short-poly channels are zero; zero-channel ports normalize as disconnected","retired_voices":"reset on retirement and activation"});
    let primary = match kind.as_str() {
        "poly_constant" | "poly_merge" | "midi_cv" | "audio_stereo" => Value::Null,
        _ => match voice_policy(&kind) {
            VoicePolicy::Generator | VoicePolicy::Maximum => Value::Null,
            VoicePolicy::Primary(index) => {
                module["inputs"].get(index).cloned().unwrap_or(Value::Null)
            }
        },
    };
    module["polyphony"]["primary_input"] = primary;
    module["input_metadata"] = json!(module["inputs"].as_array().unwrap().iter().map(|name| {
        let name = name.as_str().unwrap();
        let normal = match (kind.as_str(), name) { ("gain", "cv") => 1.0, ("analog_vca", "cv") | ("attenuverter", "signal") => 10.0, _ => 0.0 };
        json!({"name":name,"normalization":normal,"zero_channels":"disconnected","maximum_channels":16})
    }).collect::<Vec<_>>());
    if kind == "audio_stereo" {
        module["input_metadata"][1]["normalization_source"] = json!("left");
        module["audio_channels"] = json!(2);
        module["polyphony"]["maximum_channels"] = json!(2);
    }
    if kind == "midi_cv" {
        module["polyphony"]["retired_voices"] =
            json!("configured voices never retire; release retains pitch and velocity");
        module["midi_channel_numbering"] =
            json!("events 0–15; channel parameter 0 omni or 1–16 selection");
        module["configuration_state"] = json!("voices, channel, allocation edits reset MIDI state; clock division edits preserve held notes");
        module["clock_div_phase"] =
            json!("first clock after reset/start, then every clock_div clocks");
    }
    module["output_metadata"] = json!(outputs.iter().map(|name|json!({"name":name,"maximum_channels":if kind=="audio_stereo" {2} else if kind=="poly_split" || kind=="poly_sum" || matches!(voice_policy(&kind),VoicePolicy::Generator) {1} else {16},"units":if kind=="audio_out" || kind=="audio_stereo" {"normalized digital amplitude"} else if module["model"]=="legacy" {"legacy normalized"} else {"V"}})).collect::<Vec<_>>());
    routing::annotate(module);
}

#[derive(Debug)]
enum Model {
    Voices {
        template: Dsp,
        voices: Vec<Dsp>,
        active: u8,
        policy: VoicePolicy,
    },
    Constant(PortFrame),
    Split,
    Merge(u8),
    Sum,
    Midi(Box<MidiCv>),
    Stereo(f32),
    Routing(Routing),
}
impl Model {
    fn build(node: &Node, rate: u32) -> Result<Self, String> {
        if let Some(model) = Routing::build(node) {
            return model.map(Self::Routing);
        }
        if node.kind == "midi_cv" {
            return Ok(Self::Midi(Box::new(MidiCv::build(node, rate)?)));
        }
        if node.kind == "audio_stereo" {
            for key in node.params.keys() {
                if key != "gain" {
                    return Err(format!("node '{}': unknown parameter '{key}'", node.id));
                }
            }
            return Ok(Self::Stereo(parameter(node, "gain", 1.0, 0.0, 2.0)?));
        }
        let allowed = match node.kind.as_str() {
            "poly_constant" => Some(
                (0..=16)
                    .map(|i| {
                        if i == 0 {
                            "channels".into()
                        } else {
                            format!("v{i}")
                        }
                    })
                    .collect::<Vec<_>>(),
            ),
            "poly_merge" => Some(vec!["channels".into()]),
            "poly_split" | "poly_sum" => Some(Vec::new()),
            _ => None,
        };
        if let Some(allowed) = allowed {
            for key in node.params.keys() {
                if !allowed.contains(key) {
                    return Err(format!("node '{}': unknown parameter '{key}'", node.id));
                }
            }
            return Ok(match node.kind.as_str() {
                "poly_constant" => {
                    let mut frame = PortFrame {
                        channels: integer(node, "channels", 3.0)?,
                        ..PortFrame::default()
                    };
                    // Validate even inactive parameters, but never publish stale values.
                    for i in 0..16 {
                        let value =
                            parameter(node, &format!("v{}", i + 1), (i + 1) as f32, -16.0, 16.0)?;
                        if i < frame.channels as usize {
                            frame.values[i] = value;
                        }
                    }
                    Self::Constant(frame)
                }
                "poly_split" => Self::Split,
                "poly_merge" => Self::Merge(integer(node, "channels", 0.0)?),
                "poly_sum" => Self::Sum,
                _ => unreachable!(),
            });
        }
        let template = build_dsp(node, rate)?;
        let voices = (0..MAX_CHANNELS).map(|_| template.clone()).collect();
        Ok(Self::Voices {
            template,
            voices,
            active: 0,
            policy: voice_policy(&node.kind),
        })
    }
    fn template(&self) -> Option<&Dsp> {
        if let Self::Voices { template, .. } = self {
            Some(template)
        } else {
            None
        }
    }
}
#[derive(Debug)]
struct PolyNode {
    kind: String,
    model: Model,
    inputs: [Option<usize>; MAX_PORTS],
    output_start: usize,
}
#[derive(Debug)]
struct Edge {
    source_port: usize,
    source_node: usize,
    destination_node: usize,
}
type EdgeKey = (String, String, String, String);

#[derive(Debug)]
pub(crate) struct PolyEngine {
    rate: u32,
    nodes: Vec<PolyNode>,
    node_lookup: BTreeMap<String, usize>,
    edges: Vec<Edge>,
    edge_lookup: BTreeMap<EdgeKey, usize>,
    edge_frames: Vec<PortFrame>,
    outputs: Vec<PortFrame>,
    selected: Option<usize>,
    stereo: bool,
}
impl PolyEngine {
    pub(crate) fn compile(patch: &Patch) -> Result<Self, String> {
        if !(8000..=192000).contains(&patch.sample_rate) {
            return Err("sample_rate must be in 8000..=192000 Hz".into());
        }
        if patch.nodes.len() > MAX_NODES {
            return Err(format!("patch exceeds {MAX_NODES} nodes"));
        }
        if patch.cables.len() > MAX_CABLES {
            return Err(format!("patch exceeds {MAX_CABLES} cables"));
        }
        let mut node_lookup = BTreeMap::new();
        let mut nodes = Vec::with_capacity(patch.nodes.len());
        let mut output_count = 0;
        // Node order cannot affect synchronous edges. Keep the submitted order
        // to avoid unnecessary graph remapping; lookups are control-only strings.
        for (index, node) in patch.nodes.iter().enumerate() {
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
            if node_lookup.insert(node.id.clone(), index).is_some() {
                return Err(format!("duplicate node ID '{}'", node.id));
            }
            nodes.push(PolyNode {
                kind: node.kind.clone(),
                model: Model::build(node, patch.sample_rate)?,
                inputs: [None; MAX_PORTS],
                output_start: output_count,
            });
            output_count += if let Some(ports) = routing::output_names(&node.kind) {
                ports.len()
            } else {
                match node.kind.as_str() {
                    "poly_split" => 16,
                    "midi_cv" => MIDI_OUTPUTS.len(),
                    _ => 1,
                }
            };
        }
        let mut edges = Vec::with_capacity(patch.cables.len());
        let mut edge_lookup = BTreeMap::new();
        for (index, cable) in patch.cables.iter().enumerate() {
            let source = *node_lookup
                .get(&cable.from)
                .ok_or_else(|| format!("cable source '{}' does not exist", cable.from))?;
            let destination = *node_lookup
                .get(&cable.to)
                .ok_or_else(|| format!("cable destination '{}' does not exist", cable.to))?;
            let port = output_port(&nodes[source].kind, &cable.output).ok_or_else(|| {
                format!("node '{}': unknown output '{}'", cable.from, cable.output)
            })?;
            let input = input_port(
                &nodes[destination].kind,
                nodes[destination].model.template(),
                &cable.input,
            )
            .ok_or_else(|| format!("node '{}': unknown input '{}'", cable.to, cable.input))?;
            if nodes[destination].inputs[input].replace(index).is_some() {
                return Err(format!(
                    "node '{}': input '{}' already has a cable",
                    cable.to, cable.input
                ));
            }
            edges.push(Edge {
                source_port: nodes[source].output_start + port,
                source_node: source,
                destination_node: destination,
            });
            edge_lookup.insert(
                (
                    cable.from.clone(),
                    cable.output.clone(),
                    cable.to.clone(),
                    cable.input.clone(),
                ),
                index,
            );
        }
        let selected = patch
            .output
            .as_ref()
            .map(|id| {
                let node = *node_lookup
                    .get(id)
                    .ok_or_else(|| format!("output node '{id}' does not exist"))?;
                let port = output_port(&nodes[node].kind, &patch.output_port).ok_or_else(|| {
                    format!("node '{id}': unknown output '{}'", patch.output_port)
                })?;
                Ok::<_, String>(nodes[node].output_start + port)
            })
            .transpose()?;
        if patch.output.is_none() && patch.output_port != "out" {
            return Err("output_port requires a selected output node".into());
        }
        Ok(Self {
            rate: patch.sample_rate,
            nodes,
            node_lookup,
            edges,
            edge_lookup,
            edge_frames: vec![PortFrame::default(); patch.cables.len()],
            outputs: vec![PortFrame::default(); output_count],
            selected,
            stereo: patch.output.as_ref().is_some_and(|id| {
                patch
                    .nodes
                    .iter()
                    .any(|n| &n.id == id && n.kind == "audio_stereo")
            }),
        })
    }
    pub(crate) fn output_channels(&self) -> u16 {
        if self.stereo {
            2
        } else {
            1
        }
    }
    pub(crate) fn handle_midi(&mut self, event: MidiEvent) {
        for node in &mut self.nodes {
            if let Model::Midi(midi) = &mut node.model {
                midi.handle(event);
            }
        }
    }
    pub(crate) fn process_sample(&mut self) -> PortFrame {
        for node in &mut self.nodes {
            let a = node.inputs[0]
                .map(|edge| self.edge_frames[edge])
                .unwrap_or_default();
            let b = node.inputs[1]
                .map(|edge| self.edge_frames[edge])
                .unwrap_or_default();
            match &mut node.model {
                Model::Voices {
                    template,
                    voices,
                    active,
                    policy,
                } => {
                    let count = match policy {
                        VoicePolicy::Generator => 1,
                        VoicePolicy::Primary(0) => a.channels.max(1),
                        VoicePolicy::Primary(_) => b.channels.max(1),
                        VoicePolicy::Maximum => a.channels.max(b.channels).max(1),
                    };
                    // Only transitions touch inactive voice storage. All DSP
                    // instances and their large circuit histories already exist.
                    for voice in &mut voices
                        [usize::from(count.min(*active))..usize::from(count.max(*active))]
                    {
                        voice.copy_state_from(template);
                    }
                    *active = count;
                    let mut frame = PortFrame {
                        channels: count,
                        ..PortFrame::default()
                    };
                    let normal_a = if node.kind == "attenuverter" {
                        10.0
                    } else {
                        0.0
                    };
                    let normal_b = if node.kind == "gain" {
                        1.0
                    } else if node.kind == "analog_vca" {
                        10.0
                    } else {
                        0.0
                    };
                    for (voice, dsp) in voices[..usize::from(count)].iter_mut().enumerate() {
                        frame.values[voice] = dsp.process_sample(
                            a.voice(voice, normal_a),
                            b.voice(voice, normal_b),
                            a.channels > 0,
                            b.channels > 0,
                            self.rate as f64,
                        );
                    }
                    self.outputs[node.output_start] = frame;
                }
                Model::Routing(routing) => {
                    let mut inputs = [PortFrame::default(); MAX_PORTS];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.edge_frames[*edge];
                        }
                    }
                    let count = routing::output_names(&node.kind).unwrap().len();
                    routing.process(
                        &inputs,
                        &mut self.outputs[node.output_start..node.output_start + count],
                    );
                }
                Model::Midi(midi) => midi.process(
                    &mut self.outputs[node.output_start..node.output_start + MIDI_OUTPUTS.len()],
                ),
                Model::Stereo(gain) => {
                    let left = a.values[..a.channels as usize].iter().sum::<f32>() / 10.0 * *gain;
                    let right = if b.channels == 0 {
                        left
                    } else {
                        b.values[..b.channels as usize].iter().sum::<f32>() / 10.0 * *gain
                    };
                    let mut frame = PortFrame {
                        channels: 2,
                        ..PortFrame::default()
                    };
                    frame.values[0] = left;
                    frame.values[1] = right;
                    self.outputs[node.output_start] = frame;
                }
                Model::Constant(frame) => self.outputs[node.output_start] = *frame,
                Model::Split => {
                    for channel in 0..16 {
                        self.outputs[node.output_start + channel] =
                            PortFrame::mono(a.channel(channel));
                    }
                }
                Model::Merge(override_count) => {
                    let mut frame = PortFrame::default();
                    for input in 0..16 {
                        if let Some(edge) = node.inputs[input] {
                            let source = self.edge_frames[edge];
                            if source.channels > 0 {
                                frame.channels = (input + 1) as u8;
                                frame.values[input] = source.values[0];
                            }
                        }
                    }
                    if *override_count > 0 {
                        frame.channels = *override_count;
                    }
                    frame.values[frame.channels as usize..].fill(0.0);
                    self.outputs[node.output_start] = frame;
                }
                Model::Sum => {
                    self.outputs[node.output_start] =
                        PortFrame::mono(bounded(a.values[..a.channels as usize].iter().sum()))
                }
            }
        }
        // Publish counts and values only after every node has completed. All
        // cycles, including self-loops, therefore have a defined sample delay.
        for (edge, frame) in self.edges.iter().zip(&mut self.edge_frames) {
            *frame = self.outputs[edge.source_port];
        }
        self.selected
            .map(|port| self.outputs[port])
            .unwrap_or_default()
    }
    pub(crate) fn adopt_state_from(&mut self, old: &Self) {
        if self.rate != old.rate {
            return;
        }
        for (id, index) in &self.node_lookup {
            let Some(old_index) = old.node_lookup.get(id) else {
                continue;
            };
            let node = &mut self.nodes[*index];
            let previous = &old.nodes[*old_index];
            if node.kind != previous.kind {
                continue;
            }
            if let (Model::Midi(midi), Model::Midi(previous)) = (&mut node.model, &previous.model) {
                midi.adopt(previous);
            }
            if let (
                Model::Voices { voices, active, .. },
                Model::Voices {
                    voices: old_voices,
                    active: old_active,
                    ..
                },
            ) = (&mut node.model, &previous.model)
            {
                for voice in 0..usize::from(*old_active) {
                    voices[voice].copy_state_from(&old_voices[voice]);
                }
                *active = *old_active;
            }
        }
        self.edge_frames.fill(PortFrame::default());
        for (key, index) in &self.edge_lookup {
            let Some(old_index) = old.edge_lookup.get(key) else {
                continue;
            };
            let edge = &self.edges[*index];
            let previous = &old.edges[*old_index];
            let midi_compatible = match (
                &self.nodes[edge.source_node].model,
                &old.nodes[previous.source_node].model,
            ) {
                (Model::Midi(a), Model::Midi(b)) => a.compatible(b),
                _ => true,
            };
            if midi_compatible
                && self.nodes[edge.source_node].kind == old.nodes[previous.source_node].kind
                && self.nodes[edge.destination_node].kind
                    == old.nodes[previous.destination_node].kind
            {
                self.edge_frames[*index] = old.edge_frames[*old_index];
            }
        }
    }
    pub(crate) fn solver_failures(&self) -> u64 {
        self.nodes.iter().fold(0_u64, |sum, node| {
            if let Model::Voices { voices, active, .. } = &node.model {
                voices[..*active as usize].iter().fold(sum, |sum, voice| {
                    sum.saturating_add(voice.solver_failures())
                })
            } else {
                sum
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn patch(channels: f32, kind: &str) -> Patch {
        Patch {
            nodes: vec![
                Node {
                    id: "source".into(),
                    kind: "poly_constant".into(),
                    params: BTreeMap::from([
                        ("channels".into(), channels),
                        ("v1".into(), 0.0),
                        ("v2".into(), 0.0),
                        ("v3".into(), 0.0),
                        ("v4".into(), 0.0),
                    ]),
                },
                Node {
                    id: "voice".into(),
                    kind: kind.into(),
                    params: BTreeMap::new(),
                },
            ],
            cables: vec![crate::Cable {
                from: "source".into(),
                output: "out".into(),
                to: "voice".into(),
                input: if kind == "analog_vco" {
                    "pitch".into()
                } else {
                    "signal".into()
                },
            }],
            output: Some("voice".into()),
            ..Patch::new_v2()
        }
    }
    fn voices(engine: &PolyEngine) -> (&[Dsp], u8) {
        let Model::Voices { voices, active, .. } = &engine.nodes[1].model else {
            panic!("voice module")
        };
        (voices, *active)
    }
    #[test]
    fn inactive_voices_do_no_dsp_work_and_retirement_resets_phase_and_solver_state() {
        for kind in ["analog_vco", "ladder_vcf"] {
            let mut engine = PolyEngine::compile(&patch(4.0, kind)).unwrap();
            for _ in 0..17 {
                engine.process_sample();
            }
            let (states, active) = voices(&engine);
            assert_eq!(active, 4);
            for (channel, state) in states.iter().enumerate() {
                match state {
                    Dsp::Vco(vco) => assert_eq!(vco.phase == 0.0, channel >= 4),
                    Dsp::Ladder(ladder) => {
                        assert_eq!(ladder.diagnostics().max_iterations == 0, channel >= 4)
                    }
                    _ => unreachable!(),
                }
            }
            let mut retired = PolyEngine::compile(&patch(1.0, kind)).unwrap();
            retired.adopt_state_from(&engine);
            retired.process_sample(); // delayed old channel count still four
            assert_eq!(voices(&retired).1, 4);
            retired.process_sample(); // new count arrives; retired lanes reset
            let (states, active) = voices(&retired);
            assert_eq!(active, 1);
            for state in &states[1..] {
                match state {
                    Dsp::Vco(vco) => assert_eq!(vco.phase, 0.0),
                    Dsp::Ladder(ladder) => assert_eq!(ladder.diagnostics().max_iterations, 0),
                    _ => unreachable!(),
                }
            }
            let mut regrown = PolyEngine::compile(&patch(4.0, kind)).unwrap();
            regrown.adopt_state_from(&retired);
            regrown.process_sample();
            assert_eq!(voices(&regrown).1, 1);
            regrown.process_sample();
            let (states, active) = voices(&regrown);
            assert_eq!(active, 4);
            for state in &states[1..4] {
                match state {
                    Dsp::Vco(vco) => assert!((vco.phase - 261.6256 / 48000.0).abs() < 1e-9),
                    Dsp::Ladder(ladder) => assert_eq!(ladder.diagnostics().max_iterations, 1),
                    _ => unreachable!(),
                }
            }
        }
    }
}
