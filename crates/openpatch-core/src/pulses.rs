//! Original ten-button trigger/gate source. Controls are runtime-only.
use crate::{
    ModuleAction, ModuleControlState, ModuleLightState, Node, PortFrame,
    MAX_MODULE_ACTIONS_PER_SAMPLE,
};
use serde_json::{json, Value};

pub(crate) const OUTPUTS: [&str; 20] = [
    "trig1", "trig2", "trig3", "trig4", "trig5", "trig6", "trig7", "trig8", "trig9", "trig10",
    "gate1", "gate2", "gate3", "gate4", "gate5", "gate6", "gate7", "gate8", "gate9", "gate10",
];
pub(crate) fn catalog() -> Value {
    json!({"kind":"pulses","title":"PULSES","model":"functional","inputs":[],"params":{},"description":"Ten independent momentary buttons. Each rising press emits a mono 10 V trigger for 1 ms; its gate is 10 V while held. Controls and pulse tails are runtime-only, not saved parameters."})
}
pub(crate) fn annotate(module: &mut Value) {
    module["runtime_actions"] = json!(&crate::MODULE_ACTION_NAMES[7..]);
    module["polyphony"]["version_2"] = json!("twenty independent mono outputs; no inputs");
    module["polyphony"]["maximum_channels"] = json!(1);
    module["polyphony"]["primary_input"] = Value::Null;
    for output in module["output_metadata"].as_array_mut().unwrap() {
        output["maximum_channels"] = json!(1);
    }
    module["manual_control"] = json!({"buttons":10,"saved":false,"capacity_per_sample":MAX_MODULE_ACTIONS_PER_SAMPLE,"ordering":"actions in supplied order before output evaluation; press then release in one sample still emits a trigger","release_safety":"release_module_controls clears all pending actions and held buttons without clearing trigger countdowns"});
    module["trigger_policy"] = json!({"pulse":"10 V for ceil(0.001 * sample_rate) samples, including the rising-press sample","repeated_press":"held press does not retrigger; release followed by press restarts the full countdown, extending an overlapping pulse","gate":"10 V while its button is held, otherwise 0 V","startup":"all buttons released; an action before the first sample is an intentional edge; reference startup detector behavior is not claimed","bypass":"all outputs mono zero; countdowns freeze; bypass transitions release all held and pending controls"});
    module["light_names"] = json!([
        "button1", "button2", "button3", "button4", "button5", "button6", "button7", "button8",
        "button9", "button10"
    ]);
    module["light_policy"] = json!({"pairs":"index zero is held-button brightness 0 or 1; index one always zero","manual":"true if any button is held","timing":"immediate after ordered actions are processed"});
    module["state_adoption"] = json!("same ID/kind/sample rate retains held buttons and pulse countdowns; pending actions stay with their receiving engine and are never adopted; bypass transitions release controls; sample-rate changes start fresh; initialization clears all controls and countdowns");
    module["compatibility"] = json!("original Rust functional contract; explicit sample-count pulse timing and released startup state, not sample-identical Rack behavior");
}

