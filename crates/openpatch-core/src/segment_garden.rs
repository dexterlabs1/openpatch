//! Six fixed physical Stages-derived kernels with an explicit native clock,
//! voltage and patch-grouping bridge. Raw arithmetic remains in kernel.rs.
#[allow(dead_code)] // Raw source entrypoints also serve the independent oracle.
mod kernel;

use crate::{parameter, ModuleLightState, Node, PortFrame};
use kernel::{
    Configuration, Output, SegmentGenerator, SegmentType, GATE_FALLING, GATE_HIGH, GATE_RISING,
};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 12] = [
    "gate1", "gate2", "gate3", "gate4", "gate5", "gate6", "cv1", "cv2", "cv3", "cv4", "cv5", "cv6",
];
pub(crate) const OUTPUTS: [&str; 12] = [
    "out1", "out2", "out3", "out4", "out5", "out6", "active1", "active2", "active3", "active4",
    "active5", "active6",
];
const CHANNELS: usize = 6;
const BLOCK: usize = 8;
const SOURCE_RATE: u32 = 31250;
const EDGE_CAPACITY: usize = 32;
const NOTE_STATES: usize = 42;
const EMPTY_CONFIG: Configuration = Configuration {
    kind: SegmentType::Ramp,
    looped: false,
};

pub(crate) fn catalog() -> Value {
    let mut module = json!({
        "kind":"segment_garden", "title":"SEGMENT GARDEN", "model":"functional",
        "tags":["Custom","Modulation","Envelopes","Sequencers"], "width_hp":48,
        "inputs":INPUTS,
        "description":"Six ramp, step and hold segments form independent or gate-patched groups, with source loop boundaries, stage contours and active outputs.",
        "params":{"seed":{"default":1,"min":1,"max":16777215,"step":1,"description":"Deterministic module RNG seed. A changed seed applies at the next source block boundary."}}
    });
    for i in 1..=CHANNELS {
        module["params"][format!("type{i}")] = json!({"default":0,"min":0,"max":2,"step":1,"values":[0,1,2],"labels":["Ramp","Step","Hold"]});
        module["params"][format!("loop{i}")] = json!({"default":u8::from(i==1),"min":0,"max":1,"step":1,"values":[0,1],"labels":["Off","Loop"]});
        module["params"][format!("level{i}")] = json!({"default":0.5,"min":0.0,"max":1.0,"description":"Source primary: ramp time or step/hold level. CV adds channel-zero volts/8."});
        module["params"][format!("shape{i}")] = json!({"default":0.5,"min":0.0,"max":1.0,"description":"Source secondary: ramp shape, step portamento or hold time; grouped source modes retain their context-dependent meanings."});
    }
    module
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://github.com/pichenettes/eurorack/tree/f4cfa7697095a28d292bff758beb51deb49b5a86/stages");
    module["polyphony"]["maximum_channels"] = json!(1);
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["version_2"] = json!("Twelve mono outputs. Inputs accept up to sixteen channels, read channel zero and use physical gate cable presence for grouping.");
    for output in module["output_metadata"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        output["maximum_channels"] = json!(1);
    }
    module["processing_policy"] = json!("Fixed 31250 Hz source domain in eight-sample blocks. Host rates 8000..192000 Hz use an integer tick accumulator with at most four source ticks per host frame. Generated blocks play over the following eight source ticks: 256 microseconds source pipeline latency, plus host sampling quantization. Output uses zero-order hold; no interpolation or anti-aliasing is implied.");
    module["control_policy"] = json!("Types, loop flags, gate grouping and CV-influenced controls latch at the start of each eight-source-tick block. primary=clamp(level+channel0(CV)/8,-1,1.999995), secondary=shape. Unpatched CV is zero; nonfinite voltages read zero. No hardware ADC calibration, pot law or chain expanders are modeled. ALT/audio oscillator firmware is outside this contract.");
    module["grouping_policy"] = json!("Each physical gate cable starts a group extending up to the next patched gate or channel six. Unpatched prefix channels run independently. Six physical kernels retain hidden state across leader/slave changes; only changed group configuration is applied. Slaves emit the source progress contour 1-phase while their own stage is selected. ACTIVE is 5 V exactly when the delayed master segment equals that physical slot's local group offset.");
    module["clock_policy"] = json!("Host Schmitt thresholds are LOW<=0.1 V, HIGH>=1 V. Each physical slot queues up to 32 ordered transitions and consumes at most one per source tick. Newly patched gates retain queued insertion edges until the next block makes their group active. On overflow its queue clears, then reconciles to the latest host level on the next tick; sticky edge_overload lights until Initialize. Newly constructed kernels receive one LOW clock-tracker sample to avoid source internal frame-zero division by zero; generator mode/phase/output remain unchanged. Raw source new() is unmodified.");
    module["voltage_policy"] = json!("Ideal native output=clamp(8*source_value,-8,+8) V; nonfinite source values become zero. ACTIVE outputs are 0/5 V. No source DAC calibration or measured analog output claim is made. Exact secondary=1 safely clamps the source portamento table endpoint; full-capacity rounded-RNG source bounds are guarded in the raw kernel.");
    module["random_policy"] = json!("One module stmlib LCG advances in physical processing order. Forty-two shared physical note-quantizer states retain hysteresis across live grouping changes; Ramp-first sequencers use firmware 13-step/0.03 quantizers. Seed changes reset only module RNG at the next source block boundary.");
    module["initialization_policy"] = json!("Initialize resets kernels, clock priming, gates, queues, source pipeline, shared RNG/note state and overload indication while retaining controls. Same-rate adoption retains all runtime with receiving controls; topology and control edits apply at the next source block boundary. A changed sample rate starts fresh. Bypass freezes runtime and emits mono zero.");
    module["light_names"] = json!([
        "out1",
        "out2",
        "out3",
        "out4",
        "out5",
        "out6",
        "active1",
        "active2",
        "active3",
        "active4",
        "active5",
        "active6",
        "edge_overload"
    ]);
    module["light_policy"] = json!("Six actual delayed output bipolar magnitudes divided by 8, six actual delayed stage-active indicators, and a sticky queue-overload warning. Lights update independently of output demand.");
}

fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct EdgeQueue {
    events: [bool; EDGE_CAPACITY],
    head: usize,
    len: usize,
    host_high: bool,
    consumed_high: bool,
    reconcile: bool,
}
impl EdgeQueue {
    fn observe(&mut self, voltage: f32) -> bool {
        let was_high = self.host_high;
        let voltage = finite(voltage);
        if voltage >= 1.0 {
            self.host_high = true;
        } else if voltage <= 0.1 {
            self.host_high = false;
        }
        if self.host_high == was_high || self.reconcile {
            return false;
        }
        if self.len == EDGE_CAPACITY {
            self.head = 0;
            self.len = 0;
            self.reconcile = true;
            return true;
        }
        self.events[(self.head + self.len) % EDGE_CAPACITY] = self.host_high;
        self.len += 1;
        false
    }
    fn consume(&mut self) -> u8 {
        let previous = self.consumed_high;
        if self.reconcile {
            self.consumed_high = self.host_high;
            self.reconcile = false;
        } else if self.len != 0 {
            self.consumed_high = self.events[self.head];
            self.head = (self.head + 1) % EDGE_CAPACITY;
            self.len -= 1;
        }
        (u8::from(self.consumed_high) * GATE_HIGH)
            | if !previous && self.consumed_high {
                GATE_RISING
            } else {
                0
            }
            | if previous && !self.consumed_high {
                GATE_FALLING
            } else {
                0
            }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Unconfigured,
    Prefix(Configuration),
    Leader {
        len: usize,
        configurations: [Configuration; CHANNELS],
    },
    Slave {
        leader: usize,
        offset: usize,
    },
}

fn new_kernel() -> SegmentGenerator {
    let mut kernel = SegmentGenerator::new_quantized();
    kernel.prime_clock_tracker();
    kernel
}

#[derive(Debug)]
struct Runtime {
    kernels: [SegmentGenerator; CHANNELS],
    roles: [Role; CHANNELS],
    edges: [EdgeQueue; CHANNELS],
    gates: [[u8; BLOCK]; CHANNELS],
    queued: [[f32; 12]; BLOCK],
    held: [f32; 12],
    note_states: [i32; NOTE_STATES],
    source_accumulator: u32,
    cursor: usize,
    latched_gate_mask: u16,
    rng: u32,
    applied_seed: u32,
    overload: bool,
}
impl Runtime {
    fn new(seed: u32) -> Self {
        Self {
            kernels: std::array::from_fn(|_| new_kernel()),
            roles: [Role::Unconfigured; CHANNELS],
            edges: [EdgeQueue::default(); CHANNELS],
            gates: [[0; BLOCK]; CHANNELS],
            queued: [[0.0; 12]; BLOCK],
            held: [0.0; 12],
            note_states: [0; NOTE_STATES],
            source_accumulator: 0,
            cursor: 0,
            latched_gate_mask: 0,
            rng: seed,
            applied_seed: seed,
            overload: false,
        }
    }
    fn reset(&mut self, seed: u32) {
        for kernel in &mut self.kernels {
            *kernel = new_kernel();
        }
        self.roles.fill(Role::Unconfigured);
        self.edges.fill(EdgeQueue::default());
        self.gates.fill([0; BLOCK]);
        self.queued.fill([0.0; 12]);
        self.held.fill(0.0);
        self.note_states.fill(0);
        self.source_accumulator = 0;
        self.cursor = 0;
        self.latched_gate_mask = 0;
        self.rng = seed;
        self.applied_seed = seed;
        self.overload = false;
    }
    fn adopt(&mut self, old: &Self) {
        for (kernel, previous) in self.kernels.iter_mut().zip(&old.kernels) {
            kernel.clone_from(previous);
        }
        self.roles = old.roles;
        self.edges = old.edges;
        self.gates = old.gates;
        self.queued = old.queued;
        self.held = old.held;
        self.note_states = old.note_states;
        self.source_accumulator = old.source_accumulator;
        self.cursor = old.cursor;
        self.latched_gate_mask = old.latched_gate_mask;
        self.rng = old.rng;
        self.applied_seed = old.applied_seed;
        self.overload = old.overload;
    }
}

#[derive(Debug)]
pub(crate) struct SegmentGarden {
    sample_rate: u32,
    configurations: [Configuration; CHANNELS],
    levels: [f32; CHANNELS],
    shapes: [f32; CHANNELS],
    seed: u32,
    // Fixed large storage is allocated during build and reused by every callback.
    runtime: Box<Runtime>,
}
impl SegmentGarden {
    pub(crate) fn build(node: &Node, sample_rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err("Segment Garden sample rate must be in 8000..=192000 Hz".into());
        }
        for key in node.params.keys() {
            let known = key == "seed"
                || ["type", "loop", "level", "shape"].iter().any(|prefix| {
                    key.strip_prefix(*prefix)
                        .is_some_and(|suffix| matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6"))
                });
            if !known {
                return Err(format!(
                    "node '{}': unknown Segment Garden parameter '{key}'",
                    node.id
                ));
            }
        }
        if node.content.is_some() {
            return Err(format!(
                "node '{}': Segment Garden does not accept module content",
                node.id
            ));
        }
        let integer = |name: &str, default: f32, min: f32, max: f32| -> Result<u32, String> {
            let value = parameter(node, name, default, min, max)?;
            if value.fract() != 0.0 {
                return Err(format!(
                    "node '{}': parameter '{name}' must be an integer",
                    node.id
                ));
            }
            Ok(value as u32)
        };
        let mut configurations = [EMPTY_CONFIG; CHANNELS];
        let mut levels = [0.5; CHANNELS];
        let mut shapes = [0.5; CHANNELS];
        for i in 0..CHANNELS {
            let channel = i + 1;
            configurations[i] = Configuration {
                kind: match integer(&format!("type{channel}"), 0.0, 0.0, 2.0)? {
                    0 => SegmentType::Ramp,
                    1 => SegmentType::Step,
                    _ => SegmentType::Hold,
                },
                looped: integer(
                    &format!("loop{channel}"),
                    if i == 0 { 1.0 } else { 0.0 },
                    0.0,
                    1.0,
                )? != 0,
            };
            levels[i] = parameter(node, &format!("level{channel}"), 0.5, 0.0, 1.0)?;
            shapes[i] = parameter(node, &format!("shape{channel}"), 0.5, 0.0, 1.0)?;
        }
        let seed = integer("seed", 1.0, 1.0, 16777215.0)?;
        Ok(Self {
            sample_rate,
            configurations,
            levels,
            shapes,
            seed,
            runtime: Box::new(Runtime::new(seed)),
        })
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.sample_rate == old.sample_rate {
            self.runtime.adopt(&old.runtime);
        }
    }
    pub(crate) fn initialize_state(&mut self) {
        self.runtime.reset(self.seed);
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut result = ModuleLightState {
            count: 13,
            ..Default::default()
        };
        for i in 0..CHANNELS {
            let voltage = self.runtime.held[i];
            result.brightness[i] = [
                (-voltage / 8.0).clamp(0.0, 1.0),
                (voltage / 8.0).clamp(0.0, 1.0),
            ];
            result.brightness[i + CHANNELS] = [0.0, self.runtime.held[i + CHANNELS] / 5.0];
        }
        result.brightness[12] = [if self.runtime.overload { 1.0 } else { 0.0 }, 0.0];
        result
    }
    pub(crate) fn process(
        &mut self,
        inputs: &[PortFrame; 12],
        physical_inputs: u16,
        demand: u16,
        outputs: &mut [PortFrame],
    ) {
        for (i, input) in inputs.iter().enumerate().take(CHANNELS) {
            let voltage = if physical_inputs & (1 << i) != 0 {
                input.channel(0)
            } else {
                0.0
            };
            self.runtime.overload |= self.runtime.edges[i].observe(voltage);
        }
        self.runtime.source_accumulator += SOURCE_RATE;
        while self.runtime.source_accumulator >= self.sample_rate {
            self.runtime.source_accumulator -= self.sample_rate;
            self.source_tick(inputs, physical_inputs);
        }
        for (i, output) in outputs.iter_mut().enumerate() {
            *output = if i < 12 && demand & (1 << i) != 0 {
                PortFrame::mono(self.runtime.held[i])
            } else {
                PortFrame::default()
            };
        }
    }
    fn source_tick(&mut self, inputs: &[PortFrame; 12], physical_inputs: u16) {
        if self.runtime.cursor == 0 {
            self.latch_block(inputs, physical_inputs);
        }
        let cursor = self.runtime.cursor;
        self.runtime.held = self.runtime.queued[cursor];
        for i in 0..CHANNELS {
            let awaiting_group =
                physical_inputs & (1 << i) != 0 && self.runtime.latched_gate_mask & (1 << i) == 0;
            // Keep real insertion edges until the newly patched group can
            // consume them. The old prefix/slave receives its stable previous
            // level, so this does not invent an edge at either boundary.
            self.runtime.gates[i][cursor] = if awaiting_group {
                u8::from(self.runtime.edges[i].consumed_high) * GATE_HIGH
            } else {
                self.runtime.edges[i].consume()
            };
        }
        self.runtime.cursor += 1;
        if self.runtime.cursor == BLOCK {
            self.render_block();
            self.runtime.cursor = 0;
        }
    }
    fn latch_block(&mut self, inputs: &[PortFrame; 12], physical_inputs: u16) {
        let gate_mask = physical_inputs & 0x3f;
        self.runtime.latched_gate_mask = gate_mask;
        if self.runtime.applied_seed != self.seed {
            self.runtime.rng = self.seed;
            self.runtime.applied_seed = self.seed;
        }
        let mut roles = [Role::Unconfigured; CHANNELS];
        let mut channel = 0;
        while channel < CHANNELS {
            if gate_mask & (1 << channel) == 0 {
                roles[channel] = Role::Prefix(self.configurations[channel]);
                channel += 1;
            } else {
                let leader = channel;
                let mut end = leader + 1;
                while end < CHANNELS && gate_mask & (1 << end) == 0 {
                    end += 1;
                }
                let mut configurations = [EMPTY_CONFIG; CHANNELS];
                configurations[..end - leader].copy_from_slice(&self.configurations[leader..end]);
                roles[leader] = Role::Leader {
                    len: end - leader,
                    configurations,
                };
                for (slot, role) in roles.iter_mut().enumerate().take(end).skip(leader + 1) {
                    *role = Role::Slave {
                        leader,
                        offset: slot - leader,
                    };
                }
                channel = end;
            }
        }
        for (slot, role) in roles.iter().copied().enumerate() {
            if role != self.runtime.roles[slot] {
                match role {
                    Role::Prefix(configuration) => {
                        let _ = self.runtime.kernels[slot].configure_single(false, configuration);
                    }
                    Role::Leader {
                        len,
                        configurations,
                    } => {
                        let _ = self.runtime.kernels[slot].configure(true, &configurations[..len]);
                    }
                    Role::Slave { offset, .. } => {
                        let _ = self.runtime.kernels[slot].configure_slave(offset);
                    }
                    Role::Unconfigured => unreachable!(),
                }
            }
            match role {
                Role::Prefix(_) => {
                    let cv = if physical_inputs & (1 << (slot + CHANNELS)) != 0 {
                        finite(inputs[slot + CHANNELS].channel(0))
                    } else {
                        0.0
                    };
                    let primary = (self.levels[slot] + cv / 8.0).clamp(-1.0, 1.999995);
                    let _ =
                        self.runtime.kernels[slot].set_parameters(0, primary, self.shapes[slot]);
                }
                Role::Leader { len, .. } => {
                    for offset in 0..len {
                        let physical = slot + offset;
                        let cv = if physical_inputs & (1 << (physical + CHANNELS)) != 0 {
                            finite(inputs[physical + CHANNELS].channel(0))
                        } else {
                            0.0
                        };
                        let primary = (self.levels[physical] + cv / 8.0).clamp(-1.0, 1.999995);
                        let _ = self.runtime.kernels[slot].set_parameters(
                            offset,
                            primary,
                            self.shapes[physical],
                        );
                    }
                }
                _ => {}
            }
        }
        self.runtime.roles = roles;
    }
    fn render_block(&mut self) {
        let mut scratch = [Output::default(); BLOCK];
        let no_gate = [0; BLOCK];
        for slot in 0..CHANNELS {
            let role = self.runtime.roles[slot];
            let slave = matches!(role, Role::Slave { .. });
            let kernel = &mut self.runtime.kernels[slot];
            if !slave {
                for offset in 0..kernel::MAX_NUM_SEGMENTS {
                    let _ = kernel
                        .set_note_quantizer_state(offset, self.runtime.note_states[slot + offset]);
                }
                kernel.seed_rng(self.runtime.rng);
            }
            let gates = if self.runtime.latched_gate_mask & (1 << slot) != 0 {
                &self.runtime.gates[slot]
            } else {
                &no_gate
            };
            kernel.process(gates, &mut scratch);
            if !slave {
                self.runtime.rng = kernel.rng_state();
                for offset in 0..kernel::MAX_NUM_SEGMENTS {
                    self.runtime.note_states[slot + offset] =
                        kernel.note_quantizer_state(offset).unwrap_or(0);
                }
            }
            let offset = match role {
                Role::Slave { offset, .. } => offset,
                _ => 0,
            };
            for (i, output) in scratch.iter().enumerate() {
                self.runtime.queued[i][slot] = finite(output.value).clamp(-1.0, 1.0) * 8.0;
                self.runtime.queued[i][slot + CHANNELS] = if output.segment == offset as i32 {
                    5.0
                } else {
                    0.0
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> Node {
        serde_json::from_value(json!({"id":"garden","kind":"segment_garden"})).unwrap()
    }
    fn frame(garden: &mut SegmentGarden, inputs: &[PortFrame; 12], physical: u16) {
        let mut outputs = [PortFrame::default(); 12];
        garden.process(inputs, physical, 0x0fff, &mut outputs);
    }

    #[test]
    fn sub_tick_pulses_keep_both_edges_in_order() {
        let mut queue = EdgeQueue::default();
        assert!(!queue.observe(5.0));
        assert!(!queue.observe(0.0));
        assert!(!queue.observe(5.0));
        assert!(!queue.observe(0.5)); // Hysteresis retains HIGH.
        assert_eq!(queue.consume(), GATE_HIGH | GATE_RISING);
        assert_eq!(queue.consume(), GATE_FALLING);
        assert_eq!(queue.consume(), GATE_HIGH | GATE_RISING);
        assert_eq!(queue.consume(), GATE_HIGH);
    }

    #[test]
    fn overflowing_queue_reconciles_to_latest_level_without_stale_edges() {
        let mut queue = EdgeQueue::default();
        for i in 0..EDGE_CAPACITY {
            assert!(!queue.observe(if i % 2 == 0 { 5.0 } else { 0.0 }));
        }
        assert!(queue.observe(5.0)); // Thirty-third edge exceeds the queue.
        assert!(!queue.observe(0.0)); // Latest level replaces pending reconciliation.
        assert_eq!(queue.consume(), 0);
        assert_eq!(queue.consume(), 0);
        assert!(!queue.observe(5.0));
        assert_eq!(queue.consume(), GATE_HIGH | GATE_RISING);
    }

    #[test]
    fn overload_is_visible_and_initialize_clears_runtime() {
        let mut garden = SegmentGarden::build(&node(), 192000).unwrap();
        let mut inputs = [PortFrame::default(); 12];
        for i in 0..200 {
            inputs[0] = PortFrame::mono(if i % 2 == 0 { 5.0 } else { 0.0 });
            frame(&mut garden, &inputs, 1);
        }
        assert_eq!(garden.lights().brightness[12], [1.0, 0.0]);
        garden.initialize_state();
        assert_eq!(garden.lights().brightness[12], [0.0, 0.0]);
        assert!(garden
            .runtime
            .edges
            .iter()
            .all(|q| q.len == 0 && !q.reconcile && !q.host_high && !q.consumed_high));
        assert_eq!(garden.runtime.held, [0.0; 12]);
        assert_eq!(garden.runtime.cursor, 0);
    }

    #[test]
    fn mid_block_gate_insertion_preserves_real_rising_and_falling_edges() {
        let mut garden = SegmentGarden::build(&node(), SOURCE_RATE).unwrap();
        let mut inputs = [PortFrame::default(); 12];
        frame(&mut garden, &inputs, 0);
        assert_eq!(garden.runtime.cursor, 1);
        inputs[0] = PortFrame::mono(5.0);
        frame(&mut garden, &inputs, 1);
        inputs[0] = PortFrame::mono(0.0);
        frame(&mut garden, &inputs, 1);
        assert_eq!(garden.runtime.edges[0].len, 2);
        assert_eq!(garden.runtime.gates[0][1], 0);
        assert_eq!(garden.runtime.gates[0][2], 0);
        for _ in 3..BLOCK {
            frame(&mut garden, &inputs, 1);
        }
        assert_eq!(garden.runtime.cursor, 0);
        assert_eq!(garden.runtime.edges[0].len, 2);
        frame(&mut garden, &inputs, 1);
        assert!(matches!(
            garden.runtime.roles[0],
            Role::Leader { len: 6, .. }
        ));
        assert_eq!(garden.runtime.gates[0][0], GATE_HIGH | GATE_RISING);
        frame(&mut garden, &inputs, 1);
        assert_eq!(garden.runtime.gates[0][1], GATE_FALLING);
        assert_eq!(garden.runtime.edges[0].len, 0);
    }

    #[test]
    fn grouping_changes_at_block_boundary_with_independent_prefix_slots() {
        let mut garden = SegmentGarden::build(&node(), SOURCE_RATE).unwrap();
        let inputs = [PortFrame::default(); 12];
        frame(&mut garden, &inputs, 1 << 2);
        assert!(matches!(garden.runtime.roles[0], Role::Prefix(_)));
        assert!(matches!(garden.runtime.roles[1], Role::Prefix(_)));
        assert!(matches!(
            garden.runtime.roles[2],
            Role::Leader { len: 4, .. }
        ));
        assert_eq!(
            garden.runtime.roles[5],
            Role::Slave {
                leader: 2,
                offset: 3
            }
        );
        for _ in 1..BLOCK {
            frame(&mut garden, &inputs, (1 << 2) | (1 << 4));
        }
        assert!(matches!(
            garden.runtime.roles[2],
            Role::Leader { len: 4, .. }
        ));
        frame(&mut garden, &inputs, (1 << 2) | (1 << 4));
        assert!(matches!(
            garden.runtime.roles[2],
            Role::Leader { len: 2, .. }
        ));
        assert!(matches!(
            garden.runtime.roles[4],
            Role::Leader { len: 2, .. }
        ));
        assert_eq!(
            garden.runtime.roles[5],
            Role::Slave {
                leader: 4,
                offset: 1
            }
        );
    }

    #[test]
    fn adoption_keeps_runtime_and_latches_new_seed_at_next_block_start() {
        let mut previous_node = node();
        for i in 1..=CHANNELS {
            previous_node.params.insert(format!("type{i}"), 2.0);
            previous_node.params.insert(format!("loop{i}"), 1.0);
        }
        let mut old = SegmentGarden::build(&previous_node, SOURCE_RATE).unwrap();
        let mut inputs = [PortFrame::default(); 12];
        inputs[0] = PortFrame::mono(5.0);
        for _ in 0..BLOCK + 3 {
            frame(&mut old, &inputs, 0x3f);
        }
        let previous_rng = old.runtime.rng;
        assert_ne!(previous_rng, 1);
        let mut receiving_node = previous_node;
        receiving_node.params.insert("seed".into(), 7.0);
        receiving_node.params.insert("level1".into(), 0.9);
        let mut receiving = SegmentGarden::build(&receiving_node, SOURCE_RATE).unwrap();
        receiving.adopt(&old);
        assert_eq!(receiving.levels[0], 0.9);
        assert_eq!(receiving.runtime.cursor, 3);
        assert_eq!(receiving.runtime.rng, previous_rng);
        for _ in 3..BLOCK {
            frame(&mut receiving, &inputs, 0x3f);
        }
        assert_eq!(receiving.runtime.applied_seed, 1);
        assert_eq!(receiving.runtime.rng, previous_rng);
        frame(&mut receiving, &inputs, 0x3f);
        assert_eq!(receiving.runtime.applied_seed, 7);
        assert_eq!(receiving.runtime.rng, 7);
    }
}
