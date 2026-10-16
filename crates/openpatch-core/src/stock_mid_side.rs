//! Original f32 stereo sum/difference implementation of the pinned stock
//! contract. Raw ports live in the common host transport, not in this model.
use crate::host_ports::HostPortSnapshot;
use crate::{parameter, Node};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 6] = ["encode_cv", "left", "right", "decode_cv", "mid", "side"];
pub(crate) const OUTPUTS: [&str; 4] = ["mid", "side", "left", "right"];

pub(crate) fn catalog() -> Value {
    json!({"kind":"stock_mid_side","title":"STOCK MID/SIDE","model":"functional","description":"Stock stereo mid/side encoder and normalled decoder, additive width CV and source-order float32 arithmetic. Existing mid_side remains independent.","inputs":INPUTS,"params":{
        "encode_width":{"default":1.0,"min":0.0,"max":2.0,"unit":"width"},
        "decode_width":{"default":1.0,"min":0.0,"max":2.0,"unit":"width"}
    }})
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/MidSide.cpp");
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["version_2"] = json!("encoder=max(left,right); decoder uses explicit mid/side maximum if either is connected, otherwise encoder width; connected output requested0 coerces1, disconnected output stays0");
    module["polyphony"]["auxiliary_inputs"] = json!("width CV mono broadcasts but never creates voices; all four signal inputs use raw lanes without mono broadcast");
    module["polyphony"]["retired_voices"] = json!("rounded quartets write raw output storage after SDK count retirement; internal decoder normals read the common raw encoder stores including inactive lanes");
    module["width_cv"] = json!({"formula":"max(knob + (CV / 10) * 2, 0)","normal_voltage":0.0,"upper_clamp":false,"creates_voices":false});
    module["input_metadata"][4]["normalization_source"] = json!("encoder.mid");
    module["input_metadata"][5]["normalization_source"] = json!("encoder.side");
    module["light_names"] = json!([]);
    module["initialization_policy"] = json!(
        "host resets width parameters only; no custom runtime reset, raw port storage preserved"
    );
    module["arithmetic_policy"] = json!("source-order float32 arithmetic without amplitude clipping; raw diagnostics propagate through module math, outgoing host cable edges sanitize each nonfinite source value separately");
    module["qualification_domain"] = json!("bounded finite source inputs and width controls with finite intermediate arithmetic; nonfinite/overflow raw diagnostics are separate from ordinary-domain equivalence and are not a finite JSON transport claim");
    module["bypass"]["routes"] = json!([]);
    module["bypass"]["output_policy"] = json!("no bypass writes; actual enter/leave edges clear connected active prefixes to mono0, disconnected raw storage retained; consumer edges continue while bypassed");
}

#[derive(Debug)]
pub(crate) struct StockMidSide {
    encode_width: f32,
    decode_width: f32,
}

