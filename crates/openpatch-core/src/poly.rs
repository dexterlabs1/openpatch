//! Version-2 reference scheduler: all modules read last sample's cable frames,
//! then all edges publish current output frames. Strings, routing and voice
//! storage are resolved/allocated only during compilation.
use crate::adsr_eg::{self, AdsrEg};
use crate::colored_noise::{self, ColoredNoise};
use crate::gates::{self, Gates};
use crate::macro_voice::{self, MacroVoice};
use crate::midi::{MidiCv, OUTPUTS as MIDI_OUTPUTS};
use crate::midi_cc::{self, MidiCc};
use crate::midi_gate::{self, MidiGate};
use crate::mixing::{self, Mixing};
use crate::modal_resonator::{self, ModalResonator};
use crate::multi_lfo::{self, MultiLfo};
use crate::multi_vco::{self, MultiVco};
use crate::pulses::{self, Pulses};
use crate::push::{self, Push};
use crate::random_values::{self, RandomValues};
use crate::routing::{self, Routing};
use crate::scope::{self, Scope};
use crate::score::{self, ScoreSequencer};
use crate::sequence::{self, ModuleAction, ModuleTelemetry, Seq3};
use crate::shift_register::{self, ShiftRegister};
use crate::signal::{self, ModuleControlState, ModuleLightState, Signal};
use crate::stereo_output::StereoOutput;
use crate::stock_delay::{self, StockDelay};
use crate::stock_vca::{self, StockVca};
use crate::stock_vcf::{self, StockVcf};
use crate::studio_fx::{self, StudioFx};
use crate::switch::{self, SequentialSwitch};
use crate::utility_bank::{self, UtilityBank};
use crate::voice_tools::{self, VoiceTools};
use crate::wavetable_osc::{self, WavetableOscillator};
use crate::{
    build_dsp, parameter, Dsp, MidiEvent, Node, Patch, PreparedWaveBanks, SavedModuleState,
    MAX_CABLES, MAX_NODES,
};
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
    crate::content::is_annotation(kind)
        || kind == "colored_noise"
        || kind == "score_sequencer"
        || kind == "modal_resonator"
        || kind == "macro_voice"
        || studio_fx::input_names(kind).is_some()
        || voice_tools::input_names(kind).is_some()
        || routing::input_names(kind).is_some()
        || switch::input_names(kind).is_some()
        || utility_bank::input_names(kind).is_some()
        || signal::input_names(kind).is_some()
        || mixing::input_names(kind).is_some()
        || wavetable_osc::input_names(kind).is_some()
        || matches!(
            kind,
            "poly_constant"
                | "poly_split"
                | "poly_merge"
                | "poly_sum"
                | "poly_viz"
                | "midi_gate"
                | "midi_cc"
                | "midi_cv"
                | "audio_stereo"
                | "multi_lfo"
                | "multi_vco"
                | "seq3"
                | "pulses"
                | "push"
                | "adsr_eg"
                | "stock_vcf"
                | "stock_vca"
                | "stock_delay"
                | "random_values"
                | "sh_asr"
                | "scope"
                | "gates"
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
    if let Some(ports) = wavetable_osc::input_names(kind) {
        return ports.iter().position(|port| *port == name);
    }
    if kind == "macro_voice" {
        return macro_voice::INPUTS.iter().position(|p| *p == name);
    }
    if kind == "modal_resonator" {
        return modal_resonator::INPUTS.iter().position(|p| *p == name);
    }
    if kind == "score_sequencer" {
        return score::INPUTS.iter().position(|p| *p == name);
    }
    if let Some(ports) = studio_fx::input_names(kind).or_else(|| voice_tools::input_names(kind)) {
        return ports.iter().position(|p| *p == name);
    }
    if let Some(ports) = mixing::input_names(kind) {
        return ports.iter().position(|p| *p == name);
    }
    if let Some(ports) = signal::input_names(kind) {
        return ports.iter().position(|p| *p == name);
    }
    if let Some(ports) = utility_bank::input_names(kind) {
        return ports.iter().position(|port| *port == name);
    }
    if let Some(ports) = switch::input_names(kind) {
        return ports.iter().position(|port| *port == name);
    }
    if let Some(ports) = routing::input_names(kind) {
        return ports.iter().position(|port| *port == name);
    }
    match kind {
        "multi_lfo" => multi_lfo::INPUTS.iter().position(|port| *port == name),
        "multi_vco" => multi_vco::INPUTS.iter().position(|port| *port == name),
        "seq3" => sequence::INPUTS.iter().position(|port| *port == name),
        "scope" => scope::INPUTS.iter().position(|port| *port == name),
        "adsr_eg" => adsr_eg::INPUTS.iter().position(|port| *port == name),
        "stock_vcf" => stock_vcf::INPUTS.iter().position(|port| *port == name),
        "stock_vca" => stock_vca::INPUTS.iter().position(|port| *port == name),
        "stock_delay" => stock_delay::INPUTS.iter().position(|port| *port == name),
        "push" => push::INPUTS.iter().position(|port| *port == name),
        "sh_asr" => shift_register::INPUTS.iter().position(|port| *port == name),
        "random_values" => random_values::INPUTS.iter().position(|port| *port == name),
        "gates" => gates::INPUTS.iter().position(|port| *port == name),
        "poly_constant" | "midi_cv" | "midi_cc" | "midi_gate" | "pulses" | "colored_noise" => None,
        "audio_stereo" => match name {
            "left" => Some(0),
            "right" => Some(1),
            _ => None,
        },
        "poly_split" | "poly_sum" | "poly_viz" if name == "signal" => Some(0),
        "poly_merge" => numbered_port(name, "in"),
        _ => template.and_then(|dsp| dsp.port(name)),
    }
}
fn output_port(kind: &str, name: &str) -> Option<usize> {
    if let Some(ports) = wavetable_osc::output_names(kind) {
        return ports.iter().position(|port| *port == name);
    }
    if kind == "macro_voice" {
        return macro_voice::OUTPUTS.iter().position(|p| *p == name);
    }
    if kind == "colored_noise" {
        return colored_noise::OUTPUTS.iter().position(|p| *p == name);
    }
    if kind == "poly_viz" || crate::content::is_annotation(kind) {
        return None;
    }
    if kind == "modal_resonator" {
        return modal_resonator::OUTPUTS.iter().position(|p| *p == name);
    }
    if kind == "score_sequencer" {
        return score::OUTPUTS.iter().position(|p| *p == name);
    }
    if let Some(ports) = studio_fx::output_names(kind).or_else(|| voice_tools::output_names(kind)) {
        return ports.iter().position(|p| *p == name);
    }
    if let Some(ports) = mixing::output_names(kind) {
        return ports.iter().position(|p| *p == name);
    }
    if let Some(ports) = signal::output_names(kind) {
        return ports.iter().position(|p| *p == name);
    }
    if let Some(ports) = utility_bank::output_names(kind) {
        return ports.iter().position(|port| *port == name);
    }
    if let Some(ports) = switch::output_names(kind) {
        return ports.iter().position(|port| *port == name);
    }
    if let Some(ports) = routing::output_names(kind) {
        return ports.iter().position(|port| *port == name);
    }
    if kind == "scope" {
        scope::OUTPUTS.iter().position(|port| *port == name)
    } else if kind == "pulses" {
        pulses::OUTPUTS.iter().position(|port| *port == name)
    } else if kind == "sh_asr" {
        shift_register::OUTPUTS
            .iter()
            .position(|port| *port == name)
    } else if kind == "random_values" {
        random_values::OUTPUTS.iter().position(|port| *port == name)
    } else if kind == "stock_vcf" {
        stock_vcf::OUTPUTS.iter().position(|port| *port == name)
    } else if kind == "stock_delay" {
        stock_delay::OUTPUTS.iter().position(|port| *port == name)
    } else if kind == "adsr_eg" {
        adsr_eg::OUTPUTS.iter().position(|port| *port == name)
    } else if kind == "push" {
        push::OUTPUTS.iter().position(|port| *port == name)
    } else if kind == "gates" {
        gates::OUTPUTS.iter().position(|port| *port == name)
    } else if kind == "seq3" {
        sequence::OUTPUTS.iter().position(|port| *port == name)
    } else if kind == "multi_lfo" {
        multi_lfo::OUTPUTS.iter().position(|port| *port == name)
    } else if kind == "multi_vco" {
        multi_vco::OUTPUTS.iter().position(|port| *port == name)
    } else if kind == "midi_gate" {
        midi_gate::OUTPUTS.iter().position(|port| *port == name)
    } else if kind == "midi_cc" {
        midi_cc::OUTPUTS.iter().position(|port| *port == name)
    } else if kind == "midi_cv" {
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

pub(crate) fn catalog() -> [Value; 9] {
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
        midi_cc::catalog(),
        midi_gate::catalog(),
        json!({"kind":"midi_cv","title":"MIDI-CV","model":"functional","description":"MIDI notes to fixed polyphonic voltages; pitch 1 V/oct relative to C4; gates 10 V. Channel pressure aftertouch (poly pressure ignored), separate ±5 V pitchwheel, CC1 modwheel. Sustain CC64, all-sound-off CC120, reset controllers CC121, all-notes-off CC123. Triggers last 1 ms. Duplicate notes are counted; one note-off releases one pressed voice. Mono uses last-held-note priority. No MIDI events means silence.","inputs":[],"params":{"channels":{"default":4.0,"min":1.0,"max":16.0,"step":1.0},"channel":{"default":0.0,"min":0.0,"max":16.0,"step":1.0,"description":"0 omni; 1–16 selects wire channel 0–15"},"allocation":{"default":0.0,"min":0.0,"max":2.0,"step":1.0,"description":"0 rotate free then next steal; 1 reuse previous note/channel else lowest free then lane 1 steal; 2 lowest free then lane 1 steal, compact releases"},"clock_div":{"default":24.0,"min":1.0,"max":96.0,"step":1.0}}}),
        json!({"kind":"audio_stereo","title":"STEREO AUDIO","model":"functional","description":"Unity-sum polyphony independently on left/right, divide voltages by 10, multiply gain. Right normalizes to left only when disconnected or zero-channel. Selecting out explicitly renders stereo.","inputs":["left","right"],"params":{"gain":{"default":1.0,"min":0.0,"max":2.0},"dc_block":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1],"labels":["Off","On"],"description":"Optional bilinear 10 Hz DC blocker; defaults off for older patches"}}}),
        constants,
        json!({"kind":"poly_split","title":"SPLIT","model":"functional","description":"Split channels 1–16 into named mono outputs out1..out16. Missing channels produce mono zero.","inputs":["signal"],"params":{}}),
        json!({"kind":"poly_merge","title":"MERGE","model":"functional","description":"Take channel zero of in1..in16 into one polyphonic output. channels=0 auto uses the highest nonzero-channel input; holes are preserved as zero. Override 1–16 forces channel count.","inputs":(1..=16).map(|i|format!("in{i}")).collect::<Vec<_>>(),"params":{"channels":{"default":0.0,"min":0.0,"max":16.0,"step":1.0}}}),
        json!({"kind":"poly_viz","title":"VIZ","model":"functional","description":"Sixteen paired polarity lights visualize polyphonic signal voltages. Samples and publishes every 16 frames with immediate attack and 30/s Euler release; bypass freezes lights and channel count.","inputs":["signal"],"params":{}}),
        json!({"kind":"poly_sum","title":"SUM","model":"functional","description":"Sum all active signal channels with a master level into one mono output. Empty signal produces mono zero; final arithmetic bound ±1,000,000 V. Six post-level peak meter bands relative to 10 V, with instant attack and 0.1-second Euler release. Meter samples every 16 frames; channel count and lights publish every 512 frames and freeze while bypassed.","inputs":["signal"],"params":{"level":{"default":1.0,"min":0.0,"max":1.0}}}),
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
        "constant" | "gate" | "white_noise" | "colored_noise" => VoicePolicy::Generator,
        "sample_hold" => VoicePolicy::Primary(1),
        "stock_vcf" => VoicePolicy::Primary(3),
        "mixer" | "crossfade" | "modal_resonator" | "stock_vca" | "wavetable_vco" => {
            VoicePolicy::Maximum
        }
        _ => VoicePolicy::Primary(0),
    }
}
pub(crate) fn annotate_catalog(module: &mut Value) {
    let kind = module["kind"].as_str().unwrap().to_owned();
    let outputs = if let Some(names) = wavetable_osc::output_names(&kind) {
        names.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()
    } else if kind == "macro_voice" {
        macro_voice::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "colored_noise" {
        colored_noise::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "score_sequencer" {
        score::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if let Some(names) =
        studio_fx::output_names(&kind).or_else(|| voice_tools::output_names(&kind))
    {
        names.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()
    } else if let Some(names) = mixing::output_names(&kind) {
        names.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()
    } else if let Some(names) = signal::output_names(&kind) {
        names.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()
    } else if let Some(names) = utility_bank::output_names(&kind) {
        names.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()
    } else if let Some(names) = switch::output_names(&kind) {
        names.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()
    } else if let Some(names) = routing::output_names(&kind) {
        names.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()
    } else if kind == "pulses" {
        pulses::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "scope" {
        scope::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "sh_asr" {
        shift_register::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "random_values" {
        random_values::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "stock_vcf" {
        stock_vcf::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "stock_delay" {
        stock_delay::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "adsr_eg" {
        adsr_eg::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "push" {
        push::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "gates" {
        gates::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "seq3" {
        sequence::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "multi_lfo" {
        multi_lfo::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "multi_vco" {
        multi_vco::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "midi_cv" {
        MIDI_OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "poly_viz" || crate::content::is_annotation(&kind) {
        Vec::new()
    } else if kind == "midi_gate" {
        midi_gate::OUTPUTS
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
    } else if kind == "midi_cc" {
        midi_cc::OUTPUTS
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
        "poly_viz" => "input-only visualization of 0–16 independent channels",
        "notes" | "blank" => "no signal ports or DSP state",
        "poly_sum" => "mono sum of active channels with master level",
        _ => match voice_policy(&kind) {
            VoicePolicy::Generator => "one generated voice",
            VoicePolicy::Primary(1) => "trigger input sets voices, minimum one",
            VoicePolicy::Primary(_) => "first declared input sets voices, minimum one",
            VoicePolicy::Maximum => "maximum input channel count, minimum one",
        },
    };
    module["polyphony"] = json!({"version_1":"mono", "version_2":policy, "maximum_channels":16,"auxiliary_inputs":"mono broadcasts; missing short-poly channels are zero; zero-channel ports normalize as disconnected","retired_voices":"reset on retirement and activation"});
    let primary = match kind.as_str() {
        "poly_constant" | "poly_merge" | "midi_cv" | "midi_cc" | "midi_gate" | "audio_stereo" => {
            Value::Null
        }
        _ => match voice_policy(&kind) {
            VoicePolicy::Generator | VoicePolicy::Maximum => Value::Null,
            VoicePolicy::Primary(index) => {
                module["inputs"].get(index).cloned().unwrap_or(Value::Null)
            }
        },
    };
    module["polyphony"]["primary_input"] = primary;
    if crate::content::is_annotation(&kind) {
        module["polyphony"] = json!({"version_1":"unsupported","version_2":"no signal ports or DSP state","maximum_channels":0,"primary_input":null});
    }
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
    if kind == "midi_cc" {
        midi_cc::annotate(module);
    }
    if kind == "poly_viz" {
        module["light_names"] = json!((1..=16).map(|i| format!("Channel {i}")).collect::<Vec<_>>());
        module["light_policy"] = json!({"colors":["green","red"],"source":"independent active input channels, voltage / 10","reference_voltage":10,"meter_sample_frames":16,"publication_frames":16,"release_method":"f32 Euler immediate attack; decay += (target - current) * 30 * (16 / sample_rate)","input_channels":"Some(0..16), initially 0; latched with lights every 16 frames","bypass":"no outputs; lights, count, divider freeze","state_adoption":"same identity, kind, and sample rate preserve state; fresh incarnation and sample rate change reset","inactive_channels":"zero target with smooth decay"});
        module["polyphony"]["primary_input"] = json!("signal");
        module["polyphony"]["retired_voices"] = json!("inactive channel lights decay toward zero");
    }
    if kind == "poly_sum" {
        module["light_names"] = json!(["0 dB", "-3 dB", "-6 dB", "-12 dB", "-24 dB", "-36 dB"]);
        module["light_policy"] = json!({
            "source": "post-level bounded mono output",
            "reference_voltage": 10.0,
            "bands_db": [[0,0],[-3,0],[-6,-3],[-12,-6],[-24,-12],[-36,-24]],
            "meter_sample_frames": 16,
            "publication_frames": 512,
            "release_seconds": 0.1,
            "release_method": "f32 Euler peak with immediate attack",
            "input_channels": "optional ModuleLightState field; Some(0..16) for SUM, initially 0, published every 512 frames",
            "bypass": "silent mono output; meter, lights, and input channel display freeze",
            "state_adoption": "same identity and sample rate retain meter, lights, channel display, divider phase; new level applies immediately; sample rate changes reset runtime state"
        });
        module["arithmetic_policy"] = json!({"sum":"f32 active lanes in channel order", "order":"sum then level then final safety bound", "safety_bound_volts":1_000_000, "nonfinite":"graph sources sanitize nonfinite samples to zero before SUM"});
    }
    module["output_metadata"] = json!(outputs.iter().map(|name|json!({"name":name,"maximum_channels":if kind=="audio_stereo" {2} else if kind=="poly_split" || kind=="poly_sum" || matches!(voice_policy(&kind),VoicePolicy::Generator) {1} else {16},"units":if kind=="audio_out" || kind=="audio_stereo" {"normalized digital amplitude"} else if module["model"]=="legacy" {"legacy normalized"} else {"V"}})).collect::<Vec<_>>());
    if matches!(
        kind.as_str(),
        "drum_voice" | "stereo_delay" | "stereo_reverb" | "sidechain_compressor"
    ) {
        module["polyphony"]["version_2"] = json!(
            "mono per port; stereo effects use independent left/right ports and read channel zero"
        );
        module["polyphony"]["maximum_channels"] = json!(1);
        for output in module["output_metadata"].as_array_mut().unwrap() {
            output["maximum_channels"] = json!(1);
        }
    }
    if kind == "score_sequencer" {
        module["polyphony"]["version_2"] = json!(
            "four polyphonic note tracks; automation lanes packed into one polyphonic output"
        );
        for (index, output) in module["output_metadata"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            output["maximum_channels"] = json!(if index < 12 {
                4
            } else if index == 12 {
                16
            } else {
                1
            });
        }
    }
    if kind == "colored_noise" {
        colored_noise::annotate(module);
    }
    routing::annotate(module);
    switch::annotate(module);
    utility_bank::annotate(module);
    signal::annotate(module);
    mixing::annotate(module);
    wavetable_osc::annotate(module);
    if kind == "multi_lfo" {
        multi_lfo::annotate(module);
    }
    if kind == "multi_vco" {
        multi_vco::annotate(module);
    }
    if kind == "modal_resonator" {
        modal_resonator::annotate(module);
    }
    if kind == "macro_voice" {
        macro_voice::annotate(module);
    }
    if kind == "seq3" {
        sequence::annotate(module);
    }
    if kind == "pulses" {
        pulses::annotate(module);
    }
    if kind == "scope" {
        scope::annotate(module);
    }
    if kind == "push" {
        push::annotate(module);
    }
    if kind == "stock_vca" {
        stock_vca::annotate(module);
    }
    if kind == "stock_vcf" {
        stock_vcf::annotate(module);
    }
    if kind == "stock_delay" {
        stock_delay::annotate(module);
    }
    if kind == "adsr_eg" {
        adsr_eg::annotate(module);
    }
    if kind == "sh_asr" {
        shift_register::annotate(module);
    }
    if kind == "random_values" {
        random_values::annotate(module);
    }
    if kind == "gates" {
        gates::annotate(module);
    }
    if kind == "midi_gate" {
        midi_gate::annotate(module);
    }
}

#[derive(Debug)]
enum Model {
    Annotation,
    ColoredNoise(Box<ColoredNoise>),
    Score(Box<ScoreSequencer>),
    StudioFx(Box<StudioFx>),
    ModalResonator(Box<ModalResonator>),
    MacroVoice(Box<MacroVoice>),
    VoiceTools(Box<VoiceTools>),
    Voices {
        template: Dsp,
        voices: Vec<Dsp>,
        active: u8,
        policy: VoicePolicy,
    },
    Constant(PortFrame),
    Split,
    Merge(u8),
    Sum(crate::sum::Sum),
    Viz(crate::viz::Viz),
    Midi(Box<MidiCv>),
    MidiCc(Box<MidiCc>),
    MidiGate(Box<MidiGate>),
    Stereo(StereoOutput),
    Routing(Routing),
    MultiLfo(Box<MultiLfo>),
    MultiVco(Box<MultiVco>),
    Wavetable(Box<WavetableOscillator>),
    Seq3(Box<Seq3>),
    Pulses(Box<Pulses>),
    Push(Box<Push>),
    AdsrEg(Box<AdsrEg>),
    StockVcf(Box<StockVcf>),
    StockDelay(Box<StockDelay>),
    StockVca(Box<StockVca>),
    ShiftRegister(Box<ShiftRegister>),
    RandomValues(Box<RandomValues>),
    Scope(Box<Scope>),
    Gates(Box<Gates>),
    Switch(SequentialSwitch),
    UtilityBank(UtilityBank),
    Signal(Box<Signal>),
    Mixing(Mixing),
}
impl Model {
    fn build(node: &Node, rate: u32, wave_banks: &PreparedWaveBanks) -> Result<Self, String> {
        if wavetable_osc::input_names(&node.kind).is_some() {
            let bank = wave_banks
                .get(&node.id)
                .ok_or_else(|| format!("node '{}': missing prepared wavetable bank", node.id))?;
            return WavetableOscillator::build(node, rate, bank)
                .map(|oscillator| Self::Wavetable(Box::new(oscillator)));
        }
        if crate::content::is_annotation(&node.kind) {
            if let Some(key) = node.params.keys().next() {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
            return Ok(Self::Annotation);
        }
        if node.kind == "colored_noise" {
            return ColoredNoise::build(node, rate).map(|s| Self::ColoredNoise(Box::new(s)));
        }
        if node.kind == "modal_resonator" {
            return ModalResonator::build(node, rate).map(|s| Self::ModalResonator(Box::new(s)));
        }
        if node.kind == "macro_voice" {
            return MacroVoice::build(node, rate).map(|s| Self::MacroVoice(Box::new(s)));
        }
        if node.kind == "score_sequencer" {
            return ScoreSequencer::build(node, rate).map(|s| Self::Score(Box::new(s)));
        }
        if studio_fx::input_names(&node.kind).is_some() {
            return StudioFx::build(node, rate).map(|s| Self::StudioFx(Box::new(s)));
        }
        if voice_tools::input_names(&node.kind).is_some() {
            return VoiceTools::build(node, rate).map(|s| Self::VoiceTools(Box::new(s)));
        }
        if mixing::input_names(&node.kind).is_some() {
            return Mixing::build(node, rate).map(Self::Mixing);
        }
        if signal::input_names(&node.kind).is_some() {
            return Signal::build(node, rate).map(|s| Self::Signal(Box::new(s)));
        }
        if utility_bank::input_names(&node.kind).is_some() {
            return UtilityBank::build(node).map(Self::UtilityBank);
        }
        if switch::input_names(&node.kind).is_some() {
            return Ok(Self::Switch(SequentialSwitch::build(node, rate)?));
        }
        if node.kind == "gates" {
            return Ok(Self::Gates(Box::new(Gates::build(node, rate)?)));
        }
        if node.kind == "scope" {
            return Ok(Self::Scope(Box::new(Scope::build(node, rate)?)));
        }
        if node.kind == "sh_asr" {
            return Ok(Self::ShiftRegister(Box::new(ShiftRegister::build(node)?)));
        }
        if node.kind == "random_values" {
            return Ok(Self::RandomValues(Box::new(RandomValues::build(node)?)));
        }
        if node.kind == "stock_vca" {
            return Ok(Self::StockVca(Box::new(StockVca::build(node, rate)?)));
        }
        if node.kind == "stock_vcf" {
            return Ok(Self::StockVcf(Box::new(StockVcf::build(node, rate)?)));
        }
        if node.kind == "stock_delay" {
            return Ok(Self::StockDelay(Box::new(StockDelay::build(node, rate)?)));
        }
        if node.kind == "adsr_eg" {
            return Ok(Self::AdsrEg(Box::new(AdsrEg::build(node, rate)?)));
        }
        if node.kind == "push" {
            return Ok(Self::Push(Box::new(Push::build(node, rate)?)));
        }
        if node.kind == "pulses" {
            return Ok(Self::Pulses(Box::new(Pulses::build(node, rate)?)));
        }
        if node.kind == "seq3" {
            return Ok(Self::Seq3(Box::new(Seq3::build(node, rate)?)));
        }
        if node.kind == "multi_lfo" {
            return Ok(Self::MultiLfo(Box::new(MultiLfo::build(node, rate)?)));
        }
        if node.kind == "multi_vco" {
            return Ok(Self::MultiVco(Box::new(MultiVco::build(node, rate)?)));
        }
        if let Some(model) = Routing::build(node) {
            return model.map(Self::Routing);
        }
        if node.kind == "midi_gate" {
            return MidiGate::build(node, rate).map(|m| Self::MidiGate(Box::new(m)));
        }
        if node.kind == "midi_cc" {
            return MidiCc::build(node, rate).map(|m| Self::MidiCc(Box::new(m)));
        }
        if node.kind == "midi_cv" {
            return Ok(Self::Midi(Box::new(MidiCv::build(node, rate)?)));
        }
        if node.kind == "audio_stereo" {
            return StereoOutput::build(node, rate).map(Self::Stereo);
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
            "poly_split" | "poly_viz" => Some(Vec::new()),
            "poly_sum" => Some(vec!["level".to_string()]),
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
                "poly_viz" => Self::Viz(crate::viz::Viz::new(rate)),
                "poly_sum" => Self::Sum(crate::sum::Sum::build(node, rate)?),
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
    output_count: usize,
    bypassed: bool,
    bypass_inputs: [Option<usize>; MAX_PORTS],
    connected_mask: u16,
    base_connected_mask: u16,
}
#[derive(Debug)]
struct Edge {
    source_port: usize,
    source_node: usize,
    destination_node: usize,
    route: EdgeKey,
    identity: EdgeIdentity,
}
type EdgeKey = (String, String, String, String);
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum EdgeIdentity {
    Explicit(String),
    LegacyRoute(EdgeKey, usize),
}

#[derive(Debug)]
struct InputGroup {
    start: usize,
    len: usize,
}

#[derive(Debug)]
pub(crate) struct PolyEngine {
    rate: u32,
    nodes: Vec<PolyNode>,
    node_lookup: BTreeMap<String, usize>,
    edges: Vec<Edge>,
    edge_lookup: BTreeMap<EdgeIdentity, usize>,
    route_counts: BTreeMap<EdgeKey, usize>,
    unique_legacy_routes: BTreeMap<EdgeKey, Option<usize>>,
    cable_lookup: BTreeMap<String, usize>,
    edge_frames: Vec<PortFrame>,
    input_groups: Vec<InputGroup>,
    input_edges: Vec<usize>,
    input_frames: Vec<PortFrame>,
    outputs: Vec<PortFrame>,
    selected: Option<usize>,
    stereo: bool,
    pub(crate) port_lights: crate::port_lights::Observer,
}
impl PolyEngine {
    pub(crate) fn resolve_port_light(
        &self,
        id: &str,
        direction: crate::PortDirection,
        port: &str,
    ) -> Result<Option<usize>, String> {
        let &node = self
            .node_lookup
            .get(id)
            .ok_or_else(|| format!("module '{id}' does not exist"))?;
        let compiled = &self.nodes[node];
        let local = match direction {
            crate::PortDirection::Input => {
                input_port(&compiled.kind, compiled.model.template(), port)
            }
            crate::PortDirection::Output => output_port(&compiled.kind, port),
        }
        .ok_or_else(|| format!("module '{id}': unknown {direction:?} port '{port}'"))?;
        Ok(self.port_lights.find(node, direction, local))
    }
    pub(crate) fn midi_gate_mut(&mut self, index: usize) -> Option<&mut MidiGate> {
        if let Model::MidiGate(midi) = &mut self.nodes.get_mut(index)?.model {
            Some(midi)
        } else {
            None
        }
    }
    pub(crate) fn midi_gate(&self, index: usize) -> Option<&MidiGate> {
        if let Model::MidiGate(midi) = &self.nodes.get(index)?.model {
            Some(midi)
        } else {
            None
        }
    }
    pub(crate) fn midi_gate_active(&self, index: usize) -> bool {
        self.nodes
            .get(index)
            .is_some_and(|node| !node.bypassed && matches!(node.model, Model::MidiGate(_)))
    }
    pub(crate) fn midi_cc_mut(&mut self, index: usize) -> Option<&mut MidiCc> {
        let node = self.nodes.get_mut(index)?;
        if let Model::MidiCc(midi) = &mut node.model {
            Some(midi)
        } else {
            None
        }
    }
    pub(crate) fn midi_cc_active(&self, index: usize) -> bool {
        self.nodes
            .get(index)
            .is_some_and(|node| !node.bypassed && matches!(node.model, Model::MidiCc(_)))
    }
    pub(crate) fn midi_cc(&self, index: usize) -> Option<&MidiCc> {
        if let Model::MidiCc(midi) = &self.nodes.get(index)?.model {
            Some(midi)
        } else {
            None
        }
    }
    pub(crate) fn begin_taps(&mut self, taps: &[Option<usize>]) {
        for node in &mut self.nodes {
            for tap in taps.iter().flatten() {
                if matches!(node.model, Model::MidiCc(_) | Model::StockVcf(_))
                    && (node.output_start..node.output_start + node.output_count).contains(tap)
                {
                    node.connected_mask |= 1 << (tap - node.output_start);
                }
            }
        }
    }
    pub(crate) fn end_taps(&mut self) {
        for node in &mut self.nodes {
            node.connected_mask = node.base_connected_mask;
        }
    }
    pub(crate) fn resolve_cable_state(&self, id: &str) -> Result<usize, String> {
        self.cable_lookup
            .get(id)
            .copied()
            .ok_or_else(|| format!("cable '{id}' does not exist"))
    }
    pub(crate) fn resolve_module_state(&self, id: &str) -> Result<usize, String> {
        self.node_lookup
            .get(id)
            .copied()
            .ok_or_else(|| format!("module '{id}' does not exist"))
    }
    pub(crate) fn resolve_module_control(&self, id: &str) -> Result<usize, String> {
        let index = self.resolve_module_lights(id)?;
        if self.module_control_state(index).is_none() {
            return Err(format!("module '{id}' has no manual controls"));
        }
        Ok(index)
    }
    pub(crate) fn resolve_module_scope(&self, id: &str) -> Result<usize, String> {
        let index = *self
            .node_lookup
            .get(id)
            .ok_or_else(|| format!("module '{id}' does not exist"))?;
        if self.module_scope(index).is_none() {
            return Err(format!("module '{id}' has no scope"));
        }
        Ok(index)
    }
    pub(crate) fn module_scope(&self, index: usize) -> Option<&crate::ScopeSnapshot> {
        match self.nodes.get(index).map(|n| &n.model) {
            Some(Model::Scope(s)) => Some(&s.snapshot),
            _ => None,
        }
    }
    pub(crate) fn resolve_module_lights(&self, id: &str) -> Result<usize, String> {
        let index = *self
            .node_lookup
            .get(id)
            .ok_or_else(|| format!("module '{id}' does not exist"))?;
        if self.module_lights(index).is_none() {
            return Err(format!("module '{id}' has no signal utility lights"));
        }
        Ok(index)
    }
    pub(crate) fn module_control_state(&self, index: usize) -> Option<ModuleControlState> {
        match self.nodes.get(index).map(|n| &n.model) {
            Some(Model::Signal(s)) => s.controls(),
            Some(Model::Pulses(p)) => Some(p.controls()),
            Some(Model::Push(p)) => Some(p.controls()),
            Some(Model::AdsrEg(p)) => Some(p.controls()),
            Some(Model::ShiftRegister(p)) => Some(p.controls()),
            Some(Model::RandomValues(p)) => Some(p.controls()),
            Some(Model::Gates(g)) => Some(g.controls()),
            _ => None,
        }
    }
    pub(crate) fn module_lights(&self, index: usize) -> Option<ModuleLightState> {
        match self.nodes.get(index).map(|n| &n.model) {
            Some(Model::Signal(s)) => Some(s.lights()),
            Some(Model::Pulses(p)) => Some(p.lights()),
            Some(Model::Push(p)) => Some(p.lights()),
            Some(Model::AdsrEg(p)) => Some(p.lights()),
            Some(Model::StockVca(p)) => Some(p.lights()),
            Some(Model::StockDelay(p)) => {
                let mut lights = ModuleLightState {
                    count: 1,
                    ..ModuleLightState::default()
                };
                lights.brightness[0][0] = p.clock_light();
                Some(lights)
            }
            Some(Model::ShiftRegister(p)) => Some(p.lights()),
            Some(Model::RandomValues(p)) => Some(p.lights()),
            Some(Model::Gates(g)) => Some(g.lights()),
            Some(Model::Mixing(m)) => m.lights(),
            Some(Model::Sum(s)) => Some(s.lights()),
            Some(Model::Viz(v)) => Some(v.lights()),
            Some(Model::MidiGate(m)) => Some(m.lights()),
            Some(Model::Wavetable(oscillator)) => Some(oscillator.module_lights()),
            _ => None,
        }
    }
    pub(crate) fn release_module_controls(&mut self, index: usize) -> bool {
        match self.nodes.get_mut(index).map(|n| &mut n.model) {
            Some(Model::MidiGate(m)) => {
                m.host_panic();
                true
            }
            Some(Model::Signal(s)) => s.release(),
            Some(Model::Pulses(p)) => {
                p.release();
                true
            }
            Some(Model::ShiftRegister(p)) => {
                p.release();
                true
            }
            Some(Model::RandomValues(p)) => {
                p.release();
                true
            }
            Some(Model::AdsrEg(p)) => {
                p.release();
                true
            }
            Some(Model::Push(p)) => {
                p.release();
                true
            }
            Some(Model::Gates(g)) => {
                g.release();
                true
            }
            _ => false,
        }
    }
    pub(crate) fn supports_module_action(&self, index: usize, action: ModuleAction) -> bool {
        match self.nodes.get(index).map(|n| &n.model) {
            Some(Model::Seq3(_)) => matches!(
                action,
                ModuleAction::Clock | ModuleAction::Run | ModuleAction::Reset
            ),
            Some(Model::Signal(s)) => s.supports(action),
            Some(Model::Pulses(_)) => action.pulse_button().is_some(),
            Some(Model::Push(_)) => Push::supports(action),
            Some(Model::AdsrEg(_)) => AdsrEg::supports(action),
            Some(Model::ShiftRegister(_)) => ShiftRegister::supports(action),
            Some(Model::RandomValues(_)) => RandomValues::supports(action),
            Some(Model::Gates(_)) => Gates::supports(action),
            Some(Model::MidiGate(_)) => action == ModuleAction::MidiPanic,
            _ => false,
        }
    }
    pub(crate) fn resolve_module_action(
        &self,
        id: &str,
        action: ModuleAction,
    ) -> Result<usize, String> {
        let index = *self
            .node_lookup
            .get(id)
            .ok_or_else(|| format!("module '{id}' does not exist"))?;
        let release = matches!(
            action,
            ModuleAction::BRelease
                | ModuleAction::GateRelease
                | ModuleAction::PushRelease
                | ModuleAction::HoldRelease
                | ModuleAction::ResetRelease
                | ModuleAction::MidiPanic
        ) || action.pulse_button().is_some_and(|(_, press)| !press);
        if !self.supports_module_action(index, action) || (self.nodes[index].bypassed && !release) {
            return Err(format!(
                "module '{id}' does not support this action in its current state"
            ));
        }
        Ok(index)
    }
    pub(crate) fn resolve_module_telemetry(&self, id: &str) -> Result<usize, String> {
        let node = *self
            .node_lookup
            .get(id)
            .ok_or_else(|| format!("module '{id}' does not exist"))?;
        if !matches!(self.nodes[node].model, Model::Seq3(_) | Model::Switch(_)) {
            return Err(format!("module '{id}' does not expose runtime telemetry"));
        }
        Ok(node)
    }
    pub(crate) fn resolve_module_event(&self, id: &str) -> Result<usize, String> {
        let node = *self
            .node_lookup
            .get(id)
            .ok_or_else(|| format!("module '{id}' does not exist"))?;
        if !matches!(self.nodes[node].model, Model::MidiGate(_))
            && (self.nodes[node].bypassed
                || !(matches!(self.nodes[node].model, Model::Seq3(_))
                    || self.module_control_state(node).is_some()))
        {
            return Err(format!("module '{id}' does not support runtime actions"));
        }
        Ok(node)
    }
    pub(crate) fn handle_module_event(&mut self, node: usize, action: ModuleAction) -> bool {
        if action == ModuleAction::MidiPanic {
            return self.midi_gate_mut(node).is_some_and(|m| {
                m.host_panic();
                true
            });
        }
        if self.nodes.get(node).is_some_and(|node| node.bypassed) {
            if matches!(
                action,
                ModuleAction::PushRelease | ModuleAction::ResetRelease
            ) {
                if let Model::ShiftRegister(p) = &mut self.nodes[node].model {
                    p.release();
                    return true;
                }
            }
            if action == ModuleAction::PushRelease {
                if let Model::AdsrEg(p) = &mut self.nodes[node].model {
                    p.release();
                    return true;
                }
                if let Model::RandomValues(p) = &mut self.nodes[node].model {
                    p.release();
                    return true;
                }
            }
            if action == ModuleAction::ResetRelease {
                if let Model::Gates(g) = &mut self.nodes[node].model {
                    g.release();
                    return true;
                }
            }
            if matches!(
                action,
                ModuleAction::PushRelease | ModuleAction::HoldRelease
            ) {
                if let Model::Push(p) = &mut self.nodes[node].model {
                    p.release_button(action == ModuleAction::HoldRelease);
                    return true;
                }
            }
            if let Some((button, false)) = action.pulse_button() {
                if let Model::Pulses(p) = &mut self.nodes[node].model {
                    p.release_button(button);
                    return true;
                }
            }
        }
        if self.nodes.get(node).is_none_or(|node| node.bypassed) {
            if self.supports_module_action(node, action)
                && matches!(action, ModuleAction::BRelease | ModuleAction::GateRelease)
            {
                return self.release_module_controls(node);
            }
            return false;
        }
        match self.nodes.get_mut(node).map(|node| &mut node.model) {
            Some(Model::Seq3(seq)) => seq.enqueue(action),
            Some(Model::Signal(s)) => s.enqueue(action),
            Some(Model::Pulses(p)) => p.enqueue(action),
            Some(Model::Push(p)) => p.enqueue(action),
            Some(Model::AdsrEg(p)) => p.enqueue(action),
            Some(Model::ShiftRegister(p)) => p.enqueue(action),
            Some(Model::RandomValues(p)) => p.enqueue(action),
            Some(Model::Gates(g)) => g.enqueue(action),
            _ => false,
        }
    }
    pub(crate) fn module_telemetry(&self, node: usize) -> Option<ModuleTelemetry> {
        match self.nodes.get(node).map(|node| &node.model) {
            Some(Model::Seq3(seq)) => Some(seq.telemetry()),
            Some(Model::Switch(switch)) => Some(switch.telemetry()),
            _ => None,
        }
    }
    pub(crate) fn reset_module_state(&mut self, index: usize) -> bool {
        let Some(node) = self.nodes.get_mut(index) else {
            return false;
        };
        let count = match &mut node.model {
            Model::MidiGate(m) => {
                m.reset_state();
                16
            }
            Model::MidiCc(m) => {
                m.reset_state();
                16
            }
            Model::ColoredNoise(s) => {
                s.reset_state();
                colored_noise::OUTPUTS.len()
            }
            Model::Wavetable(oscillator) => {
                oscillator.reset_state();
                wavetable_osc::OUTPUTS.len()
            }
            Model::Stereo(s) => {
                s.reset_state();
                node.output_count
            }
            Model::Score(s) => {
                s.reset_state();
                node.output_count
            }
            Model::ModalResonator(s) => {
                s.reset_state();
                node.output_count
            }
            Model::MacroVoice(s) => {
                s.reset_state();
                node.output_count
            }
            Model::StudioFx(s) => {
                s.reset_state();
                node.output_count
            }
            Model::VoiceTools(s) => {
                s.reset_state();
                node.output_count
            }
            Model::Gates(g) => {
                g.reset_state();
                gates::OUTPUTS.len()
            }
            Model::Scope(s) => {
                s.reset();
                scope::OUTPUTS.len()
            }
            Model::ShiftRegister(p) => {
                p.reset_state();
                shift_register::OUTPUTS.len()
            }
            Model::RandomValues(p) => {
                p.reset_state();
                random_values::OUTPUTS.len()
            }
            Model::StockVca(p) => {
                p.initialize_state();
                stock_vca::OUTPUTS.len()
            }
            Model::StockVcf(p) => {
                p.initialize_state();
                stock_vcf::OUTPUTS.len()
            }
            Model::StockDelay(p) => {
                p.reset_state();
                stock_delay::OUTPUTS.len()
            }
            Model::AdsrEg(p) => {
                p.reset_state();
                adsr_eg::OUTPUTS.len()
            }
            Model::Push(p) => {
                p.reset_state();
                push::OUTPUTS.len()
            }
            Model::Pulses(p) => {
                p.reset_state();
                pulses::OUTPUTS.len()
            }
            Model::Seq3(seq) => {
                seq.reset_state();
                sequence::OUTPUTS.len()
            }
            Model::Switch(switch) => {
                switch.reset_state();
                switch.output_count()
            }
            Model::Signal(s) => {
                s.reset_state();
                node.output_count
            }
            Model::Mixing(m) => {
                m.reset_state();
                node.output_count
            }
            _ => return false,
        };
        self.outputs[node.output_start..node.output_start + count].fill(PortFrame::default());
        for (edge, frame) in self.edges.iter().zip(&mut self.edge_frames) {
            if edge.source_node == index || edge.destination_node == index {
                *frame = PortFrame::default();
            }
        }
        true
    }
    pub(crate) fn initialize_module_state(&mut self, index: usize) -> bool {
        let Some(node) = self.nodes.get_mut(index) else {
            return false;
        };
        match &mut node.model {
            Model::MidiCc(m) => m.initialize_state(),
            Model::MidiGate(m) => m.initialize_state(),
            Model::ColoredNoise(s) => s.reset_state(),
            Model::MultiLfo(lfo) => lfo.initialize_state(),
            Model::Wavetable(oscillator) => oscillator.initialize_state(),
            Model::Seq3(seq) => seq.initialize_state(),
            Model::Pulses(p) => p.reset_state(),
            Model::Push(p) => p.initialize_state(),
            Model::AdsrEg(p) => p.initialize_state(),
            Model::StockVcf(p) => p.initialize_state(),
            Model::StockVca(p) => p.initialize_state(),
            Model::StockDelay(p) => p.initialize_state(),
            Model::ShiftRegister(p) => p.initialize_state(),
            Model::RandomValues(p) => p.release(),
            Model::Scope(s) => s.reset(),
            Model::ModalResonator(s) => s.reset_state(),
            Model::MacroVoice(s) => s.reset_state(),
            // Parameter initialization preserves all other adopted runtime
            // histories, including VCO phase, switch position, and process
            // hold/slew. The host separately releases momentary controls.
            _ => {}
        }
        true
    }
    pub(crate) fn set_module_running(&mut self, index: usize, running: bool) -> bool {
        let Some(node) = self.nodes.get_mut(index) else {
            return false;
        };
        match &mut node.model {
            Model::Seq3(seq) => {
                seq.set_running(running);
                true
            }
            _ => false,
        }
    }
    pub(crate) fn module_saved_state(&self, index: usize) -> Option<SavedModuleState> {
        match self.nodes.get(index).map(|node| &node.model) {
            Some(Model::Seq3(seq)) => Some(SavedModuleState::Seq3Run(seq.telemetry().running)),
            Some(Model::Push(push)) => Some(SavedModuleState::PushHold(push.holding())),
            Some(Model::ShiftRegister(p)) => Some(p.saved_state()),
            Some(Model::RandomValues(p)) => Some(p.saved_state()),
            Some(Model::MidiCc(m)) => Some(m.saved_state()),
            Some(Model::MidiGate(m)) => Some(m.saved_state()),
            _ => None,
        }
    }
    pub(crate) fn set_module_saved_state(&mut self, index: usize, state: SavedModuleState) -> bool {
        if let SavedModuleState::Seq3Run(running) = state {
            return self.set_module_running(index, running);
        }
        match (self.nodes.get_mut(index).map(|node| &mut node.model), state) {
            (Some(Model::Push(push)), SavedModuleState::PushHold(holding)) => {
                push.set_holding(holding);
                true
            }
            (Some(Model::ShiftRegister(p)), state) => p.set_saved_state(state),
            (Some(Model::RandomValues(p)), state) => p.set_saved_state(state),
            (Some(Model::MidiCc(m)), state) => m.set_saved_state(state),
            (Some(Model::MidiGate(m)), state) => m.set_saved_state(state),
            _ => false,
        }
    }
    pub(crate) fn resolve_tap(&self, id: &str, port: &str) -> Result<usize, String> {
        let index = *self
            .node_lookup
            .get(id)
            .ok_or_else(|| format!("tap node '{id}' does not exist"))?;
        let node = &self.nodes[index];
        let port = output_port(&node.kind, port)
            .ok_or_else(|| format!("tap node '{id}': unknown output '{port}'"))?;
        Ok(node.output_start + port)
    }
    pub(crate) fn tap_sample(&self, index: usize, channel: u8) -> crate::TapSample {
        self.outputs
            .get(index)
            .filter(|frame| channel < frame.channels)
            .map_or(crate::TapSample::default(), |frame| crate::TapSample {
                value: frame.values[usize::from(channel)],
                active: true,
            })
    }
    #[cfg(test)]
    pub(crate) fn compile(patch: &Patch) -> Result<Self, String> {
        let cache = crate::WavePreparationCache::default();
        let wave_banks = patch.prepare_wave_banks(&cache)?;
        Self::compile_with_wave_banks(patch, &wave_banks)
    }
    pub(crate) fn compile_with_wave_banks(
        patch: &Patch,
        wave_banks: &PreparedWaveBanks,
    ) -> Result<Self, String> {
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
                model: Model::build(node, patch.sample_rate, wave_banks)?,
                inputs: [None; MAX_PORTS],
                output_start: output_count,
                output_count: 0,
                bypassed: node.bypassed,
                bypass_inputs: [None; MAX_PORTS],
                connected_mask: 0,
                base_connected_mask: 0,
            });
            let count = if let Some(ports) = wavetable_osc::output_names(&node.kind) {
                ports.len()
            } else if node.kind == "macro_voice" {
                macro_voice::OUTPUTS.len()
            } else if node.kind == "colored_noise" {
                colored_noise::OUTPUTS.len()
            } else if node.kind == "score_sequencer" {
                score::OUTPUTS.len()
            } else if let Some(ports) = studio_fx::output_names(&node.kind)
                .or_else(|| voice_tools::output_names(&node.kind))
            {
                ports.len()
            } else if let Some(ports) = mixing::output_names(&node.kind) {
                ports.len()
            } else if let Some(ports) = signal::output_names(&node.kind) {
                ports.len()
            } else if let Some(ports) = utility_bank::output_names(&node.kind) {
                ports.len()
            } else if let Some(ports) = switch::output_names(&node.kind) {
                ports.len()
            } else if let Some(ports) = routing::output_names(&node.kind) {
                ports.len()
            } else {
                match node.kind.as_str() {
                    "poly_viz" | "notes" | "blank" => 0,
                    "poly_split" | "midi_cc" | "midi_gate" => 16,
                    "midi_cv" => MIDI_OUTPUTS.len(),
                    "multi_lfo" => multi_lfo::OUTPUTS.len(),
                    "multi_vco" => multi_vco::OUTPUTS.len(),
                    "seq3" => sequence::OUTPUTS.len(),
                    "pulses" => pulses::OUTPUTS.len(),
                    "push" => push::OUTPUTS.len(),
                    "adsr_eg" => adsr_eg::OUTPUTS.len(),
                    "stock_vcf" => stock_vcf::OUTPUTS.len(),
                    "stock_delay" => stock_delay::OUTPUTS.len(),
                    "sh_asr" => shift_register::OUTPUTS.len(),
                    "random_values" => random_values::OUTPUTS.len(),
                    "scope" => scope::OUTPUTS.len(),
                    "gates" => gates::OUTPUTS.len(),
                    _ => 1,
                }
            };
            let compiled = nodes.last_mut().unwrap();
            compiled.output_count = count;
            for (input, output) in crate::bypass::routes(&node.kind) {
                compiled.bypass_inputs[output_port(&node.kind, output).unwrap()] =
                    input_port(&node.kind, compiled.model.template(), input);
            }
            output_count += count;
        }
        let mut edges = Vec::with_capacity(patch.cables.len());
        let mut edge_lookup = BTreeMap::new();
        let mut route_counts = BTreeMap::<EdgeKey, usize>::new();
        let mut unique_legacy_routes = BTreeMap::<EdgeKey, Option<usize>>::new();
        let mut cable_lookup = BTreeMap::new();
        let mut group_lookup = BTreeMap::<(usize, usize), usize>::new();
        let mut group_edges = Vec::<Vec<usize>>::new();
        let mut legacy_occurrences = BTreeMap::<EdgeKey, usize>::new();
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
            let group = *group_lookup.entry((destination, input)).or_insert_with(|| {
                let index = group_edges.len();
                group_edges.push(Vec::new());
                index
            });
            nodes[destination].inputs[input] = Some(group);
            group_edges[group].push(index);
            let route = (
                cable.from.clone(),
                cable.output.clone(),
                cable.to.clone(),
                cable.input.clone(),
            );
            *route_counts.entry(route.clone()).or_default() += 1;
            let identity = if let Some(id) = &cable.id {
                cable_lookup.insert(id.clone(), index);
                EdgeIdentity::Explicit(id.clone())
            } else {
                unique_legacy_routes
                    .entry(route.clone())
                    .and_modify(|unique| *unique = None)
                    .or_insert(Some(index));
                let occurrence = legacy_occurrences.entry(route.clone()).or_default();
                let identity = EdgeIdentity::LegacyRoute(route.clone(), *occurrence);
                *occurrence += 1;
                identity
            };
            edges.push(Edge {
                source_port: nodes[source].output_start + port,
                source_node: source,
                destination_node: destination,
                route,
                identity: identity.clone(),
            });
            edge_lookup.insert(identity, index);
        }
        let mut input_groups = Vec::with_capacity(group_edges.len());
        let mut input_edges = Vec::with_capacity(edges.len());
        for members in group_edges {
            input_groups.push(InputGroup {
                start: input_edges.len(),
                len: members.len(),
            });
            input_edges.extend(members);
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
        for edge in &edges {
            let node = &mut nodes[edge.source_node];
            if matches!(node.model, Model::MidiCc(_) | Model::StockVcf(_)) {
                node.base_connected_mask |= 1 << (edge.source_port - node.output_start);
            }
        }
        if let Some(output) = selected {
            for node in &mut nodes {
                if matches!(node.model, Model::MidiCc(_) | Model::StockVcf(_))
                    && (node.output_start..node.output_start + node.output_count).contains(&output)
                {
                    node.base_connected_mask |= 1 << (output - node.output_start);
                }
            }
        }
        for node in &mut nodes {
            node.connected_mask = node.base_connected_mask;
        }
        let mut port_lights = crate::port_lights::Observer::default();
        for (node, compiled) in nodes.iter().enumerate() {
            for (port, group) in compiled.inputs.iter().enumerate() {
                if let Some(group) = group {
                    port_lights.add(node, crate::PortDirection::Input, port, *group);
                }
            }
        }
        for edge in &edges {
            port_lights.add(
                edge.source_node,
                crate::PortDirection::Output,
                edge.source_port - nodes[edge.source_node].output_start,
                edge.source_port,
            );
        }
        Ok(Self {
            port_lights,
            rate: patch.sample_rate,
            nodes,
            node_lookup,
            edges,
            edge_lookup,
            route_counts,
            unique_legacy_routes,
            cable_lookup,
            edge_frames: vec![PortFrame::default(); patch.cables.len()],
            input_frames: vec![PortFrame::default(); input_groups.len()],
            input_groups,
            input_edges,
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
            match &mut node.model {
                Model::Midi(midi) => midi.handle(event),
                Model::MidiCc(midi) if !node.bypassed => midi.handle_midi(event),
                Model::MidiGate(midi) if !node.bypassed => midi.handle_midi(event),
                _ => {}
            }
        }
    }
    pub(crate) fn process_sample(&mut self) -> PortFrame {
        // Cable frames are from the previous sample. Groups and membership are
        // compiled in patch cable order, so this loop only reads fixed storage.
        for (index, group) in self.input_groups.iter().enumerate() {
            let members = &self.input_edges[group.start..group.start + group.len];
            if members.len() == 1 {
                // Keep the one-cable fast path and exact finite samples, while
                // matching Rack's per-source nonfinite sanitization.
                let mut frame = self.edge_frames[members[0]];
                for sample in &mut frame.values[..usize::from(frame.channels).min(MAX_CHANNELS)] {
                    if !sample.is_finite() {
                        *sample = 0.0;
                    }
                }
                self.input_frames[index] = frame;
                continue;
            }
            let mut sum = PortFrame::default();
            for &edge in members {
                sum.channels = sum.channels.max(self.edge_frames[edge].channels);
            }
            let width = usize::from(sum.channels).min(MAX_CHANNELS);
            for &edge in members {
                let source = self.edge_frames[edge];
                for channel in 0..width {
                    let sample = match source.channels {
                        0 => continue,
                        1 => source.values[0],
                        count if channel < usize::from(count) => source.values[channel],
                        _ => continue,
                    };
                    // Rack sanitizes each source sample before f32 addition.
                    // The completed sum is intentionally not clipped/sanitized.
                    sum.values[channel] += if sample.is_finite() { sample } else { 0.0 };
                }
            }
            self.input_frames[index] = sum;
        }
        for node in &mut self.nodes {
            if node.bypassed {
                if let Model::Midi(midi) = &mut node.model {
                    let mut scratch = [PortFrame::default(); MIDI_OUTPUTS.len()];
                    midi.process(&mut scratch);
                }
                for (index, output) in self.outputs
                    [node.output_start..node.output_start + node.output_count]
                    .iter_mut()
                    .enumerate()
                {
                    *output = node
                        .bypass_inputs
                        .get(index)
                        .copied()
                        .flatten()
                        .and_then(|input| node.inputs[input])
                        .map(|edge| self.input_frames[edge])
                        .filter(|frame| node.kind == "scope" || frame.channels > 0)
                        .unwrap_or_else(|| {
                            if node.kind == "scope" {
                                PortFrame::default()
                            } else {
                                PortFrame::mono(0.0)
                            }
                        });
                }
                continue;
            }
            let a = node.inputs[0]
                .map(|edge| self.input_frames[edge])
                .unwrap_or_default();
            let b = node.inputs[1]
                .map(|edge| self.input_frames[edge])
                .unwrap_or_default();
            match &mut node.model {
                Model::Annotation => {}
                Model::Score(_)
                | Model::StudioFx(_)
                | Model::VoiceTools(_)
                | Model::MacroVoice(_)
                | Model::ModalResonator(_) => {
                    let mut inputs = [PortFrame::default(); MAX_PORTS];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    let outputs =
                        &mut self.outputs[node.output_start..node.output_start + node.output_count];
                    match &mut node.model {
                        Model::Score(s) => {
                            // A patched clock is external even before its first delayed
                            // frame arrives, or while its source has zero channels.
                            if node.inputs[0].is_some() && inputs[0].channels == 0 {
                                inputs[0] = PortFrame::mono(0.0);
                            }
                            s.process(&inputs, outputs);
                        }
                        Model::ModalResonator(s) => s.process(&inputs, outputs),
                        Model::MacroVoice(s) => s.process(&inputs, outputs),
                        Model::StudioFx(s) => s.process(&inputs, outputs),
                        Model::VoiceTools(s) => s.process(&inputs, outputs),
                        _ => unreachable!(),
                    }
                }
                Model::Mixing(mixing) => {
                    let mut inputs = [PortFrame::default(); 9];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    mixing.process(
                        &inputs,
                        &mut self.outputs[node.output_start..node.output_start + node.output_count],
                    );
                }
                Model::Gates(g) => {
                    let mut inputs = [PortFrame::default(); 3];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    g.process(
                        &inputs,
                        &mut self.outputs[node.output_start..node.output_start + node.output_count],
                    );
                }
                Model::Scope(s) => {
                    let mut inputs = [PortFrame::default(); 3];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    s.process(
                        &inputs,
                        &mut self.outputs[node.output_start..node.output_start + node.output_count],
                    );
                }
                Model::ShiftRegister(p) => {
                    let mut inputs = [PortFrame::default(); 16];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    p.process(
                        &inputs,
                        &mut self.outputs[node.output_start..node.output_start + node.output_count],
                    );
                }
                Model::RandomValues(p) => p.process(
                    a,
                    &mut self.outputs[node.output_start..node.output_start + node.output_count],
                ),
                Model::StockVca(p) => {
                    self.outputs[node.output_start] = p.process(&[a, b]);
                }
                Model::StockDelay(p) => {
                    let mut inputs = [PortFrame::default(); 6];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    self.outputs[node.output_start..node.output_start + 2]
                        .copy_from_slice(&p.process(&inputs));
                }
                Model::StockVcf(p) => {
                    let mut inputs = [PortFrame::default(); 4];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    p.process(
                        &inputs,
                        &mut self.outputs[node.output_start..node.output_start + node.output_count],
                        node.connected_mask,
                    );
                }
                Model::AdsrEg(p) => {
                    let mut inputs = [PortFrame::default(); 6];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    p.process(
                        &inputs,
                        &mut self.outputs[node.output_start..node.output_start + node.output_count],
                    );
                }
                Model::Push(p) => p.process(
                    &[a, b],
                    &mut self.outputs[node.output_start..node.output_start + node.output_count],
                ),
                Model::Pulses(p) => p.process(
                    &mut self.outputs[node.output_start..node.output_start + node.output_count],
                ),
                Model::Signal(signal) => {
                    let mut inputs = [PortFrame::default(); 3];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    signal.process(
                        &inputs,
                        &mut self.outputs[node.output_start..node.output_start + node.output_count],
                    );
                }
                Model::UtilityBank(bank) => {
                    let mut inputs = [PortFrame::default(); 10];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    bank.process(
                        &inputs,
                        &mut self.outputs[node.output_start..node.output_start + node.output_count],
                    );
                }
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
                    if count == 1 {
                        frame.values[0] = voices[0].process_sample(
                            a.voice(0, normal_a),
                            b.voice(0, normal_b),
                            a.channels > 0,
                            b.channels > 0,
                            self.rate as f64,
                        );
                    } else {
                        // Independent circuit voices share FIR traversal without
                        // changing per-voice arithmetic or history. Quartets
                        // expose four decimation chains; smaller remainders
                        // retain the existing paired and scalar paths.
                        let circuit = matches!(template, Dsp::Ladder(_) | Dsp::Vca(_));
                        let quartets = if circuit { usize::from(count) & !3 } else { 0 };
                        for (quartet_index, quartet) in voices[..quartets]
                            .as_chunks_mut::<4>()
                            .0
                            .iter_mut()
                            .enumerate()
                        {
                            let voice = quartet_index * 4;
                            let inputs =
                                std::array::from_fn(|lane| a.voice(voice + lane, normal_a));
                            let controls =
                                std::array::from_fn(|lane| b.voice(voice + lane, normal_b));
                            let [first, second, third, fourth] = quartet;
                            let values = match (first, second, third, fourth) {
                                (
                                    Dsp::Ladder(first),
                                    Dsp::Ladder(second),
                                    Dsp::Ladder(third),
                                    Dsp::Ladder(fourth),
                                ) => crate::ladder::Ladder::process_four(
                                    first, second, third, fourth, inputs, controls,
                                ),
                                (
                                    Dsp::Vca(first),
                                    Dsp::Vca(second),
                                    Dsp::Vca(third),
                                    Dsp::Vca(fourth),
                                ) => crate::analog::Vca::process_four(
                                    first, second, third, fourth, inputs, controls,
                                ),
                                _ => unreachable!("circuit voices retain their template's model"),
                            };
                            frame.values[voice..voice + 4].copy_from_slice(&values);
                        }
                        let paired = if circuit {
                            (usize::from(count) - quartets) & !1
                        } else {
                            0
                        };
                        for (pair_index, pair) in voices[quartets..quartets + paired]
                            .as_chunks_mut::<2>()
                            .0
                            .iter_mut()
                            .enumerate()
                        {
                            let voice = quartets + pair_index * 2;
                            let inputs = [a.voice(voice, normal_a), a.voice(voice + 1, normal_a)];
                            let controls = [b.voice(voice, normal_b), b.voice(voice + 1, normal_b)];
                            let [first, second] = pair;
                            let values = match (first, second) {
                                (Dsp::Ladder(first), Dsp::Ladder(second)) => {
                                    crate::ladder::Ladder::process_two(
                                        first, second, inputs, controls,
                                    )
                                }
                                (Dsp::Vca(first), Dsp::Vca(second)) => {
                                    crate::analog::Vca::process_two(first, second, inputs, controls)
                                }
                                _ => unreachable!("circuit voices retain their template's model"),
                            };
                            frame.values[voice..voice + 2].copy_from_slice(&values);
                        }
                        for (offset, dsp) in voices[quartets + paired..usize::from(count)]
                            .iter_mut()
                            .enumerate()
                        {
                            let voice = quartets + paired + offset;
                            frame.values[voice] = dsp.process_sample(
                                a.voice(voice, normal_a),
                                b.voice(voice, normal_b),
                                a.channels > 0,
                                b.channels > 0,
                                self.rate as f64,
                            );
                        }
                    }
                    self.outputs[node.output_start] = frame;
                }
                Model::Switch(switch) => {
                    let mut inputs = [PortFrame::default(); 6];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    let count = switch.output_count();
                    switch.process(
                        &inputs,
                        &mut self.outputs[node.output_start..node.output_start + count],
                    );
                }
                Model::Seq3(seq) => {
                    let mut inputs = [PortFrame::default(); 5];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    seq.process(
                        &inputs,
                        &mut self.outputs
                            [node.output_start..node.output_start + sequence::OUTPUTS.len()],
                    );
                }
                Model::ColoredNoise(s) => {
                    s.process(
                        &mut self.outputs
                            [node.output_start..node.output_start + colored_noise::OUTPUTS.len()],
                    );
                }
                Model::MultiLfo(lfo) => {
                    let mut inputs = [PortFrame::default(); 4];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    lfo.process(
                        &inputs,
                        &mut self.outputs
                            [node.output_start..node.output_start + multi_lfo::OUTPUTS.len()],
                    );
                }
                Model::MultiVco(vco) => {
                    let mut inputs = [PortFrame::default(); 4];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    vco.process(
                        &inputs,
                        &mut self.outputs
                            [node.output_start..node.output_start + multi_vco::OUTPUTS.len()],
                    );
                }
                Model::Wavetable(oscillator) => {
                    let mut inputs = [PortFrame::default(); 4];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    self.outputs[node.output_start] = oscillator.process(&inputs);
                }
                Model::Routing(routing) => {
                    let mut inputs = [PortFrame::default(); MAX_PORTS];
                    for (input, edge) in inputs.iter_mut().zip(&node.inputs) {
                        if let Some(edge) = edge {
                            *input = self.input_frames[*edge];
                        }
                    }
                    let count = routing::output_names(&node.kind).unwrap().len();
                    routing.process(
                        &inputs,
                        &mut self.outputs[node.output_start..node.output_start + count],
                    );
                }
                Model::MidiGate(midi) => midi.process(
                    &mut self.outputs[node.output_start..node.output_start + node.output_count],
                ),
                Model::MidiCc(midi) => midi.process(
                    &mut self.outputs[node.output_start..node.output_start + node.output_count],
                    node.connected_mask,
                ),
                Model::Midi(midi) => midi.process(
                    &mut self.outputs[node.output_start..node.output_start + MIDI_OUTPUTS.len()],
                ),
                Model::Stereo(output) => {
                    self.outputs[node.output_start] = output.process(a, b);
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
                            let source = self.input_frames[edge];
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
                Model::Viz(v) => v.process(a),
                Model::Sum(s) => {
                    self.outputs[node.output_start] = s.process(a);
                }
            }
        }
        // Passive extraction reads the actual processed input sums and outputs.
        // edge_frames below are next sample's inputs, not this observation.
        if self.port_lights.due() {
            for binding in &mut self.port_lights.bindings {
                let frame = match binding.direction {
                    crate::PortDirection::Input => &self.input_frames[binding.source],
                    crate::PortDirection::Output => &self.outputs[binding.source],
                };
                binding.state = Some(crate::port_lights::step_port_light(
                    binding.state.unwrap_or_default(),
                    true,
                    frame.channels,
                    &frame.values,
                    self.rate,
                ));
            }
        }
        self.port_lights.advance(1);
        // Publish counts and values only after every node has completed. All
        // cycles, including self-loops, therefore have a defined sample delay.
        for (edge, frame) in self.edges.iter().zip(&mut self.edge_frames) {
            *frame = self.outputs[edge.source_port];
        }
        self.selected
            .map(|port| self.outputs[port])
            .unwrap_or_default()
    }
    fn input_unchanged(&self, node: usize, old: &Self, old_node: usize, input: usize) -> bool {
        let (Some(group), Some(old_group)) = (
            self.nodes[node].inputs[input],
            old.nodes[old_node].inputs[input],
        ) else {
            return self.nodes[node].inputs[input].is_none()
                && old.nodes[old_node].inputs[input].is_none();
        };
        let group = &self.input_groups[group];
        let old_group = &old.input_groups[old_group];
        if group.len != old_group.len {
            return false;
        }
        let current = &self.input_edges[group.start..group.start + group.len];
        let previous = &old.input_edges[old_group.start..old_group.start + old_group.len];
        current.iter().zip(previous).all(|(&index, &old_index)| {
            let edge = &self.edges[index];
            let old_edge = &old.edges[old_index];
            let source = &self.nodes[edge.source_node];
            let old_source = &old.nodes[old_edge.source_node];
            let same_identity = edge.identity == old_edge.identity
                || (matches!(edge.identity, EdgeIdentity::Explicit(_))
                    && matches!(old_edge.identity, EdgeIdentity::LegacyRoute(_, _))
                    && self.route_counts.get(&edge.route) == Some(&1)
                    && old.route_counts.get(&edge.route) == Some(&1)
                    && old.unique_legacy_routes.get(&edge.route) == Some(&Some(old_index)));
            edge.route == old_edge.route
                && source.kind == old_source.kind
                && edge.source_port - source.output_start
                    == old_edge.source_port - old_source.output_start
                && same_identity
        })
    }
    // Port histories are stricter than DSP histories: changes to either a
    // physical input route or an output's incident connections start dark.
    fn port_light_route_unchanged(
        &self,
        binding: &crate::port_lights::Binding,
        old: &Self,
        old_node: usize,
        excluded: &[bool; MAX_NODES],
        excluded_cables: &[bool; MAX_CABLES],
    ) -> bool {
        let incident = |edge: &Edge, node: usize, start: usize| match binding.direction {
            crate::PortDirection::Output => {
                edge.source_node == node && edge.source_port == start + binding.port
            }
            crate::PortDirection::Input => edge.destination_node == node,
        };
        if binding.direction == crate::PortDirection::Input
            && !self.input_unchanged(binding.node, old, old_node, binding.port)
        {
            return false;
        }
        // For inputs, restrict membership to the actual group, not all inputs.
        let contains = |engine: &Self, node: usize, index: usize, edge: &Edge| {
            if !incident(edge, node, engine.nodes[node].output_start) {
                return false;
            }
            if binding.direction == crate::PortDirection::Output {
                return true;
            }
            let Some(group) = engine.nodes[node].inputs[binding.port] else {
                return false;
            };
            let group = &engine.input_groups[group];
            engine.input_edges[group.start..group.start + group.len].contains(&index)
        };
        let count = self
            .edges
            .iter()
            .enumerate()
            .filter(|(i, e)| contains(self, binding.node, *i, e))
            .count();
        let old_count = old
            .edges
            .iter()
            .enumerate()
            .filter(|(i, e)| contains(old, old_node, *i, e))
            .count();
        if count != old_count {
            return false;
        }
        self.edges.iter().enumerate().all(|(index, edge)| {
            if !contains(self, binding.node, index, edge) {
                return true;
            }
            if excluded_cables[index]
                || excluded[edge.source_node]
                || excluded[edge.destination_node]
            {
                return false;
            }
            let old_index = old.edge_lookup.get(&edge.identity).copied().or_else(|| {
                if matches!(edge.identity, EdgeIdentity::Explicit(_))
                    && self.route_counts.get(&edge.route) == Some(&1)
                    && old.route_counts.get(&edge.route) == Some(&1)
                {
                    old.unique_legacy_routes.get(&edge.route).copied().flatten()
                } else {
                    None
                }
            });
            let Some(old_index) = old_index else {
                return false;
            };
            let old_edge = &old.edges[old_index];
            edge.route == old_edge.route
                && self.nodes[edge.source_node].kind == old.nodes[old_edge.source_node].kind
                && self.nodes[edge.destination_node].kind
                    == old.nodes[old_edge.destination_node].kind
        })
    }
    fn adopt_port_lights(
        &mut self,
        old: &Self,
        excluded: &[bool; MAX_NODES],
        excluded_cables: &[bool; MAX_CABLES],
    ) {
        self.port_lights.adopt_clock(&old.port_lights);
        if !self.port_lights.can_adopt(&old.port_lights) {
            return;
        }
        for (id, &node) in &self.node_lookup {
            let Some(&old_node) = old.node_lookup.get(id) else {
                continue;
            };
            if excluded[node] || self.nodes[node].kind != old.nodes[old_node].kind {
                continue;
            }
            for index in 0..self.port_lights.bindings.len() {
                let binding = &self.port_lights.bindings[index];
                if binding.node != node {
                    continue;
                }
                let Some(old_index) =
                    old.port_lights
                        .find(old_node, binding.direction, binding.port)
                else {
                    continue;
                };
                if self.port_light_route_unchanged(
                    binding,
                    old,
                    old_node,
                    excluded,
                    excluded_cables,
                ) {
                    self.port_lights.bindings[index].state =
                        old.port_lights.bindings[old_index].state;
                }
            }
        }
    }
    #[cfg(test)]
    pub(crate) fn adopt_state_from(&mut self, old: &Self) {
        self.adopt_state_from_excluding(old, &[false; MAX_NODES], &[false; MAX_CABLES]);
    }
    pub(crate) fn adopt_state_from_excluding(
        &mut self,
        old: &Self,
        excluded: &[bool; MAX_NODES],
        excluded_cables: &[bool; MAX_CABLES],
    ) {
        self.adopt_state_impl(old, excluded, excluded_cables, true);
    }
    /// Transfer large Delay allocations instead of copying them. Once this
    /// returns true, the retiring engine must not process any more samples.
    pub(crate) fn handoff_state_from_excluding(
        &mut self,
        old: &mut Self,
        excluded: &[bool; MAX_NODES],
        excluded_cables: &[bool; MAX_CABLES],
    ) -> bool {
        self.adopt_state_impl(old, excluded, excluded_cables, false);
        if self.rate != old.rate {
            return false;
        }
        let mut moved = false;
        for (id, &index) in &self.node_lookup {
            if excluded[index] {
                continue;
            }
            let Some(&old_index) = old.node_lookup.get(id) else {
                continue;
            };
            let node = &mut self.nodes[index];
            let previous = &mut old.nodes[old_index];
            if node.kind != previous.kind {
                continue;
            }
            if let (Model::StockDelay(current), Model::StockDelay(previous)) =
                (&mut node.model, &mut previous.model)
            {
                moved |= current.adopt(previous);
            }
        }
        moved
    }
    fn adopt_state_impl(
        &mut self,
        old: &Self,
        excluded: &[bool; MAX_NODES],
        excluded_cables: &[bool; MAX_CABLES],
        copy_delay: bool,
    ) {
        if self.rate != old.rate {
            self.port_lights.reset_epoch();
            return;
        }
        self.adopt_port_lights(old, excluded, excluded_cables);
        for (id, index) in &self.node_lookup {
            if excluded[*index] {
                continue;
            }
            let Some(old_index) = old.node_lookup.get(id) else {
                continue;
            };
            let (reset_unchanged, clock_unchanged) = if self.nodes[*index].kind == "multi_lfo" {
                (
                    self.input_unchanged(*index, old, *old_index, 2),
                    self.input_unchanged(*index, old, *old_index, 3),
                )
            } else {
                (true, true)
            };
            let sync_unchanged = if self.nodes[*index].kind == "multi_vco" {
                self.input_unchanged(*index, old, *old_index, 2)
            } else {
                true
            };
            let seq_unchanged = if self.nodes[*index].kind == "seq3" {
                [
                    self.input_unchanged(*index, old, *old_index, 1),
                    self.input_unchanged(*index, old, *old_index, 2),
                    self.input_unchanged(*index, old, *old_index, 3),
                ]
            } else {
                [true; 3]
            };
            let switch_unchanged = if switch::input_names(&self.nodes[*index].kind).is_some() {
                [
                    self.input_unchanged(*index, old, *old_index, 0),
                    self.input_unchanged(*index, old, *old_index, 1),
                ]
            } else {
                [true; 2]
            };
            let score_inputs_changed = if self.nodes[*index].kind == "score_sequencer" {
                [
                    !self.input_unchanged(*index, old, *old_index, 0),
                    !self.input_unchanged(*index, old, *old_index, 1),
                ]
            } else {
                [false; 2]
            };
            let scope_routes_unchanged = if self.nodes[*index].kind == "scope" {
                (0..3).all(|port| self.input_unchanged(*index, old, *old_index, port))
            } else {
                true
            };
            let strike_unchanged = self.nodes[*index].kind != "modal_resonator"
                || self.input_unchanged(*index, old, *old_index, 1);
            let macro_trigger_unchanged = self.nodes[*index].kind != "macro_voice"
                || self.input_unchanged(*index, old, *old_index, 1);
            let node = &mut self.nodes[*index];
            let previous = &old.nodes[*old_index];
            if node.kind != previous.kind {
                continue;
            }
            let bypass_changed = node.bypassed != previous.bypassed;
            match (&mut node.model, &previous.model) {
                (Model::ColoredNoise(s), Model::ColoredNoise(old)) => s.adopt(old),
                (Model::Stereo(s), Model::Stereo(old)) => s.adopt(old),
                (Model::Sum(s), Model::Sum(old)) => s.adopt(old),
                (Model::Viz(v), Model::Viz(old)) => v.adopt(old),
                (Model::Scope(s), Model::Scope(old)) => s.adopt(old, scope_routes_unchanged),
                (Model::Score(s), Model::Score(old)) => {
                    s.adopt(old);
                    s.reset_input_detectors(score_inputs_changed[0], score_inputs_changed[1]);
                }
                (Model::ModalResonator(s), Model::ModalResonator(old)) => {
                    s.adopt(old, strike_unchanged)
                }
                (Model::MacroVoice(s), Model::MacroVoice(old)) => {
                    s.adopt(old, macro_trigger_unchanged)
                }
                (Model::StudioFx(s), Model::StudioFx(old)) => s.adopt(old),
                (Model::VoiceTools(s), Model::VoiceTools(old)) => s.adopt(old),
                (Model::Wavetable(oscillator), Model::Wavetable(old)) => {
                    // Keep the receiving parameters and bank ownership intact;
                    // only bounded oscillator histories move on the callback.
                    oscillator.adopt_from(old);
                }
                _ => {}
            }
            if let (Model::Mixing(mixing), Model::Mixing(previous)) =
                (&mut node.model, &previous.model)
            {
                mixing.adopt(previous);
            }
            if let (Model::Signal(signal), Model::Signal(previous)) =
                (&mut node.model, &previous.model)
            {
                signal.adopt(previous);
                if bypass_changed {
                    signal.release();
                }
            }
            if let (Model::Pulses(p), Model::Pulses(previous)) = (&mut node.model, &previous.model)
            {
                p.adopt(previous);
                if bypass_changed {
                    p.release();
                }
            }
            if let (Model::ShiftRegister(p), Model::ShiftRegister(previous)) =
                (&mut node.model, &previous.model)
            {
                p.adopt(previous);
                if bypass_changed {
                    p.release();
                }
            }
            if let (Model::RandomValues(p), Model::RandomValues(previous)) =
                (&mut node.model, &previous.model)
            {
                p.adopt(previous);
                if bypass_changed {
                    p.release();
                }
            }
            if let (Model::StockVca(p), Model::StockVca(previous)) =
                (&mut node.model, &previous.model)
            {
                p.adopt(previous);
            }
            if let (Model::StockVcf(p), Model::StockVcf(previous)) =
                (&mut node.model, &previous.model)
            {
                p.adopt(previous);
            }
            if copy_delay {
                if let (Model::StockDelay(p), Model::StockDelay(previous)) =
                    (&mut node.model, &previous.model)
                {
                    p.copy_state_from(previous);
                }
            }
            if let (Model::AdsrEg(p), Model::AdsrEg(previous)) = (&mut node.model, &previous.model)
            {
                p.adopt(previous);
                if bypass_changed {
                    p.release();
                }
            }
            if let (Model::Push(p), Model::Push(previous)) = (&mut node.model, &previous.model) {
                p.adopt(previous);
                if bypass_changed {
                    p.release();
                }
            }
            if let (Model::Gates(g), Model::Gates(previous)) = (&mut node.model, &previous.model) {
                g.adopt(previous);
                if bypass_changed {
                    g.release();
                }
            }
            if let (Model::Switch(switch), Model::Switch(previous)) =
                (&mut node.model, &previous.model)
            {
                switch.adopt(previous, switch_unchanged);
            }
            if let (Model::Seq3(seq), Model::Seq3(previous)) = (&mut node.model, &previous.model) {
                seq.adopt(previous, seq_unchanged);
                if bypass_changed {
                    seq.discard_actions();
                }
            }
            if bypass_changed {
                self.outputs[node.output_start..node.output_start + node.output_count]
                    .fill(PortFrame::mono(0.0));
            }
            if let (Model::MultiLfo(lfo), Model::MultiLfo(previous)) =
                (&mut node.model, &previous.model)
            {
                lfo.adopt(previous, reset_unchanged, clock_unchanged);
            }
            if let (Model::MultiVco(vco), Model::MultiVco(previous)) =
                (&mut node.model, &previous.model)
            {
                vco.adopt(previous, sync_unchanged);
            }
            if let (Model::MidiGate(midi), Model::MidiGate(previous)) =
                (&mut node.model, &previous.model)
            {
                midi.adopt(previous);
                if bypass_changed {
                    midi.host_panic();
                }
            }
            if let (Model::MidiCc(midi), Model::MidiCc(previous)) =
                (&mut node.model, &previous.model)
            {
                midi.adopt(previous);
                if bypass_changed {
                    midi.panic();
                }
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
        for (index, edge) in self.edges.iter().enumerate() {
            if excluded_cables[index] {
                continue;
            }
            let previous_index = old.edge_lookup.get(&edge.identity).copied().or_else(|| {
                // A direct caller may add persistent IDs to an old idless
                // patch. Promote history only for an unambiguous unique route.
                if !matches!(edge.identity, EdgeIdentity::Explicit(_))
                    || self.route_counts.get(&edge.route) != Some(&1)
                    || old.route_counts.get(&edge.route) != Some(&1)
                {
                    return None;
                }
                old.unique_legacy_routes.get(&edge.route).copied().flatten()
            });
            let Some(old_index) = previous_index else {
                continue;
            };
            let previous = &old.edges[old_index];
            if edge.route != previous.route {
                continue;
            }
            if excluded[edge.source_node] || excluded[edge.destination_node] {
                continue;
            }
            if self.nodes[edge.source_node].bypassed != old.nodes[previous.source_node].bypassed {
                self.edge_frames[index] = PortFrame::mono(0.0);
                continue;
            }
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
                self.edge_frames[index] = old.edge_frames[old_index];
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
    #[test]
    fn input_route_comparison_respects_stack_order_and_identity() {
        let mut patch = Patch {
            nodes: vec![
                Node {
                    content: None,
                    score: None,
                    id: "a".into(),
                    kind: "constant".into(),
                    bypassed: false,
                    params: BTreeMap::new(),
                },
                Node {
                    content: None,
                    score: None,
                    id: "b".into(),
                    kind: "constant".into(),
                    bypassed: false,
                    params: BTreeMap::new(),
                },
                Node {
                    content: None,
                    score: None,
                    id: "target".into(),
                    kind: "gain".into(),
                    bypassed: false,
                    params: BTreeMap::new(),
                },
            ],
            cables: vec![
                crate::Cable {
                    input_order: None,
                    output_order: None,
                    color: None,
                    id: Some("a_route".into()),
                    from: "a".into(),
                    output: "out".into(),
                    to: "target".into(),
                    input: "signal".into(),
                },
                crate::Cable {
                    input_order: None,
                    output_order: None,
                    color: None,
                    id: Some("b_route".into()),
                    from: "b".into(),
                    output: "out".into(),
                    to: "target".into(),
                    input: "signal".into(),
                },
            ],
            ..Patch::new_v2()
        };
        let old = PolyEngine::compile(&patch).unwrap();
        patch.cables.reverse();
        let reordered = PolyEngine::compile(&patch).unwrap();
        assert!(!reordered.input_unchanged(2, &old, 2, 0));
        patch.cables.reverse();
        patch.cables[0].id = Some("fresh".into());
        let reincarnated = PolyEngine::compile(&patch).unwrap();
        assert!(!reincarnated.input_unchanged(2, &old, 2, 0));
    }
    #[test]
    fn stacked_frames_sanitize_each_source_but_leave_completed_f32_sum_unbounded() {
        let mut patch = Patch {
            nodes: vec![
                Node {
                    content: None,
                    score: None,
                    id: "source".into(),
                    kind: "constant".into(),
                    bypassed: false,
                    params: BTreeMap::new(),
                },
                Node {
                    content: None,
                    score: None,
                    id: "target".into(),
                    kind: "gain".into(),
                    bypassed: false,
                    params: BTreeMap::new(),
                },
            ],
            cables: vec![
                crate::Cable {
                    input_order: None,
                    output_order: None,
                    color: None,
                    id: Some("a".into()),
                    from: "source".into(),
                    output: "out".into(),
                    to: "target".into(),
                    input: "signal".into(),
                },
                crate::Cable {
                    input_order: None,
                    output_order: None,
                    color: None,
                    id: Some("b".into()),
                    from: "source".into(),
                    output: "out".into(),
                    to: "target".into(),
                    input: "signal".into(),
                },
            ],
            output: Some("target".into()),
            ..Patch::new_v2()
        };
        let mut engine = PolyEngine::compile(&patch).unwrap();
        engine.edge_frames[0] = PortFrame::mono(f32::NAN);
        engine.edge_frames[1] = PortFrame::mono(2.0);
        engine.process_sample();
        assert_eq!(engine.input_frames[0], PortFrame::mono(2.0));
        engine.edge_frames[0] = PortFrame::mono(f32::MAX);
        engine.edge_frames[1] = PortFrame::mono(f32::MAX);
        engine.process_sample();
        assert!(engine.input_frames[0].values[0].is_infinite());
        engine.edge_frames.fill(PortFrame::default());
        engine.process_sample();
        assert_eq!(engine.input_frames[0].channels, 0);
        patch.cables.pop();
        let mut single = PolyEngine::compile(&patch).unwrap();
        single.edge_frames[0] = PortFrame::mono(f32::NAN);
        single.process_sample();
        assert_eq!(single.input_frames[0], PortFrame::mono(0.0));
        let mut with_padding = PortFrame {
            channels: 2,
            ..PortFrame::default()
        };
        with_padding.values[0] = -0.0;
        with_padding.values[1] = f32::INFINITY;
        with_padding.values[2] = f32::NAN;
        single.edge_frames[0] = with_padding;
        single.process_sample();
        assert_eq!(
            single.input_frames[0].values[0].to_bits(),
            (-0.0_f32).to_bits()
        );
        assert_eq!(single.input_frames[0].values[1], 0.0);
        assert!(single.input_frames[0].values[2].is_nan());
    }
    fn patch(channels: f32, kind: &str) -> Patch {
        Patch {
            nodes: vec![
                Node {
                    content: None,
                    score: None,
                    bypassed: false,
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
                    content: None,
                    score: None,
                    bypassed: false,
                    id: "voice".into(),
                    kind: kind.into(),
                    params: BTreeMap::new(),
                },
            ],
            cables: vec![crate::Cable {
                input_order: None,
                output_order: None,
                color: None,
                id: None,
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
