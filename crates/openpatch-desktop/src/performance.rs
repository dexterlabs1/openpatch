//! Control-thread telemetry registry and bounded sequencer performance events. The only
//! producer is behind a control-only mutex; the audio consumer never locks.
use super::audio_diagnostics::{Diagnostics, Recorder};
use crate::scope_transport::{self, ScopeFrame, ScopeProducer, ScopeReader};
use openpatch::control::LiveModuleEvents;
use openpatch_core::{
    Cable, CableHandle, Engine, ModuleAction, ModuleControlState, ModuleHandle, ModuleLightState,
    ModuleTelemetry, Patch, SavedModuleState, MAX_CABLES, MAX_MODULE_LIGHTS, MAX_NODES,
};
use rtrb::{Consumer, Producer, RingBuffer};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    Arc, Mutex,
};

pub(super) const EVENT_CAPACITY: usize = 512;
const FUTURE_SECONDS: u64 = 2;
#[derive(Clone, Copy)]
struct Event {
    slot: usize,
    token: u64,
    action_epoch: u64,
    cancel_through_serial: u64,
    action: ModuleAction,
    sequence: u64,
    revision: u64,
    deadline: u64,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModuleKind {
    Sequence,
    StockSeq3,
    StockQuantizer,
    Switch1To4,
    Switch4To1,
    Logic,
    Compare,
    Process,
    Rescale,
    VcaMix,
    StockVcaMix,
    Sum,
    Merge,
    Split,
    Viz,
    Pulses,
    Push,
    Gates,
    RandomValues,
    Random,
    ShiftRegister,
    AdsrEg,
    MidiCc,
    MidiGate,
    StockVca,
    StockDelay,
    StockLfo,
    StockOctave,
    StockVco,
    WavetableVco,
    WavetableLfo,
}
impl ModuleKind {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "seq3" => Some(Self::Sequence),
            "stock_seq3" => Some(Self::StockSeq3),
            "stock_quantizer" => Some(Self::StockQuantizer),
            "switch_1to4" => Some(Self::Switch1To4),
            "switch_4to1" => Some(Self::Switch4To1),
            "logic" => Some(Self::Logic),
            "compare" => Some(Self::Compare),
            "process" => Some(Self::Process),
            "rescale" => Some(Self::Rescale),
            "vca_mix" => Some(Self::VcaMix),
            "stock_vca_mix" => Some(Self::StockVcaMix),
            "poly_sum" => Some(Self::Sum),
            "poly_merge" => Some(Self::Merge),
            "poly_split" => Some(Self::Split),
            "poly_viz" => Some(Self::Viz),
            "pulses" => Some(Self::Pulses),
            "push" => Some(Self::Push),
            "gates" => Some(Self::Gates),
            "random_values" => Some(Self::RandomValues),
            "random" => Some(Self::Random),
            "sh_asr" => Some(Self::ShiftRegister),
            "adsr_eg" => Some(Self::AdsrEg),
            "midi_cc" => Some(Self::MidiCc),
            "midi_gate" => Some(Self::MidiGate),
            "stock_vca" => Some(Self::StockVca),
            "stock_delay" => Some(Self::StockDelay),
            "stock_lfo" => Some(Self::StockLfo),
            "stock_octave" => Some(Self::StockOctave),
            "stock_vco" => Some(Self::StockVco),
            "wavetable_vco" => Some(Self::WavetableVco),
            "wavetable_lfo" => Some(Self::WavetableLfo),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Sequence => "seq3",
            Self::StockSeq3 => "stock_seq3",
            Self::StockQuantizer => "stock_quantizer",
            Self::Switch1To4 => "switch_1to4",
            Self::Switch4To1 => "switch_4to1",
            Self::Logic => "logic",
            Self::Compare => "compare",
            Self::Process => "process",
            Self::Rescale => "rescale",
            Self::VcaMix => "vca_mix",
            Self::StockVcaMix => "stock_vca_mix",
            Self::Sum => "poly_sum",
            Self::Merge => "poly_merge",
            Self::Split => "poly_split",
            Self::Viz => "poly_viz",
            Self::Pulses => "pulses",
            Self::Push => "push",
            Self::Gates => "gates",
            Self::RandomValues => "random_values",
            Self::Random => "random",
            Self::ShiftRegister => "sh_asr",
            Self::AdsrEg => "adsr_eg",
            Self::MidiCc => "midi_cc",
            Self::MidiGate => "midi_gate",
            Self::StockVca => "stock_vca",
            Self::StockDelay => "stock_delay",
            Self::StockLfo => "stock_lfo",
            Self::StockOctave => "stock_octave",
            Self::StockVco => "stock_vco",
            Self::WavetableVco => "wavetable_vco",
            Self::WavetableLfo => "wavetable_lfo",
        }
    }
    fn learn_kind(self) -> Option<openpatch_core::MidiLearnKind> {
        openpatch_core::MidiLearnKind::from_module_kind(self.name())
    }
    fn has_safety_release(self) -> bool {
        self.manual() || self == Self::MidiGate
    }
    fn manual(self) -> bool {
        matches!(
            self,
            Self::StockSeq3
                | Self::Logic
                | Self::Process
                | Self::Pulses
                | Self::Push
                | Self::Gates
                | Self::RandomValues
                | Self::ShiftRegister
                | Self::AdsrEg
        )
    }
    fn sequential(self) -> bool {
        matches!(
            self,
            Self::Sequence | Self::StockSeq3 | Self::Switch1To4 | Self::Switch4To1
        )
    }
    fn accepts(self, action: ModuleAction) -> bool {
        match self {
            Self::Sequence => matches!(
                action,
                ModuleAction::Clock | ModuleAction::Run | ModuleAction::Reset
            ),
            Self::StockSeq3 => {
                matches!(
                    action,
                    ModuleAction::Clock
                        | ModuleAction::Run
                        | ModuleAction::Reset
                        | ModuleAction::ClockPress
                        | ModuleAction::ClockRelease
                        | ModuleAction::RunPress
                        | ModuleAction::RunRelease
                        | ModuleAction::ResetPress
                        | ModuleAction::ResetRelease
                ) || action.seq_gate_button().is_some()
            }
            Self::Logic => matches!(action, ModuleAction::BPress | ModuleAction::BRelease),
            Self::Process => matches!(action, ModuleAction::GatePress | ModuleAction::GateRelease),
            Self::Pulses => action.pulse_button().is_some(),
            Self::Gates => matches!(
                action,
                ModuleAction::ResetPress | ModuleAction::ResetRelease
            ),
            Self::RandomValues | Self::AdsrEg => {
                matches!(action, ModuleAction::PushPress | ModuleAction::PushRelease)
            }
            Self::ShiftRegister => matches!(
                action,
                ModuleAction::PushPress
                    | ModuleAction::PushRelease
                    | ModuleAction::ResetPress
                    | ModuleAction::ResetRelease
            ),
            Self::MidiGate => action == ModuleAction::MidiPanic,
            Self::Push => matches!(
                action,
                ModuleAction::PushPress
                    | ModuleAction::PushRelease
                    | ModuleAction::HoldPress
                    | ModuleAction::HoldRelease
            ),
            _ => false,
        }
    }
}
fn is_release(action: ModuleAction) -> bool {
    matches!(
        action,
        ModuleAction::BRelease
            | ModuleAction::GateRelease
            | ModuleAction::PushRelease
            | ModuleAction::HoldRelease
            | ModuleAction::ResetRelease
            | ModuleAction::MidiPanic
            | ModuleAction::ClockRelease
            | ModuleAction::RunRelease
            | ModuleAction::SeqGateRelease(_)
    ) || matches!(action.pulse_button(), Some((_, false)))
}
#[derive(Clone, Copy)]
struct Entry {
    slot: usize,
    token: u64,
    born_revision: u64,
    kind: ModuleKind,
    bypassed: bool,
    action_epoch: u64,
}
struct StateEntry {
    slot: usize,
    token: u64,
    born_revision: u64,
    kind: String,
    initialize_generation: u64,
    initialize_revision: u64,
    saved_generation: u64,
    saved_revision: u64,
    desired_state: Option<SavedModuleState>,
    stock_field_generations: [u64; 10],
}
type RouteSignature = (String, String, String, String, String, String);
fn route_signature(patch: &Patch, cable: &Cable) -> RouteSignature {
    let kind = |id: &str| {
        patch
            .nodes
            .iter()
            .find(|node| node.id == id)
            .map(|node| node.kind.clone())
            .unwrap_or_default()
    };
    (
        cable.from.clone(),
        cable.output.clone(),
        cable.to.clone(),
        cable.input.clone(),
        kind(&cable.from),
        kind(&cable.to),
    )
}
struct CableEntry {
    slot: usize,
    token: u64,
    born_revision: u64,
    route: RouteSignature,
}
struct Control {
    queue: Producer<Event>,
    entries: BTreeMap<String, Entry>,
    state_entries: BTreeMap<String, StateEntry>,
    cable_entries: BTreeMap<String, CableEntry>,
    scope_readers: BTreeMap<String, ScopeReader>,
    revision: u64,
    next_token: u64,
    next_sequence: u64,
    midi_learns: BTreeMap<String, MidiLearnControl>,
    next_learn_serial: u64,
    stock_click_releases: BTreeMap<String, StockClickRelease>,
}
#[derive(Clone, Copy)]
struct ModuleSnapshot {
    revision: u64,
    frame: u64,
    state: Option<ModuleTelemetry>,
    saved_state: Option<SavedModuleState>,
    controls: Option<ModuleControlState>,
    lights: Option<ModuleLightState>,
    midi_learning: Option<(u64, u8)>,
}
struct Meter {
    serial: AtomicU64,
    token: AtomicU64,
    revision: AtomicU64,
    frame: AtomicU64,
    value: AtomicU32,
    flags: AtomicU32,
    saved_state: AtomicU32,
    random_values: [[AtomicU32; 16]; 7],
    random_rng: AtomicU32,
    shift_values: [AtomicU32; 8],
    shift_rng: AtomicU32,
    cc_assignments: [AtomicU32; 16],
    cc_values: [AtomicU32; 128],
    cc_learning_serial: AtomicU64,
    cc_learning_cell: AtomicU32,
    input_channels: AtomicU32,
    output_channels: AtomicU32,
    automatic_channels: AtomicU32,
    waveform_position: AtomicU32,
    clock_frequency: AtomicU32,
    playing_notes: AtomicU32,
    octave_shift: AtomicU32,
    brightness: [AtomicU32; MAX_MODULE_LIGHTS * 2],
}
impl Meter {
    fn new() -> Self {
        Self {
            serial: AtomicU64::new(0),
            token: AtomicU64::new(0),
            revision: AtomicU64::new(0),
            frame: AtomicU64::new(0),
            value: AtomicU32::new(0),
            flags: AtomicU32::new(0),
            saved_state: AtomicU32::new(0),
            random_values: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU32::new(0))),
            random_rng: AtomicU32::new(0),
            shift_values: std::array::from_fn(|_| AtomicU32::new(0)),
            shift_rng: AtomicU32::new(0),
            cc_assignments: std::array::from_fn(|_| AtomicU32::new(0)),
            cc_values: std::array::from_fn(|_| AtomicU32::new(0)),
            cc_learning_serial: AtomicU64::new(0),
            cc_learning_cell: AtomicU32::new(0),
            input_channels: AtomicU32::new(u32::MAX),
            output_channels: AtomicU32::new(u32::MAX),
            automatic_channels: AtomicU32::new(u32::MAX),
            waveform_position: AtomicU32::new(f32::NAN.to_bits()),
            clock_frequency: AtomicU32::new(f32::NAN.to_bits()),
            playing_notes: AtomicU32::new(u32::MAX),
            octave_shift: AtomicU32::new(0),
            brightness: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }
    fn write(&self, token: u64, snapshot: ModuleSnapshot) {
        self.serial.fetch_add(1, Ordering::SeqCst);
        self.token.store(token, Ordering::SeqCst);
        self.revision.store(snapshot.revision, Ordering::SeqCst);
        self.frame.store(snapshot.frame, Ordering::SeqCst);
        let mut flags = 0;
        if let Some(state) = snapshot.state {
            flags |= 1;
            self.value.store(
                u32::from(state.step)
                    | (u32::from(state.length) << 8)
                    | (u32::from(state.running) << 16),
                Ordering::SeqCst,
            );
        }
        self.saved_state.store(
            match snapshot.saved_state {
                Some(SavedModuleState::StockSeq3 {
                    running,
                    gates,
                    passthrough,
                }) => {
                    7 | (u32::from(running) << 4)
                        | (u32::from(gates) << 5)
                        | (u32::from(passthrough) << 13)
                }
                Some(SavedModuleState::StockSeq3Fields {
                    running,
                    gates,
                    gate_mask,
                    passthrough,
                }) => {
                    8 | (u32::from(running.is_some()) << 4)
                        | (u32::from(running.unwrap_or(false)) << 5)
                        | (u32::from(passthrough.is_some()) << 6)
                        | (u32::from(passthrough.unwrap_or(false)) << 7)
                        | (u32::from(gate_mask) << 8)
                        | (u32::from(gates) << 16)
                }
                Some(SavedModuleState::StockQuantizerNotes(mask)) => {
                    if mask <= 4095 {
                        9 | (u32::from(mask) << 4)
                    } else {
                        0
                    }
                }
                Some(SavedModuleState::Seq3Run(value)) => 1 | (u32::from(value) << 4),
                Some(SavedModuleState::PushHold(value)) => 2 | (u32::from(value) << 4),
                Some(SavedModuleState::RandomValues { values, rng }) => {
                    for (row, values) in self.random_values.iter().zip(values) {
                        for (slot, value) in row.iter().zip(values) {
                            slot.store(value, Ordering::SeqCst);
                        }
                    }
                    self.random_rng.store(rng, Ordering::SeqCst);
                    3
                }
                Some(SavedModuleState::ShiftRegister { values, rng }) => {
                    for (slot, value) in self.shift_values.iter().zip(values) {
                        slot.store(value, Ordering::SeqCst);
                    }
                    self.shift_rng.store(rng, Ordering::SeqCst);
                    4
                }
                Some(SavedModuleState::MidiCc {
                    assignments,
                    values,
                }) => {
                    for (slot, value) in self.cc_assignments.iter().zip(assignments) {
                        slot.store(value as u16 as u32, Ordering::SeqCst);
                    }
                    for (slot, value) in self.cc_values.iter().zip(values) {
                        slot.store(u32::from(value), Ordering::SeqCst);
                    }
                    5
                }
                Some(SavedModuleState::MidiGateMappings(assignments)) => {
                    for (slot, value) in self.cc_assignments.iter().zip(assignments) {
                        slot.store(value as u16 as u32, Ordering::SeqCst);
                    }
                    6
                }
                Some(SavedModuleState::MidiCcMappings(_)) => 0,
                None => 0,
            },
            Ordering::SeqCst,
        );
        self.cc_learning_serial
            .store(snapshot.midi_learning.map_or(0, |x| x.0), Ordering::SeqCst);
        self.cc_learning_cell.store(
            snapshot.midi_learning.map_or(0, |x| u32::from(x.1)),
            Ordering::SeqCst,
        );
        if let Some(controls) = snapshot.controls {
            flags |=
                2 | (u32::from(controls.pressed) << 3) | (u32::from(controls.pressed_mask) << 16);
        }
        // Position belongs to this exact light packet. Clear the slot for both
        // absent lights and absent/invalid positions to avoid retained values.
        let position = snapshot
            .lights
            .and_then(|lights| lights.waveform_position)
            .filter(|value| value.is_finite() && (0.0..=1.0).contains(value));
        self.waveform_position
            .store(position.unwrap_or(f32::NAN).to_bits(), Ordering::SeqCst);
        let clock = snapshot
            .lights
            .and_then(|lights| lights.clock_frequency)
            .filter(|value| value.is_finite() && *value > 0.0);
        self.clock_frequency
            .store(clock.unwrap_or(f32::NAN).to_bits(), Ordering::SeqCst);
        let playing = snapshot
            .lights
            .and_then(|lights| lights.playing_notes)
            .filter(|mask| *mask <= 4095);
        self.playing_notes
            .store(playing.map(u32::from).unwrap_or(u32::MAX), Ordering::SeqCst);
        let octave_shift = snapshot.lights.and_then(|lights| lights.octave_shift);
        self.octave_shift
            .store(octave_shift.unwrap_or(0) as u32, Ordering::SeqCst);
        if octave_shift.is_some() {
            flags |= 1 << 5;
        }
        // Counts are independent optional fields in the same serial boundary.
        // Clear absent and invalid values on every packet, including no lights.
        for (slot, count) in [
            (
                &self.output_channels,
                snapshot.lights.and_then(|lights| lights.output_channels),
            ),
            (
                &self.automatic_channels,
                snapshot.lights.and_then(|lights| lights.automatic_channels),
            ),
        ] {
            slot.store(
                count
                    .filter(|count| *count <= 16)
                    .map(u32::from)
                    .unwrap_or(u32::MAX),
                Ordering::SeqCst,
            );
        }
        if let Some(lights) = snapshot.lights {
            flags |= 4 | (u32::from(lights.manual) << 4) | (u32::from(lights.count) << 8);
            self.input_channels.store(
                lights.input_channels.map(u32::from).unwrap_or(u32::MAX),
                Ordering::SeqCst,
            );
            for (index, value) in lights.brightness.iter().flatten().enumerate() {
                self.brightness[index].store(value.to_bits(), Ordering::SeqCst);
            }
        }
        self.flags.store(flags, Ordering::SeqCst);
        self.serial.fetch_add(1, Ordering::SeqCst);
    }
    fn read(&self, entry: Entry, revision: u64) -> Option<ModuleSnapshot> {
        for _ in 0..4 {
            let before = self.serial.load(Ordering::SeqCst);
            if before & 1 != 0 {
                continue;
            }
            let token = self.token.load(Ordering::SeqCst);
            let actual_revision = self.revision.load(Ordering::SeqCst);
            let frame = self.frame.load(Ordering::SeqCst);
            let value = self.value.load(Ordering::SeqCst);
            let flags = self.flags.load(Ordering::SeqCst);
            let saved_state = self.saved_state.load(Ordering::SeqCst);
            let random_state = (saved_state & 15 == 3).then(|| SavedModuleState::RandomValues {
                values: std::array::from_fn(|row| {
                    std::array::from_fn(|channel| {
                        self.random_values[row][channel].load(Ordering::SeqCst)
                    })
                }),
                rng: self.random_rng.load(Ordering::SeqCst),
            });
            let shift_state = (saved_state & 15 == 4).then(|| SavedModuleState::ShiftRegister {
                values: std::array::from_fn(|row| self.shift_values[row].load(Ordering::SeqCst)),
                rng: self.shift_rng.load(Ordering::SeqCst),
            });
            let cc_state = (saved_state & 15 == 5).then(|| SavedModuleState::MidiCc {
                assignments: std::array::from_fn(|i| {
                    self.cc_assignments[i].load(Ordering::SeqCst) as u16 as i16
                }),
                values: std::array::from_fn(|i| self.cc_values[i].load(Ordering::SeqCst) as u8),
            });
            let gate_state = (saved_state & 15 == 6).then(|| {
                SavedModuleState::MidiGateMappings(std::array::from_fn(|i| {
                    self.cc_assignments[i].load(Ordering::SeqCst) as u16 as i16
                }))
            });
            let cc_serial = self.cc_learning_serial.load(Ordering::SeqCst);
            let cc_cell = self.cc_learning_cell.load(Ordering::SeqCst);
            let input_channels = self.input_channels.load(Ordering::SeqCst);
            let output_channels = self.output_channels.load(Ordering::SeqCst);
            let automatic_channels = self.automatic_channels.load(Ordering::SeqCst);
            let waveform_position = f32::from_bits(self.waveform_position.load(Ordering::SeqCst));
            let clock_frequency = f32::from_bits(self.clock_frequency.load(Ordering::SeqCst));
            let playing_notes = self.playing_notes.load(Ordering::SeqCst);
            let octave_shift = self.octave_shift.load(Ordering::SeqCst) as i32;
            let brightness = std::array::from_fn(|row| {
                std::array::from_fn(|col| {
                    f32::from_bits(self.brightness[row * 2 + col].load(Ordering::SeqCst))
                })
            });
            let after = self.serial.load(Ordering::SeqCst);
            if before != after {
                continue;
            }
            return (token == entry.token && actual_revision >= revision).then_some(
                ModuleSnapshot {
                    revision: actual_revision,
                    frame,
                    state: (flags & 1 != 0).then_some(ModuleTelemetry {
                        step: value as u8,
                        length: (value >> 8) as u8,
                        running: value & (1 << 16) != 0,
                    }),
                    saved_state: match saved_state & 15 {
                        1 => Some(SavedModuleState::Seq3Run(saved_state & 16 != 0)),
                        2 => Some(SavedModuleState::PushHold(saved_state & 16 != 0)),
                        3 => random_state,
                        4 => shift_state,
                        5 => cc_state,
                        6 => gate_state,
                        7 => Some(SavedModuleState::StockSeq3 {
                            running: saved_state & 16 != 0,
                            gates: ((saved_state >> 5) & 255) as u8,
                            passthrough: saved_state & (1 << 13) != 0,
                        }),
                        8 => Some(SavedModuleState::StockSeq3Fields {
                            running: (saved_state & 16 != 0).then_some(saved_state & 32 != 0),
                            gates: (saved_state >> 16) as u8,
                            gate_mask: (saved_state >> 8) as u8,
                            passthrough: (saved_state & 64 != 0).then_some(saved_state & 128 != 0),
                        }),
                        9 => ((saved_state >> 4) <= 4095).then_some(
                            SavedModuleState::StockQuantizerNotes((saved_state >> 4) as u16),
                        ),
                        _ => None,
                    },
                    controls: (flags & 2 != 0).then_some(ModuleControlState {
                        pressed: flags & 8 != 0,
                        pressed_mask: (flags >> 16) as u16,
                    }),
                    lights: (flags & 4 != 0).then_some(ModuleLightState {
                        playing_notes: (entry.kind == ModuleKind::StockQuantizer
                            && playing_notes <= 4095)
                            .then_some(playing_notes as u16),
                        count: (flags >> 8) as u8,
                        brightness,
                        manual: flags & 16 != 0,
                        input_channels: (input_channels <= 16).then_some(input_channels as u8),
                        output_channels: (entry.kind == ModuleKind::Merge && output_channels <= 16)
                            .then_some(output_channels as u8),
                        automatic_channels: (entry.kind == ModuleKind::Merge
                            && automatic_channels <= 16)
                            .then_some(automatic_channels as u8),
                        clock_frequency: (entry.kind == ModuleKind::StockLfo
                            && clock_frequency.is_finite()
                            && clock_frequency > 0.0)
                            .then_some(clock_frequency),
                        waveform_position: (waveform_position.is_finite()
                            && (0.0..=1.0).contains(&waveform_position))
                        .then_some(waveform_position),
                        octave_shift: (entry.kind == ModuleKind::StockOctave
                            && flags & (1 << 5) != 0)
                            .then_some(octave_shift),
                    }),
                    midi_learning: (cc_serial != 0 && cc_cell < 16)
                        .then_some((cc_serial, cc_cell as u8)),
                },
            );
        }
        None
    }
}
struct StateAcknowledgment {
    serial: AtomicU64,
    token: AtomicU64,
    generation: AtomicU64,
}
impl StateAcknowledgment {
    fn new() -> Self {
        Self {
            serial: AtomicU64::new(0),
            token: AtomicU64::new(0),
            generation: AtomicU64::new(0),
        }
    }
    fn write(&self, token: u64, generation: u64) {
        self.serial.fetch_add(1, Ordering::SeqCst);
        self.token.store(token, Ordering::SeqCst);
        self.generation.store(generation, Ordering::SeqCst);
        self.serial.fetch_add(1, Ordering::SeqCst);
    }
    fn read(&self, token: u64) -> Option<u64> {
        for _ in 0..4 {
            let serial = self.serial.load(Ordering::SeqCst);
            if serial & 1 != 0 {
                continue;
            }
            let actual = self.token.load(Ordering::SeqCst);
            let generation = self.generation.load(Ordering::SeqCst);
            if serial == self.serial.load(Ordering::SeqCst) {
                return (actual == token).then_some(generation);
            }
        }
        None
    }
}
// Preserve unconsumed assignment intents without capturing unrelated live flags.
fn stock_assignment_fields(
    value: SavedModuleState,
) -> Option<(Option<bool>, u8, u8, Option<bool>)> {
    match value {
        SavedModuleState::StockSeq3 {
            running,
            gates,
            passthrough,
        } => Some((Some(running), gates, 255, Some(passthrough))),
        SavedModuleState::StockSeq3Fields {
            running,
            gates,
            gate_mask,
            passthrough,
        } => Some((running, gates, gate_mask, passthrough)),
        _ => None,
    }
}
fn merge_stock_assignment(previous: SavedModuleState, next: SavedModuleState) -> SavedModuleState {
    let (Some((old_run, old_gates, old_mask, old_pass)), Some((run, gates, mask, pass))) = (
        stock_assignment_fields(previous),
        stock_assignment_fields(next),
    ) else {
        return next;
    };
    SavedModuleState::StockSeq3Fields {
        running: run.or(old_run),
        gates: (old_gates & !mask) | (gates & mask),
        gate_mask: old_mask | mask,
        passthrough: pass.or(old_pass),
    }
}
fn unconsumed_stock_assignment(
    value: SavedModuleState,
    generations: &[u64; 10],
    applied: u64,
) -> SavedModuleState {
    let Some((running, gates, mask, passthrough)) = stock_assignment_fields(value) else {
        return value;
    };
    let mut gate_mask = 0;
    for i in 0..8 {
        if mask & (1 << i) != 0 && generations[i + 1] > applied {
            gate_mask |= 1 << i;
        }
    }
    SavedModuleState::StockSeq3Fields {
        running: running.filter(|_| generations[0] > applied),
        gates,
        gate_mask,
        passthrough: passthrough.filter(|_| generations[9] > applied),
    }
}
// Wire visibility follows logical light groups, never the fixed callback storage
// capacity. Keep the derived optional metadata representation unchanged.
fn module_lights_value(lights: ModuleLightState) -> Option<Value> {
    let count = usize::from(lights.count);
    if count > MAX_MODULE_LIGHTS {
        return None;
    }
    let mut value =
        serde_json::to_value(lights).expect("finite module light packet is serializable");
    value["brightness"]
        .as_array_mut()
        .expect("module light brightness is an array")
        .truncate(count);
    Some(value)
}
struct Shared {
    desired: [AtomicU64; MAX_NODES],
    action_epochs: [AtomicU64; MAX_NODES],
    emergency_release_epochs: [AtomicU64; MAX_NODES],
    emergency_learn_cutoffs: [AtomicU64; MAX_NODES],
    emergency_release_requests: AtomicU64,
    emergency_release_applied: AtomicU64,
    meters: Box<[Meter; MAX_NODES]>,
    saved_acknowledgments: Box<[StateAcknowledgment; MAX_NODES]>,
    sampled_sequence: AtomicU64,
    backend: u64,
    closed: AtomicBool,
    // One writer (the audio callback) surrounds the complete meter batch.
    // Readers can reject a partial cross-module publication without waiting.
    publish_serial: AtomicU64,
    frame: AtomicU64,
    queued: AtomicU64,
    applied: AtomicU64,
    dropped: AtomicU64,
    backpressure: AtomicU64,
    rejected: AtomicU64,
    expired: AtomicU64,
    obsolete: AtomicU64,
    closed_dropped: AtomicU64,
    crossfade_rejected: AtomicU64,
    last_applied_sequence: AtomicU64,
    midi_learns: Box<[MidiLearnMailbox; MAX_NODES]>,
    midi_completion_sequence: AtomicU64,
}
impl Shared {
    fn publish_frame(&self, frame: u64, publish: impl FnOnce()) {
        self.publish_serial.fetch_add(1, Ordering::SeqCst);
        self.frame.store(frame, Ordering::SeqCst);
        publish();
        self.publish_serial.fetch_add(1, Ordering::SeqCst);
    }
}
pub(super) struct Performance {
    control: Mutex<Control>,
    pub(super) diagnostics: Arc<Diagnostics>,
    shared: Arc<Shared>,
    rate: u32,
}
#[derive(Clone, Copy)]
pub(super) struct Binding {
    pub token: u64,
    pub handle: ModuleHandle,
    kind: ModuleKind,
    pub release_epoch: u64,
}
#[derive(Clone, Copy)]
pub(super) struct StateBinding {
    pub token: u64,
    pub handle: ModuleHandle,
    initialize_generation: u64,
    applied_initialization: u64,
    saved_generation: u64,
    applied_saved: u64,
    desired_state: Option<SavedModuleState>,
    stock_field_generations: [u64; 10],
}
#[derive(Clone, Copy, Debug)]
pub(super) struct CableBinding {
    pub token: u64,
    pub handle: CableHandle,
}
struct ScopeBinding {
    token: u64,
    handle: ModuleHandle,
    producer: ScopeProducer,
    next_frame: u64,
}
pub(super) struct ModulePlan {
    midi_learns: Vec<MidiLearnBinding>,
    scopes: Vec<ScopeBinding>,
    scope_cursor: usize,
    scope_interval: u64,
    pub bindings: [Option<Binding>; MAX_NODES],
    pub states: Box<[Option<StateBinding>; MAX_NODES]>,
    pub fresh_states: [Option<ModuleHandle>; MAX_NODES],
    // Compile/control owns these fixed-capacity allocations. Queued graph
    // moves carry pointers; retired graphs free them back on the control side.
    pub cables: Box<[Option<CableBinding>; MAX_CABLES]>,
    pub fresh_cables: Box<[Option<CableHandle>; MAX_CABLES]>,
}
impl Default for ModulePlan {
    fn default() -> Self {
        Self {
            scopes: Vec::new(),
            midi_learns: Vec::new(),
            scope_cursor: 0,
            scope_interval: 1,
            bindings: [None; MAX_NODES],
            states: vec![None; MAX_NODES]
                .into_boxed_slice()
                .try_into()
                .ok()
                .expect("fixed state binding capacity"),
            fresh_states: [None; MAX_NODES],
            cables: vec![None; MAX_CABLES]
                .into_boxed_slice()
                .try_into()
                .expect("fixed cable binding capacity"),
            fresh_cables: vec![None; MAX_CABLES]
                .into_boxed_slice()
                .try_into()
                .expect("fixed fresh cable capacity"),
        }
    }
}
impl ModulePlan {
    /// At most one fixed-size waveform copy per device callback. Unobserved
    /// instances slow down under load; audio never waits for display consumers.
    pub(super) fn publish_scope(&mut self, engine: &Engine, frame: u64) {
        let count = self.scopes.len();
        for offset in 0..count {
            let index = (self.scope_cursor + offset) % count;
            let binding = &mut self.scopes[index];
            if frame < binding.next_frame {
                continue;
            }
            if let Some(snapshot) = engine.module_scope(binding.handle) {
                binding.producer.publish(snapshot, frame);
                binding.next_frame = frame.saturating_add(self.scope_interval);
                self.scope_cursor = (index + 1) % count;
                break;
            }
        }
    }
    /// Saved custom state is assigned after DSP adoption, never replayed as a
    /// button press. A load also reaches the fading instance of the same module.
    pub(super) fn load_saved_pending(
        &mut self,
        engine: &mut Engine,
        mut previous: Option<(&mut Self, &mut Engine)>,
    ) {
        for (slot, binding) in self.states.iter_mut().enumerate() {
            let Some(binding) = binding else { continue };
            let old = previous.as_mut().and_then(|(plan, engine)| {
                plan.states[slot]
                    .as_mut()
                    .filter(|old| old.token == binding.token)
                    .map(|old| (old, &mut **engine))
            });
            if let Some((old, _)) = old.as_ref() {
                binding.applied_saved = binding.applied_saved.max(old.applied_saved);
            }
            if binding.saved_generation <= binding.applied_saved {
                continue;
            }
            let Some(state) = binding.desired_state else {
                continue;
            };
            let state = unconsumed_stock_assignment(
                state,
                &binding.stock_field_generations,
                binding.applied_saved,
            );
            if engine.set_module_saved_state(binding.handle, state) {
                binding.applied_saved = binding.saved_generation;
                if let Some((old, old_engine)) = old {
                    if old_engine.set_module_saved_state(old.handle, state) {
                        old.applied_saved = old.applied_saved.max(binding.saved_generation);
                    }
                }
            }
        }
    }

