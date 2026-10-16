//! Original digital envelope equations, with fixed-capacity four-lane histories.
//! The public acceptance contract lives in docs/ADSR_EG.md.
use crate::{
    parameter, ModuleAction, ModuleControlState, ModuleLightState, Node, PortFrame,
    MAX_MODULE_ACTIONS_PER_SAMPLE, SIGNAL_LIMIT,
};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 6] = [
    "attack_cv",
    "decay_cv",
    "sustain_cv",
    "release_cv",
    "gate",
    "retrigger",
];
pub(crate) const OUTPUTS: [&str; 1] = ["out"];
const STAGES: [&str; 4] = ["attack", "decay", "sustain", "release"];
const AMOUNTS: [&str; 4] = [
    "attack_amount",
    "decay_amount",
    "sustain_amount",
    "release_amount",
];
pub(crate) fn catalog() -> Value {
    let mut params = json!({});
    for name in STAGES {
        params[name] = json!({"default":0.5,"min":0.0,"max":1.0});
        if name == "sustain" {
            params[name]["unit"] = json!("ratio");
        } else {
            params[name]["display"] =
                json!({"scale":"exponential","base":10000,"multiplier":1,"unit":"ms"});
            params[name]["description"] = json!("Time constant: 10000^value ms; midpoint 100 ms. Attack targets 1.2, so this is not time to peak.");
        }
    }
    for name in AMOUNTS {
        params[name] = json!({"default":0.0,"min":-1.0,"max":1.0,"unit":"ratio"});
    }
    json!({"kind":"adsr_eg","title":"ADSR EG","model":"functional","inputs":INPUTS,"params":params,"description":"Polyphonic digital ADSR envelope with signed A/D/S/R CV depths, retrigger and held PUSH. 1–10000 ms time constants; 12 V attack target preserves the small peak overshoot. Existing ADSR patches keep their original RC behavior."})
}
pub(crate) fn annotate(module: &mut Value) {
    module["runtime_actions"] = json!(["push_press", "push_release"]);
    module["polyphony"]["primary_input"] = json!("gate");
    module["polyphony"]["version_2"] = json!("gate width, minimum one; raw gate lanes do not broadcast; CV and retrigger mono inputs broadcast; process rounded groups of four");
    module["polyphony"]["retired_voices"] = json!("inactive whole groups freeze envelope, gate, trigger and cached coefficient histories; padding lanes in the final group continue");
    for meta in module["input_metadata"].as_array_mut().unwrap() {
        meta["channel_policy"] = json!(if meta["name"] == "gate" {
            "raw channels; mono does not broadcast"
        } else {
            "mono broadcasts; missing short-poly channels are zero"
        });
    }
    module["output_metadata"][0]["label"] = json!("ENV");
    module["light_names"] = json!(["attack", "decay", "sustain", "release", "gate"]);
    module["light_policy"] = json!({"publication_frames":128,"first_publication_frame":128,"aggregation":"Boolean over all rounded four-lane groups including padding","sustain":"sustain <= envelope < sustain + 0.01","rest":"envelope < 0.01","manual":"physical PUSH held; gate lamp means any effective gate high","input_channels":"actual gate width, 0–16, published with lights"});
    module["manual_control"] = json!({"buttons":["push"],"saved":false,"capacity_per_sample":MAX_MODULE_ACTIONS_PER_SAMPLE,"ordering":"apply queued actions in order before processing; final held state wins; same-sample press/release creates no synthetic pulse","release_safety":"release held and pending controls, preserving DSP history"});
    module["coefficient_policy"] = json!({"frames":16,"first_update_frame":16,"initial":"rates and sustain are zero","modulation":"clamp(parameter + CV / 10 * amount, 0, 1)","rate":"10000^(-value) / 0.001","integration":"f32 forward Euler; attack target 1.2; ENV = envelope * 10; no peak clamp"});
    module["gate_policy"] = json!({"gate":"raw channel >= 1 V, no hysteresis; PUSH forces every processed lane high without expanding width","retrigger":"initially high; arm at <= 0 V, rise at >= 1 V; only starts attack while gate high","nonfinite":"zero; finite inputs bounded by host voltage limits"});
    module["state_adoption"] = json!("same ID/kind/sample rate retains envelope, gate, attack, trigger, cached coefficients, lamp values and divider phases regardless of cable or parameter edits; new parameters apply on next coefficient tick; pending actions stay with receiving engine; sample rate changes start fresh");
    module["initialization_policy"] = json!("initialize and reset clear all DSP histories, cached coefficients, dividers, lamps and manual controls");
    module["bypass_policy"] = json!("mono zero output; all DSP and lights freeze; bypass transitions release held and pending controls");
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/ADSR.cpp");
}

