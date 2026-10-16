//! Original digital implementation of the pinned Fundamental Process behavior.
//! The common host owns raw ports, cable delays and output connection lifecycle.
use crate::host_ports::HostPortSnapshot;
use crate::native_float::{flush_tiny, rack_add, rack_div, rack_mul, rack_sub};
use crate::{
    ModuleAction, ModuleControlState, ModuleLightState, Node, MAX_MODULE_ACTIONS_PER_SAMPLE,
};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 3] = ["slew_cv", "signal", "gate"];
pub(crate) const OUTPUTS: [&str; 6] = ["sh1", "sh2", "track_hold", "hold_track", "slew", "glide"];

fn minimum_slew() -> f32 {
    1e-3_f32.log2()
}

fn maximum_slew() -> f32 {
    10.0_f32.log2()
}

pub(crate) fn catalog() -> Value {
    json!({"kind":"stock_process","title":"STOCK PROCESS","model":"functional",
        "description":"Fundamental digital sample and hold, track and hold, slew and glide behavior. The existing process utility remains independent.",
        "inputs":INPUTS,
        "params":{
            "slew":{"default":minimum_slew(),"min":minimum_slew(),"max":maximum_slew(),
                "raw_min":-3.4028234663852886e38,"raw_max":3.4028234663852886e38,
                "unit":"log2(s/V)","description":"Log2 seconds per volt; the hard-left value and any lower raw OpenPatch value select infinite slew rate. The source display uses milliseconds per volt. Rack's normal parameter JSON reload clamps to the authored range."}
        }
    })
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Process.cpp");
    module["min_patch_version"] = json!(2);
    module["polyphony"]["primary_input"] = json!("signal");
    module["polyphony"]["version_2"] = json!("exact raw signal input width, including zero");
    module["polyphony"]["auxiliary_inputs"] = json!("slew_cv and gate use getPolyVoltage: one channel broadcasts, otherwise raw same-index lane is read even when inactive; neither adds voices");
    module["polyphony"]["retired_voices"] = json!("all six outputs write every active lane before SDK setChannels; zero signal channels freeze every engine; lane shrink preserves retired engine histories for later regrowth");
    module["arithmetic_policy"] = json!("source-order f32 under Rack signed FTZ/DAZ; slew uses SDK exp2_taylor5((-slewPitch)+30)/2^30 then sampleTime; four independent SlewFilter values per lane; no generic voltage clamp");
    module["manual_control"] = json!({"buttons":["gate"],"actions":["gate_press","gate_release"],"capacity_per_sample":MAX_MODULE_ACTIONS_PER_SAMPLE,
        "ordering":"ordered actions are applied before gate/CV evaluation; final action of the sample wins",
        "release_safety":"explicit release clears pending manual actions and the held button; bypass transition does not release raw GATE_PARAM",
        "saved":false,"parameter":"GATE_PARAM", "threshold":"raw parameter > 0; runtime press and release map to 1 and 0", "persistence_policy":"OpenPatch does not persist the momentary gate; Rack base parameter JSON may serialize it"});
    module["light_names"] = json!(["gate"]);
    module["light_policy"] = json!("the single actual gate light equals the button's >0 state on every callback, including zero signal channels; it does not reflect gate CV or engine states");
    module["initialization_policy"] = json!("reset restores the slew parameter and releases the momentary gate; preserve engine histories, light and canonical raw ports");
    module["state_adoption"] = json!("compatible same-rate receiving incarnation retains all sixteen engine histories, raw momentary gate and current gate light; receiving slew wins. A cross-rate graph rebuild starts cold; a bounded in-place Process sample-rate event preserves histories and host storage");
}

pub(crate) fn annotate_bypass(module: &mut Value) {
    module["bypass"]["output_policy"] = json!("No source configBypass routes or Process output writes; connected outputs follow canonical host bypass-transition lifecycle and disconnected raw storage remains");
    module["bypass"]["state_policy"] = json!("all sixteen Process engines and gate light freeze while bypassed; raw momentary GATE_PARAM remains held across the transition");
    module["bypass"]["module_actions"] = json!("manual gate presses reject while bypassed; an explicit gate_release clears the button, but a bypass transition itself does not release it");
    module["bypass"]["compatibility"] = json!("Fundamental Process has no configBypass route; common native host lifecycle handles physical output transitions");
}

