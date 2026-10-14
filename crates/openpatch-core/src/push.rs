//! Original momentary PUSH and persistent HOLD source. No reference code is copied.
use crate::{
    parameter, ModuleAction, ModuleControlState, ModuleLightState, Node, PortFrame,
    MAX_MODULE_ACTIONS_PER_SAMPLE,
};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 2] = ["push", "hold"];
pub(crate) const OUTPUTS: [&str; 2] = ["trig", "gate"];
pub(crate) fn catalog() -> Value {
    json!({"kind":"push","title":"PUSH","model":"functional","inputs":INPUTS,"params":{"holding":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"description":"Persistent HOLD latch. Changed value or explicit-presence edits override adopted latch; live capture records actual HOLD."}},"description":"Momentary PUSH combined with a persistent HOLD latch: gate = (PUSH button OR PUSH CV high) XOR HOLD. HOLD button and HOLD CV rising edges independently toggle the latch. Each final gate rise emits a mono 10 V trigger for 1 ms."})
}
pub(crate) fn annotate(module: &mut Value) {
    module["runtime_actions"] = json!(&crate::MODULE_ACTION_NAMES[27..31]);
    module["polyphony"]["version_2"] =
        json!("both outputs mono; inputs read only channel one, ignoring other voices");
    module["polyphony"]["maximum_channels"] = json!(1);
    module["polyphony"]["primary_input"] = Value::Null;
    for meta in module["output_metadata"].as_array_mut().unwrap() {
        meta["maximum_channels"] = json!(1);
    }
    module["manual_control"] = json!({"buttons":["push","hold"],"saved":false,"capacity_per_sample":MAX_MODULE_ACTIONS_PER_SAMPLE,"ordering":"evaluate current CV detectors and HOLD-CV toggle without observing gate, then apply ordered physical actions; current CV is folded into the first action boundary, subsequent actions are explicit subsample boundaries; quick press-release retains its pulse","release_safety":"matching releases affect only that physical button; emergency release clears both buttons and queued actions, preserving HOLD and trigger tail"});
    module["gate_policy"] = json!({"formula":"(physical PUSH OR PUSH CV high) XOR HOLD latch","cv":"Schmitt low <=0.1 V, high >=1 V; initial high arms without rising; nonfinite becomes zero; both current CV detectors and HOLD-CV toggle are evaluated before any physical-action gate observation; with no physical actions observe gate once","hold":"physical HOLD rising press and HOLD CV rise independently toggle; both together toggle twice","startup":"loaded HOLD high gates immediately without startup trigger; initial PUSH CV high gates without startup trigger; changed physical actions before the first sample are intentional edges; no-op releases/repeated actions do not bypass initial-high suppression"});
    module["trigger_policy"] = json!({"pulse":"10 V for ceil(sample_rate/1000) samples including the rising sample; a later gate rise restarts the full countdown","release":"physical releases preserve latch and pulse countdown; releasing PUSH while HOLD is on can create a gate rise","bypass":"mono zero outputs; CV, latch and pulse countdown freeze; bypass transitions release held and pending physical controls"});
    module["light_names"] = json!(["gate", "hold"]);
    module["light_policy"] = json!({"pairs":"index zero is final gate / HOLD brightness 0 or 1; index one always zero","manual":"any physical button held","timing":"immediate after processing"});
    module["state_adoption"] = json!("same ID/kind/sample rate retains actual HOLD, physical buttons, CV detectors, gate history and pulse countdown; changed configured HOLD value/presence overrides adopted latch; pending actions stay with their receiving engine; bypass transitions release physical controls; sample-rate changes start fresh");
    module["initialization_policy"] = json!("clear HOLD and physical buttons/pending actions; preserve pulse countdown, CV detectors and gate edge history; next computed gate can legitimately rise");
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Push.cpp");
    module["compatibility"] = json!("original Rust functional implementation; literal saved holding parameter maps reference custom HOLD state; explicit ordered first-sample actions, ceiling pulse duration and immediate lights are OpenPatch policies, not sample-identical reference behavior");
}

