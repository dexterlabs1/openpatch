//! Unity: two six-input unity-gain mixers with sum/average switches, inverted
//! outputs, five-step level lights per section and an optional merge of the
//! two sections, adapted from the hidden legacy VCV Fundamental `Unity` model
//! (pinned 10dd0160, byte-identical in 2.6.4). See docs/STOCK_UNITY.md.
mod kernel;

use crate::{ModuleLightState, Node, PortFrame, SIGNAL_LIMIT};
#[doc(hidden)]
pub use kernel::{UnityInput, UnityKernel};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 12] = [
    "ch1_1", "ch1_2", "ch1_3", "ch1_4", "ch1_5", "ch1_6", "ch2_1", "ch2_2", "ch2_3", "ch2_4",
    "ch2_5", "ch2_6",
];
pub(crate) const OUTPUTS: [&str; 4] = ["mix1", "inv1", "mix2", "inv2"];
pub(crate) const PARAMS: [&str; 3] = ["average1", "average2", "merge"];

pub(crate) fn catalog() -> Value {
    json!({"kind":"stock_unity","title":"STOCK UNITY","model":"functional","inputs":INPUTS,"params":{
        "average1":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1],"description":"Section 1 mode: 0 = sum, 1 = average of the patched inputs"},
        "average2":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1],"description":"Section 2 mode: 0 = sum, 1 = average of the patched inputs"},
        "merge":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1],"source":"module_data","description":"Merge sections 1 and 2 into one twelve-input mix on both sets of outputs. Saved as the source's merge Boolean; not a Rack parameter index."}},
        "description":"Two six-input unity-gain mixers with sum or average modes, inverted outputs and level lights. Recreates the retired Unity so older patches keep their sound."})
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Unity.cpp");
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["version_2"] = json!(
        "monophonic: every input reads its first channel; all four outputs carry one channel"
    );
    module["polyphony"]["auxiliary_inputs"] = json!("a patched input counts toward the average whatever its voltage; channels above the first are ignored");
    module["polyphony"]["retired_voices"] = json!("not applicable");
    module["mix_policy"] = json!("each section adds its six first-channel voltages in port order from +0; with merge, section 1 += section 2 and both sections then carry that sum and the combined patched-input count; average divides by the count when at least one input is patched; INV is the exact negation of MIX");
    module["normalization_policy"] = json!("no jack normaling; unpatched inputs contribute 0 V and do not count; the only link between the sections is merge");
    module["light_names"] = json!([
        "ch1_clip",
        "ch1_minus6db",
        "ch1_minus12db",
        "ch1_minus18db",
        "ch1_minus24db",
        "ch2_clip",
        "ch2_minus6db",
        "ch2_minus12db",
        "ch2_minus18db",
        "ch2_minus24db"
    ]);
    module["light_policy"] = json!("Rack VuMeter2 peak meter per section on MIX/10 (instant attack, lambda 30 release); lights refresh every 256 frames; clip lights at 0 dB; the others rise across 6 dB bands and are full at -6/-12/-18/-24 dB");
    module["arithmetic_policy"] = json!("source-order f32 with signed FTZ/DAZ at every operation (Rack Engine resetFpuFlags); nonfinite input becomes zero and finite input clamps to ±1,000,000 V; no output clamp");
    module["legacy_data_policy"] = json!("module data {\"merge\": Boolean}; on import only true merges, any other value or absence leaves the sections separate");
    module["initialization_policy"] = json!("both modes return to sum and merge turns off (the source's reset handler); meters, light divider and lights keep running");
    module["state_adoption"] = json!("meters, light divider and lights carry across edits and sample-rate changes; the new sample time applies");
    module["bypass"]["routes"] = json!([]);
}

fn flag(node: &Node, name: &str) -> Result<f32, String> {
    let value = node.params.get(name).copied().unwrap_or(0.0);
    if value != 0.0 && value != 1.0 {
        return Err(format!(
            "node '{}': parameter '{name}' must be 0 or 1",
            node.id
        ));
    }
    Ok(value)
}