    /// Apply the control-thread fence only after normal DSP state adoption.
    /// The applied generation travels with the live instance, so history and
    /// stale prepared graphs cannot replay an initialization event.
    pub(super) fn initialize_pending(&mut self, engine: &mut Engine, previous: Option<&Self>) {
        for (slot, binding) in self.states.iter_mut().enumerate() {
            let Some(binding) = binding else { continue };
            if let Some(old) = previous
                .and_then(|plan| plan.states[slot])
                .filter(|old| old.token == binding.token)
            {
                binding.applied_initialization = binding
                    .applied_initialization
                    .max(old.applied_initialization);
            }
            if binding.initialize_generation > binding.applied_initialization
                && engine.initialize_module_state(binding.handle)
            {
                binding.applied_initialization = binding.initialize_generation;
            }
        }
    }

    #[cfg(test)]
    pub(super) fn adopt_state(&mut self, engine: &mut Engine, previous: &Self, old: &Engine) {
        self.with_adoption_exclusions(previous, |modules, cables| {
            engine.adopt_state_from_excluding_routes(old, modules, cables);
        });
    }
    /// Large Delay allocations move once; the caller must freeze the donor if
    /// any runtime moved. Exclusion and observer rules are shared with snapshots.
    pub(super) fn handoff_state_with_host_exclusions(
        &mut self,
        engine: &mut Engine,
        previous: &Self,
        old: &mut Engine,
        host_resets: &[ModuleHandle],
    ) -> bool {
        let mut moved = false;
        self.with_adoption_exclusions(previous, |modules, cables| {
            let first = modules.first().or_else(|| host_resets.first());
            if let Some(first) = first {
                let mut merged = [*first; MAX_NODES];
                let mut len = 0;
                for handle in modules.iter().chain(host_resets) {
                    if !merged[..len].contains(handle) {
                        merged[len] = *handle;
                        len += 1;
                    }
                }
                moved = engine.handoff_state_from_excluding_routes(old, &merged[..len], cables);
            } else {
                moved = engine.handoff_state_from_excluding_routes(old, &[], cables);
            }
        });
        moved
    }
    /// Exclusions are fixed stack storage; no control registry reaches audio.
    fn with_adoption_exclusions(
        &mut self,
        previous: &Self,
        mut apply: impl FnMut(&[ModuleHandle], &[CableHandle]),
    ) {
        // Display edits and graph rebuilds preserve the per-instance refresh
        // deadline. A new capture reader can remain unavailable until it is due.
        for binding in &mut self.scopes {
            if let Some(old) = previous
                .scopes
                .iter()
                .find(|old| old.token == binding.token)
            {
                binding.next_frame = old.next_frame;
            }
        }
        let first = self
            .states
            .iter()
            .flatten()
            .map(|binding| binding.handle)
            .chain(self.fresh_states.iter().flatten().copied())
            .next();
        let Some(first) = first else {
            apply(&[], &[]);
            return;
        };
        let mut modules = [first; MAX_NODES];
        let mut module_count = 0;
        for (slot, binding) in self.states.iter().enumerate() {
            if let Some(binding) = binding {
                if !previous.states[slot].is_some_and(|old| old.token == binding.token) {
                    modules[module_count] = binding.handle;
                    module_count += 1;
                }
            }
        }
        for handle in self.fresh_states.iter().flatten() {
            modules[module_count] = *handle;
            module_count += 1;
        }
        let first = self
            .cables
            .iter()
            .flatten()
            .map(|binding| binding.handle)
            .chain(self.fresh_cables.iter().flatten().copied())
            .next();
        if let Some(first) = first {
            let mut cables = [first; MAX_CABLES];
            let mut cable_count = 0;
            for (slot, binding) in self.cables.iter().enumerate() {
                if let Some(binding) = binding {
                    if !previous.cables[slot].is_some_and(|old| old.token == binding.token) {
                        cables[cable_count] = binding.handle;
                        cable_count += 1;
                    }
                }
            }
            for handle in self.fresh_cables.iter().flatten() {
                cables[cable_count] = *handle;
                cable_count += 1;
            }
            apply(&modules[..module_count], &cables[..cable_count]);
        } else {
            apply(&modules[..module_count], &[]);
        }
    }
}
pub(super) struct EventReader {
    queue: Consumer<Event>,
    head: Option<Event>,
    shared: Arc<Shared>,
}