/// A complete copy of one source Engine, with each SlewFilter represented by
/// its only stored field (`value`). Inactive lanes remain inspectable.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct StockProcessEngineSnapshot {
    pub state: bool,
    pub on_time: f32,
    pub sample1: f32,
    pub sample2: f32,
    pub sample1_filter: f32,
    pub sample2_filter: f32,
    pub hold_value: f32,
    pub slew_filter: f32,
    pub glide_filter: f32,
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct StockProcessSnapshot {
    pub slew_param: f32,
    pub gate_param: f32,
    pub engines: [StockProcessEngineSnapshot; 16],
    pub gate_light_brightness: f32,
    pub sample_rate: u32,
    pub sample_time: f32,
}

/// Lossless diagnostic shape for independent Rack trace comparison.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct StockProcessEngineBits {
    pub state: bool,
    pub on_time_bits: u32,
    pub sample1_bits: u32,
    pub sample2_bits: u32,
    pub sample1_filter_bits: u32,
    pub sample2_filter_bits: u32,
    pub hold_bits: u32,
    pub slew_filter_bits: u32,
    pub glide_filter_bits: u32,
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct StockProcessSnapshotBits {
    pub parameter_bits: [u32; 2],
    pub light_bits: [u32; 1],
    pub engines: [StockProcessEngineBits; 16],
    pub sample_rate: u32,
    pub sample_time_bits: u32,
}

