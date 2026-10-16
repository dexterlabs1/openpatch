//! Independent Stock Octave behavior from the pinned Fundamental Octave model.
//! Canonical raw ports and lifecycle are owned by the common host.
use crate::host_ports::HostPortSnapshot;
use crate::native_float::rack_add;
use crate::Node;
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 2] = ["pitch", "shift_cv"];
pub(crate) const OUTPUTS: [&str; 1] = ["out"];

pub(crate) fn catalog() -> Value {
    json!({"kind":"stock_octave","title":"STOCK OCTAVE","model":"functional",
        "description":"Fundamental octave pitch shifter with raw pitch reads, mono shift CV broadcast, rounded octave shifts and source-order float32 addition. Existing octave remains independent.",
        "inputs":INPUTS,"params":{"octaves":{"default":0.0,"min":-4.0,"max":4.0,"step":1.0,
            "unit":"octaves","values":[-4,-3,-2,-1,0,1,2,3,4],
            "raw_min":-3.4028234663852886e38,"raw_max":3.4028234663852886e38,
            "description":"Nine UI selectors cover -4..4; raw saved and legacy-loaded finite values may lie outside this editing range."}}})
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Octave.cpp");
    module["polyphony"]["primary_input"] = json!("pitch");
    module["polyphony"]["version_2"] =
        json!("max(raw pitch channels,1), even with no pitch connection");
    module["polyphony"]["auxiliary_inputs"] = json!("shift_cv is read with getPolyVoltage: exactly one channel broadcasts, otherwise each raw lane is read including inactive storage");
    module["polyphony"]["retired_voices"] = json!("active lanes are written in ascending order before SDK setChannels clears connected retired lanes; disconnected output retains raw writes with zero channels");
    module["light_names"] = json!([]);
    module["light_policy"] = json!("No Light objects; octave_shift reports actual channel-zero lastOctave display state, frozen in bypass");
    module["arithmetic_policy"] = json!("round knob f32 half away from zero and convert to i32; round CV f32, add float(knob i32) in Rack f32 arithmetic, then convert to i32; add float(integer shift) to raw pitch with Rack f32 arithmetic. Undefined C++ final float-to-int conversions are defined here as saturating, NaN to zero.");
    module["legacy_data_policy"] = json!("dataToJson returns null; dataFromJson root integer octave overwrites raw knob via int64-to-f32, absent key/nonobject retains, present noninteger sets zero");
    module["initialization_policy"] = json!("reset only changes octave parameter to zero; runtime lastOctave and raw host ports are retained");
    module["state_adoption"] = json!("compatible incarnation carries lastOctave and canonical raw ports; receiving raw octave parameter wins");
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct StockOctaveSnapshot {
    pub octaves: f32,
    pub last_octave: i32,
}

#[derive(Debug)]
pub(crate) struct StockOctave {
    state: StockOctaveSnapshot,
}

/// Rust-defined behavior at the source's undefined float-to-int boundary.
fn source_int(value: f32) -> i32 {
    if value.is_nan() {
        0
    } else {
        value as i32
    }
}

impl StockOctave {
    pub(crate) fn build(node: &Node) -> Result<Self, String> {
        for key in node.params.keys() {
            if key != "octaves" {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let octaves = node.params.get("octaves").copied().unwrap_or(0.0);
        if !octaves.is_finite() {
            return Err(format!(
                "node '{}': parameter 'octaves' must be finite",
                node.id
            ));
        }
        Ok(Self {
            state: StockOctaveSnapshot {
                octaves,
                last_octave: 0,
            },
        })
    }

    pub(crate) fn snapshot(&self) -> StockOctaveSnapshot {
        self.state
    }
    pub(crate) fn legacy_data(&self) -> Value {
        Value::Null
    }
    pub(crate) fn load_legacy_data(&mut self, value: &Value) {
        if let Some(octave) = value.as_object().and_then(|object| object.get("octave")) {
            self.state.octaves = octave.as_i64().unwrap_or(0) as f32;
        }
    }
    pub(crate) fn reset(&mut self) {
        self.state.octaves = 0.0;
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        self.state.last_octave = old.state.last_octave;
    }
    pub(crate) fn last_octave(&self) -> i32 {
        self.state.last_octave
    }

    pub(crate) fn process(
        &mut self,
        inputs: &[HostPortSnapshot; 16],
        outputs: &mut [HostPortSnapshot],
    ) {
        let channels = inputs[0].channels.max(1);
        let knob = source_int(self.state.octaves.round());
        for lane in 0..usize::from(channels) {
            let cv = if inputs[1].channels == 1 {
                inputs[1].values[0]
            } else {
                inputs[1].values[lane]
            };
            let octave = source_int(rack_add(knob as f32, cv.round()));
            outputs[0].values[lane] = rack_add(inputs[0].values[lane], octave as f32);
            if lane == 0 {
                self.state.last_octave = octave;
            }
        }
        outputs[0].set_channels(channels);
    }

    pub(crate) fn process_bypass(
        &self,
        inputs: &[HostPortSnapshot; 16],
        outputs: &mut [HostPortSnapshot],
    ) {
        let channels = inputs[0].channels;
        for lane in 0..usize::from(channels) {
            outputs[0].values[lane] = inputs[0].values[lane];
        }
        outputs[0].set_channels(channels);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(octaves: f32) -> StockOctave {
        StockOctave::build(
            &serde_json::from_value(
                json!({"id":"o","kind":"stock_octave","params":{"octaves":octaves}}),
            )
            .unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn all_pitch_widths_mono_cv_and_raw_inactive_reads() {
        for width in 0..=16_u8 {
            let mut inputs = [HostPortSnapshot::default(); 16];
            inputs[0].channels = width;
            inputs[0].values.fill(2.25);
            inputs[1].channels = 1;
            inputs[1].values[0] = -1.5;
            let mut output = [HostPortSnapshot {
                channels: 16,
                values: [99.0; 16],
            }];
            let mut octave = model(1.0);
            octave.process(&inputs, &mut output);
            let count = usize::from(width.max(1));
            assert_eq!(output[0].channels, width.max(1));
            assert!(output[0].values[..count].iter().all(|&v| v == 1.25));
            assert!(output[0].values[count..].iter().all(|&v| v == 0.0));
            assert_eq!(octave.last_octave(), -1);
            inputs[1].channels = 2;
            inputs[1].values[0] = 0.5;
            inputs[1].values[1..].fill(-2.5);
            let mut output = [HostPortSnapshot {
                channels: 1,
                ..Default::default()
            }];
            octave.process(&inputs, &mut output);
            assert_eq!(output[0].values[0], 4.25);
            if width > 1 {
                assert_eq!(output[0].values[1], 0.25);
            }
            if width > 2 {
                assert_eq!(output[0].values[2], 0.25);
            }
            assert_eq!(octave.last_octave(), 2);
        }
    }
    #[test]
    fn bypass_copies_raw_pitch_width_and_freezes_display() {
        let mut octave = model(4.0);
        let mut inputs = [HostPortSnapshot::default(); 16];
        let mut output = [HostPortSnapshot {
            channels: 16,
            values: [7.0; 16],
        }];
        octave.process(&inputs, &mut output);
        assert_eq!(octave.last_octave(), 4);
        inputs[0].channels = 2;
        inputs[0].values[..2].copy_from_slice(&[-0.0, 3.5]);
        octave.process_bypass(&inputs, &mut output);
        assert_eq!(output[0].values[0].to_bits(), (-0.0_f32).to_bits());
        assert_eq!(output[0].values[1], 3.5);
        assert_eq!(output[0].channels, 2);
        octave.process_bypass(&[HostPortSnapshot::default(); 16], &mut output);
        assert_eq!(output[0].channels, 1);
        assert_eq!(output[0].values[0], 0.0);
        assert_eq!(octave.last_octave(), 4);
    }
    #[test]
    fn finite_raw_knob_legacy_and_defined_integer_safety() {
        let mut octave = model(20.5);
        assert_eq!(octave.snapshot().octaves, 20.5);
        octave.load_legacy_data(&json!({"octave":-99}));
        assert_eq!(octave.snapshot().octaves, -99.0);
        octave.load_legacy_data(&json!({"octave":1.5}));
        assert_eq!(octave.snapshot().octaves, 0.0);
        octave.load_legacy_data(&json!({}));
        assert_eq!(octave.snapshot().octaves, 0.0);
        assert_eq!(octave.legacy_data(), Value::Null);
        assert_eq!(source_int(f32::NAN), 0);
        assert_eq!(source_int(f32::INFINITY), i32::MAX);
        assert_eq!(source_int(f32::NEG_INFINITY), i32::MIN);
        assert_eq!(source_int(2_147_483_648.0), i32::MAX);
        assert_eq!(source_int(-2_147_483_648.0), i32::MIN);
    }

    #[test]
    fn float_order_retains_large_values_and_knob_can_round_away() {
        let mut inputs = [HostPortSnapshot::default(); 16];
        inputs[0].channels = 1;
        inputs[0].values[0] = 2_000_000.0;
        inputs[1].channels = 1;
        inputs[1].values[0] = 16_777_216.0;
        let mut output = [HostPortSnapshot {
            channels: 1,
            ..Default::default()
        }];
        let mut octave = model(1.0);
        octave.process(&inputs, &mut output);
        assert_eq!(octave.last_octave(), 16_777_216);
        assert_eq!(output[0].values[0], rack_add(2_000_000.0, 16_777_216.0));
        inputs[1].values[0] = 0.5;
        octave.process(&inputs, &mut output);
        assert_eq!(octave.last_octave(), 2);
        assert_eq!(output[0].values[0], 2_000_002.0);
    }

    #[test]
    fn signed_half_ties_and_adjacent_cv_values_use_float_rounding() {
        let mut inputs = [HostPortSnapshot::default(); 16];
        inputs[0].channels = 1;
        inputs[1].channels = 1;
        let mut output = [HostPortSnapshot {
            channels: 1,
            ..Default::default()
        }];
        for knob in [-0.5_f32, 0.5] {
            let mut octave = model(knob);
            let rounded_knob = if knob < 0.0 { -1 } else { 1 };
            for cv in [-2.5_f32, -1.5, -0.5, 0.5, 1.5, 2.5] {
                for bits in [cv.to_bits() - 1, cv.to_bits(), cv.to_bits() + 1] {
                    let raw = f32::from_bits(bits);
                    inputs[1].values[0] = raw;
                    octave.process(&inputs, &mut output);
                    let expected = rounded_knob + raw.round() as i32;
                    assert_eq!(octave.last_octave(), expected);
                    assert_eq!(output[0].values[0], expected as f32);
                }
            }
        }
        let mut octave = model(-0.0);
        inputs[0].values[0] = -0.0;
        inputs[1].values[0] = -0.0;
        octave.process(&inputs, &mut output);
        assert_eq!(octave.last_octave(), 0);
        assert_eq!(output[0].values[0].to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn engine_seam_controls_desired_source_and_adoption_preserves_runtime() {
        use crate::{Patch, PortFrame};
        let patch: Patch = serde_json::from_value(json!({"version":2,
            "nodes":[
                {"id":"source","kind":"poly_constant","params":{"channels":1,"v1":0}},
                {"id":"o","kind":"stock_octave","params":{"octaves":2.0}}],
            "cables":[{"id":"pitch","from":"source","to":"o","input":"pitch"}],
            "output":"o"}))
        .unwrap();
        let mut engine = patch.compile().unwrap();
        let source = engine.resolve_module_state("source").unwrap();
        let octave = engine.resolve_module_state("o").unwrap();
        let mut wide = PortFrame::mono(40.25);
        wide.values[1] = 123.0;
        assert!(engine.set_host_reference_source(source, wide));
        assert_eq!(
            engine.get_host_reference_source(source).unwrap().values[1],
            0.0
        );
        engine.process_poly_block(&mut [PortFrame::default(); 3]);
        assert_eq!(engine.stock_octave_snapshot(octave).unwrap().last_octave, 2);
        assert_eq!(
            engine.host_module_snapshot(octave).unwrap().outputs[0].values[0],
            42.25
        );
        let mut receiving = patch.clone();
        receiving
            .nodes
            .iter_mut()
            .find(|node| node.id == "o")
            .unwrap()
            .params
            .insert("octaves".into(), -3.0);
        let mut next = receiving.compile().unwrap();
        next.adopt_state_from(&engine);
        assert!(next.stock_octave_snapshot(octave).is_none());
        let adopted = next.resolve_module_state("o").unwrap();
        assert_eq!(
            next.stock_octave_snapshot(adopted).unwrap(),
            StockOctaveSnapshot {
                octaves: -3.0,
                last_octave: 2
            }
        );
        assert_eq!(
            next.host_module_snapshot(adopted).unwrap().inputs[0].values[0],
            40.25
        );
        assert_eq!(
            next.host_module_snapshot(adopted).unwrap().outputs[0].values[0],
            42.25
        );
        let source = next.resolve_module_state("source").unwrap();
        assert!(next.set_host_reference_source(source, PortFrame::mono(40.25)));
        assert_eq!(
            next.get_host_reference_source(source).unwrap().values[0],
            40.25
        );
    }
    #[test]
    fn undefined_source_conversions_have_explicit_native_saturation_safety() {
        let mut inputs = [HostPortSnapshot::default(); 16];
        inputs[0].channels = 1;
        inputs[1].channels = 1;
        for (cv, wanted) in [
            (0x7f800000, i32::MAX),
            (0xff800000, i32::MIN),
            (0x7fc12345, 0),
            (0x4f000000, i32::MAX),
            (0xcf000001, i32::MIN),
            (0x7f7fffff, i32::MAX),
        ] {
            let mut octave = model(0.0);
            inputs[1].values[0] = f32::from_bits(cv);
            let mut output = [HostPortSnapshot {
                channels: 1,
                ..Default::default()
            }];
            octave.process(&inputs, &mut output);
            assert_eq!(octave.last_octave(), wanted, "CV bits {cv:08x}");
            assert_eq!(output[0].values[0], wanted as f32);
            assert!(output[0].values[0].is_finite());
        }
        for parameter in [0x7f800000, 0xff800000, 0x7fc12345] {
            let node: Node =
                serde_json::from_value(json!({"id":"o","kind":"stock_octave"})).unwrap();
            let mut node = node;
            node.params
                .insert("octaves".into(), f32::from_bits(parameter));
            assert!(StockOctave::build(&node).is_err());
        }
        for (parameter, wanted) in [(0x4f000000, i32::MAX), (0xcf000001, i32::MIN)] {
            let mut octave = model(f32::from_bits(parameter));
            inputs[1].values[0] = 0.0;
            let mut output = [HostPortSnapshot {
                channels: 1,
                ..Default::default()
            }];
            octave.process(&inputs, &mut output);
            assert_eq!(octave.last_octave(), wanted);
        }
    }
}
