//! VCA-2: two independent amplifier channels with linear and exponential CV,
//! adapted from the hidden legacy VCV Fundamental `VCA` model (pinned
//! 10dd0160, byte-identical in 2.6.4). See docs/STOCK_VCA2.md.
mod kernel;

use crate::{parameter, Node, PortFrame, SIGNAL_LIMIT};
#[doc(hidden)]
pub use kernel::{sse_mathfun_exp, Vca2Port, LN_50_BITS};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 6] = ["exp1", "lin1", "in1", "exp2", "lin2", "in2"];
pub(crate) const OUTPUTS: [&str; 2] = ["out1", "out2"];
pub(crate) const PARAMS: [&str; 2] = ["level1", "level2"];

pub(crate) fn catalog() -> Value {
    json!({"kind":"stock_vca2","title":"STOCK VCA-2","model":"functional","inputs":INPUTS,"params":{
        "level1":{"default":1.0,"min":0.0,"max":1.0,"display":{"multiplier":100.0,"unit":"%"}},
        "level2":{"default":1.0,"min":0.0,"max":1.0,"display":{"multiplier":100.0,"unit":"%"}}},
        "description":"Two polyphonic amplifiers, each with a level knob, a linear CV input and an exponential CV input. Recreates the retired VCA-2 so older patches keep their sound."})
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/VCA.cpp");
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["version_2"] = json!(
        "each channel: max(1, IN width), bounded to 16; CV widths never add voices; unpatched IN gives one channel of 0 V"
    );
    module["polyphony"]["auxiliary_inputs"] = json!("a one-channel CV applies to every voice; a polyphonic CV is read per voice and missing voices read 0 V (muted)");
    module["polyphony"]["retired_voices"] = json!("stateless; inactive output lanes are zero");
    if let Some(metadata) = module["input_metadata"].as_array_mut() {
        for meta in metadata {
            let cv = meta["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("lin") || name.starts_with("exp"));
            meta["channel_policy"] = json!(if cv {
                "one channel applies to all voices; otherwise per voice, missing voices 0 V"
            } else {
                "sets the channel's voice count"
            });
            meta["normalization"] = json!(if cv {
                "unpatched: no CV gain"
            } else {
                "unpatched: 0 V"
            });
        }
    }
    module["gain_law"] = json!("out = in * level, then * clamp(LIN/10, 0, 1) when LIN is patched, then * (50^clamp(EXP/10, 0, 1) - 1) / 49 when EXP is patched; a polyphonic EXP evaluates 50^x with Rack's SSE exp approximation, a mono EXP with powf");
    module["arithmetic_policy"] = json!("source-order f32 with signed FTZ/DAZ at every operation (Rack Engine resetFpuFlags); nonfinite input becomes zero and finite input clamps to ±1,000,000 V before the source arithmetic; no output clamp");
    module["light_names"] = json!([]);
    module["light_policy"] = json!("the source has no lights or meters");
    module["state_adoption"] = json!("stateless; parameter edits apply immediately");
    module["initialization_policy"] = json!("both levels return to 100%");
}

fn voltage(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-SIGNAL_LIMIT, SIGNAL_LIMIT)
    } else {
        0.0
    }
}

/// Raw port as the source reads it: active lanes sanitized, higher lanes zero.
fn port(frame: &PortFrame) -> Vca2Port {
    let mut port = Vca2Port {
        channels: frame.channels.min(16),
        values: [0.0; 16],
    };
    let width = usize::from(port.channels);
    for (out, value) in port.values[..width].iter_mut().zip(&frame.values) {
        *out = voltage(*value);
    }
    port
}

#[derive(Debug)]
pub(crate) struct StockVca2 {
    levels: [f32; 2],
}

