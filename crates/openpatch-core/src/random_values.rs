//! Original deterministic held random voltage source. No reference code is copied.
use crate::{
    parameter, ModuleAction, ModuleControlState, ModuleLightState, Node, PortFrame,
    SavedModuleState, MAX_MODULE_ACTIONS_PER_SAMPLE,
};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 1] = ["trig"];
pub(crate) const OUTPUTS: [&str; 7] = ["out1", "out2", "out3", "out4", "out5", "out6", "out7"];
const RANGES: [(f32, f32); 6] = [
    (0., 10.),
    (0., 5.),
    (0., 1.),
    (-10., 20.),
    (-5., 10.),
    (-1., 2.),
];
const SEED_XOR: u32 = 0xa341_316c;

pub(crate) fn catalog() -> Value {
    let mut module = json!({"kind":"random_values","title":"RANDOM VALUES","model":"functional","inputs":INPUTS,"description":"Seven held random voltage outputs. TRIG independently rerolls each active lane; manual PUSH rerolls all active lanes. Hidden lanes retain their voltages. Range changes affect future draws only.","params":{
        "range":{"default":0.,"min":0.,"max":5.,"step":1.,"values":[0,1,2,3,4,5],"labels":["0–10 V","0–5 V","0–1 V","±10 V","±5 V","±1 V"]},
        "seed":{"default":1.,"min":0.,"max":16777215.,"step":1.,"description":"Deterministic startup seed; edits reroll all 112 held voltages from the new seed."}
    }});
    for output in 1..=7 {
        for lane in 1..=16 {
            module["params"][format!("value{output}_{lane}")] = json!({"default":0.,"min":-10.,"max":10.,"hidden":true,"description":"Reserved complete runtime snapshot; optional only as a full set with rng_hi/rng_lo."});
        }
    }
    for name in ["rng_hi", "rng_lo"] {
        module["params"][name] =
            json!({"default":0.,"min":0.,"max":65535.,"step":1.,"hidden":true});
    }
    module
}
pub(crate) fn annotate(module: &mut Value) {
    module["runtime_actions"] = json!(["push_press", "push_release"]);
    module["polyphony"]["version_2"] = json!("all seven outputs have max(1, TRIG channels); each TRIG lane rerolls only its seven values; PUSH rerolls every active lane");
    module["polyphony"]["primary_input"] = json!("trig");
    module["polyphony"]["retired_voices"] = json!(
        "inactive held values and Schmitt detectors freeze; reactivation resumes retained state"
    );
    for meta in module["output_metadata"].as_array_mut().unwrap() {
        meta["maximum_channels"] = json!(16);
    }
    module["manual_control"] = json!({"buttons":["push"],"saved":false,"capacity_per_sample":MAX_MODULE_ACTIONS_PER_SAMPLE,"ordering":"current CV rises coalesce with first effective PUSH press per lane; later ordered release/press pairs reroll active lanes again; repeated press and release actions themselves do not create draws","release_safety":"release clears physical button and pending actions, preserving held voltages, RNG and trigger detectors"});
    module["trigger_policy"] = json!({"thresholds":"low <=0.1 V, high >=1 V; nonfinite treated as zero; initial high arms without a draw","range":"0:0..10, 1:0..5, 2:0..1, 3:-10..10, 4:-5..5, 5:-1..1 V; range affects next draw only","bypass":"seven mono zero outputs; held values, RNG and detectors freeze; transitions release physical controls"});
    module["random_policy"] = json!({"algorithm":"xorshift32 shifts 13,17,5; initial state seed XOR 0xA341316C; advance then top 24 bits / 16777216; voltage = low + width * uniform","draw_order":"lane ascending then output ascending; startup draws all 112 values; runtime draws only selected active lanes","seed":"integer 0..16777215, default 1; changed seed rerolls all 112 held values from a fresh RNG"});
    module["randomization_policy"] = json!("explicit host Randomize rerolls all 112 values in the current range from RNG max(1, FNV-1a32(module ID) XOR request seed), preserving visible range and startup seed; PUSH and TRIG continue the live stream");
    module["saved_state"] = json!({"values":"all 7 × 16 actual f32 voltages encoded as value1_1..value7_16","rng":"nonzero u32 encoded by integer rng_hi/rng_lo halves","validation":"all 114 reserved parameters or none; finite voltages within -10..10 independent of selected range"});
    module["state_adoption"] = json!("same identity/kind/sample rate retains held values, RNG, detectors and physical button; changed complete configured snapshot overrides values/RNG; changed seed rerolls all 112 from a fresh RNG; pending actions stay with receiving engine; bypass transitions release physical controls");
    module["initialization_policy"] = json!("restore default range and seed parameters; preserve held voltages, RNG and trigger detectors; release physical button and pending actions");
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/RandomValues.cpp");
    module["compatibility"] = json!("original Rust implementation; deterministic PRNG, initial-high suppression, explicit ordered actions and full-or-none snapshot schema are OpenPatch policies; no claim of sample-identical random sequence");
}