impl Performance {
    pub(super) fn new(rate: u32) -> (Arc<Self>, EventReader, Recorder) {
        let (diagnostics, timing) = Diagnostics::new(rate);
        let (queue, input) = RingBuffer::new(EVENT_CAPACITY);
        let shared = Arc::new(Shared {
            desired: std::array::from_fn(|_| AtomicU64::new(0)),
            action_epochs: std::array::from_fn(|_| AtomicU64::new(0)),
            emergency_release_epochs: std::array::from_fn(|_| AtomicU64::new(0)),
            emergency_learn_cutoffs: std::array::from_fn(|_| AtomicU64::new(0)),
            emergency_release_requests: AtomicU64::new(0),
            emergency_release_applied: AtomicU64::new(0),
            meters: (0..MAX_NODES)
                .map(|_| Meter::new())
                .collect::<Vec<_>>()
                .into_boxed_slice()
                .try_into()
                .ok()
                .expect("fixed meter capacity"),
            saved_acknowledgments: (0..MAX_NODES)
                .map(|_| StateAcknowledgment::new())
                .collect::<Vec<_>>()
                .into_boxed_slice()
                .try_into()
                .ok()
                .expect("fixed state acknowledgment capacity"),
            sampled_sequence: AtomicU64::new(0),
            backend: NEXT_MIDI_BACKEND.fetch_add(1, Ordering::Relaxed),
            closed: AtomicBool::new(false),
            publish_serial: AtomicU64::new(0),
            frame: AtomicU64::new(0),
            queued: AtomicU64::new(0),
            applied: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            backpressure: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            expired: AtomicU64::new(0),
            obsolete: AtomicU64::new(0),
            closed_dropped: AtomicU64::new(0),
            crossfade_rejected: AtomicU64::new(0),
            last_applied_sequence: AtomicU64::new(0),
            midi_learns: (0..MAX_NODES)
                .map(|_| MidiLearnMailbox::new())
                .collect::<Vec<_>>()
                .into_boxed_slice()
                .try_into()
                .ok()
                .expect("fixed CC Learn capacity"),
            midi_completion_sequence: AtomicU64::new(0),
        });
        let reader = EventReader {
            queue: input,
            head: None,
            shared: shared.clone(),
        };
        (
            Arc::new(Self {
                control: Mutex::new(Control {
                    queue,
                    entries: BTreeMap::new(),
                    state_entries: BTreeMap::new(),
                    cable_entries: BTreeMap::new(),
                    scope_readers: BTreeMap::new(),
                    revision: 0,
                    next_token: 1,
                    next_sequence: 1,
                    midi_learns: BTreeMap::new(),
                    next_learn_serial: 1,
                    stock_click_releases: BTreeMap::new(),
                }),
                shared,
                diagnostics,
                rate,
            }),
            reader,
            timing,
        )
    }
    pub(super) fn plan(
        &self,
        patch: &Patch,
        engine: &Engine,
        revision: u64,
    ) -> Result<ModulePlan, String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Module event registry poisoned")?;
        let mut plan = ModulePlan::default();
        for (index, node) in patch.nodes.iter().enumerate() {
            let handle = engine.resolve_module_state(&node.id)?;
            if let Some(entry) = control
                .state_entries
                .get(&node.id)
                .filter(|entry| revision >= entry.born_revision && node.kind == entry.kind)
            {
                plan.states[entry.slot] = Some(StateBinding {
                    token: entry.token,
                    handle,
                    initialize_generation: if revision >= entry.initialize_revision {
                        entry.initialize_generation
                    } else {
                        0
                    },
                    applied_initialization: 0,
                    saved_generation: if revision >= entry.saved_revision {
                        entry.saved_generation
                    } else {
                        0
                    },
                    applied_saved: 0,
                    desired_state: entry.desired_state,
                    stock_field_generations: entry.stock_field_generations,
                });
            } else {
                plan.fresh_states[index] = Some(handle);
            }
        }
        if patch.version == 2 {
            for (index, cable) in patch.cables.iter().enumerate() {
                let Some(id) = cable.id.as_ref() else {
                    continue;
                };
                let handle = engine.resolve_cable_state(id)?;
                if let Some(entry) = control.cable_entries.get(id).filter(|entry| {
                    revision >= entry.born_revision && route_signature(patch, cable) == entry.route
                }) {
                    plan.cables[entry.slot] = Some(CableBinding {
                        token: entry.token,
                        handle,
                    });
                } else {
                    plan.fresh_cables[index] = Some(handle);
                }
            }
        }
        for (id, entry) in &control.entries {
            // A stale UI snapshot must not bind a newly reincarnated instance.
            if revision < entry.born_revision
                || !patch
                    .nodes
                    .iter()
                    .any(|n| n.id == *id && ModuleKind::from_name(&n.kind) == Some(entry.kind))
            {
                continue;
            }
            plan.bindings[entry.slot] = Some(Binding {
                token: entry.token,
                handle: if entry.kind.sequential() {
                    engine.resolve_module_telemetry(id)?
                } else if entry.kind.learn_kind().is_some() {
                    engine.resolve_module_state(id)?
                } else if entry.kind.manual() {
                    engine.resolve_module_control(id)?
                } else {
                    engine.resolve_module_lights(id)?
                },
                kind: entry.kind,
                release_epoch: 0,
            });
        }
        self.plan_midi_learns(&control, patch, engine, &mut plan)?;
        plan.scope_interval = u64::from(self.rate).div_ceil(30);
        for node in patch.nodes.iter().filter(|node| node.kind == "scope") {
            let Some(entry) = control
                .state_entries
                .get(&node.id)
                .filter(|entry| entry.kind == "scope" && revision >= entry.born_revision)
            else {
                continue;
            };
            let token = entry.token;
            let handle = engine.resolve_module_scope(&node.id)?;
            let (producer, reader) = scope_transport::channel(token, revision);
            // Prepared stale graphs must never replace the latest reader.
            if revision == control.revision {
                control.scope_readers.insert(node.id.clone(), reader);
            }
            plan.scopes.push(ScopeBinding {
                token,
                handle,
                producer,
                next_frame: 0,
            });
        }
        Ok(plan)
    }
    pub(super) fn scope_frame(&self, id: &str) -> Option<ScopeFrame> {
        let control = self.control.lock().ok()?;
        if self.shared.closed.load(Ordering::Acquire) {
            return None;
        }
        let entry = control
            .state_entries
            .get(id)
            .filter(|entry| entry.kind == "scope")?;
        control
            .scope_readers
            .get(id)?
            .read(entry.token, control.revision)
    }
    pub(super) fn sequence_state(&self, id: &str) -> Option<ModuleTelemetry> {
        let control = self.control.lock().ok()?;
        if self.shared.closed.load(Ordering::Acquire) {
            return None;
        }
        let entry = *control.entries.get(id)?;
        self.shared.meters[entry.slot]
            .read(entry, control.revision)?
            .state
    }
    pub(super) fn module_control_state(&self, id: &str) -> Option<ModuleControlState> {
        let control = self.control.lock().ok()?;
        if self.shared.closed.load(Ordering::Acquire) {
            return None;
        }
        let entry = *control.entries.get(id)?;
        self.shared.meters[entry.slot]
            .read(entry, control.revision)?
            .controls
    }
    pub(super) fn module_lights(&self, id: &str) -> Option<ModuleLightState> {
        let control = self.control.lock().ok()?;
        if self.shared.closed.load(Ordering::Acquire) {
            return None;
        }
        let entry = *control.entries.get(id)?;
        self.shared.meters[entry.slot]
            .read(entry, control.revision)?
            .lights
    }
    fn request_release(&self, entry: &mut Entry, cancel_through_serial: u64) {
        entry.action_epoch = entry.action_epoch.wrapping_add(1).max(1);
        self.shared.action_epochs[entry.slot].store(entry.action_epoch, Ordering::Release);
        self.shared.emergency_learn_cutoffs[entry.slot]
            .store(cancel_through_serial, Ordering::Release);
        self.shared.emergency_release_epochs[entry.slot].fetch_add(1, Ordering::Release);
        self.shared
            .emergency_release_requests
            .fetch_add(1, Ordering::Relaxed);
    }
    pub(super) fn close(&self) {
        // Serialize against producer acceptance. This mutex never reaches audio.
        let _control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        self.shared.closed.store(true, Ordering::Release);
        for mailbox in self.shared.midi_learns.iter() {
            let _ = mailbox.cancel_armed();
        }
        self.diagnostics.close();
    }
    fn load_saved_states(&self, states: &BTreeMap<String, SavedModuleState>) {
        let mut control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        if self.shared.closed.load(Ordering::Acquire) {
            return;
        }
        let revision = control.revision;
        for (id, value) in states {
            if let Some(cc) = control.midi_learns.get_mut(id) {
                if self.shared.midi_learns[cc.slot].cancel_armed().is_ok() {
                    cc.epoch = learn_epoch();
                }
            }
            let Some(state) = control.state_entries.get_mut(id).filter(|state| {
                state
                    .desired_state
                    .is_some_and(|desired| desired.kind() == value.kind())
            }) else {
                continue;
            };
            let pending = state.saved_generation != 0
                && self.shared.saved_acknowledgments[state.slot]
                    .read(state.token)
                    .is_none_or(|applied| state.saved_generation > applied);
            state.saved_generation = state.saved_generation.saturating_add(1);
            state.saved_revision = revision;
            let assignment = if pending {
                state
                    .desired_state
                    .map_or(*value, |previous| merge_stock_assignment(previous, *value))
            } else {
                *value
            };
            if let Some((running, _, mask, passthrough)) = stock_assignment_fields(*value) {
                if running.is_some() {
                    state.stock_field_generations[0] = state.saved_generation;
                }
                if passthrough.is_some() {
                    state.stock_field_generations[9] = state.saved_generation;
                }
                for i in 0..8 {
                    if mask & (1 << i) != 0 {
                        state.stock_field_generations[i + 1] = state.saved_generation;
                    }
                }
            }
            state.desired_state = Some(assignment);
            if let Some(entry) = control
                .entries
                .get_mut(id)
                .filter(|_| !matches!(value, SavedModuleState::StockSeq3Fields { .. }))
            {
                entry.action_epoch = entry.action_epoch.wrapping_add(1).max(1);
                self.shared.action_epochs[entry.slot].store(entry.action_epoch, Ordering::Release);
            }
        }
    }

    /// Call only after the stream has stopped and its consumer has been dropped.
    pub(super) fn finish_close(&self) {
        let pending = self
            .shared
            .queued
            .load(Ordering::Acquire)
            .saturating_sub(self.shared.applied.load(Ordering::Acquire))
            .saturating_sub(self.shared.dropped.load(Ordering::Acquire));
        self.shared.dropped.fetch_add(pending, Ordering::Relaxed);
        self.shared
            .closed_dropped
            .fetch_add(pending, Ordering::Relaxed);
    }
}
impl LiveModuleEvents for Performance {
    fn learn_midi(
        &self,
        kind: openpatch_core::MidiLearnKind,
        id: &str,
        cell: u8,
    ) -> Result<Value, String> {
        self.arm_midi_learn(kind, id, cell)
    }
    fn cancel_midi_learn(
        &self,
        kind: openpatch_core::MidiLearnKind,
        id: &str,
    ) -> Result<Value, String> {
        self.cancel_learn_kind(kind, id)
    }
    fn midi_learn_ready(&self) -> Vec<openpatch::control::MidiLearnReceipt> {
        self.ready_midi_learns()
    }
    fn acknowledge_midi_learn(
        &self,
        receipt: &openpatch::control::MidiLearnReceipt,
    ) -> Result<(), String> {
        self.ack_midi_learn(receipt)
    }
    fn prepare_midi_learn_edit(&self, ids: &[String]) -> Result<(), String> {
        self.prepare_learn_edit(ids)
    }
    fn midi_learning_active(&self) -> bool {
        self.shared.midi_learns.iter().any(|m| {
            matches!(
                learn_phase(m.tag.load(Ordering::Acquire)),
                LEARN_ARMED | LEARN_PROCESSING | LEARN_READY
            )
        })
    }
    fn midi_learning_uncommitted(&self) -> bool {
        self.shared.midi_learns.iter().any(|m| {
            matches!(
                learn_phase(m.tag.load(Ordering::Acquire)),
                LEARN_PROCESSING | LEARN_READY
            )
        })
    }
    // One-way legacy adapters preserve existing clients and CC-only fake backends.
    fn learn_midi_cc(&self, id: &str, cell: u8) -> Result<Value, String> {
        self.learn_midi(openpatch_core::MidiLearnKind::Cc, id, cell)
    }
    fn cancel_midi_cc_learn(&self, id: &str) -> Result<Value, String> {
        self.cancel_midi_learn(openpatch_core::MidiLearnKind::Cc, id)
    }
    fn midi_cc_learn_ready(&self) -> Vec<openpatch::control::MidiCcLearnReceipt> {
        self.midi_learn_ready()
            .into_iter()
            .filter_map(|r| r.legacy_cc())
            .collect()
    }
    fn acknowledge_midi_cc_learn(
        &self,
        r: &openpatch::control::MidiCcLearnReceipt,
    ) -> Result<(), String> {
        let mut receipt: openpatch::control::MidiLearnReceipt = r.clone().into();
        receipt.backend = self.shared.backend;
        self.acknowledge_midi_learn(&receipt)
    }
    fn prepare_midi_cc_edit(&self, ids: &[String]) -> Result<(), String> {
        self.prepare_learn_edit_kind(ids, Some(openpatch_core::MidiLearnKind::Cc))
    }
    fn midi_cc_learning_active(&self) -> bool {
        self.learn_kind_active(openpatch_core::MidiLearnKind::Cc, false)
    }
    fn midi_cc_learning_uncommitted(&self) -> bool {
        self.learn_kind_active(openpatch_core::MidiLearnKind::Cc, true)
    }

