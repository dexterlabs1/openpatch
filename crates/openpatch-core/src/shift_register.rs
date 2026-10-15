//! Original MIT eight-stage sample-and-hold shift register.
//! Behavioral references inform the panel contract; no reference source is copied.
use crate::{
    parameter, ModuleAction, ModuleControlState, ModuleLightState, Node, PortFrame,
    SavedModuleState, MAX_MODULE_ACTIONS_PER_SAMPLE,
};
use serde_json::{json, Value};
pub(crate) const INPUTS: [&str; 16] = [
    "in1", "in2", "in3", "in4", "in5", "in6", "in7", "in8", "trig1", "trig2", "trig3", "trig4",
    "trig5", "trig6", "trig7", "trig8",
];
pub(crate) const OUTPUTS: [&str; 8] = [
    "out1", "out2", "out3", "out4", "out5", "out6", "out7", "out8",
];
const SEED_XOR: u32 = 0xA341_316C;
const LIMIT: f32 = 1_000_000.;
const RANGES: [(f32, f32); 6] = [
    (0., 10.),
    (0., 5.),
    (0., 1.),
    (-10., 20.),
    (-5., 10.),
    (-1., 2.),
];
pub(crate) fn catalog() -> Value {
    let mut m = json!({"kind":"sh_asr","title":"S&H / ASR","model":"functional","inputs":INPUTS,"description":"Eight mono sample-and-hold stages. Unpatched inputs shift the previous held stage and unpatched triggers inherit the preceding event. PUSH clocks the first normalled chain; CLEAR zeros all stages.","params":{
        "random":{"default":0.,"min":0.,"max":1.,"step":1.,"values":[0,1],"labels":["Off","On"]},
        "range":{"default":0.,"min":0.,"max":5.,"step":1.,"values":[0,1,2,3,4,5],"labels":["0–10 V","0–5 V","0–1 V","±10 V","±5 V","±1 V"]},
        "seed":{"default":1.,"min":0.,"max":16777215.,"step":1.,"description":"Startup seed; changing seed resets only the future random stream."}
    }});
    for row in 1..=8 {
        m["params"][format!("value{row}")] =
            json!({"default":0.,"min":-LIMIT,"max":LIMIT,"hidden":true});
    }
    for name in ["rng_hi", "rng_lo"] {
        m["params"][name] = json!({"default":0.,"min":0.,"max":65535.,"step":1.,"hidden":true});
    }
    m
}
pub(crate) fn annotate(m: &mut Value) {
    m["runtime_actions"] = json!(["push_press", "push_release", "reset_press", "reset_release"]);
    m["polyphony"] = json!({"version_1":"unsupported","version_2":"eight fixed mono outputs; channel zero of each input only","maximum_channels":1,"primary_input":null,"auxiliary_inputs":"zero channels means disconnected; unpatched inN samples previous old hold; unpatched trigN inherits preceding row event","retired_voices":"not applicable"});
    for meta in m["output_metadata"].as_array_mut().unwrap() {
        meta["maximum_channels"] = json!(1);
    }
    for (index, meta) in m["input_metadata"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        meta["maximum_channels"] = json!(1);
        if (1..8).contains(&index) {
            meta["normalization_source"] = json!(format!("old out{index}"));
        }
        if index > 8 {
            meta["normalization_source"] = json!(format!("trig{} event", index - 8));
        }
    }
    m["manual_control"] = json!({"buttons":["push","reset"],"labels":["PUSH","CLEAR"],"saved":false,"capacity_per_sample":MAX_MODULE_ACTIONS_PER_SAMPLE,"ordering":"CV edges coalesce with first effective PUSH; later PUSH rises capture sequentially; any effective CLEAR zeros holds after captures, retaining consumed RNG","release_safety":"release controls/actions only"});
    m["trigger_policy"] = json!({"thresholds":"low <=0.1 V, high >=1 V; initial high establishes history without edge; intermediate unknown stays unknown; disconnected detectors freeze","normaling":"snapshot old holds per capture pass; patched triggers break inherited event even when false","bypass":"eight mono zeros; values/RNG/detectors freeze; transitions release controls/actions","finite_safety":"nonfinite input becomes zero; sampled voltage clamped to ±1000000 V"});
    m["random_policy"] = json!({"algorithm":"xorshift32 shifts 13,17,5; initial seed XOR 0xA341316C; advance then top24 / 16777216; f32 low + width * uniform","draw_order":"exactly one draw per row1 capture when random=1, even with patched in1 or simultaneous CLEAR; no startup draws","seed":"change resets RNG only","range":"0:0..10, 1:0..5, 2:0..1, 3:-10..10, 4:-5..5, 5:-1..1 V; changes affect future captures only"});
    m["randomization_policy"] =
        json!("disabled for entire module, including parameters, held values and RNG");
    m["saved_state"] = json!({"values":"eight actual f32 voltages encoded as value1..value8","rng":"nonzero u32 encoded as integer rng_hi/rng_lo","validation":"all ten hidden params or none; finite values within ±1000000 V"});
    m["state_adoption"] = json!("same identity/kind/rate retains values, RNG, detectors and physical controls; changed complete snapshot overrides; changed seed resets RNG only; pending actions remain receiving-engine owned; explicit host restore overrides even equal configured values");
    m["initialization_policy"] = json!("default random/range/seed; zero holds and default RNG; release controls/actions; preserve detector history");
    m["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/SHASR.cpp");
    m["compatibility"] = json!("original MIT Rust; deterministic PRNG, initial-high suppression, ordered actions and complete snapshot validation are OpenPatch policies");
}
fn integer(node: &Node, name: &str, default: f32, max: f32) -> Result<u32, String> {
    let v = parameter(node, name, default, 0., max)?;
    if v.fract() != 0. {
        return Err(format!("node '{}': {name} must be an integer", node.id));
    }
    Ok(v as u32)
}
fn clean(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(-LIMIT, LIMIT)
    } else {
        0.
    }
}
/// Validate and return the canonical startup or complete saved state of sh_asr.
pub fn initial_shift_register_state(node: &Node) -> Result<SavedModuleState, String> {
    ShiftRegister::build(node).map(|m| m.saved_state())
}
#[derive(Debug)]
pub(crate) struct ShiftRegister {
    random: bool,
    range: usize,
    seed: u32,
    initial_values: [f32; 8],
    initial_rng: u32,
    explicit_snapshot: bool,
    values: [f32; 8],
    rng: u32,
    detectors: [u8; 8],
    push: bool,
    clear: bool,
    actions: [ModuleAction; MAX_MODULE_ACTIONS_PER_SAMPLE],
    count: usize,
}
impl ShiftRegister {
    pub(crate) fn build(node: &Node) -> Result<Self, String> {
        if node.kind != "sh_asr" {
            return Err("expected sh_asr node".into());
        }
        let mut reserved = 0;
        for name in node.params.keys() {
            if matches!(name.as_str(), "rng_hi" | "rng_lo")
                || (1..=8).any(|i| name == &format!("value{i}"))
            {
                reserved += 1;
            } else if !matches!(name.as_str(), "random" | "range" | "seed") {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        if reserved != 0 && reserved != 10 {
            return Err(format!(
                "node '{}': sh_asr snapshot requires all eight values and rng_hi/rng_lo",
                node.id
            ));
        }
        let random = integer(node, "random", 0., 1.)? != 0;
        let range = integer(node, "range", 0., 5.)? as usize;
        let seed = integer(node, "seed", 1., 16_777_215.)?;
        let mut values = [0.; 8];
        let mut rng = seed ^ SEED_XOR;
        if reserved != 0 {
            for (i, v) in values.iter_mut().enumerate() {
                *v = parameter(node, &format!("value{}", i + 1), 0., -LIMIT, LIMIT)?;
            }
            rng =
                (integer(node, "rng_hi", 0., 65535.)? << 16) | integer(node, "rng_lo", 0., 65535.)?;
            if rng == 0 {
                return Err(format!("node '{}': saved RNG must be nonzero", node.id));
            }
        }
        Ok(Self {
            random,
            range,
            seed,
            initial_values: values,
            initial_rng: rng,
            explicit_snapshot: reserved != 0,
            values,
            rng,
            detectors: [0; 8],
            push: false,
            clear: false,
            actions: [ModuleAction::PushRelease; MAX_MODULE_ACTIONS_PER_SAMPLE],
            count: 0,
        })
    }
    pub(crate) fn saved_state(&self) -> SavedModuleState {
        SavedModuleState::ShiftRegister {
            values: self.values.map(f32::to_bits),
            rng: self.rng,
        }
    }
    pub(crate) fn set_saved_state(&mut self, state: SavedModuleState) -> bool {
        let SavedModuleState::ShiftRegister { values, rng } = state else {
            return false;
        };
        let values = values.map(f32::from_bits);
        if rng == 0
            || values
                .iter()
                .any(|v| !v.is_finite() || !(-LIMIT..=LIMIT).contains(v))
        {
            return false;
        }
        self.values = values;
        self.rng = rng;
        true
    }
    pub(crate) fn supports(a: ModuleAction) -> bool {
        matches!(
            a,
            ModuleAction::PushPress
                | ModuleAction::PushRelease
                | ModuleAction::ResetPress
                | ModuleAction::ResetRelease
        )
    }
    pub(crate) fn enqueue(&mut self, a: ModuleAction) -> bool {
        if !Self::supports(a) || self.count == self.actions.len() {
            return false;
        }
        self.actions[self.count] = a;
        self.count += 1;
        true
    }
    pub(crate) fn controls(&self) -> ModuleControlState {
        ModuleControlState {
            pressed: self.push || self.clear,
            pressed_mask: u16::from(self.push) | (u16::from(self.clear) << 1),
        }
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        ModuleLightState {
            manual: self.push,
            ..ModuleLightState::default()
        }
    }
    pub(crate) fn release(&mut self) {
        self.push = false;
        self.clear = false;
        self.count = 0;
    }
    pub(crate) fn reset_state(&mut self) {
        self.values = self.initial_values;
        self.rng = self.initial_rng;
        self.detectors = [0; 8];
        self.release();
    }
    pub(crate) fn initialize_state(&mut self) {
        // Parameters already describe the final compiled graph, including edits
        // later in the same atomic host batch. Initialize runtime storage only.
        self.values = [0.; 8];
        self.rng = self.seed ^ SEED_XOR;
        self.release();
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        self.values = old.values;
        self.rng = old.rng;
        self.detectors = old.detectors;
        self.push = old.push;
        self.clear = old.clear;
        if self.explicit_snapshot
            && (!old.explicit_snapshot
                || self.initial_values.map(f32::to_bits) != old.initial_values.map(f32::to_bits)
                || self.initial_rng != old.initial_rng)
        {
            self.values = self.initial_values;
            self.rng = self.initial_rng;
        } else if self.seed != old.seed {
            self.rng = self.seed ^ SEED_XOR;
        }
    }
    fn capture(&mut self, inputs: &[PortFrame; 16], cv: [bool; 8], push: bool) {
        let old = self.values;
        let mut event = false;
        for row in 0..8 {
            event = if inputs[8 + row].channels != 0 {
                cv[row]
            } else {
                event
            };
            if row == 0 {
                event |= push;
            }
            if event {
                let normal = if row == 0 {
                    if self.random {
                        self.rng ^= self.rng << 13;
                        self.rng ^= self.rng >> 17;
                        self.rng ^= self.rng << 5;
                        let (lo, width) = RANGES[self.range];
                        lo + width * ((self.rng >> 8) as f32 / 16_777_216.)
                    } else {
                        0.
                    }
                } else {
                    old[row - 1]
                };
                self.values[row] = if inputs[row].channels == 0 {
                    normal
                } else {
                    clean(inputs[row].channel(0))
                };
            }
        }
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame; 16], outputs: &mut [PortFrame]) {
        let mut cv = [false; 8];
        for row in 0..8 {
            if inputs[8 + row].channels != 0 {
                let v = clean(inputs[8 + row].channel(0));
                cv[row] = self.detectors[row] == 1 && v >= 1.;
                if v >= 1. {
                    self.detectors[row] = 2;
                } else if v <= 0.1 {
                    self.detectors[row] = 1;
                }
            }
        }
        let mut pushes = 0;
        let mut clear = false;
        for i in 0..self.count {
            match self.actions[i] {
                ModuleAction::PushPress if !self.push => {
                    self.push = true;
                    pushes += 1;
                }
                ModuleAction::PushRelease => self.push = false,
                ModuleAction::ResetPress if !self.clear => {
                    self.clear = true;
                    clear = true;
                }
                ModuleAction::ResetRelease => self.clear = false,
                _ => {}
            }
        }
        self.count = 0;
        self.capture(inputs, cv, pushes > 0);
        for _ in 1..pushes {
            self.capture(inputs, [false; 8], true);
        }
        if clear {
            self.values = [0.; 8];
        }
        for (output, v) in outputs.iter_mut().zip(self.values) {
            *output = PortFrame::mono(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node() -> Node {
        Node {
            content: None,
            score: None,
            id: "s".into(),
            kind: "sh_asr".into(),
            bypassed: false,
            params: Default::default(),
        }
    }
    fn sample(m: &mut ShiftRegister, ins: &[PortFrame; 16]) -> [PortFrame; 8] {
        let mut out = [PortFrame::default(); 8];
        m.process(ins, &mut out);
        out
    }
    fn click(m: &mut ShiftRegister) {
        assert!(m.enqueue(ModuleAction::PushPress));
        assert!(m.enqueue(ModuleAction::PushRelease));
    }
    #[test]
    fn independent_shift_vectors_old_hold_snapshot_and_first_channel() {
        let mut m = ShiftRegister::build(&node()).unwrap();
        let mut ins = [PortFrame::default(); 16];
        ins[0] = PortFrame {
            channels: 16,
            values: [3.; 16],
        };
        ins[0].values[1] = 999.;
        for expected in [
            [3., 0., 0., 0., 0., 0., 0., 0.],
            [3., 3., 0., 0., 0., 0., 0., 0.],
            [3., 3., 3., 0., 0., 0., 0., 0.],
        ] {
            click(&mut m);
            let out = sample(&mut m, &ins);
            assert_eq!(m.values, expected);
            assert!(out
                .iter()
                .all(|f| f.channels == 1 && f.values[1..] == [0.; 15]));
        }
        ins[0] = PortFrame::default();
        click(&mut m);
        sample(&mut m, &ins);
        assert_eq!(m.values, [0., 3., 3., 3., 0., 0., 0., 0.]);
        ins[3] = PortFrame::mono(-7.);
        click(&mut m);
        sample(&mut m, &ins);
        assert_eq!(m.values, [0., 0., 3., -7., 3., 0., 0., 0.]);
    }
    #[test]
    fn split_clocks_break_inheritance_and_downstream_cv_coalesces_first_push() {
        let mut m = ShiftRegister::build(&node()).unwrap();
        m.values = [1., 2., 3., 4., 5., 6., 7., 8.];
        let mut ins = [PortFrame::default(); 16];
        ins[10] = PortFrame::mono(0.);
        ins[13] = PortFrame::mono(0.);
        sample(&mut m, &ins);
        ins[13] = PortFrame::mono(10.);
        click(&mut m);
        click(&mut m);
        sample(&mut m, &ins);
        // First pass clocks rows 1/2 and independent 6/7/8; second manual pass only rows 1/2.
        assert_eq!(m.values, [0., 0., 3., 4., 5., 5., 6., 7.]);
        ins[10] = PortFrame::mono(1.);
        sample(&mut m, &ins);
        assert_eq!(m.values, [0., 0., 0., 3., 4., 5., 6., 7.]);
    }
    #[test]
    fn initial_high_unknown_hysteresis_and_disconnect_history() {
        let mut m = ShiftRegister::build(&node()).unwrap();
        let mut ins = [PortFrame::default(); 16];
        ins[0] = PortFrame::mono(4.);
        for v in [0.5, 1., 0.5, 1.] {
            ins[8] = PortFrame::mono(v);
            sample(&mut m, &ins);
            assert_eq!(m.values, [0.; 8]);
        }
        ins[8] = PortFrame::mono(0.1);
        sample(&mut m, &ins);
        ins[8] = PortFrame::default();
        sample(&mut m, &ins);
        ins[8] = PortFrame::mono(1.);
        sample(&mut m, &ins);
        assert_eq!(m.values[0], 4.);
        ins[0] = PortFrame::mono(9.);
        ins[8] = PortFrame::default();
        sample(&mut m, &ins);
        ins[8] = PortFrame::mono(1.);
        sample(&mut m, &ins);
        assert_eq!(m.values[0], 4.);
        ins[8] = PortFrame::mono(f32::NAN);
        sample(&mut m, &ins);
        ins[8] = PortFrame::mono(1.);
        sample(&mut m, &ins);
        assert_eq!(m.values[0], 9.);
    }
    #[test]
    fn ordered_clear_wins_rng_draws_exactly_once_even_patched_and_no_startup_draw() {
        let mut n = node();
        n.params.insert("random".into(), 1.);
        let mut m = ShiftRegister::build(&n).unwrap();
        assert_eq!(m.rng, 0xa341316d);
        assert_eq!(m.values, [0.; 8]);
        let mut ins = [PortFrame::default(); 16];
        click(&mut m);
        sample(&mut m, &ins);
        assert_eq!(m.values[0].to_bits(), 0x3fccd148);
        assert_eq!(m.rng, 0x28f6a8bb);
        // Repeated press while physically held contributes only one capture.
        m.enqueue(ModuleAction::PushPress);
        m.enqueue(ModuleAction::PushPress);
        ins[0] = PortFrame::mono(12.);
        sample(&mut m, &ins);
        assert_eq!(m.values[0], 12.);
        assert_eq!(m.rng, 0x41d77f2b);
        m.enqueue(ModuleAction::ResetPress);
        m.enqueue(ModuleAction::PushRelease);
        m.enqueue(ModuleAction::PushPress);
        m.enqueue(ModuleAction::PushRelease);
        m.enqueue(ModuleAction::PushPress);
        sample(&mut m, &ins);
        assert_eq!(m.values, [0.; 8]);
        assert_eq!(m.rng, 0x3714dba8);
        assert_eq!(m.controls().pressed_mask, 3);
        sample(&mut m, &ins);
        assert_eq!(m.values, [0.; 8]);
        assert_eq!(m.rng, 0x3714dba8);
        // Held clear does not continuously zero newly captured voltage.
        m.enqueue(ModuleAction::PushRelease);
        m.enqueue(ModuleAction::PushPress);
        sample(&mut m, &ins);
        assert_eq!(m.values[0], 12.);
        let state = m.saved_state();
        m.release();
        assert_eq!(m.saved_state(), state);
        assert_eq!(m.controls().pressed_mask, 0);
    }
    #[test]
    fn current_cv_coalesces_first_manual_and_clear_order_retains_consumed_rng() {
        let mut n = node();
        n.params.insert("random".into(), 1.);
        let mut m = ShiftRegister::build(&n).unwrap();
        let mut inputs = [PortFrame::default(); 16];
        inputs[8] = PortFrame::mono(0.);
        inputs[12] = PortFrame::mono(0.);
        sample(&mut m, &inputs);
        inputs[8] = PortFrame::mono(1.);
        inputs[12] = PortFrame::mono(1.);
        // CLEAR before captures still wins; CV row1 and first PUSH share a capture.
        m.enqueue(ModuleAction::ResetPress);
        m.enqueue(ModuleAction::ResetRelease);
        click(&mut m);
        click(&mut m);
        sample(&mut m, &inputs);
        assert_eq!(m.values, [0.; 8]);
        assert_eq!(m.rng, 0x41d77f2b);
        // A downstream clock never draws from row1's random stream.
        inputs[12] = PortFrame::mono(0.);
        sample(&mut m, &inputs);
        inputs[12] = PortFrame::mono(1.);
        sample(&mut m, &inputs);
        assert_eq!(m.rng, 0x41d77f2b);
    }
    #[test]
    fn finite_input_safety_complete_snapshot_and_typed_validation() {
        let mut n = node();
        let mut m = ShiftRegister::build(&n).unwrap();
        let mut ins = [PortFrame::default(); 16];
        for (v, want) in [
            (f32::NAN, 0.),
            (f32::INFINITY, 0.),
            (2e6, 1e6),
            (-2e6, -1e6),
        ] {
            ins[0] = PortFrame::mono(v);
            click(&mut m);
            sample(&mut m, &ins);
            assert_eq!(m.values[0], want);
        }
        n.params.insert("value1".into(), 0.);
        assert!(ShiftRegister::build(&n).is_err());
        m.saved_state().write_params(&mut n.params);
        assert_eq!(initial_shift_register_state(&n).unwrap(), m.saved_state());
        for v in [f32::NAN, f32::INFINITY, 1_000_001., -1_000_001.] {
            n.params.insert("value1".into(), v);
            assert!(ShiftRegister::build(&n).is_err());
            assert!(!m.set_saved_state(SavedModuleState::ShiftRegister {
                values: [v.to_bits(); 8],
                rng: 1
            }));
        }
        assert!(!m.set_saved_state(SavedModuleState::ShiftRegister {
            values: [0; 8],
            rng: 0
        }));
        n.params.insert("value1".into(), 0.);
        n.params.insert("rng_hi".into(), 0.);
        n.params.insert("rng_lo".into(), 0.);
        assert!(ShiftRegister::build(&n).is_err());
    }
    #[test]
    fn adoption_seed_and_range_edits_snapshot_override_local_actions_and_initialize_history() {
        let mut n = node();
        n.params.insert("random".into(), 1.);
        let mut old = ShiftRegister::build(&n).unwrap();
        let mut ins = [PortFrame::default(); 16];
        ins[8] = PortFrame::mono(0.);
        click(&mut old);
        sample(&mut old, &ins);
        old.enqueue(ModuleAction::PushPress);
        n.params.insert("range".into(), 5.);
        let mut changed = ShiftRegister::build(&n).unwrap();
        click(&mut changed);
        changed.adopt(&old);
        assert_eq!(changed.saved_state(), old.saved_state());
        sample(&mut changed, &ins);
        assert!((-1. ..1.).contains(&changed.values[0]));
        assert!(!changed.push);
        n.params.insert("seed".into(), 17.);
        let mut reseed = ShiftRegister::build(&n).unwrap();
        reseed.adopt(&old);
        assert_eq!(reseed.values, old.values);
        assert_eq!(reseed.rng, 17 ^ SEED_XOR);
        assert_eq!(reseed.detectors, old.detectors);
        let restored = SavedModuleState::ShiftRegister {
            values: [(-123f32).to_bits(); 8],
            rng: 7,
        };
        restored.write_params(&mut n.params);
        let mut snapshot = ShiftRegister::build(&n).unwrap();
        snapshot.adopt(&old);
        assert_eq!(snapshot.saved_state(), restored);
        let mut bare = node();
        bare.params.insert("seed".into(), 17.);
        let mut without_snapshot = ShiftRegister::build(&bare).unwrap();
        without_snapshot.adopt(&snapshot);
        // Removing a configured snapshot is not a new complete restore.
        assert_eq!(without_snapshot.saved_state(), restored);
        snapshot.detectors = [2; 8];
        snapshot.initialize_state();
        assert_eq!(snapshot.values, [0.; 8]);
        assert_eq!(snapshot.rng, 17 ^ SEED_XOR);
        assert_eq!(snapshot.detectors, [2; 8]);
        assert!(snapshot.random);
        assert_eq!(snapshot.range, 5);
        click(&mut snapshot);
        sample(&mut snapshot, &[PortFrame::default(); 16]);
        assert_eq!(snapshot.values[0].to_bits(), 0xbf2e96ac);
        assert_eq!(snapshot.rng, 0x28b4aa8a);
        snapshot.reset_state();
        assert_eq!(snapshot.saved_state(), restored);
        assert_eq!(snapshot.detectors, [0; 8]);
    }
    #[test]
    fn ranges_and_random_off_draw_consumption_and_action_capacity() {
        for (range, (lo, width)) in RANGES.iter().enumerate() {
            let mut n = node();
            n.params.insert("random".into(), 1.);
            n.params.insert("range".into(), range as f32);
            let mut m = ShiftRegister::build(&n).unwrap();
            click(&mut m);
            sample(&mut m, &[PortFrame::default(); 16]);
            assert_eq!(m.values[0], lo + width * (0x28f6a8u32 as f32 / 16_777_216.));
        }
        let mut m = ShiftRegister::build(&node()).unwrap();
        let rng = m.rng;
        for _ in 0..MAX_MODULE_ACTIONS_PER_SAMPLE / 2 {
            click(&mut m);
        }
        assert!(!m.enqueue(ModuleAction::PushPress));
        sample(&mut m, &[PortFrame::default(); 16]);
        assert_eq!(m.rng, rng);
        assert!(!m.enqueue(ModuleAction::Clock));
    }
}
