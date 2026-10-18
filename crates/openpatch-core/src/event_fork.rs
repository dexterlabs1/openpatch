//! Two native Bernoulli routers using the pinned Branches firmware decision loop.
//! The source kernel is GPL-3.0-or-later. Voltage/ADC mapping, per-host-frame
//! scanning and cable normaling below are explicit OpenPatch adaptations.
mod kernel;

use crate::{parameter, ModuleLightState, Node, PortFrame};
use kernel::{adc_threshold, Kernel};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 4] = ["gate1", "gate2", "probability1_cv", "probability2_cv"];
pub(crate) const OUTPUTS: [&str; 4] = ["a1", "b1", "a2", "b2"];

pub(crate) fn catalog() -> Value {
    let mut module = json!({"kind":"event_fork","title":"EVENT FORK","model":"functional",
        "tags":["Custom","Random","Logic","Gates"],"width_hp":20,"inputs":INPUTS,
        "description":"Two source-derived Bernoulli gate routers with probability CV, toggle decisions, latch memory and a normalled second input.",
        "params":{"seed":{"default":1,"min":1,"max":16777215,"step":1,"unit":"seed"}}});
    for channel in 1..=2 {
        module["params"][format!("probability{channel}")] = json!({"default":0.5,"min":0,"max":1,"unit":"probability","description":"Probability of B in direct mode, or switching output in toggle mode. Native 8-bit ADC quantization preserves the source endpoint plateaus."});
        for (name, description) in [
            (
                "toggle",
                "A means stay on the previous output; B means switch.",
            ),
            (
                "latch",
                "Keep the last selected output high after the input falls.",
            ),
        ] {
            module["params"][format!("{name}{channel}")] = json!({"default":0,"min":0,"max":1,"step":1,"values":[0,1],"labels":["Off","On"],"description":description});
        }
    }
    module
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://github.com/pichenettes/eurorack/blob/f4cfa7697095a28d292bff758beb51deb49b5a86/branches/branches.cc");
    module["polyphony"]["maximum_channels"] = json!(1);
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["version_2"] = json!("Two physical mono sections. Inputs read channel zero; outputs are mono. An unpatched gate2 reads gate1; a physical cable, even with zero channels, breaks this normal.");
    for output in module["output_metadata"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        output["maximum_channels"] = json!(1);
    }
    module["probability_policy"] = json!("pB=clamp(knob+finite(CV channel0)/5,0,1); ADC=floor((1-pB)*255), then the unchanged 256-word source threshold table. Output B is random>=threshold except threshold65535 always selects A. In toggle mode B flips the previous decision. Nonfinite input volts read zero.");
    module["clock_policy"] = json!("One complete source input/decision/LFSR scan per host frame, shared low/high RNG words for sections1/2 and unconditional source LFSR advance. This is a deterministic native scan clock, not AVR loop/ADC/CPU timing. Seeded sequences depend on host rate and elapsed frames. Gate edges use 0.1V low / 1V high hysteresis, initially low; first high fires. Gates emit ideal 0/5V and follow detected input duration unless latched.");
    module["latch_policy"] = json!("Source behavior retained: disabling latch while its input is already low does not clear a held output. A subsequent rising decision updates it, and the next unlatched falling edge clears it. Both sections share one RNG, with independent input and outcome history.");
    module["light_names"] = json!(["a1", "b1", "a2", "b2"]);
    module["light_policy"] = json!("Actual four routed output voltages, normalized by5V; no invented activity. The source single bi-color indicator and its scan-count tail remain raw reference state, not native panel output indicators.");
    module["initialization_policy"] = json!("Initialize/reset clears detectors, outcomes and outputs and reseeds the kernel. Parameters persist through ordinary preset/document history; saved reopen starts fresh. Bypass emits mono zero and freezes all runtime state.");
    module["state_adoption"] = json!("Same ID/kind/rate and seed preserve the shared RNG, both decisions, gates and output holds. Receiving probability/toggle/latch controls always apply; changing seed or sample rate starts fresh. No callback allocation, locks or filesystem work.");
}

#[derive(Debug)]
pub(crate) struct EventFork {
    sample_rate: u32,
    seed: u32,
    probability: [f32; 2],
    toggle: [bool; 2],
    latch: [bool; 2],
    high: [bool; 2],
    outputs: [[bool; 2]; 2],
    kernel: Kernel,
}

fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

