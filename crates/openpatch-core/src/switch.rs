//! Original functional sequential routing with explicit bounded arithmetic.
//! The pinned behavioral reference is not copied or a sample-exactness claim.
use crate::{parameter, ModuleTelemetry, Node, PortFrame, SIGNAL_LIMIT};
use serde_json::{json, Value};

pub(crate) fn input_names(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "switch_1to4" => Some(&["clock", "reset", "signal"]),
        "switch_4to1" => Some(&["clock", "reset", "in1", "in2", "in3", "in4"]),
        _ => None,
    }
}
pub(crate) fn output_names(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "switch_1to4" => Some(&["out1", "out2", "out3", "out4"]),
        "switch_4to1" => Some(&["out"]),
        _ => None,
    }
}
pub(crate) fn catalog() -> [Value; 2] {
    ["switch_1to4", "switch_4to1"].map(|kind| json!({
        "kind":kind,"title":if kind=="switch_1to4" {"SWITCH 1:4"} else {"SWITCH 4:1"},
        "model":"functional","description":"Clocked sequential routing with reset priority and optional 400/s linear de-click gain ramps. Two to four positions. Initial position one, initial ramp gains zero. No reset guard. All outputs keep the maximum signal-input channel count, minimum one, including unselected inputs. Raw 4-to-1 zero-pads mono; de-click 4-to-1 broadcasts mono. Nonfinite signal inputs become zero; final arithmetic safety bound ±1,000,000 V.",
        "inputs":input_names(kind).unwrap(),"params":{
            "steps":{"default":4.0,"min":2.0,"max":4.0,"step":1.0},
            "declick":{"default":0.0,"min":0.0,"max":1.0,"step":1.0}
        }
    }))
}
pub(crate) fn annotate(module: &mut Value) {
    if input_names(module["kind"].as_str().unwrap()).is_none() {
        return;
    }
    module["polyphony"]["version_2"] = json!("maximum of signal inputs, minimum one, including unselected inputs; clock/reset never create voices; raw 4to1 zero-pads mono, de-click 4to1 broadcasts mono; short-poly missing lanes are zero");
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["auxiliary_inputs"] =
        json!("clock/reset read channel one only; zero-channel inputs are zero");
    module["polyphony"]["retired_voices"] = json!("routing gains are per position, independent of channel count; inactive output slots are zero");
    module["clock_policy"] = json!({"low_threshold_volts":0.1,"high_threshold_volts":2.0,"ordering":"clock advances, reset wins, then position wraps immediately if outside effective length","reset_guard":false,"initial_state":"uninitialized: first high arms high without triggering; first low arms low; middle voltage leaves uninitialized; only low-to-high fires"});
    module["declick_policy"] = json!({"rate_per_second":400.0,"initial_gains":[0,0,0,0],"update":"enabled: slew all four gains toward selected one-hot position before output; disabled: freeze histories and route immediately","mix":"sum all gain-weighted inputs before final safety clamp; mono broadcasts only in de-click 4to1"});
    module["state_adoption"] = json!("same ID/kind/sample rate retains selected position and gain histories with new parameters; clock/reset detectors retain only on unchanged source ID/kind/output; shorter length wraps on next processed sample; kind/rate changes start fresh");
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/SequentialSwitch.cpp");
    module["schmitt_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Rack/061ccf63c1758599396ac1bb10d47345d9d34076/include/dsp/digital.hpp");
    module["compatibility"] = json!("original Rust functional implementation, not sample-identical Rack parity; steps is literal 2–4 rather than encoded 0–2; declick is a validated patch parameter rather than context-menu data; no legacy Rack JSON migration; gain arithmetic uses f64 rather than source f32; finite arithmetic bound ±1,000,000 V");
}
#[derive(Clone, Copy, Debug, Default)]
struct Schmitt(u8); // 0 uninitialized, 1 low, 2 high
impl Schmitt {
    fn rising(&mut self, value: f32) -> bool {
        let value = finite(value);
        if self.0 == 2 {
            if value <= 0.1 {
                self.0 = 1;
            }
            false
        } else if value >= 2.0 {
            let rising = self.0 == 1;
            self.0 = 2;
            rising
        } else {
            if value <= 0.1 {
                self.0 = 1;
            }
            false
        }
    }
}
fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}
fn voltage(value: f64) -> f32 {
    value.clamp(-f64::from(SIGNAL_LIMIT), f64::from(SIGNAL_LIMIT)) as f32
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct SequentialSwitch {
    many_inputs: bool,
    length: u8,
    declick: bool,
    slew: f64,
    selected: u8,
    gains: [f64; 4],
    clock: Schmitt,
    reset: Schmitt,
}
impl SequentialSwitch {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for key in node.params.keys() {
            if !matches!(key.as_str(), "steps" | "declick") {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let length = parameter(node, "steps", 4.0, 2.0, 4.0)?;
        let declick = parameter(node, "declick", 0.0, 0.0, 1.0)?;
        if length.fract() != 0.0 || declick.fract() != 0.0 {
            return Err(format!(
                "node '{}': steps and declick must be integers",
                node.id
            ));
        }
        Ok(Self {
            many_inputs: node.kind == "switch_4to1",
            length: length as u8,
            declick: declick != 0.0,
            slew: 400.0 / f64::from(rate),
            selected: 0,
            gains: [0.0; 4],
            clock: Schmitt::default(),
            reset: Schmitt::default(),
        })
    }
    pub(crate) fn output_count(&self) -> usize {
        if self.many_inputs {
            1
        } else {
            4
        }
    }
    pub(crate) fn telemetry(&self) -> ModuleTelemetry {
        ModuleTelemetry {
            step: self.selected + 1,
            length: self.length,
            running: true,
        }
    }
    pub(crate) fn reset_state(&mut self) {
        self.selected = 0;
        self.gains = [0.0; 4];
        self.clock = Schmitt::default();
        self.reset = Schmitt::default();
    }
    pub(crate) fn adopt(&mut self, previous: &Self, unchanged: [bool; 2]) {
        self.selected = previous.selected;
        self.gains = previous.gains;
        self.clock = if unchanged[0] {
            previous.clock
        } else {
            Schmitt::default()
        };
        self.reset = if unchanged[1] {
            previous.reset
        } else {
            Schmitt::default()
        };
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame; 6], outputs: &mut [PortFrame]) {
        if self.clock.rising(inputs[0].channel(0)) {
            self.selected += 1;
        }
        if self.reset.rising(inputs[1].channel(0)) {
            self.selected = 0;
        }
        if self.selected >= self.length {
            self.selected = 0;
        }
        if self.declick {
            for (index, gain) in self.gains.iter_mut().enumerate() {
                let target = if index == usize::from(self.selected) {
                    1.0
                } else {
                    0.0
                };
                *gain += (target - *gain).clamp(-self.slew, self.slew);
            }
        }
        let signals = &inputs[2..if self.many_inputs { 6 } else { 3 }];
        let count = signals
            .iter()
            .map(|f| f.channels.min(16))
            .max()
            .unwrap_or(0)
            .max(1);
        for output in outputs.iter_mut() {
            *output = PortFrame {
                channels: count,
                ..PortFrame::default()
            };
        }
        for lane in 0..usize::from(count) {
            if self.many_inputs {
                let value = if self.declick {
                    signals
                        .iter()
                        .zip(self.gains)
                        .map(|(input, gain)| f64::from(finite(input.voice(lane, 0.0))) * gain)
                        .sum()
                } else {
                    f64::from(finite(signals[usize::from(self.selected)].channel(lane)))
                };
                outputs[0].values[lane] = voltage(value);
            } else {
                let value = f64::from(finite(signals[0].channel(lane)));
                for (index, output) in outputs.iter_mut().enumerate() {
                    let gain = if self.declick {
                        self.gains[index]
                    } else if index == usize::from(self.selected) {
                        1.0
                    } else {
                        0.0
                    };
                    output.values[lane] = voltage(value * gain);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(kind: &str, declick: bool) -> SequentialSwitch {
        SequentialSwitch::build(
            &Node {
                id: "s".into(),
                kind: kind.into(),
                params: [("declick".into(), if declick { 1.0 } else { 0.0 })].into(),
            },
            8000,
        )
        .unwrap()
    }
    fn process(s: &mut SequentialSwitch, inputs: &[PortFrame; 6]) -> [PortFrame; 4] {
        let mut out = [PortFrame::default(); 4];
        let count = s.output_count();
        s.process(inputs, &mut out[..count]);
        out
    }
    #[test]
    fn uninitialized_schmitt_requires_low_before_first_edge_and_exact_thresholds_work() {
        let mut d = Schmitt::default();
        assert!(!d.rising(1.0));
        assert!(!d.rising(2.0));
        assert!(!d.rising(10.0));
        assert!(!d.rising(0.1));
        assert!(d.rising(2.0));
        assert!(!d.rising(1.0));
        assert!(!d.rising(2.0));
        assert!(!d.rising(f32::NAN));
        assert!(d.rising(2.0));
        let mut direct_high = Schmitt::default();
        assert!(!direct_high.rising(10.0));
        let mut direct_low = Schmitt::default();
        assert!(!direct_low.rising(0.1));
        assert!(direct_low.rising(2.0));
    }
    #[test]
    fn raw_zero_padding_and_declick_mono_broadcast_have_the_same_global_voice_count() {
        let mut inputs = [PortFrame::default(); 6];
        inputs[2] = PortFrame::mono(3.0);
        inputs[5] = PortFrame {
            channels: 16,
            ..PortFrame::default()
        };
        let mut raw = model("switch_4to1", false);
        raw.length = 2; // Inaccessible in4 still determines all sixteen lanes.
        let out = process(&mut raw, &inputs)[0];
        assert_eq!(out.channels, 16);
        assert_eq!(out.values[0], 3.0);
        assert_eq!(&out.values[1..], &[0.0; 15]);
        let mut smooth = model("switch_4to1", true);
        smooth.length = 2;
        let out = process(&mut smooth, &inputs)[0];
        assert_eq!(out.channels, 16);
        assert_eq!(out.values, [0.15; 16]);
        let mut one = model("switch_1to4", false);
        let out = process(&mut one, &[PortFrame::default(); 6]);
        assert!(out.iter().all(|f| f.channels == 1 && f.values == [0.0; 16]));
    }
    #[test]
    fn reset_wins_simultaneous_clock_but_next_sample_clock_has_no_guard() {
        let mut s = model("switch_1to4", false);
        let mut inputs = [PortFrame::default(); 6];
        inputs[2] = PortFrame::mono(7.0);
        process(&mut s, &inputs); // Arm both detectors low.
        inputs[0] = PortFrame::mono(2.0);
        inputs[1] = PortFrame::mono(2.0);
        assert_eq!(process(&mut s, &inputs)[0].values[0], 7.0);
        inputs[0] = PortFrame::mono(0.1);
        process(&mut s, &inputs);
        inputs[0] = PortFrame::mono(2.0);
        assert_eq!(process(&mut s, &inputs)[1].values[0], 7.0);
        assert_eq!(s.telemetry().step, 2);
    }
    #[test]
    fn gain_ramps_freeze_while_disabled_resume_and_shrink_keeps_old_tail() {
        let mut s = model("switch_1to4", true);
        let mut inputs = [PortFrame::default(); 6];
        inputs[2] = PortFrame::mono(10.0);
        let first = process(&mut s, &inputs);
        assert_eq!(first[0].values[0], 0.5); // 400/8000 gain before output.
        for _ in 0..19 {
            process(&mut s, &inputs);
        }
        assert_eq!(s.gains, [1.0, 0.0, 0.0, 0.0]);
        s.selected = 3;
        process(&mut s, &inputs);
        let frozen = s.gains;
        s.declick = false;
        for _ in 0..100 {
            assert_eq!(process(&mut s, &inputs)[3].values[0], 10.0);
        }
        assert_eq!(s.gains, frozen);
        s.length = 2;
        s.declick = true;
        let out = process(&mut s, &inputs);
        assert_eq!(s.telemetry().step, 1);
        assert!(out[0].values[0] > 9.9);
        assert_eq!(out[3].values[0], 0.0);
        // A larger old-step gain survives shrinking and fades despite being inaccessible.
        s.selected = 3;
        s.length = 4;
        for _ in 0..10 {
            process(&mut s, &inputs);
        }
        let old_gain = s.gains[3];
        s.length = 2;
        let out = process(&mut s, &inputs);
        assert!((f64::from(out[3].values[0]) - 10.0 * (old_gain - 0.05)).abs() < 1e-6);
        assert!(out[3].values[0] > 0.0);
    }
    #[test]
    fn nonfinite_inputs_are_zero_and_mix_clamps_only_after_summing() {
        let mut s = model("switch_4to1", true);
        s.gains = [1.0, 1.0, 1.0, 1.0];
        let mut inputs = [PortFrame::default(); 6];
        inputs[2] = PortFrame::mono(f32::MAX);
        inputs[3] = PortFrame::mono(-f32::MAX);
        inputs[4] = PortFrame::mono(f32::NAN);
        inputs[5] = PortFrame::mono(f32::INFINITY);
        // Selected lane gains remain 1 and the other lanes fall to .95.
        assert_eq!(process(&mut s, &inputs)[0].values[0], SIGNAL_LIMIT);
        s.declick = false;
        s.selected = 2;
        assert_eq!(process(&mut s, &inputs)[0].values[0], 0.0);
        s.selected = 0;
        assert_eq!(process(&mut s, &inputs)[0].values[0], SIGNAL_LIMIT);
        s.selected = 1;
        assert_eq!(process(&mut s, &inputs)[0].values[0], -SIGNAL_LIMIT);
        s.declick = true;
        // After slewing gains both become .5, so huge opposite values cancel.
        s.selected = 0;
        s.gains = [0.45, 0.55, 0.0, 0.0];
        assert_eq!(process(&mut s, &inputs)[0].values[0], 0.0);
    }
    #[test]
    fn adoption_retains_position_gains_but_clears_only_changed_detectors() {
        let mut old = model("switch_1to4", true);
        old.selected = 3;
        old.gains = [0.25, 0.0, 0.0, 0.75];
        old.clock = Schmitt(2);
        old.reset = Schmitt(1);
        let mut new = model("switch_1to4", false);
        new.adopt(&old, [true, false]);
        assert_eq!(new.selected, 3);
        assert_eq!(new.gains, old.gains);
        assert_eq!(new.clock.0, 2);
        assert_eq!(new.reset.0, 0);
        assert!(!new.reset.rising(2.0));
        assert!(!new.clock.rising(2.0));
        new.reset_state();
        assert_eq!(new.gains, [0.0; 4]);
        assert_eq!(new.telemetry().step, 1);
    }
}
