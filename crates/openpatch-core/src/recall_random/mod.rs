//! Native Marbles adapter. Source arithmetic remains in the pinned kernel;
//! timing, voltage normaling, scale authority and capture are explicit host policy.
#[allow(dead_code)]
pub(crate) mod kernel;
mod preset_scales;
use crate::{
    parameter, ModuleAction, ModuleContent, ModuleLightState, Node, PortFrame,
    RecallRandomCaptureState, RecallRandomCaptureStatus, RecallRandomCapturedScale,
    RecallRandomData, RecallRandomDegree,
};
use kernel::{
    quantizer::Scale,
    random::RandomStream,
    record::{NoteFilter, ScaleRecorder},
    t::TGenerator,
    xy::{GroupSettings, XYGenerator},
    Ramps, Ratio, BLOCK,
};
use serde_json::{json, Value};
pub(crate) const INPUTS: [&str; 11] = [
    "t_clock",
    "x_clock",
    "reset",
    "rate_cv",
    "bias_cv",
    "jitter_cv",
    "deja_vu_cv",
    "spread_cv",
    "x_bias_cv",
    "steps_cv",
    "register_cv",
];
pub(crate) const OUTPUTS: [&str; 7] = ["t1", "t2", "t3", "x1", "x2", "x3", "y"];
const SOURCE_RATE: u32 = 32000;
const EDGE_CAPACITY: usize = 64;
const Y_RATIOS: [Ratio; 12] = [
    Ratio { p: 1, q: 64 },
    Ratio { p: 1, q: 48 },
    Ratio { p: 1, q: 32 },
    Ratio { p: 1, q: 24 },
    Ratio { p: 1, q: 16 },
    Ratio { p: 1, q: 12 },
    Ratio { p: 1, q: 8 },
    Ratio { p: 1, q: 6 },
    Ratio { p: 1, q: 4 },
    Ratio { p: 1, q: 3 },
    Ratio { p: 1, q: 2 },
    Ratio { p: 1, q: 1 },
];
pub(crate) fn default_scales() -> [Scale; 6] {
    preset_scales::preset_scales()
}
pub(crate) fn catalog() -> Value {
    let mut result = json!({"kind":"recall_random","title":"RECALL RANDOM","model":"functional","width_hp":54,"tags":["Custom","Clock","Random","Modulation","Sequencers","Quantizers"],"inputs":INPUTS,"description":"Coupled repeatable rhythms and weighted-scale voltage streams, with six authored scales and runtime scale capture.","params":{}});
    for (name, default, min, max) in [
        ("seed", 1., 1., 65535.),
        ("t_model", 0., 0., 6.),
        ("t_range", 1., 0., 2.),
        ("length", 16., 1., 16.),
        ("t_repeat", 0., 0., 1.),
        ("x_repeat", 0., 0., 1.),
        ("x_clock", 0., 0., 4.),
        ("x_control_mode", 0., 0., 2.),
        ("x_range", 2., 0., 2.),
        ("x_register", 0., 0., 1.),
        ("y_control_mode", 0., 0., 2.),
        ("y_range", 2., 0., 2.),
        ("y_ratio", 4., 0., 11.),
    ] {
        result["params"][name] = json!({"default":default,"min":min,"max":max,"step":1});
    }
    result["params"]["t_rate"] = json!({"default":0,"min":-120,"max":120,"unit":"semitones"});
    for (name, default) in [
        ("t_bias", 0.5),
        ("t_jitter", 0.),
        ("t_pulse_width", 0.5),
        ("t_pulse_spread", 0.),
        ("deja_vu", 0.5),
        ("x_spread", 0.5),
        ("x_bias", 0.5),
        ("x_steps", 0.),
        ("y_spread", 0.5),
        ("y_bias", 0.5),
        ("y_steps", 0.),
    ] {
        result["params"][name] = json!({"default":default,"min":0,"max":1});
    }
    result["params"]["t_model"]["labels"] = json!([
        "Complementary",
        "Clusters",
        "Drums",
        "Independent",
        "Divider",
        "Three states",
        "Markov"
    ]);
    result["params"]["t_range"]["labels"] = json!(["Slow", "Medium", "Fast"]);
    result["params"]["x_clock"]["labels"] =
        json!(["T1/T2/T3", "T1", "T2", "T3", "External (normal T2)"]);
    for name in ["x_control_mode", "y_control_mode"] {
        result["params"][name]["labels"] = json!(["Identical", "Bump", "Tilt"]);
    }
    for name in ["x_range", "y_range"] {
        result["params"][name]["labels"] = json!(["0–2 V", "0–5 V", "±5 V"]);
    }
    result["params"]["y_ratio"]["labels"] =
        json!(["/64", "/48", "/32", "/24", "/16", "/12", "/8", "/6", "/4", "/3", "/2", "×1"]);
    result["content_schema"] = RecallRandomData::content_schema();
    result["content_schema"]["default"] =
        serde_json::to_value(ModuleContent::RecallRandom(RecallRandomData::default()))
            .expect("finite preset scales");
    result
}
pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"]=json!("https://github.com/pichenettes/eurorack/tree/f4cfa7697095a28d292bff758beb51deb49b5a86/marbles");
    module["polyphony"]["maximum_channels"] = json!(1);
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["version_2"] = json!("Seven mono outputs; each input reads channel zero.");
    for output in module["output_metadata"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        output["maximum_channels"] = json!(1);
    }
    module["runtime_actions"] = json!(["reset"]);
    module["capture_actions"] = json!(["begin", "stop", "cancel", "inspect", "commit"]);
    module["timing_policy"]=json!("Exactly32000 source frames/s in complete5-frame blocks at host8000..192000Hz. Integer phase starts at0; first source tick after one source interval. Collect5 input frames, then run full T followed by X1,X2,X3,Y using one owned source stream. Output block begins on the next source tick:5-source-frame buffer latency156.25us, plus source public T's2-frame delay62.5us. Gates hold; CV linearly interpolates previous/current source frames, adding1 source frame31.25us. Host sampling rounds tick presentation by<1 host frame. Native cables add1 host frame separately. CV and authored controls sample at the block completion boundary; NoteFilter runs once per block.");
    module["input_policy"]=json!("0.1V low/1V high Schmitt clocks/reset, initially low.64 fixed combined chronological snapshots are consumed one per source frame (32000 snapshots/s; full two-edge cycles<=16000Hz combined). Overflow discards backlog, reconciles latest levels at next tick and lights sticky input_overflow. No lossless arbitrary host-Nyquist pulse claim. Physical connection changes are captured with levels; unpatched T clock uses internal source rate. Patched X clock forces external; unpatched X uses selected undelayed T ramps (External selection normals to T2). Unpatched reset is low. CV finite-or-zero; normalized knobs add CV/5, rate adds12 semitones/V. Register input is clamped±100V before the source once/block filter; source register value=0.5+volts/10, clamped0..1. No board ADC/DAC, entropy, calibration or physical debounce emulation.");
    module["state_adoption"]=json!("Same ID/kind/rate/seed retains RNG, loops, PLL/ramps, reacquisition, interpolation, bridge phase, edge FIFO and recorder; receiving controls and authored scales prevail. Changed scale slots copy prebuilt quantizers; unchanged slots retain hysteresis. Initialize, seed/rate change and fresh reopen restart. Bypass emits zero and freezes runtime. Runtime history is never patch content. No callback allocations/deallocations or locks.");
    module["capture_policy"]=json!("Begin targets slot0..5 and starting document revision, returns a monotonic capture_id; rejects busy. One source NoteFilter call/block samples register_cv. X clock edges (or undelayed T2 while normalled) begin/accept notes. Stop accepts a held note, publishes bounded weighted scale only with>=2 distinct degrees, otherwise Empty; Cancel clears transient recorder and any finished result. Receipts carry slot/revision/capture_id. Host must fence module incarnation and current revision before committing through shared content/history. Quantizers prepare off callback.");
    module["source_boundaries"]=json!("All seven source T algorithms, coupled sequence draw ordering and source reset preserved. Native saturating float-word endpoint, safe terminal interpolation, and unquantized fallback outside the source quantizer's representable i32 octave domain remain explicit corrections. All6 validated scales load Y too, correcting original empty Y masks; no whole-hardware equivalence claim.");
    module["light_names"] = json!([
        "t1",
        "t2",
        "t3",
        "x1",
        "x2",
        "x3",
        "y",
        "recording",
        "input_overflow"
    ]);
}
#[derive(Clone, Copy)]
struct Params {
    seed: u32,
    t_model: u8,
    t_range: u8,
    t_rate: f32,
    t_bias: f32,
    t_jitter: f32,
    t_pulse_width: f32,
    t_pulse_spread: f32,
    deja_vu: f32,
    length: i32,
    t_repeat: bool,
    x_repeat: bool,
    x_clock: u8,
    x: GroupSettings,
    y: GroupSettings,
}
impl Params {
    fn build(node: &Node, slot: u8) -> Result<Self, String> {
        let catalog = catalog();
        for name in node.params.keys() {
            if catalog["params"].get(name).is_none() {
                return Err(format!("Recall Random unknown parameter '{name}'"));
            }
        }
        let get = |name: &str| {
            let p = &catalog["params"][name];
            let v = parameter(
                node,
                name,
                p["default"].as_f64().unwrap() as f32,
                p["min"].as_f64().unwrap() as f32,
                p["max"].as_f64().unwrap() as f32,
            )?;
            if p.get("step").is_some() && v.fract() != 0. {
                return Err(format!("Recall Random {name} must be an integer"));
            }
            Ok(v)
        };
        let group = |prefix: &str| -> Result<GroupSettings, String> {
            Ok(GroupSettings {
                control_mode: get(&format!("{prefix}_control_mode"))? as u8,
                voltage_range: get(&format!("{prefix}_range"))? as u8,
                register_mode: false,
                register_value: 0.5,
                spread: get(&format!("{prefix}_spread"))?,
                bias: get(&format!("{prefix}_bias"))?,
                steps: get(&format!("{prefix}_steps"))?,
                deja_vu: 0.,
                scale_index: usize::from(slot),
                length: 1,
                ratio: Ratio { p: 1, q: 1 },
            })
        };
        let mut x = group("x")?;
        x.register_mode = get("x_register")? != 0.;
        let mut y = group("y")?;
        y.ratio = Y_RATIOS[get("y_ratio")? as usize];
        Ok(Self {
            seed: get("seed")? as u32,
            t_model: get("t_model")? as u8,
            t_range: get("t_range")? as u8,
            t_rate: get("t_rate")?,
            t_bias: get("t_bias")?,
            t_jitter: get("t_jitter")?,
            t_pulse_width: get("t_pulse_width")?,
            t_pulse_spread: get("t_pulse_spread")?,
            deja_vu: get("deja_vu")?,
            length: get("length")? as i32,
            t_repeat: get("t_repeat")? != 0.,
            x_repeat: get("x_repeat")? != 0.,
            x_clock: get("x_clock")? as u8,
            x,
            y,
        })
    }
}
#[derive(Clone, Copy, Default)]
struct Edge {
    levels: u8,
    connections: u8,
    reset: bool,
}
#[derive(Clone)]
struct Runtime {
    stream: RandomStream,
    t: TGenerator,
    xy: XYGenerator,
    filter: NoteFilter,
    recorder: ScaleRecorder,
    phase: u32,
    index: usize,
    host_levels: u8,
    host_connections: u8,
    captured: Edge,
    delivered: Edge,
    queue: [Edge; EDGE_CAPACITY],
    head: usize,
    len: usize,
    overflow: bool,
    t_clock: [u8; BLOCK],
    x_clock: [u8; BLOCK],
    reset: bool,
    connections: u8,
    block: [[f32; 7]; BLOCK],
    current: [f32; 7],
    previous: [f32; 7],
    t_delay: [[f32; 3]; 2],
    delay_index: usize,
    capture: Option<RecallRandomCaptureState>,
    capture_id: u64,
    note_high: bool,
}
impl Runtime {
    fn new(seed: u32, scales: &[Scale; 6]) -> Self {
        let mut stream = RandomStream::new(seed);
        let t = TGenerator::new(&mut stream, SOURCE_RATE as f32);
        let mut xy = XYGenerator::new(&mut stream, SOURCE_RATE as f32);
        for (slot, scale) in scales.iter().enumerate() {
            xy.load_scale(slot, scale);
            xy.load_channel_scale(3, slot, scale);
        }
        Self {
            stream,
            t,
            xy,
            filter: NoteFilter::new(),
            recorder: ScaleRecorder::new(),
            phase: 0,
            index: 0,
            host_levels: 0,
            host_connections: 0,
            captured: Edge::default(),
            delivered: Edge::default(),
            queue: [Edge::default(); EDGE_CAPACITY],
            head: 0,
            len: 0,
            overflow: false,
            t_clock: [0; BLOCK],
            x_clock: [0; BLOCK],
            reset: false,
            connections: 0,
            block: [[0.; 7]; BLOCK],
            current: [0.; 7],
            previous: [0.; 7],
            t_delay: [[0.; 3]; 2],
            delay_index: 0,
            capture: None,
            capture_id: 0,
            note_high: false,
        }
    }
    fn push(&mut self, reset: bool) {
        let edge = Edge {
            levels: self.host_levels,
            connections: self.host_connections,
            reset,
        };
        if !reset
            && edge.levels == self.captured.levels
            && edge.connections == self.captured.connections
        {
            return;
        }
        self.captured = edge;
        if self.len == EDGE_CAPACITY {
            self.head = 0;
            self.len = 0;
            self.overflow = true;
        }
        self.queue[(self.head + self.len) % EDGE_CAPACITY] = edge;
        self.len += 1;
    }
    fn pop(&mut self) -> Edge {
        if self.len == 0 {
            return Edge {
                reset: false,
                ..self.delivered
            };
        }
        let edge = self.queue[self.head];
        self.head = (self.head + 1) % EDGE_CAPACITY;
        self.len -= 1;
        edge
    }
}
pub(crate) struct RecallRandom {
    params: Params,
    sample_rate: u32,
    scales: [Scale; 6],
    pristine: Runtime,
    runtime: Runtime,
    pending_reset: bool,
}
impl std::fmt::Debug for RecallRandom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecallRandom")
            .field("seed", &self.params.seed)
            .field("sample_rate", &self.sample_rate)
            .finish_non_exhaustive()
    }
}
impl RecallRandom {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&rate) {
            return Err("Recall Random host rate must be an integer8000..192000Hz".into());
        }
        let data = match &node.content {
            Some(ModuleContent::RecallRandom(data)) => data.clone(),
            None => RecallRandomData::default(),
            _ => return Err("Recall Random content type mismatch".into()),
        };
        data.validate()?;
        let scales = std::array::from_fn(|i| data.scales[i].kernel());
        let params = Params::build(node, data.active_slot)?;
        let runtime = Runtime::new(params.seed, &scales);
        Ok(Self {
            params,
            sample_rate: rate,
            scales,
            pristine: runtime.clone(),
            runtime,
            pending_reset: false,
        })
    }
    pub(crate) fn initialize_state(&mut self) {
        let capture_id = self.runtime.capture_id;
        self.runtime.clone_from(&self.pristine);
        self.runtime.capture_id = capture_id;
        self.pending_reset = false;
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.sample_rate != old.sample_rate || self.params.seed != old.params.seed {
            return;
        }
        self.runtime.clone_from(&old.runtime);
        for i in 0..6 {
            if !same_scale(&self.scales[i], &old.scales[i]) {
                self.runtime.xy.copy_prepared_scale(i, &self.pristine.xy);
            }
        }
    }
    pub(crate) fn release(&mut self) {
        self.pending_reset = false;
    }
    pub(crate) fn enqueue(&mut self, action: ModuleAction) -> bool {
        if action != ModuleAction::Reset || self.pending_reset {
            return false;
        }
        self.pending_reset = true;
        true
    }
    pub(crate) fn begin_capture(&mut self, slot: u8, revision: u64) -> bool {
        if slot >= 6
            || self
                .runtime
                .capture
                .is_some_and(|c| c.status == RecallRandomCaptureStatus::Recording)
        {
            return false;
        }
        let r = &mut self.runtime;
        r.capture_id = r.capture_id.wrapping_add(1);
        r.recorder.clear();
        r.note_high = false;
        r.capture = Some(RecallRandomCaptureState {
            capture_id: r.capture_id,
            revision,
            slot,
            status: RecallRandomCaptureStatus::Recording,
            scale: RecallRandomCapturedScale::default(),
        });
        true
    }
    pub(crate) fn stop_capture(&mut self) -> bool {
        if !self.recording() {
            return false;
        }
        let r = &mut self.runtime;
        if r.note_high {
            r.recorder.accept_note();
            r.note_high = false;
        }
        let receipt = r.capture.as_mut().unwrap();
        if let Some(scale) = r.recorder.extract_scale() {
            receipt.status = RecallRandomCaptureStatus::Complete;
            receipt.scale.base_interval = scale.base_interval;
            receipt.scale.num_degrees = scale.num_degrees as u8;
            for (d, s) in receipt.scale.degrees.iter_mut().zip(scale.degree) {
                *d = RecallRandomDegree {
                    voltage: s.voltage,
                    weight: s.weight,
                };
            }
        } else {
            receipt.status = RecallRandomCaptureStatus::Empty;
        }
        true
    }
    pub(crate) fn cancel_capture(&mut self) -> bool {
        let Some(receipt) = self.runtime.capture.as_mut() else {
            return false;
        };
        if receipt.status == RecallRandomCaptureStatus::Cancelled {
            return false;
        }
        self.runtime.recorder.clear();
        self.runtime.note_high = false;
        receipt.status = RecallRandomCaptureStatus::Cancelled;
        receipt.scale = RecallRandomCapturedScale::default();
        true
    }
    fn recording(&self) -> bool {
        self.runtime
            .capture
            .is_some_and(|c| c.status == RecallRandomCaptureStatus::Recording)
    }
    pub(crate) fn capture(&self) -> Option<RecallRandomCaptureState> {
        self.runtime.capture
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut lights = ModuleLightState {
            count: 9,
            ..ModuleLightState::default()
        };
        for i in 0..7 {
            let v = self.runtime.current[i];
            lights.brightness[i] = if i < 3 {
                [v / 5., 0.]
            } else {
                [(v / 5.).clamp(0., 1.), (-v / 5.).clamp(0., 1.)]
            };
        }
        lights.brightness[7][0] = u8::from(self.recording()) as f32;
        lights.brightness[8][0] = u8::from(self.runtime.overflow) as f32;
        lights
    }
    fn block(&mut self, inputs: &[PortFrame; 11]) {
        let p = self.params;
        let r = &mut self.runtime;
        let knob = |value: f32, index: usize| {
            (value + finite(inputs[index].channel(0)) / 5.).clamp(0., 1.)
        };
        let deja = knob(p.deja_vu, 6);
        r.t.set_model(p.t_model);
        r.t.set_range(p.t_range);
        r.t.set_rate((p.t_rate + 12. * finite(inputs[3].channel(0))).clamp(-120., 120.));
        r.t.set_bias(knob(p.t_bias, 4));
        r.t.set_jitter(knob(p.t_jitter, 5));
        r.t.set_pulse_width_mean(p.t_pulse_width);
        r.t.set_pulse_width_std(p.t_pulse_spread);
        r.t.set_deja_vu(if p.t_repeat { deja } else { 0. });
        r.t.set_length(p.length);
        let mut x = p.x;
        x.spread = knob(x.spread, 7);
        x.bias = knob(x.bias, 8);
        x.steps = knob(x.steps, 9);
        x.deja_vu = if p.x_repeat { deja } else { 0. };
        x.length = p.length;
        let u = r
            .filter
            .process(0.5 + finite(inputs[10].channel(0)).clamp(-100., 100.) / 10.);
        x.register_value = u.clamp(0., 1.);
        let filtered = (u - 0.5) * 10.;
        let y = p.y;
        let external_t = r.connections & 1 != 0;
        let source = if r.connections & 2 != 0 {
            4
        } else if p.x_clock == 4 {
            2
        } else {
            p.x_clock
        };
        let mut tr = r.reset;
        let mut xr = r.reset;
        let mut ramps = Ramps::default();
        let mut gates = [[false; 2]; BLOCK];
        let mut voltages = [[0.; 4]; BLOCK];
        r.t.process(
            &mut r.stream,
            external_t,
            &mut tr,
            &r.t_clock,
            &mut ramps,
            &mut gates,
        );
        if source != 4 {
            xr |= tr;
        }
        r.xy.process(
            &mut r.stream,
            source,
            &x,
            &y,
            &mut xr,
            &r.x_clock,
            &mut ramps,
            &mut voltages,
        );
        for i in 0..BLOCK {
            let raw = [
                if gates[i][0] { 5. } else { 0. },
                if ramps.master[i] < 0.5 { 5. } else { 0. },
                if gates[i][1] { 5. } else { 0. },
            ];
            r.block[i][..3].copy_from_slice(&r.t_delay[r.delay_index]);
            r.t_delay[r.delay_index] = raw;
            r.delay_index = (r.delay_index + 1) % 2;
            for (output, voltage) in r.block[i][3..].iter_mut().zip(voltages[i]) {
                *output = finite(voltage);
            }
            if r.capture
                .is_some_and(|c| c.status == RecallRandomCaptureStatus::Recording)
            {
                let high = if source == 4 {
                    r.x_clock[i] & 1 != 0
                } else {
                    raw[1] != 0.
                };
                if high && !r.note_high {
                    r.recorder.new_note(filtered);
                } else if !high && r.note_high {
                    r.recorder.accept_note();
                }
                if high {
                    r.recorder.update_voltage(filtered);
                }
                r.note_high = high;
            }
        }
        r.reset = false;
    }
    pub(crate) fn process(
        &mut self,
        inputs: &[PortFrame; 11],
        physical: u16,
        demand: u16,
        outputs: &mut [PortFrame],
    ) {
        let r = &mut self.runtime;
        r.host_connections = (physical & 7) as u8;
        for (i, input) in inputs.iter().take(3).enumerate() {
            let high = r.host_connections & (1 << i) != 0
                && schmitt(r.host_levels & (1 << i) != 0, input.channel(0));
            r.host_levels = (r.host_levels & !(1 << i)) | (u8::from(high) << i);
        }
        r.push(self.pending_reset);
        self.pending_reset = false;
        r.phase += SOURCE_RATE;
        while self.runtime.phase >= self.sample_rate {
            let r = &mut self.runtime;
            r.phase -= self.sample_rate;
            let event = r.pop();
            let previous = r.delivered.levels;
            let flags = |bit: u8| {
                let high = event.levels & bit != 0;
                let was = previous & bit != 0;
                u8::from(high)
                    | if high && !was {
                        2
                    } else if !high && was {
                        4
                    } else {
                        0
                    }
            };
            r.t_clock[r.index] = flags(1);
            r.x_clock[r.index] = flags(2);
            r.reset |= event.reset || (event.levels & 4 != 0 && previous & 4 == 0);
            r.connections = event.connections;
            r.delivered = event;
            r.previous = r.current;
            r.current = r.block[r.index];
            r.index += 1;
            if r.index == BLOCK {
                self.block(inputs);
                self.runtime.index = 0;
            }
        }
        let fraction = self.runtime.phase as f32 / self.sample_rate as f32;
        for (i, out) in outputs.iter_mut().enumerate() {
            let v = if i < 3 {
                self.runtime.current[i]
            } else {
                self.runtime.previous[i]
                    + (self.runtime.current[i] - self.runtime.previous[i]) * fraction
            };
            *out = if demand & (1 << i) != 0 {
                PortFrame::mono(v)
            } else {
                PortFrame::default()
            };
        }
    }
}
fn finite(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.
    }
}
fn schmitt(high: bool, v: f32) -> bool {
    let v = finite(v);
    if high {
        v > 0.1
    } else {
        v >= 1.
    }
}
fn same_scale(a: &Scale, b: &Scale) -> bool {
    a.base_interval.to_bits() == b.base_interval.to_bits()
        && a.num_degrees == b.num_degrees
        && a.degree
            .iter()
            .zip(&b.degree)
            .all(|(a, b)| a.voltage.to_bits() == b.voltage.to_bits() && a.weight == b.weight)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn adapter(rate: u32) -> RecallRandom {
        let node: Node =
            serde_json::from_value(json!({"id":"recall","kind":"recall_random","params":{}}))
                .unwrap();
        RecallRandom::build(&node, rate).unwrap()
    }
    #[test]
    fn subscan_edges_retain_chronology_and_overflow_reconciles_latest_physical_levels() {
        let mut module = adapter(192000);
        let mut input = [PortFrame::default(); 11];
        let mut output = [PortFrame::default(); 7];
        input[0] = PortFrame::mono(5.);
        module.process(&input, 1, 127, &mut output);
        input[0] = PortFrame::mono(0.);
        for _ in 0..11 {
            module.process(&input, 1, 127, &mut output);
        }
        assert_eq!(
            &module.runtime.t_clock[..2],
            &[3, 4],
            "both edges survive a pulse shorter than one source frame"
        );
        for i in 0..240 {
            input[0] = PortFrame::mono(if i % 2 == 0 { 5. } else { 0. });
            module.process(&input, 1, 127, &mut output);
        }
        assert!(module.runtime.overflow);
        assert!(module.runtime.len <= EDGE_CAPACITY);
        input[0] = PortFrame::mono(5.);
        for _ in 0..600 {
            module.process(&input, 0, 127, &mut output);
        }
        assert_eq!(
            module.runtime.delivered.levels & 1,
            0,
            "physical disconnection defeats a held high voltage"
        );
        assert_eq!(module.runtime.len, 0);
    }
    #[test]
    fn musical_reset_keeps_stream_ownership_and_changes_traversal_without_reseeding() {
        let mut module = adapter(32000);
        let input = [PortFrame::default(); 11];
        let mut output = [PortFrame::default(); 7];
        for _ in 0..16005 {
            module.process(&input, 0, 127, &mut output);
        }
        let before = module.runtime.stream.state();
        assert_ne!(before, module.pristine.stream.state());
        assert!(module.enqueue(ModuleAction::Reset));
        assert!(!module.enqueue(ModuleAction::Reset));
        for _ in 0..5 {
            module.process(&input, 0, 127, &mut output);
        }
        assert_ne!(
            module.runtime.stream.state(),
            module.pristine.stream.state(),
            "reset retains learned random history"
        );
        module.initialize_state();
        assert_eq!(
            module.runtime.stream.state(),
            module.pristine.stream.state()
        );
    }
}