#[derive(Clone, Copy, Debug)]
struct Lane {
    envelope: f32,
    gate: bool,
    attack: bool,
    retrigger_high: bool,
    rates: [f32; 3],
    sustain: f32,
}
impl Default for Lane {
    fn default() -> Self {
        Self {
            envelope: 0.0,
            gate: false,
            attack: false,
            retrigger_high: true,
            rates: [0.0; 3],
            sustain: 0.0,
        }
    }
}
#[derive(Debug)]
pub(crate) struct AdsrEg {
    parameters: [f32; 4],
    amounts: [f32; 4],
    sample_time: f32,
    lanes: [Lane; 16],
    coefficient_phase: u8,
    light_phase: u8,
    lights: ModuleLightState,
    held: bool,
    actions: [ModuleAction; MAX_MODULE_ACTIONS_PER_SAMPLE],
    count: usize,
}
fn voltage(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-SIGNAL_LIMIT, SIGNAL_LIMIT)
    } else {
        0.0
    }
}
impl AdsrEg {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for name in node.params.keys() {
            if !STAGES.contains(&name.as_str()) && !AMOUNTS.contains(&name.as_str()) {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        let mut parameters = [0.0; 4];
        let mut amounts = [0.0; 4];
        for index in 0..4 {
            parameters[index] = parameter(node, STAGES[index], 0.5, 0.0, 1.0)?;
            amounts[index] = parameter(node, AMOUNTS[index], 0.0, -1.0, 1.0)?;
        }
        Ok(Self {
            parameters,
            amounts,
            sample_time: 1.0 / rate as f32,
            lanes: [Lane::default(); 16],
            coefficient_phase: 0,
            light_phase: 0,
            lights: Self::dark_lights(),
            held: false,
            actions: [ModuleAction::PushRelease; MAX_MODULE_ACTIONS_PER_SAMPLE],
            count: 0,
        })
    }
    fn dark_lights() -> ModuleLightState {
        ModuleLightState {
            count: 5,
            input_channels: Some(0),
            ..ModuleLightState::default()
        }
    }
    pub(crate) fn supports(action: ModuleAction) -> bool {
        matches!(action, ModuleAction::PushPress | ModuleAction::PushRelease)
    }
    pub(crate) fn enqueue(&mut self, action: ModuleAction) -> bool {
        if !Self::supports(action) || self.count == self.actions.len() {
            return false;
        }
        self.actions[self.count] = action;
        self.count += 1;
        true
    }
    pub(crate) fn controls(&self) -> ModuleControlState {
        ModuleControlState {
            pressed: self.held,
            pressed_mask: u16::from(self.held),
        }
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        ModuleLightState {
            manual: self.held,
            ..self.lights
        }
    }
    pub(crate) fn release(&mut self) {
        self.held = false;
        self.count = 0;
    }
    pub(crate) fn initialize_state(&mut self) {
        self.reset_state();
    }
    pub(crate) fn reset_state(&mut self) {
        self.release();
        self.lanes = [Lane::default(); 16];
        self.coefficient_phase = 0;
        self.light_phase = 0;
        self.lights = Self::dark_lights();
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        self.lanes = old.lanes;
        self.held = old.held;
        self.coefficient_phase = old.coefficient_phase;
        self.light_phase = old.light_phase;
        self.lights = old.lights;
        // Parameters and pending actions belong to this receiving graph.
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame; 6], outputs: &mut [PortFrame]) {
        for action in &self.actions[..self.count] {
            self.held = *action == ModuleAction::PushPress;
        }
        self.count = 0;
        let gate_channels = inputs[4].channels.min(16);
        let channels = gate_channels.max(1);
        let processed = usize::from(channels).div_ceil(4) * 4;
        self.coefficient_phase += 1;
        let coefficients = self.coefficient_phase == 16;
        if coefficients {
            self.coefficient_phase = 0;
        }
        self.light_phase += 1;
        let publish = self.light_phase == 128;
        if publish {
            self.light_phase = 0;
        }
        let mut illuminated = [false; 5];
        let mut out = PortFrame {
            channels,
            ..PortFrame::default()
        };
        for (index, lane) in self.lanes[..processed].iter_mut().enumerate() {
            if coefficients {
                let mut modulated = [0.0; 4];
                for stage in 0..4 {
                    modulated[stage] = (self.parameters[stage]
                        + voltage(inputs[stage].voice(index, 0.0)) / 10.0 * self.amounts[stage])
                        .clamp(0.0, 1.0);
                }
                lane.rates = [0, 1, 3].map(|stage| 10000.0_f32.powf(-modulated[stage]) / 0.001);
                lane.sustain = modulated[2];
            }
            let gate = self.held || voltage(inputs[4].channel(index)) >= 1.0;
            let trigger = voltage(inputs[5].voice(index, 0.0));
            let retrigger = !lane.retrigger_high && trigger >= 1.0;
            if trigger >= 1.0 {
                lane.retrigger_high = true;
            } else if trigger <= 0.0 {
                lane.retrigger_high = false;
            }
            if gate && (!lane.gate || retrigger) {
                lane.attack = true;
            }
            if !gate {
                lane.attack = false;
            }
            lane.gate = gate;
            let (target, rate) = if gate {
                if lane.attack {
                    (1.2, lane.rates[0])
                } else {
                    (lane.sustain, lane.rates[1])
                }
            } else {
                (0.0, lane.rates[2])
            };
            lane.envelope += (target - lane.envelope) * rate * self.sample_time;
            if lane.envelope >= 1.0 {
                lane.attack = false;
            }
            if index < usize::from(channels) {
                out.values[index] = lane.envelope * 10.0;
            }
            if publish {
                let sustaining =
                    lane.envelope >= lane.sustain && lane.envelope < lane.sustain + 0.01;
                let resting = lane.envelope < 0.01;
                illuminated[0] |= gate && lane.attack;
                illuminated[1] |= gate && !lane.attack && !sustaining;
                illuminated[2] |= gate && !lane.attack && sustaining;
                illuminated[3] |= !gate && !resting;
                illuminated[4] |= gate;
            }
        }
        if publish {
            self.lights.input_channels = Some(gate_channels);
            for (light, on) in self.lights.brightness.iter_mut().zip(illuminated) {
                *light = [if on { 1.0 } else { 0.0 }, 0.0];
            }
        }
        outputs[0] = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(rate: u32, params: &[(&str, f32)]) -> AdsrEg {
        AdsrEg::build(
            &Node {
                content: None,
                score: None,
                id: "e".into(),
                kind: "adsr_eg".into(),
                bypassed: false,
                params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
            },
            rate,
        )
        .unwrap()
    }
    fn step(e: &mut AdsrEg, input: [PortFrame; 6]) -> PortFrame {
        let mut output = [PortFrame::default()];
        e.process(&input, &mut output);
        assert!(output[0].values.iter().all(|v| v.is_finite()));
        assert!(output[0].values[usize::from(output[0].channels)..]
            .iter()
            .all(|v| *v == 0.0));
        output[0]
    }
    fn gate(channels: u8, volts: f32) -> [PortFrame; 6] {
        let mut input = [PortFrame::default(); 6];
        input[4].channels = channels;
        input[4].values.fill(volts);
        input
    }
    #[test]
    fn cold_start_has_no_warmup_and_first_coefficient_tick_is_sample_sixteen() {
        for rate in [44_100, 48_000, 96_000] {
            let mut e = model(rate, &[]);
            for _ in 0..15 {
                assert_eq!(step(&mut e, gate(1, 1.0)).values[0], 0.0);
            }
            let expected = 1.2_f32 * (10000.0_f32.powf(-0.5) / 0.001) * (1.0 / rate as f32) * 10.0;
            assert_eq!(step(&mut e, gate(1, 1.0)).values[0], expected);
            assert_eq!(e.coefficient_phase, 0);
            assert_eq!(e.lanes[0].sustain, 0.5);
            assert!(e.lanes[0].attack);
        }
    }
    #[test]
    fn time_endpoints_preserve_peak_overshoot_and_decay_release_are_asymptotic() {
        for rate in [44_100, 48_000, 96_000] {
            let mut e = model(
                rate,
                &[
                    ("attack", 0.0),
                    ("decay", 0.0),
                    ("release", 0.0),
                    ("sustain", 0.25),
                ],
            );
            let mut peak = 0.0_f32;
            for _ in 0..3000 {
                peak = peak.max(step(&mut e, gate(1, 10.0)).values[0]);
            }
            assert!(peak > 10.0 && peak < 10.25, "{rate}: {peak}");
            assert!(e.lanes[0].envelope >= 0.25 && e.lanes[0].envelope < 0.25001);
            let before = e.lanes[0].envelope;
            let after = step(&mut e, gate(1, 0.99)).values[0] / 10.0;
            assert!(after > 0.0 && after < before);
            assert!(!e.lanes[0].gate);
            let mut slow = model(rate, &[("attack", 1.0)]);
            for _ in 0..16 {
                step(&mut slow, gate(1, 1.0));
            }
            assert!((slow.lanes[0].rates[0] - 0.1).abs() < 1e-7);
        }
    }
    #[test]
    fn raw_gate_and_poly_broadcast_have_distinct_lane_semantics() {
        let mut e = model(48_000, &[("attack_amount", 1.0), ("sustain_amount", -1.0)]);
        let mut input = gate(5, 0.0);
        input[4].values[0] = 1.0;
        input[0] = PortFrame::mono(5.0);
        input[2] = PortFrame {
            channels: 2,
            ..PortFrame::mono(5.0)
        };
        input[2].values[1] = -5.0;
        for _ in 0..16 {
            step(&mut e, input);
        }
        for lane in &e.lanes[..8] {
            assert!((lane.rates[0] - 0.1).abs() < 1e-7);
        }
        assert_eq!(e.lanes[0].sustain, 0.0);
        assert_eq!(e.lanes[1].sustain, 1.0);
        assert_eq!(e.lanes[2].sustain, 0.5);
        assert!(e.lanes[0].envelope > 0.0);
        assert!(e.lanes[1..].iter().all(|l| l.envelope == 0.0));
        let mut mono = model(48_000, &[]);
        for _ in 0..16 {
            step(&mut mono, gate(1, 10.0));
        }
        assert!(mono.lanes[0].gate);
        assert!(mono.lanes[1..4].iter().all(|l| !l.gate));
        assert_eq!(step(&mut mono, gate(0, 0.0)).channels, 1);
    }
    #[test]
    fn retrigger_is_initially_high_arms_at_zero_and_only_fires_while_gated() {
        let mut e = model(48_000, &[("attack", 0.0)]);
        let mut input = gate(1, 1.0);
        input[5] = PortFrame::mono(1.0);
        for _ in 0..300 {
            step(&mut e, input);
        }
        assert!(!e.lanes[0].attack);
        for volts in [0.5, 0.01, 0.99, 1.0] {
            input[5] = PortFrame::mono(volts);
            step(&mut e, input);
            assert!(!e.lanes[0].attack);
        }
        input[5] = PortFrame::mono(0.0);
        step(&mut e, input);
        input[5] = PortFrame::mono(0.99);
        step(&mut e, input);
        assert!(!e.lanes[0].attack);
        input[5] = PortFrame::mono(1.0);
        step(&mut e, input);
        assert!(e.lanes[0].attack);
        input[4] = PortFrame::mono(0.0);
        input[5] = PortFrame::mono(0.0);
        step(&mut e, input);
        assert!(!e.lanes[0].attack);
        input[5] = PortFrame::mono(1.0);
        step(&mut e, input);
        assert!(!e.lanes[0].attack);
    }
    #[test]
    fn inactive_whole_groups_freeze_but_padding_lanes_keep_processing() {
        let mut e = model(48_000, &[]);
        e.enqueue(ModuleAction::PushPress);
        for _ in 0..48 {
            step(&mut e, gate(9, 0.0));
        }
        let frozen = e.lanes[8];
        for _ in 0..48 {
            step(&mut e, gate(5, 0.0));
        }
        assert_eq!(e.lanes[8].envelope, frozen.envelope);
        assert_eq!(e.lanes[8].retrigger_high, frozen.retrigger_high);
        assert_eq!(e.lanes[8].rates, frozen.rates);
        assert!(e.lanes[7].envelope > frozen.envelope);
        step(&mut e, gate(9, 0.0));
        assert!(e.lanes[8].envelope > frozen.envelope);
        assert_eq!(e.lanes[12].envelope, 0.0);
    }
    #[test]
    fn lamps_publish_at_128_include_padding_and_freeze_between_ticks() {
        let mut e = model(48_000, &[]);
        e.enqueue(ModuleAction::PushPress);
        for _ in 0..127 {
            step(&mut e, gate(1, 0.0));
            assert_eq!(e.lights().brightness, [[0.0; 2]; crate::MAX_MODULE_LIGHTS]);
        }
        step(&mut e, gate(1, 0.0));
        assert_eq!(e.lights().count, 5);
        assert_eq!(e.lights().input_channels, Some(1));
        assert_eq!(e.lights().brightness[0], [1.0, 0.0]);
        assert_eq!(e.lights().brightness[4], [1.0, 0.0]);
        assert!(e.lights().manual);
        // A nonzero gate lane contributes to the aggregate lamp.
        e.release();
        let mut input = gate(1, 0.0);
        input[4].channels = 3;
        input[4].values[2] = 1.0;
        for _ in 0..128 {
            step(&mut e, input);
        }
        assert_eq!(e.lights().brightness[4], [1.0, 0.0]);
        let lamps = e.lights();
        step(&mut e, gate(0, 0.0));
        assert_eq!(e.lights(), lamps);
        // With no gate wire, held PUSH still raises all four processed lanes.
        e.enqueue(ModuleAction::PushPress);
        for _ in 0..128 {
            assert_eq!(step(&mut e, gate(0, 0.0)).channels, 1);
        }
        assert!(e.lanes[..4].iter().all(|l| l.gate));
        assert_eq!(e.lights().input_channels, Some(0));
    }
    #[test]
    fn same_sample_manual_actions_only_use_final_state_and_release_preserves_tail() {
        let mut e = model(48_000, &[]);
        e.enqueue(ModuleAction::PushPress);
        e.enqueue(ModuleAction::PushRelease);
        step(&mut e, gate(0, 0.0));
        assert!(!e.lanes[0].attack && !e.controls().pressed);
        e.enqueue(ModuleAction::PushRelease);
        e.enqueue(ModuleAction::PushPress);
        for _ in 0..40 {
            step(&mut e, gate(0, 0.0));
        }
        let before = e.lanes[0].envelope;
        e.enqueue(ModuleAction::PushPress);
        e.release();
        step(&mut e, gate(0, 0.0));
        assert!(!e.controls().pressed && !e.lights().manual);
        assert!(e.lanes[0].envelope > 0.0 && e.lanes[0].envelope < before);
    }
    #[test]
    fn adoption_keeps_all_history_and_receivers_actions_but_uses_new_parameters_at_tick() {
        let mut old = model(48_000, &[]);
        old.enqueue(ModuleAction::PushPress);
        for _ in 0..29 {
            step(&mut old, gate(5, 0.0));
        }
        old.enqueue(ModuleAction::PushPress);
        let mut new = model(48_000, &[("attack", 0.0)]);
        new.enqueue(ModuleAction::PushRelease);
        new.adopt(&old);
        assert_eq!(new.lanes[0].envelope, old.lanes[0].envelope);
        assert_eq!(new.coefficient_phase, 13);
        assert_eq!(new.light_phase, 29);
        assert_eq!(new.lights, old.lights);
        step(&mut new, gate(1, 1.0));
        assert!(!new.held);
        assert_eq!(new.lanes[0].rates, old.lanes[0].rates);
        step(&mut new, gate(1, 1.0));
        step(&mut new, gate(1, 1.0));
        assert!((new.lanes[0].rates[0] - 1000.0).abs() < 0.0001);
        assert_eq!(new.lanes[4].rates, old.lanes[4].rates);
    }
    #[test]
    fn queue_capacity_reset_and_initialize_clear_every_history() {
        let mut e = model(48_000, &[]);
        assert!(!e.enqueue(ModuleAction::HoldPress));
        for _ in 0..MAX_MODULE_ACTIONS_PER_SAMPLE {
            assert!(e.enqueue(ModuleAction::PushPress));
        }
        assert!(!e.enqueue(ModuleAction::PushRelease));
        for _ in 0..128 {
            step(&mut e, gate(16, 10.0));
        }
        e.enqueue(ModuleAction::PushPress);
        e.initialize_state();
        assert_eq!(e.count, 0);
        assert!(!e.held);
        assert_eq!(e.coefficient_phase, 0);
        assert_eq!(e.light_phase, 0);
        assert_eq!(e.lights, AdsrEg::dark_lights());
        for l in e.lanes {
            assert_eq!(l.envelope, 0.0);
            assert_eq!(l.rates, [0.0; 3]);
            assert_eq!(l.sustain, 0.0);
            assert!(l.retrigger_high);
            assert!(!l.attack && !l.gate);
        }
        assert_eq!(step(&mut e, gate(16, 10.0)).values, [0.0; 16]);
    }
    #[test]
    fn nonfinite_inputs_become_zero_and_finite_extremes_clamp_modulation() {
        let mut e = model(
            48_000,
            &[
                ("attack_amount", -1.0),
                ("decay_amount", 1.0),
                ("sustain_amount", 1.0),
                ("release_amount", 1.0),
            ],
        );
        let mut input = gate(16, 10.0);
        input[0] = PortFrame::mono(f32::INFINITY);
        input[1] = PortFrame::mono(f32::MAX);
        input[2] = PortFrame::mono(f32::NAN);
        input[3] = PortFrame::mono(-f32::MAX);
        input[5] = PortFrame::mono(f32::NEG_INFINITY);
        for _ in 0..1000 {
            step(&mut e, input);
        }
        assert_eq!(
            e.lanes[0].rates,
            [0.5, 1.0, 0.0].map(|value| 10000.0_f32.powf(-value) / 0.001)
        );
        assert_eq!(e.lanes[0].sustain, 0.5);
        input[4] = PortFrame::mono(f32::NAN);
        step(&mut e, input);
        assert!(!e.lanes[0].gate);
    }
    #[test]
    fn stage_lamp_boundaries_and_hidden_release_lane_are_exact() {
        for (envelope, gate_high, attacking, expected) in [
            (0.0, true, true, [true, false, false, false, true]),
            (0.4999, true, false, [false, true, false, false, true]),
            (0.5, true, false, [false, false, true, false, true]),
            (0.50999, true, false, [false, false, true, false, true]),
            (0.51, true, false, [false, true, false, false, true]),
            (0.01, false, false, [false, false, false, true, false]),
            (0.00999, false, false, [false, false, false, false, false]),
        ] {
            let mut e = model(48_000, &[]);
            e.light_phase = 127;
            e.lanes[0].envelope = envelope;
            e.lanes[0].gate = gate_high;
            e.lanes[0].attack = attacking;
            e.lanes[0].sustain = 0.5;
            step(&mut e, gate(1, if gate_high { 1.0 } else { 0.0 }));
            for (index, on) in expected.into_iter().enumerate() {
                assert_eq!(
                    e.lights().brightness[index][0] == 1.0,
                    on,
                    "env {envelope}, lamp {index}"
                );
            }
        }
        let mut e = model(48_000, &[]);
        e.light_phase = 127;
        e.lanes[3].envelope = 0.5;
        let output = step(&mut e, gate(1, 0.0));
        assert_eq!(output, PortFrame::mono(0.0));
        assert_eq!(e.lights().brightness[3], [1.0, 0.0]);
        assert_eq!(e.lights().input_channels, Some(1));
    }
}