    fn inspect_scope(&self, id: &str, channel: usize) -> Result<Value, String> {
        if channel >= 16 {
            return Err("Scope channel is outside 1..16".into());
        }
        let Some(frame) = self.scope_frame(id) else {
            return Ok(json!({"available":false,"source":"live_rack_scope","id":id}));
        };
        let s = &frame.snapshot;
        let measurements = |value: openpatch_core::ScopeMeasurements| {
            let extrema = |value: Option<openpatch_core::ScopeExtrema>| {
                value.map(|value| {
                    json!({"min":value.min,"max":value.max,
                    "peak_to_peak":value.peak_to_peak,"observations":value.observations})
                })
            };
            json!({"x":extrema(value.x),"y":extrema(value.y)})
        };
        let bins =
            |axis: &[[openpatch_core::ScopePoint; openpatch_core::SCOPE_BINS]; 16]| -> Vec<Value> {
                axis[channel]
                    .iter()
                    .map(|p| {
                        if p.min.is_finite() && p.max.is_finite() && p.min <= p.max {
                            json!({"min":p.min,"max":p.max})
                        } else {
                            Value::Null
                        }
                    })
                    .collect()
            };
        Ok(json!({"available":true,"source":"live_rack_scope","id":id,
            "revision":frame.revision,"instance":frame.token,"frame":frame.frame,"sequence":frame.sequence,
            "sample_rate":self.rate,"channel":channel+1,"x_channels":s.x_channels,"y_channels":s.y_channels,
            "x_active":channel<usize::from(s.x_channels),"y_active":channel<usize::from(s.y_channels),
            "buffer_index":s.buffer_index,"samples_per_bin":s.samples_per_bin,"sample_index":s.sample_index,
            "waiting":s.waiting,"xy":s.xy,"requested_seconds":s.time,
            "window_seconds":f64::from(s.samples_per_bin)*256.0/f64::from(self.rate),
            "x_scale":s.x_scale,"y_scale":s.y_scale,"x_offset":s.x_offset,"y_offset":s.y_offset,
            "trigger_enabled":s.trigger_enabled,"threshold":s.threshold,"units":"V",
            "trigger_input_channels":s.trigger_input_channels,
            "trigger_source":if s.trigger_input_channels > 0 {"external"} else {"x"},
            "display":{"x_volts_per_screen":20.0 / 2.0_f64.powi(i32::from(s.x_scale)),
                "y_volts_per_screen":20.0 / 2.0_f64.powi(i32::from(s.y_scale))},
            "measurements":{"basis":"completed_display_bins",
                "selected_voice":measurements(s.voice_measurements(channel)),
                "all_active_voices":measurements(s.measurements())},
            "publication":{"maximum_hz":30,"budget":"one Scope snapshot per device callback, round-robin; busy racks reduce refresh cadence"},
            "x":bins(&s.x),"y":bins(&s.y)}))
    }
    fn load_module_running(&self, states: &BTreeMap<String, bool>) {
        self.load_saved_states(
            &states
                .iter()
                .map(|(id, value)| (id.clone(), SavedModuleState::Seq3Run(*value)))
                .collect(),
        );
    }

