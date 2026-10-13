//! Original stateless voltage routing. The public Free manual informs the
//! functional roles; CV ranges, rounding and polyphony are explicit OpenPatch
//! contracts, not claims of sample-identical behavior to another implementation.
use crate::{parameter, Node, PortFrame, SIGNAL_LIMIT};
use serde_json::{json, Value};

pub(crate) fn input_names(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "mid_side" => &["left", "right", "mid", "side", "encode_cv", "decode_cv"],
        "mix6" => &["in1", "in2", "in3", "in4", "in5", "in6"],
        "octave" => &["pitch", "shift_cv"],
        "mult" => &["signal"],
        _ => return None,
    })
}
pub(crate) fn output_names(kind: &str) -> Option<&'static [&'static str]> {
    Some(match kind {
        "mid_side" => &["mid", "side", "left", "right"],
        "mix6" | "octave" => &["out"],
        "mult" => &[
            "out1", "out2", "out3", "out4", "out5", "out6", "out7", "out8",
        ],
        _ => return None,
    })
}
pub(crate) fn catalog() -> [Value; 4] {
    [
        json!({"kind":"mid_side","title":"MID/SIDE","model":"functional","description":"Mid/side encoder and decoder: M=(L+R)/2; S=(L-R)/2×encode width; L=M+S×decode width; R=M-S×decode width. Decoder inputs independently normalize to the complete encoder output frames. Effective width clamps knob×CV/10 to 0–2; CV normal 10 V. CV never creates voices. Arithmetic safety ±1,000,000 V; nonfinite inputs become zero.","inputs":input_names("mid_side").unwrap(),"params":{"encode_width":{"default":1.0,"min":0.0,"max":2.0},"decode_width":{"default":1.0,"min":0.0,"max":2.0}}}),
        json!({"kind":"mix6","title":"MIX6","model":"functional","description":"Six-input unity mixer with master level, inversion and averaging. Average divides by input jacks carrying at least one channel, including zero-valued signals; zero-channel jacks do not count. Sum, average, level and invert precede final ±1,000,000 V safety bound. Nonfinite inputs become zero.","inputs":input_names("mix6").unwrap(),"params":{"level":{"default":1.0,"min":0.0,"max":1.0},"invert":{"default":0.0,"min":0.0,"max":1.0,"step":1.0},"average":{"default":0.0,"min":0.0,"max":1.0,"step":1.0}}}),
        json!({"kind":"octave","title":"OCTAVE","model":"functional","description":"1 V/oct transposition: pitch + octaves + round(shift_cv). CV rounds to nearest integer volt, half ties away from zero. CV normal zero; only pitch determines voice count. Unpatched pitch generates the selected shift. No musical pitch clipping; arithmetic safety ±1,000,000 V. Nonfinite inputs become zero.","inputs":input_names("octave").unwrap(),"params":{"octaves":{"default":0.0,"min":-4.0,"max":4.0,"step":1.0,"unit":"octaves"}}}),
        json!({"kind":"mult","title":"MULT","model":"functional","description":"Copy the complete signal frame to eight outputs, preserving 0–16 channels and values exactly. No arithmetic or normalization. Every outgoing cable retains its ordinary one-sample delay.","inputs":input_names("mult").unwrap(),"params":{}}),
    ]
}
pub(crate) fn annotate(module: &mut Value) {
    let kind = module["kind"].as_str().unwrap();
    let policy = match kind {
        "mid_side" => "encoder=max(left,right,1); decoder=max(normalized mid,normalized side,1); width CV counts never create voices",
        "mix6" => "maximum channel count of all six signal inputs, minimum one; mono broadcasts; missing short-poly lanes are zero",
        "octave" => "pitch channel count, minimum one; shift CV mono broadcasts; missing short-poly lanes are zero",
        "mult" => "exact signal frame copies, including zero channels",
        _ => return,
    };
    let kind = kind.to_owned();
    module["polyphony"]["version_2"] = json!(policy);
    module["polyphony"]["primary_input"] = if kind == "octave" {
        json!("pitch")
    } else if kind == "mult" {
        json!("signal")
    } else {
        Value::Null
    };
    module["polyphony"]["retired_voices"] = json!("stateless; inactive output slots are zero");
    module["functional_reference"] = json!("https://vcvrack.com/Free");
    module["compatibility"] = json!("original functional implementation; OpenPatch CV, rounding, polyphony and safety policies are explicit, not sample-identical parity");
    if kind == "mid_side" {
        module["input_metadata"][2]["normalization_source"] = json!("encoder.mid");
        module["input_metadata"][3]["normalization_source"] = json!("encoder.side");
        for index in [4, 5] {
            module["input_metadata"][index]["normalization"] = json!(10.0);
        }
        module["width_cv"] = json!({"formula":"clamp(knob * CV / 10, 0, 2)","normal_voltage":10.0,"creates_voices":false});
    }
}
fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}
fn arithmetic(value: f64) -> f32 {
    value.clamp(-f64::from(SIGNAL_LIMIT), f64::from(SIGNAL_LIMIT)) as f32
}
fn voice(frame: &PortFrame, index: usize, normal: f32) -> f64 {
    f64::from(finite(frame.voice(index, normal)))
}
fn width(knob: f32, cv: &PortFrame, index: usize) -> f64 {
    (f64::from(knob) * voice(cv, index, 10.0) / 10.0).clamp(0.0, 2.0)
}
fn frame(channels: u8) -> PortFrame {
    PortFrame {
        channels,
        ..PortFrame::default()
    }
}
fn integer(node: &Node, name: &str, default: f32, min: f32, max: f32) -> Result<f32, String> {
    let value = parameter(node, name, default, min, max)?;
    if value.fract() != 0.0 {
        return Err(format!(
            "node '{}': parameter '{name}' must be an integer",
            node.id
        ));
    }
    Ok(value)
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Routing {
    MidSide {
        encode: f32,
        decode: f32,
    },
    Mix {
        level: f32,
        invert: bool,
        average: bool,
    },
    Octave(f32),
    Mult,
}
impl Routing {
    pub(crate) fn build(node: &Node) -> Option<Result<Self, String>> {
        let allowed: &[&str] = match node.kind.as_str() {
            "mid_side" => &["encode_width", "decode_width"],
            "mix6" => &["level", "invert", "average"],
            "octave" => &["octaves"],
            "mult" => &[],
            _ => return None,
        };
        Some((|| {
            for key in node.params.keys() {
                if !allowed.contains(&key.as_str()) {
                    return Err(format!("node '{}': unknown parameter '{key}'", node.id));
                }
            }
            Ok(match node.kind.as_str() {
                "mid_side" => Self::MidSide {
                    encode: parameter(node, "encode_width", 1.0, 0.0, 2.0)?,
                    decode: parameter(node, "decode_width", 1.0, 0.0, 2.0)?,
                },
                "mix6" => Self::Mix {
                    level: parameter(node, "level", 1.0, 0.0, 1.0)?,
                    invert: integer(node, "invert", 0.0, 0.0, 1.0)? == 1.0,
                    average: integer(node, "average", 0.0, 0.0, 1.0)? == 1.0,
                },
                "octave" => Self::Octave(integer(node, "octaves", 0.0, -4.0, 4.0)?),
                "mult" => Self::Mult,
                _ => unreachable!(),
            })
        })())
    }
    pub(crate) fn process(&self, inputs: &[PortFrame; 16], outputs: &mut [PortFrame]) {
        match *self {
            Self::MidSide { encode, decode } => {
                let count = inputs[0].channels.max(inputs[1].channels).max(1);
                let mut mid = frame(count);
                let mut side = frame(count);
                for i in 0..count as usize {
                    let l = voice(&inputs[0], i, 0.0);
                    let r = voice(&inputs[1], i, 0.0);
                    mid.values[i] = arithmetic((l + r) * 0.5);
                    side.values[i] = arithmetic((l - r) * 0.5 * width(encode, &inputs[4], i));
                }
                let m = if inputs[2].channels == 0 {
                    mid
                } else {
                    inputs[2]
                };
                let s = if inputs[3].channels == 0 {
                    side
                } else {
                    inputs[3]
                };
                let count = m.channels.max(s.channels).max(1);
                let mut left = frame(count);
                let mut right = frame(count);
                for i in 0..count as usize {
                    let middle = voice(&m, i, 0.0);
                    let spread = voice(&s, i, 0.0) * width(decode, &inputs[5], i);
                    left.values[i] = arithmetic(middle + spread);
                    right.values[i] = arithmetic(middle - spread);
                }
                outputs.copy_from_slice(&[mid, side, left, right]);
            }
            Self::Mix {
                level,
                invert,
                average,
            } => {
                let count = inputs[..6].iter().map(|f| f.channels).max().unwrap().max(1);
                let divisor = if average {
                    inputs[..6].iter().filter(|f| f.channels > 0).count().max(1) as f64
                } else {
                    1.0
                };
                let gain = f64::from(level) * if invert { -1.0 } else { 1.0 } / divisor;
                let mut out = frame(count);
                for i in 0..count as usize {
                    out.values[i] = arithmetic(
                        inputs[..6].iter().map(|f| voice(f, i, 0.0)).sum::<f64>() * gain,
                    );
                }
                outputs[0] = out;
            }
            Self::Octave(octaves) => {
                let count = inputs[0].channels.max(1);
                let mut out = frame(count);
                for i in 0..count as usize {
                    out.values[i] = arithmetic(
                        voice(&inputs[0], i, 0.0)
                            + f64::from(octaves)
                            + voice(&inputs[1], i, 0.0).round(),
                    );
                }
                outputs[0] = out;
            }
            Self::Mult => outputs.fill(inputs[0]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inputs() -> [PortFrame; 16] {
        [PortFrame::default(); 16]
    }
    #[test]
    fn arithmetic_sanitizes_nonfinite_inputs_without_early_mixer_saturation() {
        let mut i = inputs();
        let mut o = [PortFrame::default(); 1];
        for input in &mut i[..6] {
            *input = PortFrame::mono(SIGNAL_LIMIT);
        }
        Routing::Mix {
            level: 1.0,
            invert: false,
            average: true,
        }
        .process(&i, &mut o);
        assert_eq!(o[0], PortFrame::mono(SIGNAL_LIMIT));
        i[0] = PortFrame::mono(f32::NAN);
        i[1] = PortFrame::mono(f32::INFINITY);
        i[2] = PortFrame::mono(f32::NEG_INFINITY);
        Routing::Mix {
            level: 1.0,
            invert: true,
            average: true,
        }
        .process(&i, &mut o);
        assert_eq!(o[0], PortFrame::mono(-SIGNAL_LIMIT / 2.0));
        i[0] = PortFrame::mono(f32::MAX);
        i[1] = PortFrame::mono(f32::MAX);
        Routing::Octave(4.0).process(&i, &mut o);
        assert_eq!(o[0], PortFrame::mono(SIGNAL_LIMIT));
        i[0] = PortFrame::mono(f32::NAN);
        i[1] = PortFrame::mono(f32::INFINITY);
        Routing::Octave(-4.0).process(&i, &mut o);
        assert_eq!(o[0], PortFrame::mono(-4.0));
    }
    #[test]
    fn mid_side_normals_use_the_same_bounded_encoder_frames_as_named_outputs() {
        let mut i = inputs();
        let mut o = [PortFrame::default(); 4];
        i[0] = PortFrame::mono(SIGNAL_LIMIT);
        i[1] = PortFrame::mono(-SIGNAL_LIMIT);
        Routing::MidSide {
            encode: 2.0,
            decode: 0.5,
        }
        .process(&i, &mut o);
        assert_eq!(
            o,
            [
                PortFrame::mono(0.0),
                PortFrame::mono(SIGNAL_LIMIT),
                PortFrame::mono(SIGNAL_LIMIT / 2.0),
                PortFrame::mono(-SIGNAL_LIMIT / 2.0)
            ]
        );
        i[0] = PortFrame::mono(f32::INFINITY);
        i[1] = PortFrame::mono(4.0);
        i[4] = PortFrame::mono(f32::NAN);
        Routing::MidSide {
            encode: 1.0,
            decode: 1.0,
        }
        .process(&i, &mut o);
        assert_eq!(
            o,
            [
                PortFrame::mono(2.0),
                PortFrame::mono(0.0),
                PortFrame::mono(2.0),
                PortFrame::mono(2.0)
            ]
        );
        i[0] = PortFrame::mono(f32::MAX);
        i[1] = PortFrame::mono(f32::MAX);
        Routing::MidSide {
            encode: 1.0,
            decode: 1.0,
        }
        .process(&i, &mut o);
        assert!(o.iter().flat_map(|f| f.values).all(f32::is_finite));
    }
    #[test]
    fn mult_is_a_bit_exact_copy_including_inactive_slots() {
        let mut i = inputs();
        let mut o = [PortFrame::default(); 8];
        // Engine frames clear inactive slots, but the multiplier itself must
        // still copy rather than reinterpret or repair any of the frame bits.
        i[0].channels = 0;
        i[0].values[0] = -0.0;
        i[0].values[15] = f32::from_bits(0x7fc00001);
        Routing::Mult.process(&i, &mut o);
        for f in o {
            assert_eq!(f.channels, 0);
            assert_eq!(f.values.map(f32::to_bits), i[0].values.map(f32::to_bits));
        }
    }
}