fn voltage(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-SIGNAL_LIMIT, SIGNAL_LIMIT)
    } else {
        0.0
    }
}

/// Merge flag from saved Rack module data, as a fresh module's
/// `dataFromJson` leaves it: only a JSON `true` merges.
pub(crate) fn merge_from_data(data: &Value) -> bool {
    data.get("merge") == Some(&Value::Bool(true))
}

#[derive(Debug)]
pub(crate) struct StockUnity {
    average: [f32; 2],
    rate: u32,
    state: UnityKernel,
}

impl StockUnity {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        if rate == 0 {
            return Err("stock_unity sample rate must be positive".into());
        }
        for name in node.params.keys() {
            if !PARAMS.contains(&name.as_str()) {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        let mut state = UnityKernel::new(rate);
        state.merge = flag(node, "merge")? == 1.0;
        Ok(Self {
            average: [flag(node, "average1")?, flag(node, "average2")?],
            rate,
            state,
        })
    }

    /// Initialize: parameter defaults arrive through the rebuilt node; the
    /// source's reset handler leaves meters and lights running.
    pub(crate) fn initialize_state(&mut self) {}

    pub(crate) fn adopt(&mut self, old: &Self) {
        let merge = self.state.merge;
        self.state = old.state;
        self.state.merge = merge;
        self.state.sample_time = 1.0_f32 / self.rate as f32;
    }

    pub(crate) fn snapshot(&self) -> UnityKernel {
        self.state
    }

    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut lights = ModuleLightState {
            count: 10,
            ..ModuleLightState::default()
        };
        for (slot, value) in lights.brightness.iter_mut().zip(self.state.lights) {
            slot[0] = value;
        }
        lights
    }

    pub(crate) fn process(&mut self, inputs: &[PortFrame; 12]) -> [PortFrame; 4] {
        let mut sources = [UnityInput::default(); 12];
        for (source, frame) in sources.iter_mut().zip(inputs) {
            if frame.channels > 0 {
                *source = UnityInput {
                    connected: true,
                    voltage: voltage(frame.values[0]),
                };
            }
        }
        let values = self.state.process(&sources, self.average);
        values.map(|value| {
            let mut frame = PortFrame {
                channels: 1,
                ..PortFrame::default()
            };
            frame.values[0] = value;
            frame
        })
    }
}

