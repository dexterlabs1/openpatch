//! Control-thread parameter actions driven by the public module descriptors.
//! Policies are explicit for each supported control; unknown controls are never
//! randomized merely because a numeric range happens to be present.
use crate::{catalog, Node};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct ModuleParameterActions {
    sample_rate: u32,
    descriptors: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Randomization {
    Disabled,
    Linear,
    Log2,
    InclusiveInteger,
    Enum,
}
impl Randomization {
    fn name(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Linear => "linear",
            Self::Log2 => "log2",
            Self::InclusiveInteger => "inclusive_integer",
            Self::Enum => "enum",
        }
    }
}

fn numbered(name: &str, prefix: &str, maximum: u8) -> bool {
    name.strip_prefix(prefix)
        .and_then(|suffix| suffix.parse::<u8>().ok())
        .is_some_and(|number| (1..=maximum).contains(&number))
}

fn seq_row(name: &str) -> bool {
    (1..=3).any(|row| numbered(name, &format!("row{row}_"), 8))
}

fn policy(kind: &str, name: &str) -> Randomization {
    use Randomization::{Disabled, Enum, InclusiveInteger, Linear, Log2};
    match (kind, name) {
        ("random_values" | "sh_asr", _) => Disabled,
        ("stock_vco", "frequency" | "pulse_width") => Linear,
        ("stock_vco", "linear_fm" | "soft_sync") => Enum,
        ("stock_vco", _) => Disabled,
        ("stock_lfo", "rate" | "pulse_width") => Linear,
        ("stock_lfo", "offset" | "invert") => Enum,
        ("stock_lfo", _) => Disabled,
        ("random", "rate" | "probability" | "spread" | "shape") => Linear,
        ("random", "unipolar") => Enum,
        ("random", _) => Disabled,
        ("macro_voice", "engine") => Enum,
        ("macro_voice", "frequency") => Log2,
        ("macro_voice", _) => Linear,
        ("wavetable_vco" | "wavetable_lfo", "tune" | "rate" | "position") => Linear,
        ("wavetable_vco", "sync_mode" | "fm_mode") | ("wavetable_lfo", "offset" | "invert") => Enum,
        ("wavetable_vco" | "wavetable_lfo", _) => Disabled,
        ("stock_vca", "level") => Linear,
        ("stock_vca", "response") => Enum,
        ("stock_delay", "time" | "feedback" | "tone" | "mix") => Linear,
        ("stock_delay", _) => Disabled,
        ("adsr_eg", "attack" | "decay" | "sustain" | "release") => Linear,
        ("adsr_eg", _) => Disabled,
        ("seq3", name) if seq_row(name) => Linear,
        ("seq3", name) if numbered(name, "gate", 8) => Enum,
        ("seq3", _) => Disabled,
        ("multi_vco" | "multi_lfo", "frequency") | ("gates", "length") => Log2,
        ("multi_vco" | "multi_lfo", "pulse_width") => Linear,
        ("multi_vco" | "multi_lfo", "fm_amount" | "pwm_amount") => Disabled,
        ("multi_vco", "linear_fm" | "soft_sync") | ("multi_lfo", "unipolar" | "invert") => Enum,
        ("modal_resonator", "frequency" | "decay") => Log2,
        (
            "modal_resonator",
            "stiffness" | "damping" | "strike_position" | "pickup_position" | "strike_strength"
            | "level",
        ) => Linear,
        ("lowpass", "cutoff")
        | ("analog_vco", "frequency")
        | ("ladder_vcf", "cutoff")
        | ("adsr", "attack" | "decay" | "release")
        | ("gate", "frequency") => Log2,
        ("oscillator" | "lfo", "frequency" | "amplitude")
        | ("lfo", "offset")
        | ("constant", "value")
        | ("gain", "gain")
        | ("analog_vco", "pulse_width")
        | ("ladder_vcf", "resonance" | "drive")
        | ("analog_vca", "gain")
        | ("adsr", "sustain")
        | ("gate", "duty")
        | ("quantizer", "offset")
        | ("sample_hold", "initial")
        | ("white_noise" | "colored_noise", "level")
        | ("attenuverter", "gain" | "offset")
        | ("crossfade", "mix")
        | ("audio_stereo", "gain")
        | ("mid_side", "encode_width" | "decode_width")
        | ("mix6", "level")
        | ("compare", "b_offset")
        | ("process", "slew")
        | ("rescale", "gain" | "offset" | "min" | "max")
        | ("vca_mix", "mix_level")
        | ("fade", "position" | "cv_amount") => Linear,
        ("analog_vco", "waveform") | ("crossfade", "law") => Enum,
        ("quantizer", "scale_mask")
        | ("poly_constant", "channels")
        | ("octave", "octaves")
        | ("switch_1to4" | "switch_4to1", "steps") => InclusiveInteger,
        ("mix6", "invert" | "average")
        | ("vca_mix", "channel_exp" | "mix_exp")
        | ("switch_1to4" | "switch_4to1", "declick")
        | ("fade", "law")
        | ("rescale", "multiplier" | "reflect_min" | "reflect_max")
        | ("poly_merge", "channels") => Disabled,
        ("white_noise" | "colored_noise", "seed") | ("midi_cv" | "midi_cc" | "midi_gate", _) => {
            Disabled
        }
        ("poly_constant", name) if numbered(name, "v", 16) => Linear,
        ("attenuverter8", name) if numbered(name, "gain", 8) => Linear,
        ("mutes10", name) if numbered(name, "mute", 10) => Enum,
        ("cv_mix", name) if numbered(name, "level", 3) => Linear,
        ("vca_mix", name) if numbered(name, "level", 4) => Linear,
        _ => Disabled,
    }
}

