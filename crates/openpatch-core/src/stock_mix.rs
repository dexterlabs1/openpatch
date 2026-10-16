//! Original fixed-storage six-input mixing arithmetic. The pinned Fundamental
//! model defines f32 evaluation order and durable flags; canonical raw ports
//! belong to the common host. No upstream source or panel artwork is embedded.
use crate::host_ports::HostPortSnapshot;
#[cfg(test)]
use crate::native_float::flush_tiny;
use crate::native_float::{rack_add, rack_div, rack_mul};
use crate::{parameter, Node};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 6] = ["in1", "in2", "in3", "in4", "in5", "in6"];
pub(crate) const OUTPUTS: [&str; 1] = ["out"];

pub(crate) fn catalog() -> Value {
    json!({"kind":"stock_mix","title":"STOCK MIX","model":"functional","description":"Stock six-input raw-lane mix, ordered float32 addition, master level, inversion and averaging. Existing mix6 remains independent.","inputs":INPUTS,"params":{
        "level":{"default":1.0,"min":0.0,"max":1.0,"unit":"gain"},
        "invert":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1],"source":"module_data","description":"Durable host alias for the source Boolean Invert flag; not a Rack parameter index"},
        "average":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1],"source":"module_data","description":"Durable host alias for the source Boolean Average flag; not a Rack parameter index"}
    }})
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Mixer.cpp");
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["version_2"] = json!("max(1,six effective input channel counts); disconnected output stays0 while still computing raw quartets");
    module["polyphony"]["auxiliary_inputs"] = json!("all six signals read raw lanes; no module mono broadcast; actual host stacked-input transfer broadcasts mono sources before module processing");
    module["polyphony"]["retired_voices"] = json!("rounded quartet stores precede SDK setChannels retirement; retained canonical raw output is separate from public inactive padding");
    module["mix_policy"] = json!("f32 gain=level, optionally *= -1 then /= max(1,effective connected inputs); six raw input additions in port order from +0, then multiply by gain; connected mono0 counts toward averaging");
    module["light_names"] = json!([]);
    module["light_policy"] = json!("NUM_LIGHTS0; no fabricated activity or gain lamps");
    module["initialization_policy"] = json!("reset level through host parameter defaults; preserve durable Invert/Average flags and canonical ports; source has no custom reset callback");
    module["randomization_policy"] = json!("randomize level with declared host seeded uniform policy; preserve both durable flags; no claim of Rack RNG stream identity");
    module["legacy_data_policy"] = json!("save actual invert/average Booleans; load absent keys or nonobject root retains flags, present nonboolean key sets that flag false; partial assignments affect only supplied keys");
    module["state_adoption"] = json!("common canonical port history survives compatible incarnation adoption; receiving level and durable flag aliases win");
    module["arithmetic_policy"] = json!("source-order nearest f32 math with bit-defined signed FTZ/DAZ around each add/multiply/divide, matching Rack Engine resetFpuFlags; no amplitude clipping or nonfinite module sanitizer; raw port copies do not flush; outgoing host cable edges sanitize each nonfinite source independently");
    module["qualification_domain"] = json!("bounded finite inputs including declared minnormal/subnormal/underflow boundaries; raw nonfinite/overflow diagnostics require their separately declared scope; no global floating-point environment mutation");
    module["bypass"]["routes"] = json!([]);
    module["bypass"]["output_policy"] = json!("no bypass writes; actual enter/leave edges clear connected active prefixes to mono0, disconnected raw history retained, consumer edges still apply while bypassed");
}

/// Actual production controls, including the source's two durable data flags.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct StockMixSnapshot {
    pub level: f32,
    pub invert: bool,
    pub average: bool,
}

#[derive(Debug)]
pub(crate) struct StockMix {
    controls: StockMixSnapshot,
}

fn flag(node: &Node, name: &str) -> Result<bool, String> {
    let value = parameter(node, name, 0.0, 0.0, 1.0)?;
    let bits = value.to_bits();
    if bits & 0x7fff_ffff != 0 && bits != 1.0_f32.to_bits() {
        return Err(format!(
            "node '{}': parameter '{name}' must be 0 or 1",
            node.id
        ));
    }
    Ok(bits == 1.0_f32.to_bits())
}