#[derive(Debug)]
pub(crate) struct Pulses {
    held: u16,
    remaining: [u32; 10],
    pulse_samples: u32,
    actions: [ModuleAction; MAX_MODULE_ACTIONS_PER_SAMPLE],
    count: usize,
}
impl Pulses {
    pub(crate) fn build(node: &Node, sample_rate: u32) -> Result<Self, String> {
        if let Some(name) = node.params.keys().next() {
            return Err(format!("node '{}': unknown parameter '{name}'", node.id));
        }
        Ok(Self {
            held: 0,
            remaining: [0; 10],
            // Integer ceiling avoids floating-point threshold ambiguity.
            pulse_samples: sample_rate.div_ceil(1000),
            actions: [ModuleAction::PulseRelease(1); MAX_MODULE_ACTIONS_PER_SAMPLE],
            count: 0,
        })
    }
    pub(crate) fn enqueue(&mut self, action: ModuleAction) -> bool {
        if action.pulse_button().is_none() || self.count == self.actions.len() {
            return false;
        }
        self.actions[self.count] = action;
        self.count += 1;
        true
    }
    pub(crate) fn release(&mut self) {
        self.held = 0;
        self.count = 0;
    }
    /// Matching release remains available while bypassed, without affecting siblings.
    pub(crate) fn release_button(&mut self, button: usize) {
        self.held &= !(1 << button);
        // Bypass must not retain an older queued press for the released row.
        let mut kept = 0;
        for index in 0..self.count {
            if self.actions[index]
                .pulse_button()
                .is_some_and(|(row, _)| row != button)
            {
                self.actions[kept] = self.actions[index];
                kept += 1;
            }
        }
        self.count = kept;
    }
    pub(crate) fn reset_state(&mut self) {
        self.release();
        self.remaining.fill(0);
    }
    pub(crate) fn adopt(&mut self, previous: &Self) {
        self.held = previous.held;
        self.remaining = previous.remaining;
    }
    pub(crate) fn controls(&self) -> ModuleControlState {
        ModuleControlState {
            pressed: self.held != 0,
            pressed_mask: self.held,
        }
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        ModuleLightState {
            count: 10,
            brightness: std::array::from_fn(|button| {
                [
                    if self.held & (1 << button) != 0 {
                        1.0
                    } else {
                        0.0
                    },
                    0.0,
                ]
            }),
            manual: self.held != 0,
        }
    }
    pub(crate) fn process(&mut self, outputs: &mut [PortFrame]) {
        for action in &self.actions[..self.count] {
            let (button, press) = action.pulse_button().unwrap();
            let bit = 1 << button;
            if press {
                if self.held & bit == 0 {
                    self.remaining[button] = self.pulse_samples;
                }
                self.held |= bit;
            } else {
                self.held &= !bit;
            }
        }
        self.count = 0;
        for button in 0..10 {
            outputs[button] = PortFrame::mono(if self.remaining[button] > 0 {
                10.0
            } else {
                0.0
            });
            outputs[10 + button] = PortFrame::mono(if self.held & (1 << button) != 0 {
                10.0
            } else {
                0.0
            });
            self.remaining[button] = self.remaining[button].saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(rate: u32) -> Pulses {
        Pulses::build(
            &Node {
                id: "p".into(),
                kind: "pulses".into(),
                bypassed: false,
                params: Default::default(),
            },
            rate,
        )
        .unwrap()
    }
    fn sample(pulses: &mut Pulses) -> [PortFrame; 20] {
        let mut outputs = [PortFrame::default(); 20];
        pulses.process(&mut outputs);
        assert!(outputs
            .iter()
            .all(|frame| frame.channels == 1 && frame.values[1..].iter().all(|v| *v == 0.0)));
        outputs
    }
    #[test]
    fn pulse_ceil_duration_includes_rising_sample_at_each_rate() {
        for (rate, length) in [(44_100, 45), (48_000, 48), (96_000, 96)] {
            let mut p = model(rate);
            assert!(sample(&mut p).iter().all(|f| f.values[0] == 0.0));
            p.enqueue(ModuleAction::PulsePress(1));
            p.enqueue(ModuleAction::PulseRelease(1));
            for _ in 0..length {
                let output = sample(&mut p);
                assert_eq!(output[0].values[0], 10.0, "{rate} Hz");
                assert_eq!(output[10].values[0], 0.0);
            }
            assert_eq!(sample(&mut p)[0].values[0], 0.0);
            assert_eq!(p.controls().pressed_mask, 0);
        }
    }
    #[test]
    fn independent_rows_ordered_edges_and_held_press_does_not_retrigger() {
        let mut p = model(48_000);
        p.enqueue(ModuleAction::PulsePress(1));
        p.enqueue(ModuleAction::PulsePress(10));
        let first = sample(&mut p);
        assert_eq!(p.controls().pressed_mask, 0x201);
        assert!(p.controls().pressed);
        assert_eq!(p.lights().brightness[9], [1.0, 0.0]);
        for row in 0..10 {
            let expected = if row == 0 || row == 9 { 10.0 } else { 0.0 };
            assert_eq!(first[row].values[0], expected);
            assert_eq!(first[10 + row].values[0], expected);
        }
        // Pressing an already-held button on every sample cannot prolong its pulse.
        for _ in 1..48 {
            p.enqueue(ModuleAction::PulsePress(10));
            sample(&mut p);
        }
        assert_eq!(sample(&mut p)[9].values[0], 0.0);
        assert_eq!(p.controls().pressed_mask, 0x201);
        p.enqueue(ModuleAction::PulseRelease(10));
        let released = sample(&mut p);
        assert_eq!(released[10].values[0], 10.0);
        assert_eq!(released[19].values[0], 0.0);
        assert_eq!(p.controls().pressed_mask, 1);
    }
    #[test]
    fn repress_extends_pulse_and_emergency_release_preserves_tail() {
        let mut p = model(48_000);
        p.enqueue(ModuleAction::PulsePress(10));
        for _ in 0..24 {
            assert_eq!(sample(&mut p)[9].values[0], 10.0);
        }
        p.enqueue(ModuleAction::PulseRelease(10));
        p.enqueue(ModuleAction::PulsePress(10));
        assert_eq!(sample(&mut p)[9].values[0], 10.0);
        p.enqueue(ModuleAction::PulsePress(2)); // must be cancelled, not processed
        p.release();
        assert_eq!(p.controls().pressed_mask, 0);
        assert!(!p.lights().manual);
        for _ in 1..48 {
            let out = sample(&mut p);
            assert_eq!(out[9].values[0], 10.0);
            assert_eq!(out[1].values[0], 0.0);
            assert!(out[10..].iter().all(|f| f.values[0] == 0.0));
        }
        assert_eq!(sample(&mut p)[9].values[0], 0.0);
    }
    #[test]
    fn sixty_four_ordered_actions_are_bounded_and_reset_clears_every_row() {
        let mut p = model(44_100);
        for _ in 0..32 {
            assert!(p.enqueue(ModuleAction::PulsePress(10)));
            assert!(p.enqueue(ModuleAction::PulseRelease(10)));
        }
        assert!(!p.enqueue(ModuleAction::PulsePress(1)));
        let out = sample(&mut p);
        assert_eq!(out[9].values[0], 10.0);
        assert_eq!(out[19].values[0], 0.0);
        assert_eq!(p.controls().pressed_mask, 0);
        assert!(!p.enqueue(ModuleAction::PulseRelease(0)));
        assert!(!p.enqueue(ModuleAction::PulsePress(11)));
        assert!(!p.enqueue(ModuleAction::Clock));
        p.enqueue(ModuleAction::PulsePress(1));
        p.reset_state();
        assert!(sample(&mut p).iter().all(|f| f.values[0] == 0.0));
    }
    #[test]
    fn adoption_keeps_receiving_queue_and_never_copies_previous_pending_actions() {
        let mut old = model(48_000);
        old.enqueue(ModuleAction::PulsePress(1));
        sample(&mut old);
        old.enqueue(ModuleAction::PulsePress(2));
        let mut new = model(48_000);
        new.enqueue(ModuleAction::PulseRelease(1));
        new.enqueue(ModuleAction::PulsePress(10));
        new.adopt(&old);
        let out = sample(&mut new);
        assert_eq!(out[0].values[0], 10.0);
        assert_eq!(out[1].values[0], 0.0);
        assert_eq!(out[9].values[0], 10.0);
        assert_eq!(new.controls().pressed_mask, 1 << 9);
    }
}
