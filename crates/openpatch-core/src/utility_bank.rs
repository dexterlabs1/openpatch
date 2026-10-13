//! Original stateless row utilities. Normaling uses input frames, never outputs.
use crate::{parameter, Node, PortFrame, SIGNAL_LIMIT};
use serde_json::{json, Value};

const INPUTS: [&str; 10] = [
    "in1", "in2", "in3", "in4", "in5", "in6", "in7", "in8", "in9", "in10",
];
const OUTPUTS: [&str; 10] = [
    "out1", "out2", "out3", "out4", "out5", "out6", "out7", "out8", "out9", "out10",
];
pub(crate) fn input_names(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "attenuverter8" => Some(&INPUTS[..8]),
        "mutes10" => Some(&INPUTS),
        "cv_mix" => Some(&INPUTS[..3]),
        _ => None,
    }
}
pub(crate) fn output_names(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "attenuverter8" => Some(&OUTPUTS[..8]),
        "mutes10" => Some(&OUTPUTS),
        "cv_mix" => Some(&["out"]),
        _ => None,
    }
}
pub(crate) fn catalog() -> [Value; 3] {
    ["attenuverter8","mutes10","cv_mix"].map(|kind|{
        let (title,prefix,count,min)=match kind {
            "attenuverter8"=>("8VERT","gain",8,-1.0),
            "mutes10"=>("MUTES","mute",10,0.0),
            _=>("CV MIX","level",3,-1.0),
        };
        let params:serde_json::Map<String,Value>=(1..=count).map(|i|{
            let mut p=json!({"default":0.0,"min":min,"max":1.0});
            if kind=="mutes10" {p["step"]=json!(1.0);} else {p["unit"]=json!("x");}
            (format!("{prefix}{i}"),p)
        }).collect();
        json!({"kind":kind,"title":title,"model":"functional","inputs":input_names(kind).unwrap(),"params":params,"description":match kind {
            "attenuverter8"=>"Eight independent gains. Top input normal is mono 10 V; unpatched rows inherit the nearest patched input above with its complete channel count. Normaling uses input, not attenuated output.",
            "mutes10"=>"Ten independent mutes; zero passes, one silences. Top normal is mono zero. Unpatched rows inherit upstream input channel one as MONO, matching the pinned source's per-row channel reset. Muting never interrupts input normaling.",
            _=>"Three attenuverted CV inputs summed into one output. Each independently normals to 10 V. Maximum input channel count, minimum one; mono broadcasts and missing short-poly lanes are zero. No averaging or musical voltage clipping."
        }})
    })
}
pub(crate) fn annotate(module: &mut Value) {
    let kind = module["kind"].as_str().unwrap().to_owned();
    if input_names(&kind).is_none() {
        return;
    }
    module["polyphony"]["version_2"]=json!(match kind.as_str() {
        "attenuverter8"=>"each row inherits complete nearest nonzero-channel input above; top is mono 10 V; no maximum across independent rows",
        "mutes10"=>"patched row uses its input channel count; unpatched row is mono with nearest upstream input channel one's voltage; top mono zero; muted rows keep their count",
        _=>"maximum count of all three inputs, minimum one; mono broadcasts; short-poly missing lanes are zero; independent 10 V normal broadcasts"
    });
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["auxiliary_inputs"] = json!("row utilities use the documented input-normaling chain; CV Mix mono broadcasts, zero-channel inputs normal to 10 V, and missing short-poly lanes are zero");
    module["polyphony"]["retired_voices"] = json!("stateless; inactive output slots are zero");
    for (i, meta) in module["input_metadata"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        if kind == "cv_mix" || (kind == "attenuverter8" && i == 0) {
            meta["normalization"] = json!(10.0);
        } else if i > 0 {
            meta["normalization_source"] = json!(format!("in{i}"));
        }
    }
    module["arithmetic_policy"]=json!("nonfinite input voltages become zero; use f64 arithmetic and clamp each final output to ±1,000,000 V; CV mix sums before clamp");
    module["functional_reference"]=json!(format!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/{}.cpp",match kind.as_str() {"attenuverter8"=>"8vert","mutes10"=>"Mutes",_=>"CVMix"}));
    module["compatibility"]=json!("original Rust functional implementation, not sample-identical Rack parity; explicit f64/finite/bounded arithmetic; all named outputs compute even without outgoing cables; no Rack legacy JSON migration");
    if kind == "mutes10" {
        module["normalization_policy"]=json!("pinned Mutes.cpp initializes channel count to one separately in every row; unpatched rows retain upstream first voltage but not upstream polyphony, independently of mute state");
    }
    if kind == "attenuverter8" {
        module["gain_display_policy"]=json!("gain remains -1..1; when this and all preceding inputs are disconnected the equivalent constant-voltage display is gain×10 V, otherwise gain×100 percent; presentation does not affect DSP");
    }
}
fn finite(value: f32) -> f64 {
    if value.is_finite() {
        f64::from(value)
    } else {
        0.0
    }
}
fn bounded(value: f64) -> f32 {
    value.clamp(-f64::from(SIGNAL_LIMIT), f64::from(SIGNAL_LIMIT)) as f32
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum UtilityBank {
    Attenuverters([f32; 8]),
    Mutes([bool; 10]),
    CvMix([f32; 3]),
}
impl UtilityBank {
    pub(crate) fn build(node: &Node) -> Result<Self, String> {
        let (prefix, count, min) = match node.kind.as_str() {
            "attenuverter8" => ("gain", 8, -1.0),
            "mutes10" => ("mute", 10, 0.0),
            _ => ("level", 3, -1.0),
        };
        for key in node.params.keys() {
            if !(1..=count).any(|i| *key == format!("{prefix}{i}")) {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let mut values = [0.0; 10];
        for (i, value) in values[..count].iter_mut().enumerate() {
            *value = parameter(node, &format!("{prefix}{}", i + 1), 0.0, min, 1.0)?;
            if node.kind == "mutes10" && value.fract() != 0.0 {
                return Err(format!(
                    "node '{}': mute parameters must be boolean integers",
                    node.id
                ));
            }
        }
        Ok(match node.kind.as_str() {
            "attenuverter8" => Self::Attenuverters(std::array::from_fn(|i| values[i])),
            "mutes10" => Self::Mutes(values.map(|v| v != 0.0)),
            _ => Self::CvMix(std::array::from_fn(|i| values[i])),
        })
    }
    pub(crate) fn process(&self, inputs: &[PortFrame; 10], outputs: &mut [PortFrame]) {
        match self {
            Self::Attenuverters(gains) => {
                let mut source = PortFrame::mono(10.0);
                for ((input, output), gain) in inputs.iter().zip(outputs).zip(gains) {
                    if input.channels > 0 {
                        source = *input;
                    }
                    *output = PortFrame {
                        channels: source.channels.min(16),
                        ..PortFrame::default()
                    };
                    for lane in 0..usize::from(output.channels) {
                        output.values[lane] =
                            bounded(finite(source.channel(lane)) * f64::from(*gain));
                    }
                }
            }
            Self::Mutes(mutes) => {
                let mut source = PortFrame::mono(0.0);
                for ((input, output), mute) in inputs.iter().zip(outputs).zip(mutes) {
                    let count = if input.channels > 0 {
                        source = *input;
                        input.channels.min(16)
                    } else {
                        1
                    };
                    *output = PortFrame {
                        channels: count,
                        ..PortFrame::default()
                    };
                    if !mute {
                        for lane in 0..usize::from(count) {
                            output.values[lane] = bounded(finite(source.channel(lane)));
                        }
                    }
                }
            }
            Self::CvMix(levels) => {
                let count = inputs[..3]
                    .iter()
                    .map(|f| f.channels.min(16))
                    .max()
                    .unwrap_or(0)
                    .max(1);
                let output = &mut outputs[0];
                *output = PortFrame {
                    channels: count,
                    ..PortFrame::default()
                };
                for lane in 0..usize::from(count) {
                    output.values[lane] = bounded(
                        inputs
                            .iter()
                            .zip(levels)
                            .map(|(input, level)| {
                                finite(input.voice(lane, 10.0)) * f64::from(*level)
                            })
                            .sum(),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finite_safety_is_applied_after_cv_sum_and_inherited_input_keeps_full_precision() {
        let mut inputs = [PortFrame::default(); 10];
        inputs[0] = PortFrame::mono(f32::MAX);
        inputs[1] = PortFrame::mono(-f32::MAX);
        inputs[2] = PortFrame::mono(f32::NAN);
        let mut out = [PortFrame::default()];
        UtilityBank::CvMix([1.0; 3]).process(&inputs, &mut out);
        assert_eq!(out[0], PortFrame::mono(0.0));
        UtilityBank::CvMix([1.0, 0.0, 0.0]).process(&inputs, &mut out);
        assert_eq!(out[0].values[0], SIGNAL_LIMIT);
        let mut out = [PortFrame::default(); 8];
        let mut gains = [0.0; 8];
        gains[0] = 1.0;
        gains[1] = f32::MIN_POSITIVE;
        inputs[1] = PortFrame::default();
        UtilityBank::Attenuverters(gains).process(&inputs, &mut out);
        assert_eq!(out[0].values[0], SIGNAL_LIMIT);
        assert!((out[1].values[0] - 4.0).abs() < 1e-6);
        inputs[0] = PortFrame::mono(f32::INFINITY);
        UtilityBank::Attenuverters([1.0; 8]).process(&inputs, &mut out);
        assert!(out.iter().all(|f| f.values == [0.0; 16]));
        let mut mute_out = [PortFrame::default(); 10];
        UtilityBank::Mutes([false; 10]).process(&inputs, &mut mute_out);
        assert!(mute_out.iter().all(|f| f.values == [0.0; 16]));
    }
}