impl StockVca2 {
    pub(crate) fn build(node: &Node) -> Result<Self, String> {
        for name in node.params.keys() {
            if !PARAMS.contains(&name.as_str()) {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        Ok(Self {
            levels: [
                parameter(node, "level1", 1.0, 0.0, 1.0)?,
                parameter(node, "level2", 1.0, 0.0, 1.0)?,
            ],
        })
    }

    pub(crate) fn process(&self, inputs: &[PortFrame; 6]) -> [PortFrame; 2] {
        let mut outputs = [PortFrame::default(); 2];
        for (channel, output) in outputs.iter_mut().enumerate() {
            let exp = port(&inputs[channel * 3]);
            let lin = port(&inputs[channel * 3 + 1]);
            let input = port(&inputs[channel * 3 + 2]);
            output.channels = kernel::process_channel(
                &input,
                self.levels[channel],
                &lin,
                &exp,
                &mut output.values,
            );
        }
        outputs
    }
}

/// Production VCA-2 behind the public probe used by the reference runner.
#[doc(hidden)]
pub struct StockVca2Probe(StockVca2);
impl StockVca2Probe {
    pub fn new(node: &Node) -> Result<Self, String> {
        StockVca2::build(node).map(Self)
    }
    pub fn step(&self, inputs: &[PortFrame; 6]) -> [PortFrame; 2] {
        self.0.process(inputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(params: &[(&str, f32)]) -> Node {
        let mut node: Node =
            serde_json::from_value(json!({"id":"vca","kind":"stock_vca2"})).unwrap();
        node.params = params.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        node
    }
    fn frame(channels: u8, value: f32) -> PortFrame {
        let mut frame = PortFrame {
            channels,
            values: [f32::NAN; 16],
        };
        frame.values[..usize::from(channels)].fill(value);
        frame
    }
    fn run(params: &[(&str, f32)], inputs: [PortFrame; 6]) -> [PortFrame; 2] {
        StockVca2::build(&node(params)).unwrap().process(&inputs)
    }
    const OFF: PortFrame = PortFrame {
        channels: 0,
        values: [0.0; 16],
    };

    #[test]
    fn parameters_are_validated() {
        for (name, value) in [
            ("unknown", 0.0),
            ("level1", -0.01),
            ("level2", 1.01),
            ("level1", f32::NAN),
        ] {
            assert!(StockVca2::build(&node(&[(name, value)])).is_err());
        }
        assert!(StockVca2::build(&node(&[("level1", 0.0), ("level2", 1.0)])).is_ok());
    }

    #[test]
    fn constants_match_the_reference_build() {
        // logf(50) printed by the C++ probe's `constants` mode.
        assert_eq!(50.0_f32.ln().to_bits(), LN_50_BITS);
        assert_eq!(sse_mathfun_exp(0.0), 1.0);
    }

    #[test]
    fn width_follows_the_signal_input_only() {
        let out = run(
            &[],
            [frame(16, 10.0), frame(4, 5.0), frame(3, 2.0), OFF, OFF, OFF],
        );
        assert_eq!(out[0].channels, 3);
        assert_eq!(out[1].channels, 1);
        assert_eq!(out[1].values[0], 0.0);
        assert!(out[0].values[3..].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn linear_and_exponential_gain_law() {
        let lin = run(&[], [OFF, frame(1, 5.0), frame(1, 8.0), OFF, OFF, OFF]);
        assert_eq!(lin[0].values[0], 4.0);
        let exp = run(&[], [frame(1, 10.0), OFF, frame(1, 8.0), OFF, OFF, OFF]);
        assert_eq!(exp[0].values[0], 8.0);
        let zero = run(&[], [frame(1, 0.0), OFF, frame(1, 8.0), OFF, OFF, OFF]);
        assert_eq!(zero[0].values[0], 0.0);
        // Half-scale exponential CV: (sqrt(50) - 1) / 49, far below linear.
        let half = run(&[], [frame(1, 5.0), OFF, frame(1, 1.0), OFF, OFF, OFF]);
        assert!((half[0].values[0] - (50.0_f32.sqrt() - 1.0) / 49.0).abs() < 1e-6);
        let levels = run(
            &[("level1", 0.5), ("level2", 0.25)],
            [OFF, OFF, frame(1, 8.0), OFF, OFF, frame(1, 8.0)],
        );
        assert_eq!([levels[0].values[0], levels[1].values[0]], [4.0, 2.0]);
    }

    #[test]
    fn short_polyphonic_cv_mutes_missing_voices() {
        let out = run(&[], [OFF, frame(2, 10.0), frame(4, 8.0), OFF, OFF, OFF]);
        assert_eq!(out[0].channels, 4);
        assert_eq!(&out[0].values[..4], &[8.0, 8.0, 0.0, 0.0]);
        let mono = run(&[], [OFF, frame(1, 10.0), frame(4, 8.0), OFF, OFF, OFF]);
        assert_eq!(&mono[0].values[..4], &[8.0; 4]);
    }

    #[test]
    fn nonfinite_inputs_become_zero() {
        let out = run(
            &[],
            [
                OFF,
                frame(1, f32::NAN),
                frame(1, 8.0),
                OFF,
                OFF,
                frame(1, f32::INFINITY),
            ],
        );
        assert_eq!(out[0].values[0], 0.0);
        assert_eq!(out[1].values[0], 0.0);
    }
}