impl EventFork {
    pub(crate) fn build(node: &Node, sample_rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err("Event Fork sample rate outside 8000..192000".into());
        }
        for key in node.params.keys() {
            if ![
                "seed",
                "probability1",
                "probability2",
                "toggle1",
                "toggle2",
                "latch1",
                "latch2",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("Event Fork unknown parameter '{key}'"));
            }
        }
        let seed = parameter(node, "seed", 1.0, 1.0, 16777215.0)?;
        if seed.fract() != 0.0 {
            return Err("Event Fork seed must be an integer".into());
        }
        let mut probability = [0.0; 2];
        let mut toggle = [false; 2];
        let mut latch = [false; 2];
        for channel in 0..2 {
            probability[channel] =
                parameter(node, &format!("probability{}", channel + 1), 0.5, 0.0, 1.0)?;
            for (prefix, target) in [
                ("toggle", &mut toggle[channel]),
                ("latch", &mut latch[channel]),
            ] {
                let value = parameter(node, &format!("{prefix}{}", channel + 1), 0.0, 0.0, 1.0)?;
                if value.fract() != 0.0 {
                    return Err(format!("Event Fork {prefix} must be Off or On"));
                }
                *target = value != 0.0;
            }
        }
        Ok(Self {
            sample_rate,
            seed: seed as u32,
            probability,
            toggle,
            latch,
            high: [false; 2],
            outputs: [[false; 2]; 2],
            kernel: Kernel::new(seed as u32),
        })
    }

    pub(crate) fn initialize_state(&mut self) {
        self.kernel = Kernel::new(self.seed);
        self.high = [false; 2];
        self.outputs = self.kernel.snapshot().outputs;
    }

    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.sample_rate == old.sample_rate && self.seed == old.seed {
            self.kernel.clone_from(&old.kernel);
            self.high = old.high;
            self.outputs = old.outputs;
        }
    }

    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut lights = ModuleLightState {
            count: 4,
            ..Default::default()
        };
        for (i, value) in self.outputs.iter().flatten().enumerate() {
            lights.brightness[i] = [f32::from(*value), 0.0];
        }
        lights
    }

    pub(crate) fn process(
        &mut self,
        inputs: &[PortFrame; 4],
        physical_inputs: u16,
        demand: u16,
        outputs: &mut [PortFrame],
    ) {
        let gate1 = finite(inputs[0].channel(0));
        let gates = [
            gate1,
            if physical_inputs & 2 != 0 {
                finite(inputs[1].channel(0))
            } else {
                gate1
            },
        ];
        let mut thresholds = [0; 2];
        for channel in 0..2 {
            if gates[channel] >= 1.0 {
                self.high[channel] = true;
            } else if gates[channel] <= 0.1 {
                self.high[channel] = false;
            }
            let probability = (self.probability[channel]
                + finite(inputs[channel + 2].channel(0)) / 5.0)
                .clamp(0.0, 1.0);
            thresholds[channel] = adc_threshold(((1.0 - probability) * 255.0).floor() as u8);
        }
        self.outputs = self
            .kernel
            .process(self.high, thresholds, self.toggle, self.latch)
            .outputs;
        for (i, output) in outputs.iter_mut().enumerate() {
            *output = if demand & (1 << i) != 0 {
                PortFrame::mono(if self.outputs[i / 2][i % 2] { 5.0 } else { 0.0 })
            } else {
                PortFrame::default()
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(params: Value) -> Node {
        serde_json::from_value(json!({"id":"fork","kind":"event_fork","params":params})).unwrap()
    }
    fn model(params: Value) -> EventFork {
        EventFork::build(&node(params), 48000).unwrap()
    }
    fn tick(
        model: &mut EventFork,
        gate1: f32,
        gate2: f32,
        cv: [f32; 2],
        physical: u16,
    ) -> [f32; 4] {
        let inputs = [gate1, gate2, cv[0], cv[1]].map(PortFrame::mono);
        let mut outputs = [PortFrame::default(); 4];
        model.process(&inputs, physical, 15, &mut outputs);
        assert!(outputs.iter().all(|p| p.channels == 1));
        outputs.map(|p| p.channel(0))
    }

    #[test]
    fn endpoints_normaling_and_physical_zero_channel_override_are_distinct() {
        let mut fork = model(json!({"probability1":0,"probability2":1}));
        assert_eq!(tick(&mut fork, 5.0, 0.0, [0.0; 2], 1), [5.0, 0.0, 0.0, 5.0]);
        assert_eq!(tick(&mut fork, 0.0, 0.0, [0.0; 2], 1), [0.0; 4]);
        let mut outputs = [PortFrame::default(); 4];
        fork.process(
            &[
                PortFrame::mono(5.0),
                PortFrame::default(),
                PortFrame::default(),
                PortFrame::default(),
            ],
            3,
            15,
            &mut outputs,
        );
        assert_eq!(outputs.map(|p| p.channel(0)), [5.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn toggle_switches_only_on_rising_edges_and_direct_choices_hold_for_gate_duration() {
        let mut fork = model(json!({"probability1":1,"toggle1":1}));
        assert_eq!(tick(&mut fork, 5.0, 0.0, [0.0; 2], 3)[..2], [0.0, 5.0]);
        assert_eq!(tick(&mut fork, 5.0, 0.0, [-5.0, 0.0], 3)[..2], [0.0, 5.0]);
        tick(&mut fork, 0.0, 0.0, [0.0; 2], 3);
        assert_eq!(tick(&mut fork, 5.0, 0.0, [0.0; 2], 3)[..2], [5.0, 0.0]);
        tick(&mut fork, 0.0, 0.0, [0.0; 2], 3);
        assert_eq!(tick(&mut fork, 5.0, 0.0, [-5.0, 0.0], 3)[..2], [5.0, 0.0]);
    }

    #[test]
    fn latch_off_while_low_preserves_source_hold_until_a_subsequent_falling_edge() {
        let mut held = model(json!({"probability1":1,"latch1":1}));
        tick(&mut held, 5.0, 0.0, [0.0; 2], 3);
        assert_eq!(tick(&mut held, 0.0, 0.0, [0.0; 2], 3)[..2], [0.0, 5.0]);
        let mut receiver = model(json!({"probability1":0,"latch1":0}));
        receiver.adopt(&held);
        assert_eq!(tick(&mut receiver, 0.0, 0.0, [0.0; 2], 3)[..2], [0.0, 5.0]);
        assert_eq!(tick(&mut receiver, 5.0, 0.0, [0.0; 2], 3)[..2], [5.0, 0.0]);
        assert_eq!(tick(&mut receiver, 0.0, 0.0, [0.0; 2], 3)[..2], [0.0; 2]);
    }

    #[test]
    fn hysteresis_cv_adc_plateaus_and_nonfinite_volts_are_explicit_native_policies() {
        let mut fork = model(json!({"probability1":0.001,"probability2":0}));
        assert_eq!(tick(&mut fork, 0.9, 0.0, [0.0; 2], 1), [0.0; 4]);
        assert_eq!(
            tick(&mut fork, 1.0, 0.0, [0.0, 5.0], 1),
            [5.0, 0.0, 0.0, 5.0]
        );
        assert_eq!(tick(&mut fork, 0.5, 0.0, [0.0; 2], 1), [5.0, 0.0, 0.0, 5.0]);
        assert_eq!(tick(&mut fork, 0.1, 0.0, [0.0; 2], 1), [0.0; 4]);
        assert_eq!(
            tick(&mut fork, 5.0, 0.0, [f32::INFINITY, f32::NAN], 1),
            [5.0, 0.0, 5.0, 0.0]
        );
        assert_eq!(tick(&mut fork, f32::NAN, 0.0, [0.0; 2], 1), [0.0; 4]);
    }

    #[test]
    fn same_seed_adoption_preserves_rng_while_changed_seed_rate_and_initialize_restart() {
        let mut old = model(json!({}));
        for _ in 0..137 {
            tick(&mut old, 0.0, 0.0, [0.0; 2], 1);
        }
        let mut receiver = model(json!({"probability1":1}));
        receiver.adopt(&old);
        assert_eq!(receiver.kernel.snapshot(), old.kernel.snapshot());
        assert_eq!(tick(&mut receiver, 5.0, 0.0, [0.0; 2], 1)[..2], [0.0, 5.0]);
        let mut changed = model(json!({"seed":2}));
        let initial = changed.kernel.snapshot();
        changed.adopt(&old);
        assert_eq!(changed.kernel.snapshot(), initial);
        let mut rate_changed = EventFork::build(&node(json!({})), 44100).unwrap();
        let initial = rate_changed.kernel.snapshot();
        rate_changed.adopt(&old);
        assert_eq!(rate_changed.kernel.snapshot(), initial);
        receiver.initialize_state();
        assert_eq!(receiver.kernel.snapshot(), Kernel::new(1).snapshot());
        assert!(receiver.lights().brightness[..4]
            .iter()
            .all(|p| *p == [0.0; 2]));
    }

    #[test]
    fn invalid_controls_reject_and_lights_publish_only_real_routed_volts() {
        for params in [
            json!({"bogus":0}),
            json!({"seed":0}),
            json!({"seed":1.5}),
            json!({"toggle1":0.5}),
            json!({"latch2":2}),
            json!({"probability1":1.01}),
        ] {
            assert!(EventFork::build(&node(params), 48000).is_err());
        }
        let mut fork = model(json!({"probability1":1,"probability2":0}));
        assert_eq!(fork.lights().count, 4);
        assert!(fork.lights().brightness[..4].iter().all(|p| *p == [0.0; 2]));
        tick(&mut fork, 5.0, 0.0, [0.0; 2], 1);
        assert_eq!(
            fork.lights().brightness[..4],
            [[0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [0.0, 0.0]]
        );
    }
}