    fn load_module_holding(&self, states: &BTreeMap<String, bool>) {
        self.load_saved_states(
            &states
                .iter()
                .map(|(id, value)| (id.clone(), SavedModuleState::PushHold(*value)))
                .collect(),
        );
    }

    fn load_module_saved_states(&self, states: &BTreeMap<String, SavedModuleState>) {
        self.load_saved_states(states);
    }

    fn release_module_controls(&self, ids: &[String]) {
        let mut control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        if self.shared.closed.load(Ordering::Acquire) {
            return;
        }
        let cancel_through_serial = control.next_learn_serial.saturating_sub(1);
        for id in ids {
            if let Some(entry) = control
                .entries
                .get_mut(id)
                .filter(|entry| entry.kind.manual())
            {
                self.request_release(entry, cancel_through_serial);
            }
        }
    }

    fn initialize_modules(&self, ids: &[String]) {
        let mut control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        if self.shared.closed.load(Ordering::Acquire) {
            return;
        }
        let revision = control.revision;
        let cancel_through_serial = control.next_learn_serial.saturating_sub(1);
        for id in ids {
            if let Some(cc) = control.midi_learns.get_mut(id) {
                if self.shared.midi_learns[cc.slot].cancel_armed().is_ok() {
                    cc.epoch = learn_epoch();
                }
            }
            if let Some(entry) = control.state_entries.get_mut(id) {
                entry.initialize_generation = entry.initialize_generation.saturating_add(1);
                entry.initialize_revision = revision;
            }
            if let Some(entry) = control.entries.get_mut(id) {
                // Cancel older queued actions without changing the DSP token.
                // A held momentary button must also be released in both sides
                // of a graph crossfade, even if initialization is coalesced.
                if entry.kind.manual() {
                    self.request_release(entry, cancel_through_serial);
                } else {
                    entry.action_epoch = entry.action_epoch.wrapping_add(1).max(1);
                    self.shared.action_epochs[entry.slot]
                        .store(entry.action_epoch, Ordering::Release);
                }
            }
        }
    }