pub(crate) fn annotate_catalog(module: &mut Value) {
    let kind = module["kind"].as_str().unwrap().to_owned();
    for (name, descriptor) in module["params"].as_object_mut().unwrap() {
        descriptor["initialize"] = json!(
            !((matches!(kind.as_str(), "random_values" | "sh_asr")
                && descriptor["hidden"] == true)
                || (kind == "seq3" && name == "running")
                || (kind == "mix6" && (name == "invert" || name == "average")))
        );
        descriptor["randomization"] = json!(policy(&kind, name).name());
    }
}

struct Random(u64);
impl Random {
    fn new(seed: u32) -> Self {
        Self(u64::from(seed))
    }
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        (value ^ (value >> 31)) as u32
    }
    fn unit(&mut self) -> f64 {
        f64::from(self.next_u32()) / f64::from(u32::MAX)
    }
    fn index(&mut self, count: u32) -> u32 {
        let threshold = count.wrapping_neg() % count;
        loop {
            let value = self.next_u32();
            if value >= threshold {
                return value % count;
            }
        }
    }
}

impl ModuleParameterActions {
    pub fn new(sample_rate: u32) -> Self {
        let catalog = catalog();
        let descriptors = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|module| (module["kind"].as_str().unwrap().to_owned(), module.clone()))
            .collect();
        Self {
            sample_rate,
            descriptors,
        }
    }

    fn descriptors(&self, node: &Node) -> Result<&Value, String> {
        if !(8000..=192_000).contains(&self.sample_rate) {
            return Err("sample_rate must be in 8000..=192000 Hz".into());
        }
        let descriptor = self
            .descriptors
            .get(&node.kind)
            .ok_or_else(|| format!("node '{}': unknown module kind '{}'", node.id, node.kind))?;
        let params = descriptor["params"].as_object().unwrap();
        for key in node.params.keys() {
            if !params.contains_key(key) {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        Ok(descriptor)
    }

    pub fn initialize(&self, node: &mut Node) -> Result<(), String> {
        let descriptor = self.descriptors(node)?;
        let params = descriptor["params"].as_object().unwrap();
        if crate::content::is_annotation(&node.kind)
            || matches!(node.kind.as_str(), "wavetable_vco" | "wavetable_lfo")
        {
            node.content = None;
        }
        for (name, spec) in params {
            if spec["initialize"].as_bool() == Some(false) {
                continue;
            }
            let value = spec["default"].as_f64().unwrap() as f32;
            node.params.insert(name.clone(), value);
        }
        Ok(())
    }

    pub fn randomize(&self, node: &mut Node, seed: u32) -> Result<(), String> {
        if crate::content::is_annotation(&node.kind) {
            return Err(format!(
                "node '{}': Randomize is unsupported for {}",
                node.id, node.kind
            ));
        }
        let descriptor = self.descriptors(node)?;
        let params = descriptor["params"].as_object().unwrap();
        let mut names = params.keys().collect::<Vec<_>>();
        names.sort_unstable();
        let mut random = Random::new(seed);
        for name in names {
            let spec = &params[name];
            let policy = spec["randomization"].as_str().unwrap();
            if policy == "disabled" {
                continue;
            }
            let min = spec["min"].as_f64().unwrap();
            let declared_max = spec["max"].as_f64().unwrap();
            let max = spec["sample_rate_max_ratio"]
                .as_f64()
                .map_or(declared_max, |ratio| {
                    declared_max.min(f64::from(self.sample_rate) * ratio)
                });
            if min > max {
                return Err(format!("node '{}': no valid range for '{name}'", node.id));
            }
            let value = match policy {
                "linear" => min + random.unit() * (max - min),
                "log2" => {
                    if min <= 0.0 {
                        return Err(format!(
                            "node '{}': log2 range for '{name}' must be positive",
                            node.id
                        ));
                    }
                    (min.log2() + random.unit() * (max.log2() - min.log2())).exp2()
                }
                "inclusive_integer" | "enum" => {
                    if let Some(values) = spec["values"].as_array() {
                        let eligible = values
                            .iter()
                            .filter_map(Value::as_f64)
                            .filter(|value| *value >= min && *value <= max)
                            .collect::<Vec<_>>();
                        if eligible.is_empty() {
                            return Err(format!("node '{}': empty enum for '{name}'", node.id));
                        }
                        eligible[random.index(eligible.len() as u32) as usize]
                    } else {
                        let low = min.ceil() as i32;
                        let high = max.floor() as i32;
                        if low > high {
                            return Err(format!(
                                "node '{}': empty integer range for '{name}'",
                                node.id
                            ));
                        }
                        f64::from(low + random.index((high - low + 1) as u32) as i32)
                    }
                }
                _ => return Err(format!("node '{}': unknown randomization policy", node.id)),
            };
            node.params.insert(name.clone(), value as f32);
        }
        Ok(())
    }
}