impl StockProcessSnapshot {
    pub fn bits(self) -> StockProcessSnapshotBits {
        StockProcessSnapshotBits {
            parameter_bits: [self.slew_param.to_bits(), self.gate_param.to_bits()],
            light_bits: [self.gate_light_brightness.to_bits()],
            engines: self.engines.map(|engine| StockProcessEngineBits {
                state: engine.state,
                on_time_bits: engine.on_time.to_bits(),
                sample1_bits: engine.sample1.to_bits(),
                sample2_bits: engine.sample2.to_bits(),
                sample1_filter_bits: engine.sample1_filter.to_bits(),
                sample2_filter_bits: engine.sample2_filter.to_bits(),
                hold_bits: engine.hold_value.to_bits(),
                slew_filter_bits: engine.slew_filter.to_bits(),
                glide_filter_bits: engine.glide_filter.to_bits(),
            }),
            sample_rate: self.sample_rate,
            sample_time_bits: self.sample_time.to_bits(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct StockProcess {
    state: StockProcessSnapshot,
    actions: [ModuleAction; MAX_MODULE_ACTIONS_PER_SAMPLE],
    action_count: usize,
}

/// `dsp::exp2_taylor5` in Rack 2.6.6 first makes an exponent bit pattern from
/// `x + 127`, then evaluates a fifth-order polynomial with ordered f32 Horner
/// operations. No source-defined pitch or slew CV clamp is inserted here.
#[allow(clippy::excessive_precision)]
fn exp2_taylor5(x: f32) -> f32 {
    let shifted = rack_add(x, 127.0);
    // Rack C++ leaves out-of-range float-to-int conversion undefined. Rust's
    // saturation makes this diagnostic boundary deterministic without changing
    // the normal, source-defined domain.
    let exponent = shifted as i32;
    let fraction = rack_sub(shifted, exponent as f32);
    let scale = f32::from_bits((exponent as u32).wrapping_shl(23));
    let coefficients = [
        1.0_f32,
        0.69315169353961_f32,
        0.2401595990753_f32,
        0.055817908652_f32,
        0.008991698010_f32,
        0.001879100722_f32,
    ];
    let mut polynomial = coefficients[5];
    for coefficient in coefficients[..5].iter().rev() {
        polynomial = rack_add(*coefficient, rack_mul(polynomial, fraction));
    }
    rack_mul(scale, polynomial)
}

/// Rack math::clamp(x, a, b) is fmax(fmin(x, b), a). Rust min/max carry the
/// same NaN behavior for the source's finite bounds and preserve the range's
/// endpoint signs. The caller does not sanitize an unusual computed slew.
fn slew_step(value: &mut f32, input: f32, slew_delta: f32) -> f32 {
    let distance = rack_sub(input, *value);
    let change = distance.min(slew_delta).max(-slew_delta);
    *value = rack_add(*value, change);
    *value
}

fn poly_voltage(port: &HostPortSnapshot, lane: usize) -> f32 {
    port.values[if port.channels == 1 { 0 } else { lane }]
}

impl StockProcess {
    pub(crate) fn build(node: &Node, sample_rate: u32) -> Result<Self, String> {
        if sample_rate == 0 {
            return Err("stock_process sample rate must be positive".into());
        }
        for key in node.params.keys() {
            if key != "slew" {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let slew_param = node
            .params
            .get("slew")
            .copied()
            .unwrap_or_else(minimum_slew);
        if !slew_param.is_finite() {
            return Err(format!(
                "node '{}': parameter 'slew' must be finite",
                node.id
            ));
        }
        Ok(Self {
            state: StockProcessSnapshot {
                slew_param,
                gate_param: 0.0,
                engines: [StockProcessEngineSnapshot::default(); 16],
                gate_light_brightness: 0.0,
                sample_rate,
                sample_time: rack_div(1.0, sample_rate as f32),
            },
            actions: [ModuleAction::GateRelease; MAX_MODULE_ACTIONS_PER_SAMPLE],
            action_count: 0,
        })
    }

    pub(crate) fn snapshot(&self) -> StockProcessSnapshot {
        self.state
    }

    pub(crate) fn legacy_data(&self) -> Value {
        Value::Null
    }

    pub(crate) fn load_legacy_data(&mut self, _value: &Value) {}

    pub(crate) fn set_sample_rate(&mut self, sample_rate: u32) -> Result<(), String> {
        if sample_rate == 0 {
            return Err("stock_process sample rate must be positive".into());
        }
        self.state.sample_rate = sample_rate;
        self.state.sample_time = rack_div(1.0, sample_rate as f32);
        Ok(())
    }

    pub(crate) fn set_gate_pressed(&mut self, pressed: bool) {
        self.state.gate_param = if pressed { 1.0 } else { 0.0 };
    }

    pub(crate) fn enqueue(&mut self, action: ModuleAction) -> bool {
        if !matches!(action, ModuleAction::GatePress | ModuleAction::GateRelease)
            || self.action_count == self.actions.len()
        {
            return false;
        }
        self.actions[self.action_count] = action;
        self.action_count += 1;
        true
    }

    pub(crate) fn release(&mut self) {
        self.action_count = 0;
        self.set_gate_pressed(false);
    }

    pub(crate) fn controls(&self) -> ModuleControlState {
        let pressed = flush_tiny(self.state.gate_param) > 0.0;
        ModuleControlState {
            pressed,
            pressed_mask: u16::from(pressed),
        }
    }

    /// Diagnostic equivalent of writing Rack's raw momentary GATE_PARAM.
    /// Normal UI actions should use `set_gate_pressed`.
    #[doc(hidden)]
    pub(crate) fn set_gate_value(&mut self, value: f32) {
        self.state.gate_param = value;
    }

    pub(crate) fn reset(&mut self) {
        self.state.slew_param = minimum_slew();
        self.release();
    }

    pub(crate) fn adopt(&mut self, old: &Self) {
        self.state.engines = old.state.engines;
        self.state.gate_param = old.state.gate_param;
        self.state.gate_light_brightness = old.state.gate_light_brightness;
    }

    pub(crate) fn recreate(&mut self) {
        self.state.engines = [StockProcessEngineSnapshot::default(); 16];
        self.state.gate_param = 0.0;
        self.state.gate_light_brightness = 0.0;
        self.action_count = 0;
    }

    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut result = ModuleLightState {
            count: 1,
            ..ModuleLightState::default()
        };
        result.brightness[0][0] = self.state.gate_light_brightness;
        result.manual = self.controls().pressed;
        result
    }

    pub(crate) fn process(
        &mut self,
        inputs: &[HostPortSnapshot; 16],
        outputs: &mut [HostPortSnapshot],
    ) {
        for action in &self.actions[..self.action_count] {
            self.state.gate_param = if *action == ModuleAction::GatePress {
                1.0
            } else {
                0.0
            };
        }
        self.action_count = 0;
        let channels = inputs[1].channels;
        let gate_button = flush_tiny(self.state.gate_param) > 0.0;
        let mut slew_param = self.state.slew_param;
        if slew_param <= minimum_slew() {
            slew_param = f32::NEG_INFINITY;
        }

        for lane in 0..usize::from(channels) {
            let engine = &mut self.state.engines[lane];
            let input = inputs[1].values[lane];
            let gate_value = poly_voltage(&inputs[2], lane);
            let mut slew = f32::INFINITY;
            if slew_param.is_finite() {
                let slew_pitch = rack_add(slew_param, poly_voltage(&inputs[0], lane));
                let negative_pitch = f32::from_bits(slew_pitch.to_bits() ^ 0x8000_0000);
                let exponent = rack_add(negative_pitch, 30.0);
                slew = rack_div(exp2_taylor5(exponent), 1_073_741_824.0);
            }
            let slew_delta = rack_mul(slew, self.state.sample_time);

            if !engine.state {
                if gate_value >= 2.0 || gate_button {
                    engine.state = true;
                    engine.on_time = 0.0;
                    engine.hold_value = input;
                    engine.sample2 = engine.sample1;
                    engine.sample1 = input;
                }
            } else if gate_value <= 0.1 && !gate_button {
                engine.state = false;
                engine.hold_value = input;
            }
            let track_hold = if engine.state {
                engine.hold_value
            } else {
                input
            };
            let hold_track = if engine.state {
                input
            } else {
                engine.hold_value
            };

            if engine.state {
                engine.slew_filter = input;
                engine.on_time = rack_add(engine.on_time, self.state.sample_time);
            } else {
                slew_step(&mut engine.slew_filter, input, slew_delta);
            }
            if engine.state && engine.on_time > 1e-3_f32 {
                slew_step(&mut engine.glide_filter, input, slew_delta);
            } else {
                engine.glide_filter = input;
            }

            outputs[0].values[lane] =
                slew_step(&mut engine.sample1_filter, engine.sample1, slew_delta);
            outputs[1].values[lane] =
                slew_step(&mut engine.sample2_filter, engine.sample2, slew_delta);
            outputs[2].values[lane] = track_hold;
            outputs[3].values[lane] = hold_track;
            outputs[4].values[lane] = engine.slew_filter;
            outputs[5].values[lane] = engine.glide_filter;
        }
        for output in &mut outputs[..6] {
            output.set_channels(channels);
        }
        self.state.gate_light_brightness = if gate_button { 1.0 } else { 0.0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(rate: u32, slew: f32) -> StockProcess {
        let node: Node = serde_json::from_value(json!({
            "id":"process", "kind":"stock_process", "params":{"slew":slew}
        }))
        .unwrap();
        StockProcess::build(&node, rate).unwrap()
    }

    fn ports() -> [HostPortSnapshot; 16] {
        [HostPortSnapshot::default(); 16]
    }

    fn outputs(channels: u8) -> [HostPortSnapshot; 6] {
        [HostPortSnapshot {
            channels,
            ..HostPortSnapshot::default()
        }; 6]
    }

    #[test]
    fn gate_edges_update_sample_chain_and_track_hold_in_source_order() {
        let mut process = model(1_000, 0.0);
        let mut input = ports();
        input[1].channels = 1;
        let mut output = outputs(1);

        input[1].values[0] = 3.0;
        input[2].channels = 1;
        input[2].values[0] = 2.0;
        process.process(&input, &mut output);
        let first = process.snapshot().engines[0];
        assert!(first.state);
        assert_eq!(
            (first.sample1, first.sample2, first.hold_value),
            (3.0, 0.0, 3.0)
        );
        assert_eq!(first.on_time, 0.001);
        assert_eq!((output[2].values[0], output[3].values[0]), (3.0, 3.0));
        assert_eq!(first.glide_filter, 3.0);

        input[1].values[0] = 5.0;
        input[2].values[0] = 0.5; // Hysteresis retains high.
        process.process(&input, &mut output);
        let legato = process.snapshot().engines[0];
        assert_eq!(legato.on_time, 0.002);
        assert_eq!(legato.glide_filter, 3.001);
        assert_eq!((output[2].values[0], output[3].values[0]), (3.0, 5.0));

        input[1].values[0] = 7.0;
        input[2].values[0] = 0.1;
        process.process(&input, &mut output);
        let fallen = process.snapshot().engines[0];
        assert!(!fallen.state);
        assert_eq!(fallen.hold_value, 7.0);
        assert_eq!(fallen.sample1, 3.0);
        assert_eq!(fallen.on_time, 0.002);
        assert_eq!((output[2].values[0], output[3].values[0]), (7.0, 7.0));

        input[1].values[0] = 9.0;
        input[2].values[0] = 2.0;
        process.process(&input, &mut output);
        let again = process.snapshot().engines[0];
        assert_eq!(
            (again.sample1, again.sample2, again.hold_value),
            (9.0, 3.0, 9.0)
        );
        assert_eq!(again.on_time, 0.001);
    }

    #[test]
    fn raw_button_threshold_and_light_are_independent_of_cv_and_voice_width() {
        let mut process = model(48_000, minimum_slew());
        let mut input = ports();
        let mut output = outputs(1);
        input[1].channels = 1;
        input[1].values[0] = 4.0;
        input[2].channels = 1;
        input[2].values[0] = 2.0;
        process.process(&input, &mut output);
        assert!(process.snapshot().engines[0].state);
        assert_eq!(process.snapshot().gate_light_brightness, 0.0);

        input[2].values[0] = 0.0;
        process.set_gate_value(0.25);
        process.process(&input, &mut output);
        assert!(process.snapshot().engines[0].state);
        assert_eq!(process.snapshot().gate_light_brightness, 1.0);
        assert_eq!(process.snapshot().gate_param, 0.25);

        input[1].channels = 0;
        let frozen = process.snapshot().engines;
        process.set_gate_pressed(false);
        process.process(&input, &mut output);
        assert_eq!(process.snapshot().engines, frozen);
        assert_eq!(process.snapshot().gate_light_brightness, 0.0);
        assert!(output.iter().all(|port| port.channels == 1));
        assert!(output.iter().all(|port| port.values[0] == 0.0));
    }

    #[test]
    fn raw_auxiliary_lanes_and_regrowth_preserve_distinct_engine_histories() {
        let mut process = model(1_000, minimum_slew());
        let mut input = ports();
        let mut output = outputs(0);
        input[1].channels = 3;
        input[1].values[..3].copy_from_slice(&[1.0, 2.0, 3.0]);
        input[2].channels = 2;
        input[2].values[..3].copy_from_slice(&[0.0, 0.0, 2.0]);
        process.process(&input, &mut output);
        assert!(!process.snapshot().engines[0].state);
        assert!(!process.snapshot().engines[1].state);
        assert!(process.snapshot().engines[2].state);
        assert_eq!(process.snapshot().engines[2].sample1, 3.0);
        assert_eq!(output[3].values[2], 3.0);
        assert!(output.iter().all(|port| port.channels == 0));

        input[1].channels = 1;
        input[2].values[2] = 0.0;
        let retired = process.snapshot().engines[2];
        process.process(&input, &mut output);
        assert_eq!(process.snapshot().engines[2], retired);

        input[1].channels = 3;
        input[1].values[2] = 8.0;
        process.process(&input, &mut output);
        let resumed = process.snapshot().engines[2];
        assert!(!resumed.state);
        assert_eq!(resumed.sample1, 3.0);
        assert_eq!(resumed.hold_value, 8.0);
        assert_eq!(output[0].values[2], 3.0);
    }

    #[test]
    fn sample_rate_and_adoption_keep_filter_history_without_overwriting_controls() {
        let mut old = model(1_000, 0.0);
        let mut input = ports();
        let mut output = outputs(1);
        input[1].channels = 1;
        input[1].values[0] = 2.0;
        old.process(&input, &mut output);
        old.set_gate_value(0.25);
        let before = old.snapshot().engines;
        old.set_sample_rate(48_000).unwrap();
        assert_eq!(old.snapshot().engines, before);
        assert_eq!(old.snapshot().sample_rate, 48_000);
        assert!(old.set_sample_rate(0).is_err());
        assert_eq!(old.snapshot().sample_rate, 48_000);

        let mut receiving = model(96_000, -10.0);
        receiving.set_gate_pressed(true);
        receiving.adopt(&old);
        assert_eq!(receiving.snapshot().engines, before);
        assert_eq!(receiving.snapshot().slew_param, -10.0);
        assert_eq!(receiving.snapshot().gate_param, 0.25);
        assert_eq!(receiving.snapshot().sample_rate, 96_000);
        receiving.reset();
        assert_eq!(receiving.snapshot().engines, before);
        assert_eq!(receiving.snapshot().slew_param, minimum_slew());
        assert_eq!(receiving.snapshot().gate_param, 0.0);
        assert_eq!(
            receiving.snapshot().bits().engines[0].slew_filter_bits,
            before[0].slew_filter.to_bits()
        );
    }
}