    fn retire_modules(&self, ids: &[String]) {
        let mut control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        for id in ids {
            if let Some(cc) = control.midi_learns.get(id) {
                let _ = self.shared.midi_learns[cc.slot].cancel_armed();
            }
            control.state_entries.remove(id);
            control.scope_readers.remove(id);
            if let Some(entry) = control.entries.remove(id) {
                // The following sync assigns a fresh token even when the final
                // patch recreates an identical module in the same transaction.
                // The consumer rejects queued actions carrying the old token.
                self.shared.desired[entry.slot].store(0, Ordering::Release);
            }
        }
    }
    fn retire_cables(&self, ids: &[String]) {
        let mut control = self
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for id in ids {
            control.cable_entries.remove(id);
        }
    }
    fn sync_patch(&self, patch: &Patch, revision: u64) {
        let mut control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        if revision < control.revision || self.shared.closed.load(Ordering::Acquire) {
            return;
        }
        control.cable_entries.retain(|id, entry| {
            patch.cables.iter().any(|cable| {
                cable.id.as_ref() == Some(id) && route_signature(patch, cable) == entry.route
            })
        });
        let mut cable_used = [false; MAX_CABLES];
        for entry in control.cable_entries.values() {
            cable_used[entry.slot] = true;
        }
        for cable in &patch.cables {
            let Some(id) = cable.id.as_ref() else {
                continue;
            };
            if control.cable_entries.contains_key(id) {
                continue;
            }
            let Some(slot) = cable_used.iter().position(|used| !used) else {
                break;
            };
            cable_used[slot] = true;
            let token = control.next_token;
            control.next_token = control.next_token.wrapping_add(1).max(1);
            control.cable_entries.insert(
                id.clone(),
                CableEntry {
                    slot,
                    token,
                    born_revision: revision,
                    route: route_signature(patch, cable),
                },
            );
        }
        control.state_entries.retain(|id, entry| {
            patch
                .nodes
                .iter()
                .any(|node| node.id == *id && node.kind == entry.kind)
        });
        control.scope_readers.retain(|id, _| {
            patch
                .nodes
                .iter()
                .any(|node| node.id == *id && node.kind == "scope")
        });
        let mut state_used = [false; MAX_NODES];
        for entry in control.state_entries.values() {
            state_used[entry.slot] = true;
        }
        for node in &patch.nodes {
            if control.state_entries.contains_key(&node.id) {
                continue;
            }
            let Some(slot) = state_used.iter().position(|used| !used) else {
                break;
            };
            state_used[slot] = true;
            let token = control.next_token;
            control.next_token = control.next_token.wrapping_add(1).max(1);
            control.state_entries.insert(
                node.id.clone(),
                StateEntry {
                    slot,
                    token,
                    born_revision: revision,
                    kind: node.kind.clone(),
                    initialize_generation: 0,
                    initialize_revision: revision,
                    saved_generation: 0,
                    saved_revision: revision,
                    desired_state: SavedModuleState::from_node(node),
                    stock_field_generations: [0; 10],
                },
            );
        }
        control.entries.retain(|id, entry| {
            let retained = patch.version == 2
                && patch.nodes.iter().any(|node| {
                    node.id == *id && ModuleKind::from_name(&node.kind) == Some(entry.kind)
                });
            if !retained {
                self.shared.desired[entry.slot].store(0, Ordering::Release);
            }
            retained
        });
        let mut used = [false; MAX_NODES];
        for entry in control.entries.values() {
            used[entry.slot] = true;
        }
        let cancel_through_serial = control.next_learn_serial.saturating_sub(1);
        if patch.version == 2 {
            for (node, kind) in patch
                .nodes
                .iter()
                .filter_map(|node| ModuleKind::from_name(&node.kind).map(|kind| (node, kind)))
            {
                if let Some(entry) = control.entries.get_mut(&node.id) {
                    if node.bypassed && !entry.bypassed {
                        // Cancel pending actions without replacing DSP identity.
                        // This survives a bypass/resume coalesced before audio.
                        entry.action_epoch = entry.action_epoch.wrapping_add(1).max(1);
                        self.shared.action_epochs[entry.slot]
                            .store(entry.action_epoch, Ordering::Release);
                        if entry.kind.has_safety_release() {
                            self.shared.emergency_learn_cutoffs[entry.slot]
                                .store(cancel_through_serial, Ordering::Release);
                            self.shared.emergency_release_epochs[entry.slot]
                                .fetch_add(1, Ordering::Release);
                            self.shared
                                .emergency_release_requests
                                .fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    entry.bypassed = node.bypassed;
                    continue;
                }
                let Some(slot) = used.iter().position(|used| !used) else {
                    break;
                };
                used[slot] = true;
                let token = control.next_token;
                control.next_token = control.next_token.wrapping_add(1).max(1);
                control.entries.insert(
                    node.id.clone(),
                    Entry {
                        slot,
                        token,
                        born_revision: revision,
                        kind,
                        bypassed: node.bypassed,
                        action_epoch: 1,
                    },
                );
                self.shared.emergency_release_epochs[slot].store(0, Ordering::Release);
                self.shared.emergency_learn_cutoffs[slot].store(0, Ordering::Release);
                self.shared.action_epochs[slot].store(1, Ordering::Release);
                self.shared.desired[slot].store(token, Ordering::Release);
            }
        }
        let Control {
            entries,
            stock_click_releases,
            ..
        } = &mut *control;
        stock_click_releases.retain(|id, release| {
            entries
                .get(id)
                .is_some_and(|entry| entry.token == release.token)
        });
        self.sync_midi_learns(&mut control, patch, revision);
        control.revision = revision;
    }
    fn trigger(&self, id: &str, action: ModuleAction) -> Result<Value, String> {
        self.enqueue_guarded(id, action, None)
            .map(|(receipt, _)| receipt)
    }
    fn snapshot(&self) -> Value {
        let control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        let shared = &self.shared;
        let queued = shared.queued.load(Ordering::Acquire);
        let applied = shared.applied.load(Ordering::Acquire);
        let dropped = shared.dropped.load(Ordering::Acquire);
        let available = !shared.closed.load(Ordering::Acquire);
        let mut snapshots = vec![None; control.entries.len()];
        let mut coherent_frame = None;
        if available {
            for _ in 0..4 {
                let before = shared.publish_serial.load(Ordering::SeqCst);
                if before & 1 != 0 {
                    continue;
                }
                let frame = shared.frame.load(Ordering::SeqCst);
                for (snapshot, (_, entry)) in snapshots.iter_mut().zip(&control.entries) {
                    *snapshot = shared.meters[entry.slot]
                        .read(*entry, control.revision)
                        .filter(|actual| actual.frame == frame);
                }
                if shared.publish_serial.load(Ordering::SeqCst) == before {
                    coherent_frame = Some(frame);
                    break;
                }
            }
        }
        if coherent_frame.is_none() {
            snapshots.fill(None);
        }
        let modules: serde_json::Map<String, Value> = control.entries.iter().zip(snapshots).map(|((id, entry), actual)| {
            (id.clone(), json!({"kind":entry.kind.name(),"bypassed":entry.bypassed,"available":actual.is_some(),
                "revision":actual.map(|snapshot| snapshot.revision),"frame":actual.map(|snapshot|snapshot.frame),"state":actual.and_then(|snapshot| {
                    if let Some(SavedModuleState::StockQuantizerNotes(mask)) = snapshot.saved_state {
                        Some(json!({"scale_mask":mask,"enabled_notes":std::array::from_fn::<_,12,_>(|i|mask & (1<<i)!=0)}))
                    } else if let Some(SavedModuleState::StockSeq3 { running, gates, passthrough }) = snapshot.saved_state {
                        Some(json!({"running":running,"gates":std::array::from_fn::<_,8,_>(|i| gates & (1 << i) != 0),
                            "passthrough":passthrough,"step":snapshot.state.map(|s|s.step),"length":snapshot.state.map(|s|s.length)}))
                    } else if let Some(SavedModuleState::PushHold(holding)) = snapshot.saved_state {
                        Some(json!({"holding":holding}))
                    } else if let Some(SavedModuleState::RandomValues { values, rng }) = snapshot.saved_state {
                        let values = values.map(|row| row.map(f32::from_bits));
                        Some(json!({"values":values,"rng_state":rng,"active_channels":snapshot.lights.and_then(|lights| lights.input_channels).unwrap_or(1)}))
                    } else if let Some(SavedModuleState::MidiCc { assignments, values }) = snapshot.saved_state {
                        Some(json!({"assignments":assignments,"values":values.to_vec(),"learning":snapshot.midi_learning.map(|(serial,cell)|json!({"serial":serial,"cell":cell+1}))}))
                    } else if let Some(SavedModuleState::MidiGateMappings(assignments)) = snapshot.saved_state {
                        Some(json!({"assignments":assignments,"learning":snapshot.midi_learning.map(|(serial,cell)|json!({"serial":serial,"cell":cell+1}))}))
                    } else if let Some(SavedModuleState::ShiftRegister { values, rng }) = snapshot.saved_state {
                        Some(json!({"values":values.map(f32::from_bits),"rng_state":rng}))
                    } else {
                        snapshot.state.map(|state|json!({"step":state.step,"length":state.length,"running":state.running}))
                    }
                }),
                "controls":actual.and_then(|snapshot|snapshot.controls),"lights":actual.and_then(|snapshot|snapshot.lights).and_then(module_lights_value)}))
        }).collect();
        json!({"audio_timing":self.diagnostics.snapshot(),"available":available,"coherent":coherent_frame.is_some(),"frame":coherent_frame,"modules":modules,"revision":control.revision,"emergency_release_requests":shared.emergency_release_requests.load(Ordering::Relaxed),"emergency_release_applied":shared.emergency_release_applied.load(Ordering::Relaxed),"queued":queued,"applied":applied,"dropped":dropped,
            "pending":queued.saturating_sub(applied).saturating_sub(dropped),"backpressure":shared.backpressure.load(Ordering::Relaxed),
            "rejected":shared.rejected.load(Ordering::Relaxed),"expired":shared.expired.load(Ordering::Relaxed),"obsolete":shared.obsolete.load(Ordering::Relaxed),
            "closed_dropped":shared.closed_dropped.load(Ordering::Relaxed),"crossfade_rejected":shared.crossfade_rejected.load(Ordering::Relaxed),
            "last_applied_sequence":shared.last_applied_sequence.load(Ordering::Acquire),"future_timeout_seconds":FUTURE_SECONDS})
    }
}
impl EventReader {
    /// Return true when the next ordered event needs a new sample because the
    /// current target already has 64 core actions. Total budget spans callback.
    pub(super) fn apply(
        &mut self,
        active: (&mut Engine, &mut ModulePlan, u64),
        mut previous: Option<(&mut Engine, &mut ModulePlan)>,
        frame: u64,
        budget: &mut usize,
    ) -> bool {
        let (active, plan, revision) = active;
        self.release_pending(active, plan, true);
        if let Some((engine, plan)) = previous.as_mut() {
            self.release_pending(engine, plan, false);
        }
        let mut per_node = [0_u8; MAX_NODES];
        while *budget > 0 {
            if self.head.is_none() {
                self.head = self.queue.pop().ok();
            }
            let Some(event) = self.head else { break };
            if self.shared.closed.load(Ordering::Acquire) {
                self.head = None;
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                self.shared.closed_dropped.fetch_add(1, Ordering::Relaxed);
                *budget -= 1;
                continue;
            }
            if self.shared.desired[event.slot].load(Ordering::Acquire) != event.token {
                let shared = &self.shared;
                self.head = None;
                shared.dropped.fetch_add(1, Ordering::Relaxed);
                shared.obsolete.fetch_add(1, Ordering::Relaxed);
                *budget -= 1;
                continue;
            }
            if self.shared.action_epochs[event.slot].load(Ordering::Acquire) != event.action_epoch {
                self.head = None;
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                self.shared.rejected.fetch_add(1, Ordering::Relaxed);
                *budget -= 1;
                continue;
            }
            if frame > event.deadline {
                self.head = None;
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                self.shared.expired.fetch_add(1, Ordering::Relaxed);
                *budget -= 1;
                continue;
            }
            if revision < event.revision {
                break;
            }
            let binding = plan.bindings[event.slot].filter(|binding| binding.token == event.token);
            let Some(binding) = binding else {
                self.head = None;
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                self.shared.rejected.fetch_add(1, Ordering::Relaxed);
                *budget -= 1;
                continue;
            };
            // A producer may request a safety release while this callback is
            // draining. Observe it before accepting any post-epoch press.
            self.release_binding(active, &mut plan.bindings[event.slot], event.slot, true);
            if let Some((engine, old_plan)) = previous.as_mut() {
                self.release_binding(
                    engine,
                    &mut old_plan.bindings[event.slot],
                    event.slot,
                    false,
                );
            }
            if self.shared.action_epochs[event.slot].load(Ordering::Acquire) != event.action_epoch {
                self.head = None;
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                self.shared.rejected.fetch_add(1, Ordering::Relaxed);
                *budget -= 1;
                continue;
            }
            if per_node[event.slot] == 64 {
                return true;
            }
            per_node[event.slot] += 1;
            self.head = None;
            *budget -= 1;
            if event.action == ModuleAction::MidiPanic && binding.kind == ModuleKind::MidiGate {
                let _ = self.shared.midi_learns[event.slot]
                    .cancel_armed_through(event.cancel_through_serial);
            }
            if active.handle_module_event(binding.handle, event.action) {
                if let Some((engine, plan)) = previous.as_mut() {
                    if let Some(old) =
                        plan.bindings[event.slot].filter(|old| old.token == event.token)
                    {
                        if !engine.handle_module_event(old.handle, event.action) {
                            self.shared
                                .crossfade_rejected
                                .fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                self.shared.applied.fetch_add(1, Ordering::Relaxed);
                self.shared
                    .last_applied_sequence
                    .store(event.sequence, Ordering::Release);
            } else {
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                self.shared.rejected.fetch_add(1, Ordering::Relaxed);
            }
        }
        false
    }
    fn release_pending(&self, engine: &mut Engine, plan: &mut ModulePlan, active: bool) {
        for (slot, binding) in plan.bindings.iter_mut().enumerate() {
            self.release_binding(engine, binding, slot, active);
        }
    }
    fn release_binding(
        &self,
        engine: &mut Engine,
        binding: &mut Option<Binding>,
        slot: usize,
        active: bool,
    ) {
        let Some(binding) = binding else { return };
        if !binding.kind.has_safety_release()
            || self.shared.desired[slot].load(Ordering::Acquire) != binding.token
        {
            return;
        }
        let epoch = self.shared.emergency_release_epochs[slot].load(Ordering::Acquire);
        if binding.release_epoch != epoch && binding.kind == ModuleKind::MidiGate {
            let cutoff = self.shared.emergency_learn_cutoffs[slot].load(Ordering::Acquire);
            let _ = self.shared.midi_learns[slot].cancel_armed_through(cutoff);
        }
        if binding.release_epoch != epoch && engine.release_module_controls(binding.handle) {
            if active {
                self.shared.emergency_release_applied.fetch_add(
                    epoch.saturating_sub(binding.release_epoch),
                    Ordering::Relaxed,
                );
            }
            binding.release_epoch = epoch;
        }
    }
    pub(super) fn publish(&self, engine: &Engine, plan: &ModulePlan, revision: u64, frame: u64) {
        self.shared.publish_frame(frame, || {
            for (slot, binding) in plan.states.iter().enumerate() {
                if let Some(binding) = binding {
                    self.shared.saved_acknowledgments[slot]
                        .write(binding.token, binding.applied_saved);
                }
            }
            self.shared.sampled_sequence.store(
                self.shared.last_applied_sequence.load(Ordering::Acquire),
                Ordering::Release,
            );
            for (slot, binding) in plan.bindings.iter().enumerate() {
                if let Some(binding) = binding {
                    self.shared.meters[slot].write(
                        binding.token,
                        ModuleSnapshot {
                            revision,
                            frame,
                            state: if binding.kind.sequential() {
                                engine.module_telemetry(binding.handle)
                            } else {
                                None
                            },
                            saved_state: engine.module_saved_state(binding.handle),
                            controls: if binding.kind.manual() {
                                engine.module_control_state(binding.handle)
                            } else {
                                None
                            },
                            midi_learning: binding
                                .kind
                                .learn_kind()
                                .and_then(|kind| engine.midi_learning(binding.handle, kind)),
                            lights: if !binding.kind.sequential()
                                || binding.kind == ModuleKind::StockSeq3
                            {
                                engine.module_lights(binding.handle).map(|mut lights| {
                                    if binding.kind != ModuleKind::StockQuantizer {
                                        lights.playing_notes = None;
                                    }
                                    if binding.kind != ModuleKind::StockLfo {
                                        lights.clock_frequency = None;
                                    }
                                    if binding.kind != ModuleKind::Merge {
                                        lights.output_channels = None;
                                        lights.automatic_channels = None;
                                    }
                                    lights
                                })
                            } else {
                                None
                            },
                        },
                    );
                }
            }
        });
    }
}

include!("guarded_manual.rs");
include!("midi_learn_performance.rs");
#[cfg(test)]
mod midi_cc_performance_tests;

#[cfg(test)]
mod tests {
    use super::*;

    include!("split_performance_tests.rs");
    include!("stock_lfo_performance_tests.rs");
    include!("stock_vco_performance_tests.rs");
    include!("stock_seq3_performance_tests.rs");
    include!("stock_quantizer_performance_tests.rs");
    include!("stock_octave_performance_tests.rs");

    #[test]
    fn merge_count_packets_clear_absent_invalid_and_foreign_fields_independently() {
        assert!(ModuleKind::from_name("poly_merge") == Some(ModuleKind::Merge));
        assert_eq!(ModuleKind::Merge.name(), "poly_merge");
        assert!(!ModuleKind::Merge.manual());
        assert!(!ModuleKind::Merge.sequential());
        assert!(!ModuleKind::Merge.accepts(ModuleAction::BPress));
        let meter = Meter::new();
        let entry = Entry {
            slot: 0,
            token: 11,
            born_revision: 1,
            kind: ModuleKind::Merge,
            bypassed: false,
            action_epoch: 1,
        };
        let options = [Some(0), Some(16), None, Some(1), Some(17), Some(u8::MAX)];
        for output in options {
            for automatic in options {
                for present in [true, false] {
                    meter.write(
                        entry.token,
                        ModuleSnapshot {
                            revision: 7,
                            frame: 101,
                            state: None,
                            saved_state: None,
                            controls: None,
                            midi_learning: None,
                            lights: present.then_some(ModuleLightState {
                                count: 0,
                                output_channels: output,
                                automatic_channels: automatic,
                                ..ModuleLightState::default()
                            }),
                        },
                    );
                    let packet = meter.read(entry, 7).unwrap();
                    assert_eq!(packet.lights.is_some(), present);
                    let valid = |value: Option<u8>| value.filter(|value| *value <= 16);
                    assert_eq!(
                        packet.lights.and_then(|lights| lights.output_channels),
                        if present { valid(output) } else { None }
                    );
                    assert_eq!(
                        packet.lights.and_then(|lights| lights.automatic_channels),
                        if present { valid(automatic) } else { None }
                    );
                    for (slot, expected) in [
                        (&meter.output_channels, output),
                        (&meter.automatic_channels, automatic),
                    ] {
                        assert_eq!(
                            slot.load(Ordering::SeqCst),
                            if present {
                                valid(expected).map(u32::from).unwrap_or(u32::MAX)
                            } else {
                                u32::MAX
                            }
                        );
                    }
                    // A reused slot cannot expose Merge-only fields as another kind.
                    let foreign = meter
                        .read(
                            Entry {
                                kind: ModuleKind::Sum,
                                ..entry
                            },
                            7,
                        )
                        .unwrap();
                    assert!(foreign
                        .lights
                        .and_then(|lights| lights.output_channels)
                        .is_none());
                    assert!(foreign
                        .lights
                        .and_then(|lights| lights.automatic_channels)
                        .is_none());
                    assert!(meter.read(entry, 8).is_none());
                    assert!(meter.read(Entry { token: 12, ..entry }, 7).is_none());
                }
            }
        }
    }

    #[test]
    fn merge_count_pairs_never_mix_frame_revision_or_instance_generation() {
        let meter = Arc::new(Meter::new());
        let entry = Entry {
            slot: 0,
            token: 11,
            born_revision: 1,
            kind: ModuleKind::Merge,
            bypassed: false,
            action_epoch: 1,
        };
        let counts = |frame: u64| match frame % 6 {
            0 => (Some(0), Some(16)),
            1 => (Some(16), Some(0)),
            2 => (None, Some(7)),
            3 => (Some(4), None),
            4 => (Some(255), Some(0)),
            _ => (Some(0), Some(17)),
        };
        let writer_meter = meter.clone();
        let writer = std::thread::spawn(move || {
            for frame in 1..=2_000_u64 {
                let (output, automatic) = counts(frame);
                writer_meter.write(
                    11 + frame % 2,
                    ModuleSnapshot {
                        revision: frame + 1,
                        frame,
                        state: None,
                        saved_state: None,
                        controls: None,
                        midi_learning: None,
                        lights: (frame % 7 != 0).then_some(ModuleLightState {
                            count: 0,
                            output_channels: output,
                            automatic_channels: automatic,
                            ..ModuleLightState::default()
                        }),
                    },
                );
                std::thread::yield_now();
            }
        });
        let check = |packet: ModuleSnapshot, token: u64| {
            assert_eq!(packet.revision, packet.frame + 1);
            assert_eq!(token, 11 + packet.frame % 2);
            assert_eq!(packet.lights.is_some(), !packet.frame.is_multiple_of(7));
            if let Some(lights) = packet.lights {
                let (output, automatic) = counts(packet.frame);
                assert_eq!(lights.count, 0);
                assert_eq!(lights.output_channels, output.filter(|count| *count <= 16));
                assert_eq!(
                    lights.automatic_channels,
                    automatic.filter(|count| *count <= 16)
                );
            }
        };
        for _ in 0..2_000 {
            for token in [11, 12] {
                if let Some(packet) = meter.read(Entry { token, ..entry }, 1) {
                    check(packet, token);
                }
            }
            std::thread::yield_now();
        }
        writer.join().unwrap();
        check(meter.read(entry, 2_001).unwrap(), 11);
        assert!(meter.read(Entry { token: 12, ..entry }, 1).is_none());
        assert!(meter.read(entry, 2_002).is_none());
    }

    #[test]
    fn merge_host_snapshot_keeps_both_counts_in_the_same_module_batch() {
        let (performance, _reader, _timing) = Performance::new(48_000);
        {
            let mut control = performance.control.lock().unwrap();
            control.revision = 1;
            for slot in 0..2 {
                control.entries.insert(
                    format!("merge{slot}"),
                    Entry {
                        slot,
                        token: slot as u64 + 11,
                        born_revision: 1,
                        kind: ModuleKind::Merge,
                        bypassed: false,
                        action_epoch: 1,
                    },
                );
            }
        }
        let shared = performance.shared.clone();
        let writer = std::thread::spawn(move || {
            for frame in 1..=1_000_u64 {
                shared.publish_frame(frame, || {
                    for slot in 0..2 {
                        shared.meters[slot].write(
                            slot as u64 + 11,
                            ModuleSnapshot {
                                revision: frame + 1,
                                frame,
                                state: None,
                                saved_state: None,
                                controls: None,
                                midi_learning: None,
                                lights: Some(ModuleLightState {
                                    count: 0,
                                    output_channels: Some((frame % 17) as u8),
                                    automatic_channels: Some((16 - frame % 17) as u8),
                                    ..ModuleLightState::default()
                                }),
                            },
                        );
                        std::thread::yield_now();
                    }
                });
            }
        });
        let check = |snapshot: Value| {
            if snapshot["coherent"] == false {
                for id in ["merge0", "merge1"] {
                    assert_eq!(snapshot["modules"][id]["available"], false);
                    assert!(snapshot["modules"][id]["lights"].is_null());
                }
            } else if snapshot["modules"]["merge0"]["available"] == true {
                let frame = snapshot["frame"].as_u64().unwrap();
                for id in ["merge0", "merge1"] {
                    let module = &snapshot["modules"][id];
                    assert_eq!(module["available"], true);
                    assert_eq!(module["frame"], frame);
                    assert_eq!(module["revision"], frame + 1);
                    assert!(module["state"].is_null());
                    assert!(module["controls"].is_null());
                    assert_eq!(module["lights"]["count"], 0);
                    assert_eq!(module["lights"]["output_channels"], frame % 17);
                    assert_eq!(module["lights"]["automatic_channels"], 16 - frame % 17);
                }
            }
        };
        for _ in 0..1_000 {
            check(performance.snapshot());
            std::thread::yield_now();
        }
        writer.join().unwrap();
        check(performance.snapshot());
        assert_eq!(performance.snapshot()["frame"], 1_000);
    }

    #[test]
    fn wavetable_position_and_lights_share_a_coherent_packet_without_carryover() {
        assert!(ModuleKind::from_name("wavetable_vco") == Some(ModuleKind::WavetableVco));
        assert!(ModuleKind::from_name("wavetable_lfo") == Some(ModuleKind::WavetableLfo));
        assert_eq!(ModuleKind::WavetableVco.name(), "wavetable_vco");
        assert_eq!(ModuleKind::WavetableLfo.name(), "wavetable_lfo");

        let meter = Arc::new(Meter::new());
        let entry = Entry {
            slot: 0,
            token: 11,
            born_revision: 1,
            kind: ModuleKind::WavetableVco,
            bypassed: false,
            action_epoch: 1,
        };
        let writer_meter = meter.clone();
        let writer = std::thread::spawn(move || {
            for frame in 1..=2_000_u64 {
                let position = (frame % 257) as f32 / 256.0;
                let mut brightness = [[0.0; 2]; MAX_MODULE_LIGHTS];
                brightness[0] = [position, 1.0 - position];
                writer_meter.write(
                    entry.token,
                    ModuleSnapshot {
                        revision: 1,
                        frame,
                        state: None,
                        saved_state: None,
                        controls: None,
                        midi_learning: None,
                        lights: (frame % 3 != 0).then_some(ModuleLightState {
                            octave_shift: None,
                            clock_frequency: None,
                            playing_notes: None,
                            count: 5,
                            manual: false,
                            brightness,
                            input_channels: Some((frame % 17) as u8),
                            output_channels: None,
                            automatic_channels: None,
                            waveform_position: (frame % 3 == 1).then_some(position),
                        }),
                    },
                );
                std::thread::yield_now();
            }
        });
        let check_packet = |snapshot: ModuleSnapshot| {
            let frame = snapshot.frame;
            if frame.is_multiple_of(3) {
                assert!(snapshot.lights.is_none());
            } else {
                let lights = snapshot.lights.unwrap();
                let position = (frame % 257) as f32 / 256.0;
                assert_eq!(lights.count, 5);
                assert_eq!(lights.input_channels, Some((frame % 17) as u8));
                assert_eq!(lights.brightness[0], [position, 1.0 - position]);
                assert_eq!(
                    lights.waveform_position,
                    (frame % 3 == 1).then_some(position)
                );
            }
        };
        for _ in 0..2_000 {
            if let Some(snapshot) = meter.read(entry, 1) {
                check_packet(snapshot);
            }
            std::thread::yield_now();
        }
        writer.join().unwrap();
        let final_packet = meter.read(entry, 1).unwrap();
        assert_eq!(final_packet.frame, 2_000);
        check_packet(final_packet);

        // Deterministically exercise present -> absent and rejected values,
        // including a publication with no lights between two light packets.
        for position in [
            Some(0.0),
            Some(1.0),
            None,
            Some(f32::NAN),
            Some(f32::INFINITY),
            Some(-0.001),
            Some(1.001),
        ] {
            for lights_present in [false, true] {
                meter.write(
                    entry.token,
                    ModuleSnapshot {
                        revision: 1,
                        frame: 2_001,
                        state: None,
                        saved_state: None,
                        controls: None,
                        midi_learning: None,
                        lights: lights_present.then_some(ModuleLightState {
                            count: 5,
                            waveform_position: position,
                            ..ModuleLightState::default()
                        }),
                    },
                );
                let packet = meter.read(entry, 1).unwrap();
                assert_eq!(packet.lights.is_some(), lights_present);
                let expected =
                    position.filter(|value| value.is_finite() && (0.0..=1.0).contains(value));
                assert_eq!(
                    packet.lights.and_then(|lights| lights.waveform_position),
                    if lights_present { expected } else { None }
                );
            }
        }
    }

    #[test]
    fn multi_module_snapshot_never_exposes_a_partial_publish() {
        let (performance, _reader, _timing) = Performance::new(48_000);
        {
            let mut control = performance.control.lock().unwrap();
            control.revision = 1;
            for (slot, id) in ["first", "second"].into_iter().enumerate() {
                control.entries.insert(
                    id.into(),
                    Entry {
                        slot,
                        token: slot as u64 + 11,
                        born_revision: 1,
                        kind: if slot == 0 {
                            ModuleKind::Sequence
                        } else {
                            ModuleKind::Push
                        },
                        bypassed: false,
                        action_epoch: 1,
                    },
                );
            }
        }
        let shared = performance.shared.clone();
        shared.publish_serial.fetch_add(1, Ordering::SeqCst);
        let interrupted = performance.snapshot();
        assert_eq!(interrupted["coherent"], false);
        assert!(interrupted["frame"].is_null());
        for id in ["first", "second"] {
            assert_eq!(interrupted["modules"][id]["available"], false);
            assert!(interrupted["modules"][id]["state"].is_null());
        }
        shared.publish_serial.fetch_add(1, Ordering::SeqCst);

        let writer = std::thread::spawn(move || {
            for frame in 1..=2_000_u64 {
                shared.publish_frame(frame, || {
                    for slot in 0..2 {
                        shared.meters[slot].write(
                            slot as u64 + 11,
                            ModuleSnapshot {
                                midi_learning: None,
                                revision: 1,
                                frame,
                                state: (slot == 0).then_some(ModuleTelemetry {
                                    step: frame as u8,
                                    length: 8,
                                    running: true,
                                }),
                                saved_state: (slot == 1)
                                    .then_some(SavedModuleState::PushHold(frame & 1 != 0)),
                                controls: (slot == 1).then_some(ModuleControlState {
                                    pressed: true,
                                    pressed_mask: 3,
                                }),
                                lights: (slot == 1).then_some(ModuleLightState {
                                    count: 2,
                                    manual: true,
                                    input_channels: Some((frame % 17) as u8),
                                    ..ModuleLightState::default()
                                }),
                            },
                        );
                        std::thread::yield_now();
                    }
                });
            }
        });
        for _ in 0..2_000 {
            let snapshot = performance.snapshot();
            if snapshot["coherent"] == false {
                assert!(snapshot["frame"].is_null());
                assert_eq!(snapshot["modules"]["first"]["available"], false);
                assert_eq!(snapshot["modules"]["second"]["available"], false);
            } else if snapshot["modules"]["first"]["available"] == true {
                let frame = snapshot["frame"].as_u64().unwrap();
                for id in ["first", "second"] {
                    assert_eq!(snapshot["modules"][id]["available"], true);
                    assert_eq!(snapshot["modules"][id]["frame"], frame);
                }
                assert_eq!(snapshot["modules"]["first"]["state"]["step"], frame as u8);
                assert_eq!(
                    snapshot["modules"]["second"]["state"],
                    json!({"holding": frame & 1 != 0})
                );
                assert_eq!(snapshot["modules"]["second"]["controls"]["pressed_mask"], 3);
                assert_eq!(snapshot["modules"]["second"]["lights"]["count"], 2);
                assert_eq!(
                    snapshot["modules"]["second"]["lights"]["input_channels"],
                    frame % 17
                );
            }
        }
        writer.join().unwrap();
        let final_snapshot = performance.snapshot();
        assert_eq!(final_snapshot["coherent"], true);
        assert_eq!(final_snapshot["frame"], 2_000);
        for id in ["first", "second"] {
            assert_eq!(final_snapshot["modules"][id]["available"], true);
            assert_eq!(final_snapshot["modules"][id]["frame"], 2_000);
        }
    }
    #[test]
    fn random_values_host_snapshot_keeps_all_voltages_rng_and_channels_in_one_frame() {
        let (performance, _reader, _timing) = Performance::new(48_000);
        {
            let mut control = performance.control.lock().unwrap();
            control.revision = 1;
            for slot in 0..2 {
                control.entries.insert(
                    format!("random{slot}"),
                    Entry {
                        slot,
                        token: slot as u64 + 11,
                        born_revision: 1,
                        kind: ModuleKind::RandomValues,
                        bypassed: false,
                        action_epoch: 1,
                    },
                );
            }
        }
        let shared = performance.shared.clone();
        let writer = std::thread::spawn(move || {
            for frame in 1..=1_000_u64 {
                shared.publish_frame(frame, || {
                    for slot in 0..2 {
                        let voltage = (frame % 20) as f32 * 0.5;
                        shared.meters[slot].write(
                            slot as u64 + 11,
                            ModuleSnapshot {
                                midi_learning: None,
                                revision: 1,
                                frame,
                                state: None,
                                saved_state: Some(SavedModuleState::RandomValues {
                                    values: [[voltage.to_bits(); 16]; 7],
                                    rng: frame as u32,
                                }),
                                controls: None,
                                lights: Some(ModuleLightState {
                                    input_channels: Some((frame % 16 + 1) as u8),
                                    ..ModuleLightState::default()
                                }),
                            },
                        );
                        std::thread::yield_now();
                    }
                });
            }
        });
        for _ in 0..1_000 {
            let snapshot = performance.snapshot();
            if snapshot["coherent"] == false {
                for id in ["random0", "random1"] {
                    assert_eq!(snapshot["modules"][id]["available"], false);
                }
            } else if snapshot["modules"]["random0"]["available"] == true {
                let frame = snapshot["frame"].as_u64().unwrap();
                for id in ["random0", "random1"] {
                    let module = &snapshot["modules"][id];
                    assert_eq!(module["available"], true);
                    assert_eq!(module["frame"], frame);
                    assert_eq!(module["state"]["rng_state"], frame);
                    assert_eq!(module["state"]["active_channels"], frame % 16 + 1);
                    for row in module["state"]["values"].as_array().unwrap() {
                        for value in row.as_array().unwrap() {
                            assert_eq!(*value, json!((frame % 20) as f32 * 0.5));
                        }
                    }
                }
            }
        }
        writer.join().unwrap();
    }
    #[test]
    fn shift_register_host_snapshot_keeps_all_eight_voltages_and_rng_in_one_frame() {
        let (performance, _reader, _timing) = Performance::new(48_000);
        {
            let mut control = performance.control.lock().unwrap();
            control.revision = 1;
            for slot in 0..2 {
                control.entries.insert(
                    format!("random{slot}"),
                    Entry {
                        slot,
                        token: slot as u64 + 11,
                        born_revision: 1,
                        kind: ModuleKind::ShiftRegister,
                        bypassed: false,
                        action_epoch: 1,
                    },
                );
            }
        }
        let shared = performance.shared.clone();
        let writer = std::thread::spawn(move || {
            for frame in 1..=1_000_u64 {
                shared.publish_frame(frame, || {
                    for slot in 0..2 {
                        let voltage = (frame as f32 - 500.0) * 1_000.0;
                        shared.meters[slot].write(
                            slot as u64 + 11,
                            ModuleSnapshot {
                                midi_learning: None,
                                revision: 1,
                                frame,
                                state: None,
                                saved_state: Some(SavedModuleState::ShiftRegister {
                                    values: [voltage.to_bits(); 8],
                                    rng: frame as u32,
                                }),
                                controls: None,
                                lights: Some(ModuleLightState {
                                    input_channels: Some((frame % 16 + 1) as u8),
                                    ..ModuleLightState::default()
                                }),
                            },
                        );
                        std::thread::yield_now();
                    }
                });
            }
        });
        for _ in 0..1_000 {
            let snapshot = performance.snapshot();
            if snapshot["coherent"] == false {
                for id in ["random0", "random1"] {
                    assert_eq!(snapshot["modules"][id]["available"], false);
                }
            } else if snapshot["modules"]["random0"]["available"] == true {
                let frame = snapshot["frame"].as_u64().unwrap();
                for id in ["random0", "random1"] {
                    let module = &snapshot["modules"][id];
                    assert_eq!(module["available"], true);
                    assert_eq!(module["frame"], frame);
                    assert_eq!(module["state"]["rng_state"], frame);
                    assert!(module["state"]["active_channels"].is_null());
                    let values = module["state"]["values"].as_array().unwrap();
                    assert_eq!(values.len(), 8);
                    for value in values {
                        assert_eq!(*value, json!((frame as f32 - 500.0) * 1_000.0));
                    }
                }
            }
        }
        writer.join().unwrap();
    }
}