#[derive(Clone, Copy, Debug, Default)]
struct Schmitt(u8); // 0 unknown, 1 low, 2 high
impl Schmitt {
    fn update(&mut self, value: f32) -> (bool, bool) {
        let value = if value.is_finite() { value } else { 0.0 };
        let initial_high = self.0 == 0 && value >= 1.0;
        let rising = self.0 == 1 && value >= 1.0;
        if value >= 1.0 {
            self.0 = 2;
        } else if value <= 0.1 {
            self.0 = 1;
        }
        (rising, initial_high)
    }
    fn high(self) -> bool {
        self.0 == 2
    }
}
#[derive(Debug)]
pub(crate) struct Push {
    initial_holding: bool,
    explicit_holding: bool,
    holding: bool,
    buttons: u16,
    push_cv: Schmitt,
    hold_cv: Schmitt,
    gate: bool,
    remaining: u32,
    pulse_samples: u32,
    actions: [ModuleAction; MAX_MODULE_ACTIONS_PER_SAMPLE],
    count: usize,
}
impl Push {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for name in node.params.keys() {
            if name != "holding" {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        let value = parameter(node, "holding", 0.0, 0.0, 1.0)?;
        if value.fract() != 0.0 {
            return Err(format!("node '{}': holding must be 0 or 1", node.id));
        }
        let holding = value != 0.0;
        Ok(Self {
            initial_holding: holding,
            explicit_holding: node.params.contains_key("holding"),
            holding,
            buttons: 0,
            push_cv: Schmitt::default(),
            hold_cv: Schmitt::default(),
            gate: holding,
            remaining: 0,
            pulse_samples: rate.div_ceil(1000),
            actions: [ModuleAction::PushRelease; MAX_MODULE_ACTIONS_PER_SAMPLE],
            count: 0,
        })
    }
    pub(crate) fn supports(action: ModuleAction) -> bool {
        matches!(
            action,
            ModuleAction::PushPress
                | ModuleAction::PushRelease
                | ModuleAction::HoldPress
                | ModuleAction::HoldRelease
        )
    }
    pub(crate) fn enqueue(&mut self, action: ModuleAction) -> bool {
        if !Self::supports(action) || self.count == self.actions.len() {
            return false;
        }
        self.actions[self.count] = action;
        self.count += 1;
        true
    }
    pub(crate) fn holding(&self) -> bool {
        self.holding
    }
    pub(crate) fn set_holding(&mut self, holding: bool) {
        self.holding = holding;
    }
    pub(crate) fn controls(&self) -> ModuleControlState {
        ModuleControlState {
            pressed: self.buttons != 0,
            pressed_mask: self.buttons,
        }
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut lights = ModuleLightState {
            count: 2,
            manual: self.buttons != 0,
            ..ModuleLightState::default()
        };
        lights.brightness[0][0] = if self.gate { 1.0 } else { 0.0 };
        lights.brightness[1][0] = if self.holding { 1.0 } else { 0.0 };
        lights
    }
    pub(crate) fn release(&mut self) {
        self.buttons = 0;
        self.count = 0;
    }
    pub(crate) fn release_button(&mut self, hold: bool) {
        self.buttons &= !(if hold { 2 } else { 1 });
        let mut kept = 0;
        for index in 0..self.count {
            let same_button = if hold {
                matches!(
                    self.actions[index],
                    ModuleAction::HoldPress | ModuleAction::HoldRelease
                )
            } else {
                matches!(
                    self.actions[index],
                    ModuleAction::PushPress | ModuleAction::PushRelease
                )
            };
            if !same_button {
                self.actions[kept] = self.actions[index];
                kept += 1;
            }
        }
        self.count = kept;
    }
    pub(crate) fn initialize_state(&mut self) {
        self.holding = false;
        self.release();
    }
    pub(crate) fn reset_state(&mut self) {
        self.holding = self.initial_holding;
        self.release();
        self.push_cv = Schmitt::default();
        self.hold_cv = Schmitt::default();
        self.gate = self.holding;
        self.remaining = 0;
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        self.holding = old.holding;
        self.buttons = old.buttons;
        self.push_cv = old.push_cv;
        self.hold_cv = old.hold_cv;
        self.gate = old.gate;
        self.remaining = old.remaining;
        if self.initial_holding != old.initial_holding
            || self.explicit_holding != old.explicit_holding
        {
            self.holding = self.initial_holding;
        }
    }
    fn update_gate(&mut self, allow_trigger: bool) {
        let gate = (self.buttons & 1 != 0 || self.push_cv.high()) ^ self.holding;
        if gate && !self.gate && allow_trigger {
            self.remaining = self.pulse_samples;
        }
        self.gate = gate;
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame; 2], outputs: &mut [PortFrame]) {
        // Current CV belongs to this frame, not a later synthetic boundary.
        // Defer observing the gate until the first physical action (or the end
        // of a CV-only frame), so cancelling CV/button changes cannot invent a
        // transient edge. Further queued actions are deliberate ordered edges.
        let (hold_rise, _) = self.hold_cv.update(inputs[1].channel(0));
        let (_, initial_push_high) = self.push_cv.update(inputs[0].channel(0));
        if hold_rise {
            self.holding = !self.holding;
        }
        let mut physical_edge = false;
        for index in 0..self.count {
            let previous_buttons = self.buttons;
            match self.actions[index] {
                ModuleAction::PushPress => self.buttons |= 1,
                ModuleAction::PushRelease => self.buttons &= !1,
                ModuleAction::HoldPress => {
                    if self.buttons & 2 == 0 {
                        self.holding = !self.holding;
                    }
                    self.buttons |= 2;
                }
                ModuleAction::HoldRelease => self.buttons &= !2,
                _ => unreachable!("only supported actions enter the fixed queue"),
            }
            if self.buttons != previous_buttons {
                physical_edge = true;
                self.update_gate(true);
            }
        }
        self.count = 0;
        self.update_gate(!initial_push_high || physical_edge);
        outputs[0] = PortFrame::mono(if self.remaining > 0 { 10.0 } else { 0.0 });
        outputs[1] = PortFrame::mono(if self.gate { 10.0 } else { 0.0 });
        self.remaining = self.remaining.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(rate: u32, holding: bool) -> Push {
        Push::build(
            &Node {
                score: None,
                id: "p".into(),
                kind: "push".into(),
                bypassed: false,
                params: [("holding".into(), if holding { 1.0 } else { 0.0 })].into(),
            },
            rate,
        )
        .unwrap()
    }
    fn sample(push: &mut Push, cv: f32, hold: f32) -> [f32; 2] {
        let mut outputs = [PortFrame::default(); 2];
        push.process(&[PortFrame::mono(cv), PortFrame::mono(hold)], &mut outputs);
        assert!(outputs.iter().all(
            |frame| frame.channels == 1 && frame.values[1..].iter().all(|value| *value == 0.0)
        ));
        outputs.map(|frame| frame.values[0])
    }
    #[test]
    fn xor_truth_table_and_loaded_hold_has_no_startup_trigger() {
        for holding in [false, true] {
            for pressed in [false, true] {
                let mut p = model(48_000, holding);
                assert_eq!(
                    sample(&mut p, 0.0, 0.0),
                    [0.0, if holding { 10.0 } else { 0.0 }]
                );
                if pressed {
                    p.enqueue(ModuleAction::PushPress);
                }
                let [trigger, gate] = sample(&mut p, 0.0, 0.0);
                assert_eq!(gate, if pressed ^ holding { 10.0 } else { 0.0 });
                assert_eq!(trigger, if pressed && !holding { 10.0 } else { 0.0 });
            }
        }
        let mut p = model(48_000, true);
        p.enqueue(ModuleAction::PushPress);
        p.enqueue(ModuleAction::PushRelease);
        assert_eq!(sample(&mut p, 0.0, 0.0), [10.0, 10.0]);
    }
    #[test]
    fn initial_high_cv_suppression_does_not_erase_explicit_first_sample_edges() {
        let mut p = model(48_000, false);
        assert_eq!(sample(&mut p, 1.0, 1.0), [0.0, 10.0]);
        assert!(!p.holding());
        assert_eq!(sample(&mut p, 0.5, 0.5), [0.0, 10.0]);
        assert_eq!(sample(&mut p, 0.1, 0.1), [0.0, 0.0]);
        assert_eq!(sample(&mut p, 1.0, 0.0), [10.0, 10.0]);
        let mut quick = model(48_000, false);
        quick.enqueue(ModuleAction::PushPress);
        quick.enqueue(ModuleAction::PushRelease);
        assert_eq!(sample(&mut quick, 1.0, 1.0), [10.0, 10.0]);
        assert_eq!(quick.controls().pressed_mask, 0);
    }
    #[test]
    fn current_cv_and_first_physical_action_cancel_without_a_ghost_trigger() {
        for action in [ModuleAction::HoldPress, ModuleAction::PushPress] {
            let mut p = model(48_000, false);
            assert_eq!(sample(&mut p, 0.0, 0.0), [0.0, 0.0]);
            p.enqueue(action);
            assert_eq!(sample(&mut p, 0.0, 1.0), [0.0, 0.0]);
            assert_eq!(p.holding(), action == ModuleAction::PushPress);
        }
        let mut quick = model(48_000, false);
        sample(&mut quick, 0.0, 0.0);
        quick.enqueue(ModuleAction::PushPress);
        quick.enqueue(ModuleAction::PushRelease);
        assert_eq!(sample(&mut quick, 0.0, 0.0), [10.0, 0.0]);
    }
    #[test]
    fn no_op_release_cannot_turn_initial_high_cv_into_an_intentional_edge() {
        for action in [ModuleAction::PushRelease, ModuleAction::HoldRelease] {
            let mut p = model(48_000, false);
            p.enqueue(action);
            p.enqueue(action);
            assert_eq!(sample(&mut p, 1.0, 1.0), [0.0, 10.0]);
        }
        let mut intentional = model(48_000, false);
        intentional.enqueue(ModuleAction::PushRelease);
        intentional.enqueue(ModuleAction::PushPress);
        intentional.enqueue(ModuleAction::PushPress);
        assert_eq!(sample(&mut intentional, 1.0, 1.0), [10.0, 10.0]);
        let mut loaded = model(48_000, true);
        loaded.enqueue(ModuleAction::HoldRelease);
        assert_eq!(sample(&mut loaded, 0.0, 0.0), [0.0, 10.0]);
    }
    #[test]
    fn cv_channel_zero_threshold_hysteresis_and_simultaneous_cv_frame() {
        let mut p = model(48_000, false);
        sample(&mut p, 0.0, 0.0);
        assert_eq!(sample(&mut p, 0.99, 0.99), [0.0, 0.0]);
        // Both CV rises are evaluated together: latch on XOR PUSH high is low.
        assert_eq!(sample(&mut p, 1.0, 1.0), [0.0, 0.0]);
        assert!(p.holding());
        assert_eq!(sample(&mut p, 0.11, 0.11), [0.0, 0.0]);
        assert_eq!(sample(&mut p, 0.1, 0.1), [10.0, 10.0]);
        let mut p = model(48_000, false);
        let mut outputs = [PortFrame::default(); 2];
        let mut extra_voice = PortFrame::mono(0.0);
        extra_voice.channels = 2;
        extra_voice.values[1] = 10.0;
        p.process(&[extra_voice; 2], &mut outputs);
        assert!(!p.holding());
        assert_eq!(outputs, [PortFrame::mono(0.0); 2]);
        assert_eq!(sample(&mut p, f32::NAN, f32::INFINITY), [0.0, 0.0]);
    }
    #[test]
    fn hold_button_and_cv_toggle_independently_repeated_press_does_not_toggle() {
        let mut p = model(48_000, false);
        sample(&mut p, 0.0, 0.0);
        p.enqueue(ModuleAction::HoldPress);
        assert_eq!(sample(&mut p, 0.0, 1.0), [0.0, 0.0]);
        assert!(!p.holding()); // physical and CV rises each toggle once
        assert_eq!(p.controls().pressed_mask, 2);
        p.enqueue(ModuleAction::HoldPress);
        sample(&mut p, 0.0, 1.0);
        assert!(!p.holding());
        p.enqueue(ModuleAction::HoldRelease);
        p.enqueue(ModuleAction::HoldPress);
        assert_eq!(sample(&mut p, 0.0, 1.0)[1], 10.0);
        assert!(p.holding());
        let lights = p.lights();
        assert_eq!(lights.count, 2);
        assert_eq!(lights.brightness[0], [1.0, 0.0]);
        assert_eq!(lights.brightness[1], [1.0, 0.0]);
        assert!(lights.manual);
    }
    #[test]
    fn pulse_ceiling_quick_click_and_repress_extension_at_each_rate() {
        for (rate, length) in [(44_100, 45), (48_000, 48), (96_000, 96)] {
            let mut p = model(rate, false);
            p.enqueue(ModuleAction::PushPress);
            p.enqueue(ModuleAction::PushRelease);
            for _ in 0..length {
                assert_eq!(sample(&mut p, 0.0, 0.0), [10.0, 0.0]);
            }
            assert_eq!(sample(&mut p, 0.0, 0.0), [0.0, 0.0]);
            p.enqueue(ModuleAction::PushPress);
            for _ in 0..length / 2 {
                sample(&mut p, 0.0, 0.0);
            }
            p.enqueue(ModuleAction::PushRelease);
            p.enqueue(ModuleAction::PushPress);
            for _ in 0..length {
                assert_eq!(sample(&mut p, 0.0, 0.0), [10.0, 10.0]);
            }
            assert_eq!(sample(&mut p, 0.0, 0.0), [0.0, 10.0]);
        }
    }
    #[test]
    fn releases_preserve_latch_and_tail_initialize_preserves_history_reset_is_fresh() {
        let mut p = model(48_000, false);
        p.enqueue(ModuleAction::HoldPress);
        assert_eq!(sample(&mut p, 0.0, 0.0), [10.0, 10.0]);
        p.enqueue(ModuleAction::HoldRelease);
        assert_eq!(sample(&mut p, 0.0, 0.0), [10.0, 10.0]);
        assert!(p.holding());
        p.enqueue(ModuleAction::PushPress);
        assert_eq!(sample(&mut p, 0.0, 0.0), [10.0, 0.0]);
        p.release();
        assert!(p.holding());
        assert_eq!(p.controls().pressed_mask, 0);
        // Releasing PUSH under HOLD creates a legitimate future gate rise.
        assert_eq!(sample(&mut p, 0.0, 0.0), [10.0, 10.0]);
        assert_eq!(p.remaining, 47);
        p.initialize_state();
        assert!(!p.holding());
        assert!(p.gate); // edge history is retained until next computed frame
        assert_eq!(p.remaining, 47);
        assert_eq!(sample(&mut p, 0.0, 0.0), [10.0, 0.0]);
        p.reset_state();
        assert_eq!(sample(&mut p, 0.0, 0.0), [0.0, 0.0]);
    }
    #[test]
    fn ordered_queue_is_bounded_and_adoption_does_not_copy_pending_actions() {
        let mut old = model(48_000, false);
        old.enqueue(ModuleAction::HoldPress);
        sample(&mut old, 0.0, 0.0);
        old.enqueue(ModuleAction::PushPress);
        let mut new = model(48_000, false);
        new.enqueue(ModuleAction::HoldRelease);
        new.adopt(&old);
        sample(&mut new, 0.0, 0.0);
        assert!(new.holding());
        assert_eq!(new.controls().pressed_mask, 0);
        new.release();
        for _ in 0..32 {
            assert!(new.enqueue(ModuleAction::PushPress));
            assert!(new.enqueue(ModuleAction::PushRelease));
        }
        assert!(!new.enqueue(ModuleAction::HoldPress));
        assert!(!new.enqueue(ModuleAction::Clock));
        sample(&mut new, 0.0, 0.0);
        assert_eq!(new.controls().pressed_mask, 0);
    }
}