impl StockMix {
    pub(crate) fn build(node: &Node) -> Result<Self, String> {
        for key in node.params.keys() {
            if !["level", "invert", "average"].contains(&key.as_str()) {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let level = parameter(node, "level", 1.0, 0.0, 1.0)?;
        if level.to_bits() & 0x8000_0000 != 0 && level.to_bits() & 0x7fff_ffff != 0 {
            return Err(format!("node '{}': level must be nonnegative", node.id));
        }
        Ok(Self {
            controls: StockMixSnapshot {
                level,
                invert: flag(node, "invert")?,
                average: flag(node, "average")?,
            },
        })
    }

    pub(crate) fn snapshot(&self) -> StockMixSnapshot {
        self.controls
    }

    pub(crate) fn legacy_data(&self) -> Value {
        json!({"invert":self.controls.invert,"average":self.controls.average})
    }

    pub(crate) fn load_legacy_data(&mut self, value: &Value) {
        let Some(object) = value.as_object() else {
            return;
        };
        if let Some(invert) = object.get("invert") {
            self.controls.invert = invert.as_bool().unwrap_or(false);
        }
        if let Some(average) = object.get("average") {
            self.controls.average = average.as_bool().unwrap_or(false);
        }
    }

    pub(crate) fn process(
        &self,
        inputs: &[HostPortSnapshot; 16],
        outputs: &mut [HostPortSnapshot],
    ) {
        let signals = &inputs[..INPUTS.len()];
        let channels = signals
            .iter()
            .map(|input| input.channels)
            .max()
            .unwrap_or(0)
            .max(1);
        let mut gain = self.controls.level;
        if self.controls.invert {
            gain = rack_mul(gain, -1.0);
        }
        if self.controls.average {
            let connected = signals
                .iter()
                .filter(|input| input.channels != 0)
                .count()
                .max(1);
            gain = rack_div(gain, connected as f32);
        }
        for lane in 0..usize::from(channels).div_ceil(4) * 4 {
            let mut sum = 0.0_f32;
            for input in signals {
                sum = rack_add(sum, input.values[lane]);
            }
            outputs[0].values[lane] = rack_mul(sum, gain);
        }
        outputs[0].set_channels(channels);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(invert: bool, average: bool) -> StockMix {
        StockMix::build(
            &serde_json::from_value(json!({"id":"m","kind":"stock_mix",
            "params":{"invert":u8::from(invert),"average":u8::from(average)}}))
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn mono_signals_use_raw_lanes_and_connected_zero_counts_in_average() {
        let mut inputs = [HostPortSnapshot::default(); 16];
        inputs[0].channels = 1;
        inputs[0].values[0] = 2.0;
        inputs[1].channels = 3;
        inputs[1].values[..3].copy_from_slice(&[4.0, 6.0, 8.0]);
        for invert in [false, true] {
            for average in [false, true] {
                let mut output = [HostPortSnapshot {
                    channels: 1,
                    ..Default::default()
                }];
                model(invert, average).process(&inputs, &mut output);
                let gain = (if invert { -1.0_f32 } else { 1.0 }) / if average { 2.0 } else { 1.0 };
                assert_eq!(output[0].channels, 3);
                assert_eq!(
                    &output[0].values[..3],
                    &[6.0 * gain, 6.0 * gain, 8.0 * gain]
                );
            }
        }
        inputs[2].channels = 1;
        let mut output = [HostPortSnapshot {
            channels: 1,
            ..Default::default()
        }];
        model(false, true).process(&inputs, &mut output);
        assert_eq!(&output[0].values[..3], &[2.0, 2.0, 8.0 * (1.0_f32 / 3.0)]);
    }

    #[test]
    fn all_widths_and_input_masks_preserve_counts_without_mono_broadcast() {
        for width in 0..=16_u8 {
            for mask in 0..64_u8 {
                let mut inputs = [HostPortSnapshot::default(); 16];
                for (port, input) in inputs[..6].iter_mut().enumerate() {
                    if mask & (1 << port) != 0 {
                        input.channels = width;
                        input.values[..usize::from(width)].fill((port + 1) as f32);
                    }
                }
                for connected in [false, true] {
                    let mut output = [HostPortSnapshot {
                        channels: u8::from(connected),
                        ..Default::default()
                    }];
                    model(false, false).process(&inputs, &mut output);
                    let count = if mask == 0 { 1 } else { width.max(1) };
                    assert_eq!(output[0].channels, if connected { count } else { 0 });
                    let expected: f32 = (0..6)
                        .filter(|port| mask & (1 << port) != 0)
                        .map(|port| (port + 1) as f32)
                        .sum();
                    for value in &output[0].values[..usize::from(width)] {
                        assert_eq!(*value, expected);
                    }
                }
            }
        }
    }

    #[test]
    fn six_additions_are_f32_source_order_and_module_does_not_clip() {
        let mut inputs = [HostPortSnapshot::default(); 16];
        for (input, value) in inputs.iter_mut().zip([16.0, 2.0_f32.powi(-21), -16.0]) {
            input.channels = 1;
            input.values[0] = value;
        }
        let mut output = [HostPortSnapshot {
            channels: 1,
            ..Default::default()
        }];
        model(false, false).process(&inputs, &mut output);
        assert_eq!(output[0].values[0].to_bits(), 0);
        inputs[0].values[0] = 2e6;
        inputs[1].values[0] = 0.0;
        inputs[2].values[0] = 0.0;
        model(false, false).process(&inputs, &mut output);
        assert_eq!(output[0].values[0], 2e6);
        inputs[0].values[0] = f32::INFINITY;
        model(false, false).process(&inputs, &mut output);
        assert!(output[0].values[0].is_infinite());
    }

    #[test]
    fn quartet_stores_precede_retirement_and_disconnected_tails_remain_raw() {
        let mut inputs = [HostPortSnapshot::default(); 16];
        inputs[0].channels = 16;
        inputs[0].values.fill(2.0);
        let mut connected = [HostPortSnapshot {
            channels: 1,
            ..Default::default()
        }];
        let mut disconnected = [HostPortSnapshot::default()];
        model(false, false).process(&inputs, &mut connected);
        model(false, false).process(&inputs, &mut disconnected);
        inputs.fill(HostPortSnapshot::default());
        model(true, false).process(&inputs, &mut connected);
        model(true, false).process(&inputs, &mut disconnected);
        assert_eq!(connected[0].channels, 1);
        assert_eq!(connected[0].values[0].to_bits(), (-0.0_f32).to_bits());
        assert!(connected[0].values[1..]
            .iter()
            .all(|value| value.to_bits() == 0));
        assert_eq!(disconnected[0].channels, 0);
        assert!(disconnected[0].values[..4]
            .iter()
            .all(|value| value.to_bits() == (-0.0_f32).to_bits()));
        assert_eq!(&disconnected[0].values[4..], &[2.0; 12]);
    }

    #[test]
    fn actual_legacy_data_loader_retains_absent_and_clears_present_nonbooleans() {
        let mut model = model(true, true);
        for value in [
            json!(null),
            json!([]),
            json!(false),
            json!("text"),
            json!(4),
            json!({"other":false}),
        ] {
            model.load_legacy_data(&value);
            assert_eq!(model.legacy_data(), json!({"invert":true,"average":true}));
        }
        for bad in [
            json!(null),
            json!(1),
            json!(0),
            json!("true"),
            json!([]),
            json!({}),
        ] {
            model.load_legacy_data(&json!({"invert":true,"average":true}));
            model.load_legacy_data(&json!({"average":bad}));
            assert!(model.snapshot().invert);
            assert!(!model.snapshot().average);
        }
        model.load_legacy_data(&json!({"average":true,"invert":false}));
        assert_eq!(model.legacy_data(), json!({"invert":false,"average":true}));
        assert_eq!(model.snapshot().level, 1.0);
    }

    #[test]
    fn signed_ftz_daz_is_bit_defined_and_leaves_normal_nonfinite_and_raw_storage_unchanged() {
        for bits in [0x0000_0001, 0x007f_ffff, 0x8000_0001, 0x807f_ffff] {
            assert_eq!(
                flush_tiny(f32::from_bits(bits)).to_bits(),
                bits & 0x8000_0000
            );
        }
        for bits in [
            0,
            0x8000_0000,
            0x0080_0000,
            0x8080_0000,
            0x7f7f_ffff,
            0x7f80_0000,
            0xff80_0000,
            0x7fc1_2345,
        ] {
            assert_eq!(flush_tiny(f32::from_bits(bits)).to_bits(), bits);
        }
        assert_eq!(
            rack_mul(f32::from_bits(1), -1.0).to_bits(),
            (-0.0_f32).to_bits()
        );
        assert_eq!(
            rack_add(-f32::from_bits(1), -0.0).to_bits(),
            (-0.0_f32).to_bits()
        );
        let mut inputs = [HostPortSnapshot::default(); 16];
        inputs[0].channels = 1;
        inputs[0].values[0] = f32::from_bits(0x8000_0001);
        let before = inputs[0].values.map(f32::to_bits);
        let mut output = [HostPortSnapshot {
            channels: 1,
            ..Default::default()
        }];
        model(true, false).process(&inputs, &mut output);
        assert_eq!(output[0].values[0].to_bits(), (-0.0_f32).to_bits());
        assert_eq!(inputs[0].values.map(f32::to_bits), before);
    }

    #[test]
    fn normal_operand_cancellation_and_gain_underflow_flush_results_with_sign() {
        let minimum = f32::MIN_POSITIVE;
        let next = f32::from_bits(0x0080_0001);
        assert_eq!(rack_add(minimum, -next).to_bits(), (-0.0_f32).to_bits());
        assert_eq!(rack_add(next, -minimum).to_bits(), 0);
        assert_eq!(rack_mul(minimum, 0.5).to_bits(), 0);
        assert_eq!(rack_mul(-minimum, 0.5).to_bits(), (-0.0_f32).to_bits());
        assert_eq!(rack_div(minimum, 3.0).to_bits(), 0);
        assert_eq!(rack_div(-minimum, 3.0).to_bits(), (-0.0_f32).to_bits());
        let mut mixer = model(false, true);
        mixer.controls.level = minimum;
        let mut inputs = [HostPortSnapshot::default(); 16];
        for input in &mut inputs[..3] {
            input.channels = 1;
            input.values[0] = 1.0;
        }
        let mut output = [HostPortSnapshot {
            channels: 1,
            ..Default::default()
        }];
        mixer.process(&inputs, &mut output);
        assert_eq!(output[0].values[0].to_bits(), 0);
        mixer.controls.invert = true;
        mixer.process(&inputs, &mut output);
        assert_eq!(output[0].values[0].to_bits(), (-0.0_f32).to_bits());
        assert_eq!(mixer.snapshot().level.to_bits(), minimum.to_bits());
    }

    #[test]
    fn final_multiply_flushes_tiny_result_but_control_snapshot_retains_tiny_bits() {
        let mut mixer = model(false, false);
        mixer.controls.level = f32::MIN_POSITIVE;
        let mut inputs = [HostPortSnapshot::default(); 16];
        inputs[0].channels = 1;
        inputs[0].values[0] = 0.5;
        let mut output = [HostPortSnapshot {
            channels: 1,
            ..Default::default()
        }];
        mixer.process(&inputs, &mut output);
        assert_eq!(output[0].values[0].to_bits(), 0);
        mixer.controls.invert = true;
        mixer.process(&inputs, &mut output);
        assert_eq!(output[0].values[0].to_bits(), (-0.0_f32).to_bits());
        mixer.controls.level = f32::from_bits(0x0000_0042);
        mixer.process(&inputs, &mut output);
        assert_eq!(output[0].values[0].to_bits(), (-0.0_f32).to_bits());
        assert_eq!(mixer.snapshot().level.to_bits(), 0x0000_0042);
    }
}
