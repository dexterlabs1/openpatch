//! Original signal utilities with bounded f64 arithmetic and fixed voice state.
use crate::{
    parameter, ModuleAction, Node, PortFrame, MAX_MODULE_ACTIONS_PER_SAMPLE, SIGNAL_LIMIT,
};
use serde_json::{json, Value};

pub const MAX_MODULE_LIGHTS: usize = 10;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ModuleControlState {
    pub pressed: bool,
    /// One bit per held button; single-button utilities use bit zero.
    pub pressed_mask: u16,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct ModuleLightState {
    /// SUM input channel count, refreshed together with its six meter bands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_channels: Option<u8>,
    pub count: u8,
    pub brightness: [[f32; 2]; MAX_MODULE_LIGHTS],
    pub manual: bool,
}
pub(crate) fn input_names(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "logic" | "compare" => Some(&["a", "b"]),
        "process" => Some(&["slew_cv", "signal", "gate"]),
        "rescale" => Some(&["signal"]),
        _ => None,
    }
}
pub(crate) fn output_names(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "logic" => Some(&["not_a", "not_b", "or", "nor", "and", "nand", "xor", "xnor"]),
        "compare" => Some(&[
            "max",
            "min",
            "clip",
            "lim",
            "clip_gate",
            "lim_gate",
            "greater",
            "less",
        ]),
        "process" => Some(&["sh1", "sh2", "track_hold", "hold_track", "slew", "glide"]),
        "rescale" => Some(&["out"]),
        _ => None,
    }
}
fn slew_min() -> f32 {
    0.001_f32.log2()
}
fn slew_max() -> f32 {
    10.0_f32.log2()
}
pub(crate) fn catalog() -> [Value; 4] {
    [
        json!({"kind":"logic","title":"LOGIC","model":"functional","inputs":input_names("logic").unwrap(),"params":{},"description":"Eight Boolean outputs at 0/10 V. Inputs high at >=1 V, without hysteresis; mono broadcasts. Momentary B press forces B high on every lane and releases back to the input. Button state is runtime-only, never a saved latch parameter."}),
        json!({"kind":"compare","title":"COMPARE","model":"functional","inputs":input_names("compare").unwrap(),"params":{"b_offset":{"default":0.0,"min":-10.0,"max":10.0,"unit":"V"}},"description":"Max/min, symmetric clipping and remainder, outside/inside gates, and strict A>B/A<B gates. B offset applies to every lane including absent B lanes. Raw mono inputs zero-pad rather than broadcast. Boundaries count as inside; equal A/B makes both comparison gates low."}),
        json!({"kind":"process","title":"PROCESS","model":"functional","inputs":input_names("process").unwrap(),"params":{"slew":{"default":slew_min(),"min":slew_min(),"max":slew_max(),"unit":"log2(s/V)","description":"Log2 seconds per volt; minimum is instant and ignores slew CV. Other values produce rate 2^-(slew+CV) V/s."}},"description":"Two-stage sample-and-hold, track-and-hold, hold-and-track, gate-dependent slew and legato glide. Signal sets active voices; gate/slew CV mono broadcast. Momentary gate is runtime-only. Empty signal publishes mono zero without processing lanes; inactive lane histories freeze."}),
        json!({"kind":"rescale","title":"RESCALE","model":"functional","inputs":input_names("rescale").unwrap(),"params":{
            "gain":{"default":0.0,"min":-1.0,"max":1.0,"unit":"x"},"offset":{"default":0.0,"min":-10.0,"max":10.0,"unit":"V"},
            "min":{"default":-10.0,"min":-10.0,"max":10.0,"unit":"V"},"max":{"default":10.0,"min":-10.0,"max":10.0,"unit":"V"},
            "multiplier":{"default":1.0,"min":1.0,"max":1000.0,"values":[1,10,100,1000]},
            "reflect_min":{"default":0.0,"min":0.0,"max":1.0,"step":1.0},"reflect_max":{"default":0.0,"min":0.0,"max":1.0,"step":1.0}},
            "description":"Affine gain × multiplier then offset, followed by limiting or reflection. If max<=min returns min. One reflected bound mirrors once then clips at the other; both reflect cyclically with positive modulo. Multiplier and reflection settings are saved validated parameters."}),
    ]
}
pub(crate) fn annotate(module: &mut Value) {
    let kind = module["kind"].as_str().unwrap().to_owned();
    if input_names(&kind).is_none() {
        return;
    }
    module["polyphony"]["version_2"]=json!(match kind.as_str(){
        "logic"=>"max(a,b,1); mono broadcasts, absent short-poly lanes zero; manual B never creates voices",
        "compare"=>"max(a,b,1); raw channel reads, mono zero-pads; B offset applies on every lane",
        "process"=>"signal channel count; zero-channel signal publishes mono zero without processing any lane; gate/slew CV mono broadcast, short-poly lanes zero; inactive lane histories freeze",
        _=>"signal count, minimum one; disconnected signal is zero"
    });
    module["polyphony"]["primary_input"] = if kind == "process" || kind == "rescale" {
        json!("signal")
    } else {
        Value::Null
    };
    module["polyphony"]["retired_voices"] = json!(if kind == "process" {
        "inactive fixed lanes freeze and resume, including after signal disconnect; no generic retirement reset"
    } else {
        "stateless; inactive output slots zero"
    });
    module["polyphony"]["auxiliary_inputs"] = json!(if kind == "compare" {
        "raw zero-padding, including mono; B offset remains global"
    } else {
        "mono broadcasts; missing short-poly lanes zero; no nonzero normals"
    });
    module["functional_reference"]=json!(format!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/{}.cpp",match kind.as_str(){"logic"=>"Logic","compare"=>"Compare","process"=>"Process",_=>"Rescale"}));
    module["arithmetic_policy"]=json!("nonfinite input becomes zero; f64 arithmetic; final output clamp ±1,000,000 V; no source f32/SIMD bit-exactness claim");
    if kind == "logic" || kind == "process" {
        module["manual_control"] = json!({"actions":if kind=="logic" {vec!["b_press","b_release"]} else {vec!["gate_press","gate_release"]},"capacity_per_sample":MAX_MODULE_ACTIONS_PER_SAMPLE,"ordering":"ordered actions before CV evaluation; final same-frame physical button state wins","saved":false,"release_safety":"release_module_controls clears pending manual actions and held button while preserving DSP histories; bypass transitions release controls"});
    }
    module["runtime_actions"] = json!(match kind.as_str() {
        "logic" => vec!["b_press", "b_release"],
        "process" => vec!["gate_press", "gate_release"],
        _ => vec![],
    });
    module["light_names"] = json!(match kind.as_str() {
        "logic" => vec!["not_a", "not_b", "or", "nor", "and", "nand", "xor", "xnor"],
        "compare" => vec!["clip_gate", "lim_gate", "greater", "less"],
        "rescale" => vec!["max", "min"],
        _ => vec![],
    });
    module["light_policy"] = json!({"pairs":"index zero mono, index one poly; any active lane turns on the applicable indicator","brightness":"immediate upward changes, decay by value += (target-value)*30*elapsed seconds","divider":if kind=="rescale"{16}else if kind=="process"{1}else{32},"rescale":"inclusive affine pre-limit max/min comparisons","manual":"logic B and Process gate held state only; Process has no paired indicators","bypass":"paired values and divider freeze; emergency release clears manual indicator"});
    if kind == "process" {
        module["slew_policy"] = json!({"rate":"instant at minimum, otherwise 2^clamp(-(slew+slewCV),-64,64) V/s; CV adds in log2 space","gates":"starts low; rise >=2 V or held manual gate, fall <=0.1 V while manual gate released","glide":"gate high for strictly more than 1 ms starts gliding; high-sample counter increments before this test","sample_hold":"rising edge shifts raw sample1 to sample2, captures new sample1; each output has its own slew filter","track_hold":"high holds rising-edge value, low tracks; hold_track high tracks, low holds falling-edge value","slew":"high follows immediately, low slews","initial_state":"all sample/hold/filter values zero; gate low","parameter_reset":"reset/load parameters alone does not imply clearing adopted lane histories"});
    }
    module["compatibility"]=json!("original Rust functional implementation; context settings are validated parameters, manual buttons are runtime events; bounded arithmetic and deterministic sample-based glide timing are explicit OpenPatch policies");
}
fn finite(v: f32) -> f64 {
    if v.is_finite() {
        f64::from(v)
    } else {
        0.0
    }
}
fn voltage(v: f64) -> f32 {
    v.clamp(-f64::from(SIGNAL_LIMIT), f64::from(SIGNAL_LIMIT)) as f32
}
#[derive(Clone, Debug)]
struct Manual {
    pressed: bool,
    actions: [ModuleAction; MAX_MODULE_ACTIONS_PER_SAMPLE],
    count: usize,
}
impl Default for Manual {
    fn default() -> Self {
        Self {
            pressed: false,
            actions: [ModuleAction::BRelease; MAX_MODULE_ACTIONS_PER_SAMPLE],
            count: 0,
        }
    }
}
impl Manual {
    fn enqueue(&mut self, action: ModuleAction) -> bool {
        if self.count == self.actions.len() {
            return false;
        }
        self.actions[self.count] = action;
        self.count += 1;
        true
    }
    fn apply(&mut self) {
        for action in &self.actions[..self.count] {
            self.pressed = matches!(action, ModuleAction::BPress | ModuleAction::GatePress);
        }
        self.count = 0;
    }
    fn release(&mut self) {
        self.pressed = false;
        self.count = 0;
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct Lane {
    high: bool,
    on_samples: u32,
    sample1: f64,
    sample2: f64,
    filtered1: f64,
    filtered2: f64,
    hold: f64,
    slew: f64,
    glide: f64,
}
fn slew(value: &mut f64, target: f64, delta: f64) -> f64 {
    *value += (target - *value).clamp(-delta, delta);
    *value
}
#[derive(Clone, Debug)]
pub(crate) struct Processor {
    setting: f32,
    rate: f64,
    legato_samples: u32,
    lanes: [Lane; 16],
    manual: Manual,
}
impl Processor {
    fn process(&mut self, inputs: &[PortFrame; 3], out: &mut [PortFrame]) {
        self.manual.apply();
        let count = inputs[1].channels.min(16);
        for frame in out.iter_mut() {
            *frame = PortFrame {
                channels: count.max(1),
                ..PortFrame::default()
            };
        }
        for index in 0..usize::from(count) {
            let input = finite(inputs[1].channel(index));
            let gate = finite(inputs[2].voice(index, 0.0));
            let delta = if self.setting <= slew_min() {
                f64::INFINITY
            } else {
                2_f64.powf(
                    (-(f64::from(self.setting) + finite(inputs[0].voice(index, 0.0))))
                        .clamp(-64.0, 64.0),
                ) / self.rate
            };
            let lane = &mut self.lanes[index];
            if !lane.high && (gate >= 2.0 || self.manual.pressed) {
                lane.high = true;
                lane.on_samples = 0;
                lane.hold = input;
                lane.sample2 = lane.sample1;
                lane.sample1 = input;
            } else if lane.high && gate <= f64::from(0.1_f32) && !self.manual.pressed {
                lane.high = false;
                lane.hold = input;
            }
            let track_hold = if lane.high { lane.hold } else { input };
            let hold_track = if lane.high { input } else { lane.hold };
            if lane.high {
                lane.slew = input;
                lane.on_samples = lane.on_samples.saturating_add(1).min(self.legato_samples);
            } else {
                slew(&mut lane.slew, input, delta);
            }
            if lane.high && lane.on_samples >= self.legato_samples {
                slew(&mut lane.glide, input, delta);
            } else {
                lane.glide = input;
            }
            let values = [
                slew(&mut lane.filtered1, lane.sample1, delta),
                slew(&mut lane.filtered2, lane.sample2, delta),
                track_hold,
                hold_track,
                lane.slew,
                lane.glide,
            ];
            for (frame, value) in out.iter_mut().zip(values) {
                frame.values[index] = voltage(value);
            }
        }
    }
}
#[derive(Clone, Debug)]
enum SignalDsp {
    Logic(Manual),
    Compare(f64),
    Process(Box<Processor>),
    Rescale {
        gain: f64,
        offset: f64,
        min: f64,
        max: f64,
        reflect_min: bool,
        reflect_max: bool,
    },
}
impl SignalDsp {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        let allowed: &[&str] = match node.kind.as_str() {
            "logic" => &[],
            "compare" => &["b_offset"],
            "process" => &["slew"],
            _ => &[
                "gain",
                "offset",
                "min",
                "max",
                "multiplier",
                "reflect_min",
                "reflect_max",
            ],
        };
        for name in node.params.keys() {
            if !allowed.contains(&name.as_str()) {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        let p = |name, default, min, max| parameter(node, name, default, min, max);
        Ok(match node.kind.as_str() {
            "logic" => Self::Logic(Manual::default()),
            "compare" => Self::Compare(f64::from(p("b_offset", 0.0, -10.0, 10.0)?)),
            "process" => Self::Process(Box::new(Processor {
                setting: p("slew", slew_min(), slew_min(), slew_max())?,
                rate: f64::from(rate),
                legato_samples: (f64::from(rate) * 0.001).floor() as u32 + 1,
                lanes: [Lane::default(); 16],
                manual: Manual::default(),
            })),
            _ => {
                let multiplier = p("multiplier", 1.0, 1.0, 1000.0)?;
                if ![1.0, 10.0, 100.0, 1000.0].contains(&multiplier) {
                    return Err(format!(
                        "node '{}': multiplier must be 1, 10, 100 or 1000",
                        node.id
                    ));
                }
                let flag = |name| -> Result<bool, String> {
                    let v = p(name, 0.0, 0.0, 1.0)?;
                    if v.fract() != 0.0 {
                        return Err(format!(
                            "node '{}': {name} must be a boolean integer",
                            node.id
                        ));
                    }
                    Ok(v != 0.0)
                };
                Self::Rescale {
                    gain: f64::from(p("gain", 0.0, -1.0, 1.0)?) * f64::from(multiplier),
                    offset: f64::from(p("offset", 0.0, -10.0, 10.0)?),
                    min: f64::from(p("min", -10.0, -10.0, 10.0)?),
                    max: f64::from(p("max", 10.0, -10.0, 10.0)?),
                    reflect_min: flag("reflect_min")?,
                    reflect_max: flag("reflect_max")?,
                }
            }
        })
    }
    pub(crate) fn supports(&self, action: ModuleAction) -> bool {
        match self {
            Self::Logic(_) => matches!(action, ModuleAction::BPress | ModuleAction::BRelease),
            Self::Process(_) => {
                matches!(action, ModuleAction::GatePress | ModuleAction::GateRelease)
            }
            _ => false,
        }
    }
    pub(crate) fn enqueue(&mut self, action: ModuleAction) -> bool {
        if !self.supports(action) {
            return false;
        }
        match self {
            Self::Logic(m) => m.enqueue(action),
            Self::Process(p) => p.manual.enqueue(action),
            _ => false,
        }
    }
    pub(crate) fn controls(&self) -> Option<ModuleControlState> {
        match self {
            Self::Logic(m) => Some(ModuleControlState {
                pressed: m.pressed,
                pressed_mask: u16::from(m.pressed),
            }),
            Self::Process(p) => Some(ModuleControlState {
                pressed: p.manual.pressed,
                pressed_mask: u16::from(p.manual.pressed),
            }),
            _ => None,
        }
    }
    pub(crate) fn release(&mut self) -> bool {
        match self {
            Self::Logic(m) => m.release(),
            Self::Process(p) => p.manual.release(),
            _ => return false,
        }
        true
    }
    pub(crate) fn reset_state(&mut self) {
        self.release();
        if let Self::Process(p) = self {
            p.lanes = [Lane::default(); 16];
        }
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        match (self, old) {
            (Self::Logic(m), Self::Logic(previous)) => m.pressed = previous.pressed,
            (Self::Process(p), Self::Process(previous)) => {
                p.lanes = previous.lanes;
                p.manual.pressed = previous.manual.pressed;
            }
            _ => {}
        }
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame; 3], outputs: &mut [PortFrame]) {
        match self {
            Self::Process(p) => p.process(inputs, outputs),
            Self::Logic(manual) => {
                manual.apply();
                let count = inputs[0].channels.max(inputs[1].channels).clamp(1, 16);
                for f in outputs.iter_mut() {
                    *f = PortFrame {
                        channels: count,
                        ..PortFrame::default()
                    };
                }
                for lane in 0..usize::from(count) {
                    let a = finite(inputs[0].voice(lane, 0.0)) >= 1.0;
                    let b = manual.pressed || finite(inputs[1].voice(lane, 0.0)) >= 1.0;
                    let states = [!a, !b, a || b, !(a || b), a && b, !(a && b), a != b, a == b];
                    for (frame, state) in outputs.iter_mut().zip(states) {
                        frame.values[lane] = if state { 10.0 } else { 0.0 };
                    }
                }
            }
            Self::Compare(offset) => {
                let count = inputs[0].channels.max(inputs[1].channels).clamp(1, 16);
                for f in outputs.iter_mut() {
                    *f = PortFrame {
                        channels: count,
                        ..PortFrame::default()
                    };
                }
                for lane in 0..usize::from(count) {
                    let a = finite(inputs[0].channel(lane));
                    let b = finite(inputs[1].channel(lane)) + *offset;
                    let clip = a.clamp(-b.abs(), b.abs());
                    let outside = a.abs() > b.abs();
                    let values = [
                        a.max(b),
                        a.min(b),
                        clip,
                        a - clip,
                        if outside { 10.0 } else { 0.0 },
                        if outside { 0.0 } else { 10.0 },
                        if a > b { 10.0 } else { 0.0 },
                        if a < b { 10.0 } else { 0.0 },
                    ];
                    for (frame, value) in outputs.iter_mut().zip(values) {
                        frame.values[lane] = voltage(value);
                    }
                }
            }
            Self::Rescale {
                gain,
                offset,
                min,
                max,
                reflect_min,
                reflect_max,
            } => {
                outputs[0] = PortFrame {
                    channels: inputs[0].channels.clamp(1, 16),
                    ..PortFrame::default()
                };
                for lane in 0..usize::from(outputs[0].channels) {
                    let x = finite(inputs[0].channel(lane)) * *gain + *offset;
                    let value = if *max <= *min {
                        *min
                    } else if *reflect_min && *reflect_max {
                        let range = *max - *min;
                        let t = (x - *min).rem_euclid(2.0 * range);
                        *min + if t <= range { t } else { 2.0 * range - t }
                    } else if *reflect_min {
                        ((x - *min).abs() + *min).min(*max)
                    } else if *reflect_max {
                        (*max - (*max - x).abs()).max(*min)
                    } else {
                        x.clamp(*min, *max)
                    };
                    outputs[0].values[lane] = voltage(value);
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Signal {
    dsp: SignalDsp,
    rate: f64,
    divider: u8,
    frame: u8,
    light_count: u8,
    lights: [[f64; 2]; MAX_MODULE_LIGHTS],
}
impl Signal {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        let (light_count, divider) = match node.kind.as_str() {
            "logic" => (8, 32),
            "compare" => (4, 32),
            "rescale" => (2, 16),
            _ => (0, 1),
        };
        Ok(Self {
            dsp: SignalDsp::build(node, rate)?,
            rate: f64::from(rate),
            divider,
            frame: 0,
            light_count,
            lights: [[0.0; 2]; MAX_MODULE_LIGHTS],
        })
    }
    pub(crate) fn supports(&self, a: ModuleAction) -> bool {
        self.dsp.supports(a)
    }
    pub(crate) fn enqueue(&mut self, a: ModuleAction) -> bool {
        self.dsp.enqueue(a)
    }
    pub(crate) fn controls(&self) -> Option<ModuleControlState> {
        self.dsp.controls()
    }
    pub(crate) fn release(&mut self) -> bool {
        self.dsp.release()
    }
    pub(crate) fn reset_state(&mut self) {
        self.dsp.reset_state();
        self.lights = [[0.0; 2]; MAX_MODULE_LIGHTS];
        self.frame = 0;
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        self.dsp.adopt(&old.dsp);
        self.lights = old.lights;
        self.frame = old.frame;
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        ModuleLightState {
            input_channels: None,
            count: self.light_count,
            brightness: self.lights.map(|row| row.map(|v| v as f32)),
            manual: self.dsp.controls().is_some_and(|m| m.pressed),
        }
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame; 3], outputs: &mut [PortFrame]) {
        self.dsp.process(inputs, outputs);
        self.frame += 1;
        if self.frame < self.divider {
            return;
        }
        self.frame = 0;
        let mut active = [false; MAX_MODULE_LIGHTS];
        let count = outputs[0].channels;
        match &self.dsp {
            SignalDsp::Logic(_) => {
                for (i, frame) in outputs.iter().enumerate() {
                    active[i] =
                        (0..usize::from(frame.channels)).any(|lane| frame.values[lane] > 0.0);
                }
            }
            SignalDsp::Compare(_) => {
                for (i, frame) in outputs[4..].iter().enumerate() {
                    active[i] =
                        (0..usize::from(frame.channels)).any(|lane| frame.values[lane] > 0.0);
                }
            }
            SignalDsp::Rescale {
                gain,
                offset,
                min,
                max,
                ..
            } => {
                for lane in 0..usize::from(count) {
                    let x = finite(inputs[0].channel(lane)) * *gain + *offset;
                    active[0] |= x >= *max;
                    active[1] |= x <= *min;
                }
            }
            _ => {}
        }
        let elapsed = f64::from(self.divider) / self.rate;
        for (index, row) in self.lights[..usize::from(self.light_count)]
            .iter_mut()
            .enumerate()
        {
            for (poly, value) in row.iter_mut().enumerate() {
                let target = if active[index] && ((count > 1) == (poly == 1)) {
                    1.0
                } else {
                    0.0
                };
                if target >= *value {
                    *value = target;
                } else {
                    *value = (*value + (target - *value) * 30.0 * elapsed).max(0.0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(kind: &str, params: &[(&str, f32)]) -> Signal {
        Signal::build(
            &Node {
                score: None,
                id: "s".into(),
                kind: kind.into(),
                bypassed: false,
                params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
            },
            8000,
        )
        .unwrap()
    }
    fn frame(values: &[f32]) -> PortFrame {
        let mut f = PortFrame {
            channels: values.len() as u8,
            ..PortFrame::default()
        };
        f.values[..values.len()].copy_from_slice(values);
        f
    }
    fn sample(s: &mut Signal, inputs: [PortFrame; 3]) -> [PortFrame; 8] {
        let mut out = [PortFrame::default(); 8];
        let count = match &s.dsp {
            SignalDsp::Logic(_) | SignalDsp::Compare(_) => 8,
            SignalDsp::Process(_) => 6,
            _ => 1,
        };
        s.process(&inputs, &mut out[..count]);
        out
    }
    fn process(s: &mut Signal, input: f32, gate: f32) -> [f32; 6] {
        let out = sample(
            s,
            [
                PortFrame::default(),
                PortFrame::mono(input),
                PortFrame::mono(gate),
            ],
        );
        std::array::from_fn(|i| out[i].values[0])
    }
    #[test]
    fn all_eight_logic_truth_tables_and_thresholds_are_exact() {
        let mut s = model("logic", &[]);
        let out = sample(
            &mut s,
            [
                frame(&[0.0, 0.0, 10.0, 10.0]),
                frame(&[0.0, 10.0, 0.0, 10.0]),
                PortFrame::default(),
            ],
        );
        let expected = [
            [10, 10, 0, 0],
            [10, 0, 10, 0],
            [0, 10, 10, 10],
            [10, 0, 0, 0],
            [0, 0, 0, 10],
            [10, 10, 10, 0],
            [0, 10, 10, 0],
            [10, 0, 0, 10],
        ];
        for (out, expected) in out.iter().zip(expected) {
            assert_eq!(out.channels, 4);
            for (v, e) in out.values[..4].iter().zip(expected) {
                assert_eq!(*v, e as f32);
            }
        }
        let out = sample(
            &mut s,
            [
                frame(&[0.999, 1.0, f32::NAN]),
                PortFrame::mono(1.0),
                PortFrame::default(),
            ],
        );
        assert_eq!(&out[4].values[..3], &[0.0, 10.0, 0.0]);
    }
    #[test]
    fn compare_raw_mono_padding_offset_and_all_eight_roles_are_independent() {
        let mut s = model("compare", &[("b_offset", 1.0)]);
        let out = sample(
            &mut s,
            [
                PortFrame::mono(10.0),
                frame(&[2.0, -4.0]),
                PortFrame::default(),
            ],
        );
        for (f, e) in out.iter().zip([
            [10.0, 0.0],
            [3.0, -3.0],
            [3.0, 0.0],
            [7.0, 0.0],
            [10.0, 0.0],
            [0.0, 10.0],
            [10.0, 10.0],
            [0.0, 0.0],
        ]) {
            assert_eq!(&f.values[..2], &e);
        }
        let out = sample(
            &mut s,
            [
                frame(&[-3.0, 3.0, 3.01]),
                PortFrame::mono(2.0),
                PortFrame::default(),
            ],
        );
        assert_eq!(&out[4].values[..3], &[0.0, 10.0, 10.0]); // B mono is zero on later lanes, then offset=1.
        let mut s = model("compare", &[]);
        let out = sample(
            &mut s,
            [
                frame(&[-3.0, 3.0, 3.01]),
                frame(&[-3.0, 3.0, 3.0]),
                PortFrame::default(),
            ],
        );
        assert_eq!(&out[4].values[..3], &[0.0, 0.0, 10.0]);
        assert_eq!(&out[6].values[..2], &[0.0, 0.0]);
        assert_eq!(&out[7].values[..2], &[0.0, 0.0]);
    }
    #[test]
    fn rescale_all_reflection_modes_positive_negative_periods_and_reversed_bounds() {
        let input = frame(&[-5.0, -4.0, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        for (min, max, expected) in [
            (
                0.0,
                0.0,
                [-1.0, -1.0, -1.0, -1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            ),
            (
                1.0,
                0.0,
                [1.0, 1.0, 1.0, 0.0, -1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            ),
            (
                0.0,
                1.0,
                [
                    -1.0, -1.0, -1.0, -1.0, -1.0, 0.0, 1.0, 0.0, -1.0, -1.0, -1.0,
                ],
            ),
            (
                1.0,
                1.0,
                [-1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 1.0],
            ),
        ] {
            let mut s = model(
                "rescale",
                &[
                    ("gain", 1.0),
                    ("min", -1.0),
                    ("max", 1.0),
                    ("reflect_min", min),
                    ("reflect_max", max),
                ],
            );
            assert_eq!(
                &sample(&mut s, [input, PortFrame::default(), PortFrame::default()])[0].values
                    [..11],
                &expected
            );
        }
        for max in [-2.0, 2.0] {
            let mut s = model("rescale", &[("min", 2.0), ("max", max)]);
            assert_eq!(
                sample(&mut s, [input, PortFrame::default(), PortFrame::default()])[0].values[..11],
                [2.0; 11]
            );
        }
        for multiplier in [1.0, 10.0, 100.0, 1000.0] {
            let mut s = model(
                "rescale",
                &[("gain", 0.5), ("offset", 1.0), ("multiplier", multiplier)],
            );
            let out = sample(
                &mut s,
                [
                    PortFrame::mono(0.001),
                    PortFrame::default(),
                    PortFrame::default(),
                ],
            );
            assert!((out[0].values[0] - (0.0005 * multiplier + 1.0)).abs() < 1e-6);
        }
    }
    #[test]
    fn process_captures_raw_shift_register_and_complementary_track_hold_edges() {
        let mut s = model("process", &[]);
        assert_eq!(process(&mut s, 5.0, 2.0), [5.0, 0.0, 5.0, 5.0, 5.0, 5.0]);
        assert_eq!(process(&mut s, 6.0, 1.0), [5.0, 0.0, 5.0, 6.0, 6.0, 6.0]);
        assert_eq!(process(&mut s, 7.0, 0.1), [5.0, 0.0, 7.0, 7.0, 7.0, 7.0]);
        assert_eq!(process(&mut s, 8.0, 0.0), [5.0, 0.0, 8.0, 7.0, 8.0, 8.0]);
        assert_eq!(process(&mut s, 9.0, 2.0), [9.0, 5.0, 9.0, 9.0, 9.0, 9.0]);
    }
    #[test]
    fn process_rate_sentinel_slew_cv_and_strict_glide_delay_are_deterministic() {
        let mut s = model("process", &[("slew", 0.0)]); // One second per volt, delta 1/8000.
        let low = process(&mut s, 1.0, 0.0);
        assert!((low[4] - 0.000125).abs() < 1e-10);
        assert_eq!(low[5], 1.0);
        let mut s = model("process", &[("slew", 0.0)]);
        process(&mut s, 0.0, 2.0);
        for i in 2..=8 {
            let out = process(&mut s, i as f32, 2.0);
            assert_eq!(out[5], i as f32);
        }
        let out = process(&mut s, 9.0, 2.0);
        assert!((out[5] - 8.000125).abs() < 1e-6);
        assert_eq!(out[4], 9.0);
        let out = sample(
            &mut s,
            [
                PortFrame::mono(1.0),
                PortFrame::mono(10.0),
                PortFrame::mono(0.0),
            ],
        );
        assert!((f64::from(out[4].values[0]) - 9.0000625).abs() < 1e-6);
        let mut instant = model("process", &[]);
        let out = sample(
            &mut instant,
            [
                PortFrame::mono(f32::MAX),
                PortFrame::mono(4.0),
                PortFrame::mono(2.0),
            ],
        );
        assert_eq!(out[0].values[0], 4.0); // Sentinel ignores slew CV.
    }
    #[test]
    fn process_dormant_lanes_freeze_and_zero_input_does_not_process_manual_gate() {
        let mut s = model("process", &[]);
        sample(
            &mut s,
            [
                PortFrame::default(),
                frame(&[3.0, 7.0]),
                PortFrame::mono(2.0),
            ],
        );
        sample(
            &mut s,
            [
                PortFrame::default(),
                PortFrame::mono(4.0),
                PortFrame::mono(0.0),
            ],
        );
        let out = sample(&mut s, [PortFrame::default(); 3]);
        assert!(out[..6].iter().all(|f| *f == PortFrame::mono(0.0)));
        let out = sample(
            &mut s,
            [
                PortFrame::default(),
                frame(&[5.0, 9.0]),
                PortFrame::mono(2.0),
            ],
        );
        assert_eq!(&out[0].values[..2], &[5.0, 7.0]); // Lane two remained high, so it does not recapture.
        assert_eq!(&out[1].values[..2], &[3.0, 0.0]);
    }
    #[test]
    fn manual_order_capacity_emergency_release_and_control_state_are_runtime_only() {
        let mut s = model("logic", &[]);
        for _ in 0..MAX_MODULE_ACTIONS_PER_SAMPLE {
            assert!(s.enqueue(ModuleAction::BPress));
        }
        assert!(!s.enqueue(ModuleAction::BRelease));
        s.release();
        assert!(!s.controls().unwrap().pressed);
        s.enqueue(ModuleAction::BPress);
        s.enqueue(ModuleAction::BRelease);
        let out = sample(&mut s, [PortFrame::default(); 3]);
        assert_eq!(out[1].values[0], 10.0);
        s.enqueue(ModuleAction::BRelease);
        s.enqueue(ModuleAction::BPress);
        let out = sample(&mut s, [PortFrame::default(); 3]);
        assert_eq!(out[1].values[0], 0.0);
        assert!(s.controls().unwrap().pressed);
        assert!(s.lights().manual);
        assert!(!s.enqueue(ModuleAction::GatePress));
        assert!(!s.enqueue(ModuleAction::Clock));
        let mut p = model("process", &[]);
        p.enqueue(ModuleAction::GatePress);
        assert_eq!(process(&mut p, 6.0, 0.0)[0], 6.0);
        p.release();
        assert!(!p.lights().manual);
        assert_eq!(process(&mut p, 7.0, 0.0)[0], 6.0);
    }
    #[test]
    fn indicator_dividers_mono_poly_pairs_decay_and_rescale_prelimit_flags() {
        let mut s = model("logic", &[]);
        for _ in 0..31 {
            sample(&mut s, [PortFrame::default(); 3]);
        }
        assert_eq!(s.lights().brightness, [[0.0; 2]; MAX_MODULE_LIGHTS]);
        sample(&mut s, [PortFrame::default(); 3]);
        assert_eq!(s.lights().brightness[0], [1.0, 0.0]);
        for _ in 0..32 {
            sample(
                &mut s,
                [
                    frame(&[2.0, 2.0]),
                    PortFrame::default(),
                    PortFrame::default(),
                ],
            );
        }
        assert!((s.lights().brightness[0][0] - 0.88).abs() < 1e-6);
        assert_eq!(s.lights().brightness[2], [0.0, 1.0]);
        let mut r = model("rescale", &[("gain", 1.0), ("reflect_max", 1.0)]);
        for _ in 0..16 {
            sample(
                &mut r,
                [
                    PortFrame::mono(12.0),
                    PortFrame::default(),
                    PortFrame::default(),
                ],
            );
        }
        assert_eq!(r.lights().brightness[0], [1.0, 0.0]);
        assert_eq!(r.lights().brightness[1], [0.0, 0.0]);
        let mut p = model("process", &[]);
        p.enqueue(ModuleAction::GatePress);
        sample(&mut p, [PortFrame::default(); 3]);
        assert_eq!(p.lights().count, 0);
        assert!(p.lights().manual);
    }
    #[test]
    fn nonfinite_sanitization_and_extreme_cv_never_poison_process_or_reflection_state() {
        let mut p = model("process", &[("slew", 0.0)]);
        for cv in [f32::MAX, -f32::MAX, f32::NAN, f32::INFINITY] {
            let out = sample(
                &mut p,
                [
                    PortFrame::mono(cv),
                    PortFrame::mono(f32::MAX),
                    PortFrame::mono(2.0),
                ],
            );
            assert!(out.iter().all(|f| f
                .values
                .iter()
                .all(|v| v.is_finite() && v.abs() <= SIGNAL_LIMIT)));
        }
        let mut r = model(
            "rescale",
            &[
                ("gain", 1.0),
                ("multiplier", 1000.0),
                ("reflect_min", 1.0),
                ("reflect_max", 1.0),
            ],
        );
        for x in [f32::MAX, -f32::MAX, f32::NAN, f32::INFINITY] {
            let out = sample(
                &mut r,
                [
                    PortFrame::mono(x),
                    PortFrame::default(),
                    PortFrame::default(),
                ],
            );
            assert!(out[0].values[0].is_finite());
            assert!(out[0].values[0].abs() <= 10.0);
        }
    }
}
