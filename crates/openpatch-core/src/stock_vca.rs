//! Original ideal VCA with polyphonic gain telemetry, qualified against pinned
//! VCV Free VCA-1. See docs/STOCK_VCA.md for arithmetic and host policies.
use crate::{parameter, ModuleLightState, Node, PortFrame, SIGNAL_LIMIT};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 2] = ["cv", "signal"];
pub(crate) const OUTPUTS: [&str; 1] = ["out"];

pub(crate) fn catalog() -> Value {
    json!({"kind":"stock_vca","title":"Stock VCA","model":"functional","inputs":INPUTS,"params":{
        "level":{"default":1.0,"min":0.0,"max":1.0,"display":{"multiplier":100.0,"unit":"%"}},
        "response":{"default":1.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1],"description":"0 = exponential (fourth power), 1 = linear"}},
        "description":"Ideal polyphonic VCA with linear or fourth-power CV response and per-channel effective gain meter."})
}
pub(crate) fn annotate(module: &mut Value) {
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["version_2"] = json!("max(1, cv width, signal width), bounded to 16; both mono inputs broadcast; missing short-poly lanes zero");
    module["polyphony"]["retired_voices"] =
        json!("stateless; inactive output and exposed gain slots clear every frame");
    module["polyphony"]["auxiliary_inputs"] =
        json!("zero-channel CV normalizes to unity; connected zero volts mutes");
    if let Some(metadata) = module["input_metadata"].as_array_mut() {
        for meta in metadata {
            meta["channel_policy"] = json!("mono broadcasts; missing short-poly lanes zero");
            meta["normalization"] = json!(if meta["name"] == "cv" { 10.0 } else { 0.0 });
        }
    }
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/VCA-1.cpp");
    module["connectivity_policy"] =
        json!("processing and gain telemetry continue with unused output");
    module["arithmetic_policy"] = json!("nonfinite input becomes zero; finite input clamps to ±1,000,000 V; source f32 order; exponential CV fourth power");
    module["light_names"] = json!([
        "gain_1", "gain_2", "gain_3", "gain_4", "gain_5", "gain_6", "gain_7", "gain_8", "gain_9",
        "gain_10", "gain_11", "gain_12", "gain_13", "gain_14", "gain_15", "gain_16"
    ]);
    module["light_policy"] = json!("count 16; input_channels is active width; first brightness component is effective gain, second zero; acquired every frame, independent of audio amplitude; fresh state one channel with zero gain");
    module["state_adoption"] = json!("same ID/kind/sample rate preserves gain telemetry; parameter edits apply immediately; new identity/rate starts fresh");
    module["initialization_policy"] = json!("initialize/reset immediately clear gain telemetry; parameter defaults are level 1 and linear response");
    module["bypass_policy"] = json!(
        "raw signal passes without CV multiplication or mono broadcast; gain telemetry freezes"
    );
}
fn voltage(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-SIGNAL_LIMIT, SIGNAL_LIMIT)
    } else {
        0.0
    }
}
#[derive(Debug)]
pub(crate) struct StockVca {
    rate: u32,
    level: f32,
    exponential: bool,
    lights: ModuleLightState,
}
impl StockVca {
    pub(crate) fn level(&self) -> f32 {
        self.level
    }
    pub(crate) fn set_level(&mut self, level: f32) {
        self.level = level;
    }
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for name in node.params.keys() {
            if name != "level" && name != "response" {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        let level = parameter(node, "level", 1.0, 0.0, 1.0)?;
        let response = parameter(node, "response", 1.0, 0.0, 1.0)?;
        if response.fract() != 0.0 {
            return Err(format!("node '{}': response must be an integer", node.id));
        }
        if rate == 0 {
            return Err("stock_vca sample rate must be positive".into());
        }
        let mut module = Self {
            rate,
            level,
            exponential: response == 0.0,
            lights: ModuleLightState::default(),
        };
        module.initialize_state();
        Ok(module)
    }
    pub(crate) fn initialize_state(&mut self) {
        self.lights = ModuleLightState {
            count: 16,
            input_channels: Some(1),
            ..ModuleLightState::default()
        };
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.rate == old.rate {
            self.lights = old.lights;
        }
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        self.lights
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame; 2]) -> PortFrame {
        let channels = inputs[0].channels.max(inputs[1].channels).clamp(1, 16);
        let mut output = PortFrame {
            channels,
            ..PortFrame::default()
        };
        self.lights.input_channels = Some(channels);
        self.lights.brightness = [[0.0; 2]; crate::MAX_MODULE_LIGHTS];
        for channel in 0..usize::from(channels) {
            let mut gain = self.level;
            if inputs[0].channels != 0 {
                let cv = (voltage(inputs[0].voice(channel, 0.0)) / 10.0).clamp(0.0, 1.0);
                // Parenthesization retains f32 rounding for each multiply.
                let cv = if self.exponential {
                    let square = cv * cv;
                    square * square
                } else {
                    cv
                };
                gain *= cv;
            }
            output.values[channel] = voltage(inputs[1].voice(channel, 0.0)) * gain;
            self.lights.brightness[channel][0] = gain;
        }
        output
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn node(params: &[(&str, f32)]) -> Node {
        let mut node: Node =
            serde_json::from_value(json!({"id":"vca","kind":"stock_vca"})).unwrap();
        node.params = params
            .iter()
            .map(|(name, value)| (name.to_string(), *value))
            .collect();
        node
    }
    fn model(params: &[(&str, f32)]) -> StockVca {
        StockVca::build(&node(params), 48_000).unwrap()
    }
    fn port(width: u8, value: f32) -> PortFrame {
        PortFrame {
            channels: width,
            values: [value; 16],
        }
    }
    #[test]
    fn validated_parameters_and_integer_modes() {
        for (name, value) in [
            ("unknown", 0.0),
            ("level", -0.01),
            ("level", 1.01),
            ("level", f32::NAN),
            ("level", f32::INFINITY),
            ("response", -1.0),
            ("response", 2.0),
            ("response", 0.5),
            ("response", f32::NAN),
            ("response", f32::INFINITY),
        ] {
            assert!(StockVca::build(&node(&[(name, value)]), 48_000).is_err());
        }
        for value in [0.0, 1.0] {
            assert!(
                StockVca::build(&node(&[("level", value), ("response", value)]), 48_000).is_ok()
            );
        }
        assert!(StockVca::build(&node(&[]), 0).is_err());
    }
    #[test]
    fn both_responses_and_absent_versus_zero_cv() {
        for (mode, gain) in [(0.0, 0.0625), (1.0, 0.5)] {
            let mut m = model(&[("response", mode)]);
            assert_eq!(
                m.process(&[port(1, 5.0), port(1, 8.0)]).values[0],
                8.0 * gain
            );
            assert_eq!(m.lights().brightness[0], [gain, 0.0]);
            assert_eq!(m.process(&[port(0, 999.0), port(1, 8.0)]).values[0], 8.0);
            assert_eq!(m.process(&[port(1, 0.0), port(1, 8.0)]).values[0], 0.0);
        }
    }
    #[test]
    fn widths_mono_broadcast_short_poly_and_padding() {
        let mut m = model(&[]);
        for cv_width in [0, 1, 3, 5, 16] {
            for audio_width in [0, 1, 3, 5, 16] {
                let out = m.process(&[port(cv_width, 5.0), port(audio_width, 8.0)]);
                let width = cv_width.max(audio_width).max(1);
                assert_eq!(out.channels, width);
                assert_eq!(m.lights().input_channels, Some(width));
                for lane in 0..16 {
                    let gain = if lane >= width as usize {
                        0.0
                    } else if cv_width == 0 {
                        1.0
                    } else if cv_width == 1 || lane < cv_width as usize {
                        0.5
                    } else {
                        0.0
                    };
                    let audio = if audio_width == 1 || lane < audio_width as usize {
                        8.0
                    } else {
                        0.0
                    };
                    assert_eq!(m.lights().brightness[lane], [gain, 0.0]);
                    assert_eq!(out.values[lane], audio * gain);
                }
            }
        }
    }
    #[test]
    fn bounds_and_nonfinite_inputs() {
        let mut m = model(&[]);
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(m.process(&[port(0, 0.0), port(1, value)]).values[0], 0.0);
            assert_eq!(m.process(&[port(1, value), port(1, 8.0)]).values[0], 0.0);
        }
        for (cv, want) in [(-1e20, 0.0), (1e20, 1.0)] {
            assert_eq!(
                m.process(&[port(1, cv), port(1, 8.0)]).values[0],
                8.0 * want
            );
        }
        for sign in [-1.0, 1.0] {
            assert_eq!(
                m.process(&[port(0, 0.0), port(1, sign * 1e20)]).values[0],
                sign * SIGNAL_LIMIT
            );
        }
    }
    #[test]
    fn silent_audio_observes_gain_and_shrinking_clears_inactive() {
        let mut m = model(&[]);
        assert_eq!(m.lights().input_channels, Some(1));
        assert_eq!(m.lights().brightness, [[0.0; 2]; crate::MAX_MODULE_LIGHTS]);
        for width in [16, 1, 5, 16] {
            let out = m.process(&[port(width, 5.0), port(0, 0.0)]);
            assert!(out.values.iter().all(|value| *value == 0.0));
            for lane in 0..16 {
                assert_eq!(
                    m.lights().brightness[lane],
                    [if lane < width as usize { 0.5 } else { 0.0 }, 0.0]
                );
            }
        }
    }
    #[test]
    fn adoption_preserves_matching_rate_and_init_clears() {
        let mut old = model(&[]);
        old.process(&[port(5, 5.0), port(1, 8.0)]);
        let mut next = model(&[("level", 0.2), ("response", 0.0)]);
        next.adopt(&old);
        assert_eq!(next.lights(), old.lights());
        assert_eq!(next.process(&[port(1, 5.0), port(1, 8.0)]).values[0], 0.1);
        let mut fresh = StockVca::build(&node(&[]), 96_000).unwrap();
        fresh.adopt(&old);
        assert_eq!(fresh.lights().input_channels, Some(1));
        assert_eq!(
            fresh.lights().brightness,
            [[0.0; 2]; crate::MAX_MODULE_LIGHTS]
        );
        next.initialize_state();
        assert_eq!(next.lights().input_channels, Some(1));
        assert_eq!(
            next.lights().brightness,
            [[0.0; 2]; crate::MAX_MODULE_LIGHTS]
        );
    }
}