fn integer(node: &Node, name: &str, default: f32, max: f32) -> Result<u32, String> {
    let value = parameter(node, name, default, 0., max)?;
    if value.fract() != 0. {
        return Err(format!("node '{}': {name} must be an integer", node.id));
    }
    Ok(value as u32)
}
fn value_key(name: &str) -> bool {
    let Some((output, lane)) = name.strip_prefix("value").and_then(|s| s.split_once('_')) else {
        return false;
    };
    let (Ok(output_num), Ok(lane_num)) = (output.parse::<usize>(), lane.parse::<usize>()) else {
        return false;
    };
    (1..=7).contains(&output_num)
        && (1..=16).contains(&lane_num)
        && name == format!("value{output_num}_{lane_num}")
}
fn next(rng: &mut u32) -> f32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 17;
    *rng ^= *rng << 5;
    (*rng >> 8) as f32 / 16_777_216.
}
fn draw(values: &mut [[f32; 16]; 7], rng: &mut u32, range: usize, lane: usize) {
    let (lo, width) = RANGES[range];
    for output in values.iter_mut() {
        output[lane] = lo + width * next(rng);
    }
}

/// Canonical validated startup or saved snapshot for Random Values.
pub fn initial_random_values_state(node: &Node) -> Result<SavedModuleState, String> {
    RandomValues::build(node).map(|model| model.saved_state())
}
/// Reroll all 112 saved values using the current range and saved RNG stream.
pub fn randomize_random_values_state(
    node: &Node,
    state: SavedModuleState,
) -> Result<SavedModuleState, String> {
    let mut model = RandomValues::build(node)?;
    if !model.set_saved_state(state) {
        return Err(format!("node '{}': invalid Random Values state", node.id));
    }
    for lane in 0..16 {
        model.reroll(lane);
    }
    Ok(model.saved_state())
}
#[derive(Debug)]
pub(crate) struct RandomValues {
    range: usize,
    seed: u32,
    initial_values: [[f32; 16]; 7],
    initial_rng: u32,
    explicit_snapshot: bool,
    values: [[f32; 16]; 7],
    rng: u32,
    detectors: [u8; 16], // unknown, low, high
    pressed: bool,
    channels: u8,
    actions: [ModuleAction; MAX_MODULE_ACTIONS_PER_SAMPLE],
    count: usize,
}
impl RandomValues {
    pub(crate) fn build(node: &Node) -> Result<Self, String> {
        if node.kind != "random_values" {
            return Err("expected random_values node".into());
        }
        let mut reserved = 0;
        for name in node.params.keys() {
            if value_key(name) || matches!(name.as_str(), "rng_hi" | "rng_lo") {
                reserved += 1;
            } else if !matches!(name.as_str(), "range" | "seed") {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        if reserved != 0 && reserved != 114 {
            return Err(format!(
                "node '{}': Random Values snapshot requires all 112 values and rng_hi/rng_lo",
                node.id
            ));
        }
        let range = integer(node, "range", 0., 5.)? as usize;
        let seed = integer(node, "seed", 1., 16_777_215.)?;
        let mut rng = seed ^ SEED_XOR;
        let mut values = [[0.; 16]; 7];
        if reserved != 0 {
            for (output, row) in values.iter_mut().enumerate() {
                for (lane, value) in row.iter_mut().enumerate() {
                    *value = parameter(
                        node,
                        &format!("value{}_{}", output + 1, lane + 1),
                        0.,
                        -10.,
                        10.,
                    )?;
                }
            }
            rng = (integer(node, "rng_hi", 0., 65_535.)? << 16)
                | integer(node, "rng_lo", 0., 65_535.)?;
            if rng == 0 {
                return Err(format!(
                    "node '{}': saved random generator state must be nonzero",
                    node.id
                ));
            }
        } else {
            for lane in 0..16 {
                draw(&mut values, &mut rng, range, lane);
            }
        }
        Ok(Self {
            range,
            seed,
            initial_values: values,
            initial_rng: rng,
            explicit_snapshot: reserved != 0,
            values,
            rng,
            detectors: [0; 16],
            pressed: false,
            channels: 1,
            actions: [ModuleAction::PushRelease; MAX_MODULE_ACTIONS_PER_SAMPLE],
            count: 0,
        })
    }
    pub(crate) fn saved_state(&self) -> SavedModuleState {
        SavedModuleState::RandomValues {
            values: self.values.map(|row| row.map(f32::to_bits)),
            rng: self.rng,
        }
    }
    pub(crate) fn set_saved_state(&mut self, state: SavedModuleState) -> bool {
        let SavedModuleState::RandomValues { values, rng } = state else {
            return false;
        };
        let values = values.map(|row| row.map(f32::from_bits));
        if rng == 0
            || values
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || !(-10. ..=10.).contains(v))
        {
            return false;
        }
        self.values = values;
        self.rng = rng;
        true
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
            pressed: self.pressed,
            pressed_mask: u16::from(self.pressed),
        }
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        ModuleLightState {
            input_channels: Some(self.channels),
            manual: self.pressed,
            ..ModuleLightState::default()
        }
    }
    pub(crate) fn release(&mut self) {
        self.pressed = false;
        self.count = 0;
    }
    pub(crate) fn reset_state(&mut self) {
        self.values = self.initial_values;
        self.rng = self.initial_rng;
        self.detectors = [0; 16];
        self.channels = 1;
        self.release();
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        self.values = old.values;
        self.rng = old.rng;
        self.detectors = old.detectors;
        self.pressed = old.pressed;
        self.channels = old.channels;
        if self.explicit_snapshot != old.explicit_snapshot
            || (self.explicit_snapshot
                && (self.initial_values.map(|row| row.map(f32::to_bits))
                    != old.initial_values.map(|row| row.map(f32::to_bits))
                    || self.initial_rng != old.initial_rng))
        {
            self.values = self.initial_values;
            self.rng = self.initial_rng;
        } else if self.seed != old.seed {
            self.rng = self.seed ^ SEED_XOR;
            for lane in 0..16 {
                self.reroll(lane);
            }
        }
    }
    fn reroll(&mut self, lane: usize) {
        draw(&mut self.values, &mut self.rng, self.range, lane);
    }
    pub(crate) fn process(&mut self, input: PortFrame, outputs: &mut [PortFrame]) {
        let channels = usize::from(input.channels).clamp(1, 16);
        self.channels = channels as u8;
        let mut rises = [false; 16];
        for (lane, rising) in rises.iter_mut().enumerate().take(channels) {
            let value = input.channel(lane);
            let value = if value.is_finite() { value } else { 0. };
            *rising = self.detectors[lane] == 1 && value >= 1.;
            if value >= 1. {
                self.detectors[lane] = 2;
            } else if value <= 0.1 {
                self.detectors[lane] = 1;
            }
        }
        let mut first_press = true;
        for index in 0..self.count {
            match self.actions[index] {
                ModuleAction::PushPress if !self.pressed => {
                    self.pressed = true;
                    for lane in 0..channels {
                        self.reroll(lane);
                    }
                    if first_press {
                        rises.fill(false);
                        first_press = false;
                    }
                }
                ModuleAction::PushRelease => self.pressed = false,
                _ => {}
            }
        }
        self.count = 0;
        for (lane, rise) in rises.iter().enumerate().take(channels) {
            if *rise {
                self.reroll(lane);
            }
        }
        for (output, row) in outputs.iter_mut().zip(&self.values) {
            *output = PortFrame {
                channels: channels as u8,
                ..PortFrame::default()
            };
            output.values[..channels].copy_from_slice(&row[..channels]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node() -> Node {
        Node {
            content: None,
            rackmcp_sequence: None,
            score: None,
            id: "r".into(),
            kind: "random_values".into(),
            bypassed: false,
            params: Default::default(),
        }
    }
    fn sample(model: &mut RandomValues, voltages: &[f32]) -> [PortFrame; 7] {
        let mut input = PortFrame {
            channels: voltages.len() as u8,
            ..PortFrame::default()
        };
        input.values[..voltages.len()].copy_from_slice(voltages);
        let mut output = [PortFrame::default(); 7];
        model.process(input, &mut output);
        output
    }
    fn values(model: &RandomValues) -> [[f32; 16]; 7] {
        model.values
    }
    #[test]
    fn deterministic_startup_draw_order_and_all_ranges() {
        let mut expected_rng = 1 ^ SEED_XOR;
        let mut expected = [[0.; 16]; 7];
        for lane in 0..16 {
            for row in &mut expected {
                row[lane] = 10. * next(&mut expected_rng);
            }
        }
        let base = RandomValues::build(&node()).unwrap();
        assert_eq!(values(&base), expected);
        assert_eq!(base.rng, expected_rng);
        assert_eq!(expected[0][0].to_bits(), 0x3fcc_d148);
        for (index, (lo, width)) in RANGES.iter().enumerate() {
            let mut n = node();
            n.params.insert("range".into(), index as f32);
            let model = RandomValues::build(&n).unwrap();
            assert!(model
                .values
                .iter()
                .flatten()
                .all(|v| *v >= *lo && *v < lo + width));
        }
        for seed in [0., 1., 16_777_215.] {
            let mut n = node();
            n.params.insert("seed".into(), seed);
            assert_ne!(RandomValues::build(&n).unwrap().rng, 0);
        }
    }
    #[test]
    fn independent_poly_triggers_hysteresis_startup_and_retirement() {
        let mut model = RandomValues::build(&node()).unwrap();
        let initial = values(&model);
        sample(&mut model, &[10., 0., 0.]);
        assert_eq!(values(&model), initial);
        sample(&mut model, &[0.11, 0.99, 0.]);
        assert_eq!(values(&model), initial);
        sample(&mut model, &[0.1, 1., 0.]);
        let second = values(&model);
        for output in 0..7 {
            assert_eq!(second[output][0], initial[output][0]);
            assert_ne!(second[output][1], initial[output][1]);
            assert_eq!(second[output][2..], initial[output][2..]);
        }
        sample(&mut model, &[1.]);
        let first = values(&model);
        for output in 0..7 {
            assert_ne!(first[output][0], second[output][0]);
            assert_eq!(first[output][1..], second[output][1..]);
        }
        // Frozen high detector does not reroll when its lane returns high.
        let frames = sample(&mut model, &[1., 10., 0.]);
        assert_eq!(values(&model), first);
        assert!(frames
            .iter()
            .all(|f| f.channels == 3 && f.values[3..] == [0.; 13]));
        sample(&mut model, &[f32::NAN, f32::INFINITY, 0.]);
        sample(&mut model, &[1., 1., 0.]);
        assert_ne!(values(&model), first);
    }
    #[test]
    fn ordered_push_edges_coalesce_current_cv_and_preserve_quick_clicks() {
        let mut a = RandomValues::build(&node()).unwrap();
        let mut b = RandomValues::build(&node()).unwrap();
        sample(&mut a, &[0.; 16]);
        sample(&mut b, &[0.; 16]);
        a.enqueue(ModuleAction::PushPress);
        a.enqueue(ModuleAction::PushRelease);
        b.enqueue(ModuleAction::PushPress);
        b.enqueue(ModuleAction::PushRelease);
        assert_eq!(sample(&mut a, &[1.; 16]), sample(&mut b, &[0.; 16]));
        assert_eq!(a.saved_state(), b.saved_state());
        assert!(!a.controls().pressed);
        let previous = a.saved_state();
        a.enqueue(ModuleAction::PushRelease);
        sample(&mut a, &[1.; 16]);
        assert_eq!(a.saved_state(), previous);
        a.enqueue(ModuleAction::PushPress);
        a.enqueue(ModuleAction::PushPress);
        sample(&mut a, &[1.; 16]);
        b.enqueue(ModuleAction::PushPress);
        sample(&mut b, &[0.; 16]);
        assert_eq!(a.saved_state(), b.saved_state());
        a.enqueue(ModuleAction::PushRelease);
        a.enqueue(ModuleAction::PushPress);
        a.enqueue(ModuleAction::PushRelease);
        a.enqueue(ModuleAction::PushPress);
        b.enqueue(ModuleAction::PushRelease);
        b.enqueue(ModuleAction::PushPress);
        sample(&mut b, &[0.; 16]);
        b.enqueue(ModuleAction::PushRelease);
        b.enqueue(ModuleAction::PushPress);
        sample(&mut b, &[0.; 16]);
        sample(&mut a, &[1.; 16]);
        assert_eq!(a.saved_state(), b.saved_state());
        for _ in 0..MAX_MODULE_ACTIONS_PER_SAMPLE {
            assert!(a.enqueue(ModuleAction::PushRelease));
        }
        assert!(!a.enqueue(ModuleAction::PushPress));
        assert!(!a.enqueue(ModuleAction::HoldPress));
        a.release();
        assert!(!a.controls().pressed);
    }
    #[test]
    fn snapshots_validate_complete_state_and_do_not_rescale_restored_values() {
        let mut n = node();
        n.params.insert("value1_1".into(), 1.);
        assert!(RandomValues::build(&n).is_err());
        n.params.clear();
        for output in 1..=7 {
            for lane in 1..=16 {
                n.params.insert(format!("value{output}_{lane}"), -9.5);
            }
        }
        n.params.insert("rng_hi".into(), 1.);
        n.params.insert("rng_lo".into(), 2.);
        n.params.insert("range".into(), 2.);
        let mut model = RandomValues::build(&n).unwrap();
        assert_eq!(model.values, [[-9.5; 16]; 7]);
        assert_eq!(model.rng, 65_538);
        let saved = model.saved_state();
        let output = sample(&mut model, &[]);
        assert_eq!(output[0].values[0], -9.5);
        assert_eq!(output[0].channels, 1);
        assert!(model.set_saved_state(saved));
        for invalid in [f32::NAN, f32::INFINITY, -10.01, 10.01] {
            let mut vals = [[0.; 16]; 7];
            vals[0][0] = invalid;
            assert!(!model.set_saved_state(SavedModuleState::RandomValues {
                values: vals.map(|row| row.map(f32::to_bits)),
                rng: 1
            }));
            n.params.insert("value1_1".into(), invalid);
            assert!(RandomValues::build(&n).is_err());
        }
        n.params.insert("value1_1".into(), 0.);
        n.params.insert("rng_hi".into(), 0.);
        n.params.insert("rng_lo".into(), 0.);
        assert!(RandomValues::build(&n).is_err());
    }
    #[test]
    fn range_edits_preserve_held_state_seed_edits_reroll_and_adoption_keeps_local_queue() {
        let mut old = RandomValues::build(&node()).unwrap();
        old.enqueue(ModuleAction::PushPress);
        sample(&mut old, &[0.; 3]);
        old.enqueue(ModuleAction::PushRelease);
        let mut n = node();
        n.params.insert("range".into(), 5.);
        let mut changed = RandomValues::build(&n).unwrap();
        changed.enqueue(ModuleAction::PushRelease);
        changed.adopt(&old);
        assert_eq!(changed.saved_state(), old.saved_state());
        sample(&mut changed, &[0.; 3]);
        assert!(!changed.pressed);
        changed.enqueue(ModuleAction::PushPress);
        sample(&mut changed, &[0.; 3]);
        assert!(changed
            .values
            .iter()
            .all(|row| row[..3].iter().all(|v| (-1. ..1.).contains(v))));
        n.params.insert("seed".into(), 2.);
        let mut reseeded = RandomValues::build(&n).unwrap();
        let expected = reseeded.saved_state();
        reseeded.adopt(&old);
        assert_eq!(reseeded.saved_state(), expected);
        let snapshot = changed.saved_state();
        changed.release();
        assert_eq!(changed.saved_state(), snapshot);
        changed.reset_state();
        assert_eq!(changed.values, changed.initial_values);
        assert_eq!(changed.detectors, [0; 16]);
    }
}