impl StockMidSide {
    pub(crate) fn build(node: &Node) -> Result<Self, String> {
        for key in node.params.keys() {
            if !["encode_width", "decode_width"].contains(&key.as_str()) {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        Ok(Self {
            encode_width: parameter(node, "encode_width", 1.0, 0.0, 2.0)?,
            decode_width: parameter(node, "decode_width", 1.0, 0.0, 2.0)?,
        })
    }

    pub(crate) fn process(
        &self,
        inputs: &[HostPortSnapshot; 16],
        outputs: &mut [HostPortSnapshot],
    ) {
        let encoder_count = inputs[1].channels.max(inputs[2].channels);
        outputs[0].set_channels(encoder_count);
        outputs[1].set_channels(encoder_count);
        for lane in 0..usize::from(encoder_count).div_ceil(4) * 4 {
            let width = effective_width(self.encode_width, &inputs[0], lane);
            let left = inputs[1].values[lane];
            let right = inputs[2].values[lane];
            outputs[0].values[lane] = (left + right) / 2.0;
            outputs[1].values[lane] = ((left - right) / 2.0) * width;
        }
        let decoder_count = if inputs[4].channels != 0 || inputs[5].channels != 0 {
            inputs[4].channels.max(inputs[5].channels)
        } else {
            encoder_count
        };
        outputs[2].set_channels(decoder_count);
        outputs[3].set_channels(decoder_count);
        for lane in 0..usize::from(decoder_count).div_ceil(4) * 4 {
            let width = effective_width(self.decode_width, &inputs[3], lane);
            let middle = if inputs[4].channels == 0 {
                outputs[0].values[lane]
            } else {
                inputs[4].values[lane]
            };
            let side = if inputs[5].channels == 0 {
                outputs[1].values[lane]
            } else {
                inputs[5].values[lane]
            };
            let spread = side * width;
            outputs[2].values[lane] = middle + spread;
            outputs[3].values[lane] = middle - spread;
        }
    }
}

fn effective_width(knob: f32, cv: &HostPortSnapshot, lane: usize) -> f32 {
    let value = cv.values[if cv.channels == 1 { 0 } else { lane }];
    let width = knob + (value / 10.0) * 2.0;
    // SIMD fmax(width,0) returns its second operand for NaN and either zero.
    if width > 0.0 {
        width
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(encode: f32, decode: f32) -> StockMidSide {
        StockMidSide::build(
            &serde_json::from_value(json!({
                "id":"m","kind":"stock_mid_side",
                "params":{"encode_width":encode,"decode_width":decode}
            }))
            .unwrap(),
        )
        .unwrap()
    }

    fn outputs(mask: u16) -> [HostPortSnapshot; 4] {
        std::array::from_fn(|port| HostPortSnapshot {
            channels: u8::from(mask & (1 << port) != 0),
            ..HostPortSnapshot::default()
        })
    }

    #[test]
    fn additive_width_opens_zero_knob_and_is_not_capped_at_two() {
        let mut inputs = [HostPortSnapshot::default(); 16];
        inputs[1].channels = 1;
        inputs[1].values[0] = 2.0;
        inputs[0].channels = 1;
        inputs[0].values[0] = 5.0;
        let mut out = outputs(15);
        model(0.0, 1.0).process(&inputs, &mut out);
        assert_eq!(out[1].values[0], 1.0);
        model(1.0, 1.0).process(&inputs, &mut out);
        assert_eq!(out[1].values[0], 2.0);
        inputs[0].values[0] = 20.0;
        model(2.0, 1.0).process(&inputs, &mut out);
        assert_eq!(out[1].values[0], 6.0);
        inputs[0].values[0] = -20.0;
        model(2.0, 1.0).process(&inputs, &mut out);
        assert_eq!(out[1].values[0], 0.0);
    }

    #[test]
    fn every_width_and_consumer_mask_uses_raw_signals_and_mono_width_cv() {
        for count in 0..=16_u8 {
            for mask in 0..16_u16 {
                let mut inputs = [HostPortSnapshot::default(); 16];
                inputs[1].channels = u8::from(count != 0);
                inputs[1].values[0] = if count != 0 { 4.0 } else { 0.0 };
                inputs[2].channels = count;
                inputs[2].values[..usize::from(count)].fill(2.0);
                inputs[0].channels = 1;
                inputs[0].values[0] = 5.0;
                let mut out = outputs(mask);
                model(1.0, 1.0).process(&inputs, &mut out);
                for (port, output) in out.iter().enumerate() {
                    assert_eq!(
                        output.channels,
                        if mask & (1 << port) != 0 {
                            count.max(1)
                        } else {
                            0
                        }
                    );
                }
                if count != 0 {
                    assert_eq!(out[0].values[0], 3.0);
                    assert_eq!(out[1].values[0], 2.0);
                    assert_eq!(out[2].values[0], 5.0);
                    assert_eq!(out[3].values[0], 1.0);
                }
                for lane in 1..usize::from(count) {
                    assert_eq!(out[0].values[lane], 1.0);
                    assert_eq!(out[1].values[lane], -2.0);
                    assert_eq!(out[2].values[lane], -1.0);
                    assert_eq!(out[3].values[lane], 3.0);
                }
            }
        }
    }

    #[test]
    fn explicit_decoder_width_overrides_encoder_and_raw_normals_retain_tails() {
        let mut inputs = [HostPortSnapshot::default(); 16];
        inputs[1].channels = 16;
        inputs[1].values.fill(6.0);
        let mut unconsumed_encoder = outputs(12);
        let mut consumed_encoder = outputs(15);
        let model = model(1.0, 1.0);
        model.process(&inputs, &mut unconsumed_encoder);
        model.process(&inputs, &mut consumed_encoder);
        inputs[1].channels = 1;
        inputs[1].values.fill(0.0);
        inputs[1].values[0] = 2.0;
        inputs[4].channels = 16;
        model.process(&inputs, &mut unconsumed_encoder);
        model.process(&inputs, &mut consumed_encoder);
        assert_eq!(unconsumed_encoder[2].channels, 16);
        assert_eq!(unconsumed_encoder[2].values[4], 3.0);
        assert_eq!(consumed_encoder[2].values[4], 0.0);
        assert_eq!(unconsumed_encoder[2].values[1], 0.0);
        inputs[4].channels = 1;
        inputs[1].channels = 16;
        model.process(&inputs, &mut unconsumed_encoder);
        assert_eq!(unconsumed_encoder[0].channels, 0);
        assert_eq!(unconsumed_encoder[2].channels, 1);
    }

    #[test]
    fn raw_nonfinite_math_is_separate_from_outgoing_cable_sanitization() {
        let mut inputs = [HostPortSnapshot::default(); 16];
        inputs[1].channels = 1;
        inputs[1].values[0] = f32::INFINITY;
        let mut out = outputs(15);
        model(1.0, 1.0).process(&inputs, &mut out);
        assert!(out[0].values[0].is_infinite());
        assert!(out[1].values[0].is_infinite());
        assert!(out[3].values[0].is_nan());
        assert_eq!(
            effective_width(
                1.0,
                &HostPortSnapshot {
                    channels: 1,
                    values: [f32::NAN; 16]
                },
                0
            )
            .to_bits(),
            0
        );
    }
}
