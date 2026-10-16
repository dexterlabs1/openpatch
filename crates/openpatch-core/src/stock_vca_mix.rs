//! Original functional implementation of Fundamental VCMixer behavior.
//! This is a digital reference, not a physical hardware model. Canonical raw
//! ports, cables and connection transitions belong to the common host.
use crate::host_ports::HostPortSnapshot;
use crate::native_float::{rack_add, rack_div, rack_mul, rack_sub};
use crate::{ModuleLightState, Node};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 9] = [
    "mix_cv", "in1", "in2", "in3", "in4", "cv1", "cv2", "cv3", "cv4",
];
pub(crate) const OUTPUTS: [&str; 5] = ["mix", "out1", "out2", "out3", "out4"];
const PARAMS: [&str; 5] = ["mix_level", "level1", "level2", "level3", "level4"];

pub(crate) fn catalog() -> Value {
    json!({"kind":"stock_vca_mix","title":"STOCK VCA MIX","model":"functional",
    "description":"Four-channel voltage-controlled mixer with independent strip outputs, polyphonic mix and four runtime meters. Existing vca_mix remains independent.",
    "inputs":INPUTS,"params":{
        "mix_level":{"default":1.0,"min":0.0,"max":2.0,"raw_min":-3.4028234663852886e38,"raw_max":3.4028234663852886e38,"unit":"gain","description":"Native mix knob; raw saved finite values may lie outside the editing range."},
        "level1":{"default":1.0,"min":0.0,"max":std::f32::consts::SQRT_2,"raw_min":-3.4028234663852886e38,"raw_max":3.4028234663852886e38,"unit":"gain"},
        "level2":{"default":1.0,"min":0.0,"max":std::f32::consts::SQRT_2,"raw_min":-3.4028234663852886e38,"raw_max":3.4028234663852886e38,"unit":"gain"},
        "level3":{"default":1.0,"min":0.0,"max":std::f32::consts::SQRT_2,"raw_min":-3.4028234663852886e38,"raw_max":3.4028234663852886e38,"unit":"gain"},
        "level4":{"default":1.0,"min":0.0,"max":std::f32::consts::SQRT_2,"raw_min":-3.4028234663852886e38,"raw_max":3.4028234663852886e38,"unit":"gain"},
        "channel_exp":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1],"source":"module_data","description":"Durable alias for native chExp Boolean; not a Rack parameter index."},
        "mix_exp":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1],"source":"module_data","description":"Durable alias for native mixExp Boolean; not a Rack parameter index."}
    }})
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/VCMixer.cpp");
    module["min_patch_version"] = json!(2);
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["version_2"] =
        json!("max(1, widths of four signal inputs); CV widths never add voices");
    module["polyphony"]["auxiliary_inputs"] = json!("all nine inputs read physical four-lane SIMD groups; exactly one channel broadcasts, otherwise raw lanes including padding are read");
    module["polyphony"]["retired_voices"] = json!("four-lane strip writes and consumed mix writes precede all output setChannels calls; disconnected storage retains raw writes");
    module["arithmetic_policy"] = json!("source-order f32 with signed FTZ/DAZ at every operation, no output clamp; strip gain squares raw level; CV divides by ten then SIMD max(0,cv), optionally fourth power; master gain/CV only if mix consumed");
    module["light_names"] = json!(["channel_1", "channel_2", "channel_3", "channel_4"]);
    module["light_policy"] = json!("four actual VuMeter2 peak states; signed padded strip sum divided by five, immediate attack and ordered f32 release at lambda30; ClockDivider512 publishes brightness");
    module["legacy_data_policy"] = json!("dataToJson emits chExp and mixExp Booleans; present non-true values set false, absent keys and nonobject roots retain existing flags");
    module["initialization_policy"] = json!("reset all five parameters to one and both flags false; preserve meters, divider, lights and raw ports");
    module["state_adoption"] = json!("compatible receiving incarnation preserves meters, divider, lights and canonical raw ports; receiving controls win");
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct StockVcaMixMeterSnapshot {
    pub v: f32,
    pub mode: u8,
    pub lambda: f32,
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct StockVcaMixSnapshot {
    pub parameters: [f32; 5],
    pub ch_exp: bool,
    pub mix_exp: bool,
    pub meters: [StockVcaMixMeterSnapshot; 4],
    pub divider_clock: u32,
    pub divider_division: u32,
    pub light_brightness: [f32; 4],
    pub sample_rate: u32,
    pub sample_time: f32,
}

#[derive(Debug)]
pub(crate) struct StockVcaMix {
    state: StockVcaMixSnapshot,
}

fn flag(node: &Node, name: &str) -> Result<bool, String> {
    let value = node.params.get(name).copied().unwrap_or(0.0);
    if !value.is_finite() || (value != 0.0 && value != 1.0) {
        return Err(format!(
            "node '{}': parameter '{name}' must be 0 or 1",
            node.id
        ));
    }
    Ok(value == 1.0)
}

fn raw_parameter(node: &Node, name: &str) -> Result<f32, String> {
    let value = node.params.get(name).copied().unwrap_or(1.0);
    if !value.is_finite() {
        return Err(format!(
            "node '{}': parameter '{name}' must be finite",
            node.id
        ));
    }
    Ok(value)
}

/// The second operand wins equal and unordered SIMD fmax lanes, including -0.
fn simd_max_zero_second(cv: f32) -> f32 {
    if 0.0 > cv {
        0.0
    } else {
        cv
    }
}

fn poly_lane(port: &HostPortSnapshot, lane: usize) -> f32 {
    port.values[if port.channels == 1 { 0 } else { lane }]
}

fn brightness(v: f32) -> f32 {
    let db = rack_mul(v.log10(), 20.0);
    if db >= 0.0 {
        1.0
    } else if db <= -24.0 {
        0.0
    } else {
        // Rack math::rescale(db, -24, 0, 0, 1), preserving source order.
        rack_add(0.0, rack_mul(rack_div(rack_sub(db, -24.0), 24.0), 1.0))
    }
}

impl StockVcaMix {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        if rate == 0 {
            return Err("stock_vca_mix sample rate must be positive".into());
        }
        for key in node.params.keys() {
            if !PARAMS.contains(&key.as_str()) && key != "channel_exp" && key != "mix_exp" {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let mut parameters = [1.0; 5];
        for (index, name) in PARAMS.iter().enumerate() {
            parameters[index] = raw_parameter(node, name)?;
        }
        Ok(Self {
            state: StockVcaMixSnapshot {
                parameters,
                ch_exp: flag(node, "channel_exp")?,
                mix_exp: flag(node, "mix_exp")?,
                meters: [StockVcaMixMeterSnapshot {
                    v: 0.0,
                    mode: 0,
                    lambda: 30.0,
                }; 4],
                divider_clock: 0,
                divider_division: 512,
                light_brightness: [0.0; 4],
                sample_rate: rate,
                sample_time: 1.0_f32 / rate as f32,
            },
        })
    }

    pub(crate) fn snapshot(&self) -> StockVcaMixSnapshot {
        self.state
    }
    pub(crate) fn legacy_data(&self) -> Value {
        json!({"chExp":self.state.ch_exp,"mixExp":self.state.mix_exp})
    }
    pub(crate) fn load_legacy_data(&mut self, value: &Value) {
        let Some(object) = value.as_object() else {
            return;
        };
        if let Some(value) = object.get("chExp") {
            self.state.ch_exp = value.as_bool().unwrap_or(false);
        }
        if let Some(value) = object.get("mixExp") {
            self.state.mix_exp = value.as_bool().unwrap_or(false);
        }
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        self.state.meters = old.state.meters;
        self.state.divider_clock = old.state.divider_clock;
        self.state.divider_division = old.state.divider_division;
        self.state.light_brightness = old.state.light_brightness;
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut result = ModuleLightState {
            count: 4,
            ..ModuleLightState::default()
        };
        for index in 0..4 {
            result.brightness[index][0] = self.state.light_brightness[index];
        }
        result
    }

    pub(crate) fn process(
        &mut self,
        inputs: &[HostPortSnapshot; 16],
        input_presence: u16,
        outputs: &mut [HostPortSnapshot],
    ) {
        let channels = inputs[1..5]
            .iter()
            .map(|p| p.channels)
            .max()
            .unwrap_or(0)
            .max(1);
        let mix_connected = outputs[0].channels != 0;
        let mut channel_sums = [0.0_f32; 4];
        for quartet in (0..usize::from(channels)).step_by(4) {
            for offset in 0..4 {
                let lane = quartet + offset;
                let mut mix = 0.0_f32;
                for channel in 0..4 {
                    let signal_port = channel + 1;
                    let mut out = 0.0_f32;
                    if input_presence & (1 << signal_port) != 0 {
                        out = poly_lane(&inputs[signal_port], lane);
                        let gain = rack_mul(
                            self.state.parameters[channel + 1],
                            self.state.parameters[channel + 1],
                        );
                        out = rack_mul(out, gain);
                        let cv_port = channel + 5;
                        if input_presence & (1 << cv_port) != 0 {
                            let mut cv = simd_max_zero_second(rack_div(
                                poly_lane(&inputs[cv_port], lane),
                                10.0,
                            ));
                            if self.state.ch_exp {
                                let square = rack_mul(cv, cv);
                                cv = rack_mul(square, square);
                            }
                            out = rack_mul(out, cv);
                        }
                        channel_sums[channel] = rack_add(channel_sums[channel], out);
                        mix = rack_add(mix, out);
                    }
                    outputs[channel + 1].values[lane] = out;
                }
                if mix_connected {
                    mix = rack_mul(mix, self.state.parameters[0]);
                    if input_presence & 1 != 0 {
                        let mut cv =
                            simd_max_zero_second(rack_div(poly_lane(&inputs[0], lane), 10.0));
                        if self.state.mix_exp {
                            let square = rack_mul(cv, cv);
                            cv = rack_mul(square, square);
                        }
                        mix = rack_mul(mix, cv);
                    }
                    outputs[0].values[lane] = mix;
                }
            }
        }
        for output in &mut outputs[1..5] {
            output.set_channels(channels);
        }
        outputs[0].set_channels(channels);
        for (meter, sum) in self.state.meters.iter_mut().zip(channel_sums) {
            let target = rack_div(sum, 5.0).abs();
            if target >= meter.v {
                meter.v = target;
            } else {
                let decay = rack_mul(
                    rack_mul(rack_sub(target, meter.v), meter.lambda),
                    self.state.sample_time,
                );
                meter.v = rack_add(meter.v, decay);
            }
        }
        self.state.divider_clock += 1;
        if self.state.divider_clock >= self.state.divider_division {
            self.state.divider_clock = 0;
            for index in 0..4 {
                self.state.light_brightness[index] = brightness(self.state.meters[index].v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(params: Value) -> StockVcaMix {
        let node: Node =
            serde_json::from_value(json!({"id":"mix","kind":"stock_vca_mix","params":params}))
                .unwrap();
        StockVcaMix::build(&node, 48_000).unwrap()
    }

    #[test]
    fn mono_signal_writes_four_padded_lanes_and_meter_sees_all_four() {
        let mut mixer = model(json!({}));
        let mut inputs = [HostPortSnapshot::default(); 16];
        inputs[1].channels = 1;
        inputs[1].values[0] = 1.25;
        let mut outputs = [HostPortSnapshot::default(); 5];
        mixer.process(&inputs, 1 << 1, &mut outputs);
        assert_eq!(outputs[1].channels, 0);
        assert_eq!(&outputs[1].values[..4], &[1.25; 4]);
        assert_eq!(outputs[0].values, [0.0; 16]);
        assert_eq!(mixer.snapshot().meters[0].v.to_bits(), 1.0_f32.to_bits());
        assert_eq!(mixer.snapshot().divider_clock, 1);
    }

    #[test]
    fn short_poly_cv_reads_raw_padding_and_mix_gain_requires_consumer() {
        let mut mixer = model(json!({"mix_level":2.0,"level1":1.0}));
        let mut inputs = [HostPortSnapshot::default(); 16];
        inputs[1].channels = 3;
        inputs[1].values[..3].copy_from_slice(&[1.0, 2.0, 3.0]);
        inputs[5].channels = 2;
        inputs[5].values[..3].copy_from_slice(&[10.0, 5.0, -0.0]);
        let mut outputs = [HostPortSnapshot::default(); 5];
        outputs[0].channels = 1;
        outputs[1].channels = 1;
        mixer.process(&inputs, (1 << 1) | (1 << 5), &mut outputs);
        assert_eq!(outputs[0].channels, 3);
        assert_eq!(outputs[1].channels, 3);
        assert_eq!(&outputs[1].values[..3], &[1.0, 1.0, -0.0]);
        assert_eq!(&outputs[0].values[..2], &[2.0, 2.0]);
    }

    #[test]
    fn raw_controls_json_reset_and_adoption_keep_runtime_separate() {
        let mut old = model(json!({"mix_level":-9.0,"level1":20.5,"channel_exp":1.0}));
        old.load_legacy_data(&json!({"mixExp":true,"chExp":7}));
        assert_eq!(old.legacy_data(), json!({"chExp":false,"mixExp":true}));
        let mut inputs = [HostPortSnapshot::default(); 16];
        inputs[1].channels = 1;
        inputs[1].values[0] = 1.0;
        old.process(&inputs, 1 << 1, &mut [HostPortSnapshot::default(); 5]);
        let mut receiving = model(json!({"mix_level":2.0,"level1":0.5,"channel_exp":1.0}));
        receiving.adopt(&old);
        assert_eq!(receiving.snapshot().parameters[0], 2.0);
        assert!(receiving.snapshot().ch_exp);
        assert!(!receiving.snapshot().mix_exp);
        assert_eq!(receiving.snapshot().meters, old.snapshot().meters);
        let mut reset = model(json!({}));
        reset.adopt(&receiving);
        assert_eq!(reset.snapshot().parameters, [1.0; 5]);
        assert!(!reset.snapshot().ch_exp && !reset.snapshot().mix_exp);
        assert_eq!(reset.snapshot().meters, old.snapshot().meters);
        assert_eq!(reset.snapshot().divider_clock, 1);
        for bad in [f32::INFINITY, f32::NAN] {
            let node: Node =
                serde_json::from_value(json!({"id":"bad","kind":"stock_vca_mix"})).unwrap();
            let mut node = node;
            node.params.insert("level1".into(), bad);
            assert!(StockVcaMix::build(&node, 48_000).is_err());
        }
    }
}