/// Production Unity behind the public probe used by the reference runner.
#[doc(hidden)]
pub struct StockUnityProbe(StockUnity);
impl StockUnityProbe {
    pub fn new(node: &Node, rate: u32) -> Result<Self, String> {
        StockUnity::build(node, rate).map(Self)
    }
    pub fn step(&mut self, inputs: &[PortFrame; 12]) -> [PortFrame; 4] {
        self.0.process(inputs)
    }
    /// Rebuild with new controls, carrying running state as a graph edit does.
    pub fn edit(&mut self, node: &Node, rate: u32) -> Result<(), String> {
        let mut next = StockUnity::build(node, rate)?;
        next.adopt(&self.0);
        self.0 = next;
        Ok(())
    }
    pub fn initialize(&mut self) {
        self.0.initialize_state();
    }
    pub fn snapshot(&self) -> UnityKernel {
        self.0.snapshot()
    }
    pub fn lights(&self) -> ModuleLightState {
        self.0.lights()
    }
    pub fn merge_from_data(data: &Value) -> bool {
        merge_from_data(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(params: &[(&str, f32)]) -> Node {
        let mut node: Node =
            serde_json::from_value(json!({"id":"unity","kind":"stock_unity"})).unwrap();
        node.params = params.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        node
    }
    fn model(params: &[(&str, f32)]) -> StockUnity {
        StockUnity::build(&node(params), 48_000).unwrap()
    }
    fn inputs(values: &[(usize, f32)]) -> [PortFrame; 12] {
        let mut inputs = [PortFrame::default(); 12];
        for &(index, value) in values {
            inputs[index].channels = 1;
            inputs[index].values[0] = value;
        }
        inputs
    }
    fn outs(frames: [PortFrame; 4]) -> [f32; 4] {
        frames.map(|frame| {
            assert_eq!(frame.channels, 1);
            frame.values[0]
        })
    }

    #[test]
    fn parameters_are_validated() {
        for (name, value) in [
            ("unknown", 0.0),
            ("average1", 0.5),
            ("average2", 2.0),
            ("merge", f32::NAN),
        ] {
            assert!(StockUnity::build(&node(&[(name, value)]), 48_000).is_err());
        }
        assert!(StockUnity::build(&node(&[]), 0).is_err());
    }

    #[test]
    fn sum_average_and_inverse() {
        let patch = inputs(&[(0, 1.0), (1, 2.0), (2, 0.0), (6, 4.0)]);
        assert_eq!(outs(model(&[]).process(&patch)), [3.0, -3.0, 4.0, -4.0]);
        // A patched 0 V input still counts toward the average.
        assert_eq!(
            outs(model(&[("average1", 1.0), ("average2", 1.0)]).process(&patch)),
            [1.0, -1.0, 4.0, -4.0]
        );
        // Nothing patched: +0 on MIX and -0 on INV, even when averaging.
        let silent = outs(model(&[("average1", 1.0)]).process(&inputs(&[])));
        assert_eq!(silent.map(f32::to_bits), [0, 0x8000_0000, 0, 0x8000_0000]);
    }

    #[test]
    fn merge_joins_both_sections() {
        let patch = inputs(&[(0, 1.0), (1, 2.0), (6, 3.0)]);
        assert_eq!(
            outs(model(&[("merge", 1.0)]).process(&patch)),
            [6.0, -6.0, 6.0, -6.0]
        );
        assert_eq!(
            outs(model(&[("merge", 1.0), ("average2", 1.0)]).process(&patch)),
            [6.0, -6.0, 2.0, -2.0]
        );
        assert!(merge_from_data(&json!({"merge": true})));
        for data in [
            json!({"merge": 1}),
            json!({}),
            json!(null),
            json!({"merge": false}),
        ] {
            assert!(!merge_from_data(&data));
        }
    }

    #[test]
    fn only_the_first_channel_is_read() {
        let mut patch = inputs(&[]);
        patch[0] = PortFrame {
            channels: 4,
            values: [
                2.0, 9.0, 9.0, 9.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        };
        assert_eq!(outs(model(&[]).process(&patch))[0], 2.0);
        patch[1].channels = 1;
        patch[1].values[0] = f32::NAN;
        assert_eq!(outs(model(&[]).process(&patch))[0], 2.0);
    }

    #[test]
    fn lights_refresh_every_256_frames_and_survive_edits() {
        let mut unity = model(&[]);
        let patch = inputs(&[(0, 10.0)]);
        for _ in 0..255 {
            unity.process(&patch);
        }
        assert_eq!(unity.lights().brightness[0][0], 0.0);
        unity.process(&patch);
        let lights = unity.lights();
        assert_eq!(lights.count, 10);
        assert_eq!(
            (0..5).map(|i| lights.brightness[i][0]).collect::<Vec<_>>(),
            [1.0, 1.0, 1.0, 1.0, 1.0]
        );
        assert!((5..10).all(|i| lights.brightness[i][0] == 0.0));
        let mut next = StockUnity::build(&node(&[("merge", 1.0)]), 96_000).unwrap();
        next.adopt(&unity);
        assert_eq!(next.lights(), lights);
        assert_eq!(next.snapshot().clock, 0);
        assert!(next.snapshot().merge);
        assert_eq!(next.snapshot().sample_time, 1.0 / 96_000.0);
    }
}
