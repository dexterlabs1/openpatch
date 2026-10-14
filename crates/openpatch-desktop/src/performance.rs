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
    action: ModuleAction,
    sequence: u64,
    revision: u64,
    deadline: u64,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModuleKind {
    Sequence,
    Switch1To4,
    Switch4To1,
    Logic,
    Compare,
    Process,
    Rescale,
    VcaMix,
    Sum,
    Viz,
    Pulses,
    Push,
    Gates,
}
impl ModuleKind {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "seq3" => Some(Self::Sequence),
            "switch_1to4" => Some(Self::Switch1To4),
            "switch_4to1" => Some(Self::Switch4To1),
            "logic" => Some(Self::Logic),
            "compare" => Some(Self::Compare),
            "process" => Some(Self::Process),
            "rescale" => Some(Self::Rescale),
            "vca_mix" => Some(Self::VcaMix),
            "poly_sum" => Some(Self::Sum),
            "poly_viz" => Some(Self::Viz),
            "pulses" => Some(Self::Pulses),
            "push" => Some(Self::Push),
            "gates" => Some(Self::Gates),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Sequence => "seq3",
            Self::Switch1To4 => "switch_1to4",
            Self::Switch4To1 => "switch_4to1",
            Self::Logic => "logic",
            Self::Compare => "compare",
            Self::Process => "process",
            Self::Rescale => "rescale",
            Self::VcaMix => "vca_mix",
            Self::Sum => "poly_sum",
            Self::Viz => "poly_viz",
            Self::Pulses => "pulses",
            Self::Push => "push",
            Self::Gates => "gates",
        }
    }
    fn manual(self) -> bool {
        matches!(
            self,
            Self::Logic | Self::Process | Self::Pulses | Self::Push | Self::Gates
        )
    }
    fn sequential(self) -> bool {
        matches!(self, Self::Sequence | Self::Switch1To4 | Self::Switch4To1)
    }
    fn accepts(self, action: ModuleAction) -> bool {
        match self {
            Self::Sequence => matches!(
                action,
                ModuleAction::Clock | ModuleAction::Run | ModuleAction::Reset
            ),
            Self::Logic => matches!(action, ModuleAction::BPress | ModuleAction::BRelease),
            Self::Process => matches!(action, ModuleAction::GatePress | ModuleAction::GateRelease),
            Self::Pulses => action.pulse_button().is_some(),
            Self::Gates => matches!(
                action,
                ModuleAction::ResetPress | ModuleAction::ResetRelease
            ),
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
}
#[derive(Clone, Copy)]
struct ModuleSnapshot {
    revision: u64,
    frame: u64,
    state: Option<ModuleTelemetry>,
    saved_state: Option<SavedModuleState>,
    controls: Option<ModuleControlState>,
    lights: Option<ModuleLightState>,
}
struct Meter {
    serial: AtomicU64,
    token: AtomicU64,
    revision: AtomicU64,
    frame: AtomicU64,
    value: AtomicU32,
    flags: AtomicU32,
    saved_state: AtomicU32,
    input_channels: AtomicU32,
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
            input_channels: AtomicU32::new(u32::MAX),
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
                Some(SavedModuleState::Seq3Run(value)) => 1 | (u32::from(value) << 2),
                Some(SavedModuleState::PushHold(value)) => 2 | (u32::from(value) << 2),
                None => 0,
            },
            Ordering::SeqCst,
        );
        if let Some(controls) = snapshot.controls {
            flags |=
                2 | (u32::from(controls.pressed) << 3) | (u32::from(controls.pressed_mask) << 16);
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
            let input_channels = self.input_channels.load(Ordering::SeqCst);
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
                    saved_state: match saved_state & 3 {
                        1 => Some(SavedModuleState::Seq3Run(saved_state & 4 != 0)),
                        2 => Some(SavedModuleState::PushHold(saved_state & 4 != 0)),
                        _ => None,
                    },
                    controls: (flags & 2 != 0).then_some(ModuleControlState {
                        pressed: flags & 8 != 0,
                        pressed_mask: (flags >> 16) as u16,
                    }),
                    lights: (flags & 4 != 0).then_some(ModuleLightState {
                        count: (flags >> 8) as u8,
                        brightness,
                        manual: flags & 16 != 0,
                        input_channels: (input_channels <= 16).then_some(input_channels as u8),
                    }),
                },
            );
        }
        None
    }
}
struct Shared {
    desired: [AtomicU64; MAX_NODES],
    action_epochs: [AtomicU64; MAX_NODES],
    emergency_release_epochs: [AtomicU64; MAX_NODES],
    emergency_release_requests: AtomicU64,
    emergency_release_applied: AtomicU64,
    meters: [Meter; MAX_NODES],
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
    scopes: Vec<ScopeBinding>,
    scope_cursor: usize,
    scope_interval: u64,
    pub bindings: [Option<Binding>; MAX_NODES],
    pub states: [Option<StateBinding>; MAX_NODES],
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
            scope_cursor: 0,
            scope_interval: 1,
            bindings: [None; MAX_NODES],
            states: [None; MAX_NODES],
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

    /// Exclusions are fixed stack storage; no control registry reaches audio.
    pub(super) fn adopt_state(&mut self, engine: &mut Engine, previous: &Self, old: &Engine) {
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
            engine.adopt_state_from_excluding_routes(old, &[], &[]);
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
            engine.adopt_state_from_excluding_routes(
                old,
                &modules[..module_count],
                &cables[..cable_count],
            );
        } else {
            engine.adopt_state_from_excluding_routes(old, &modules[..module_count], &[]);
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
            emergency_release_requests: AtomicU64::new(0),
            emergency_release_applied: AtomicU64::new(0),
            meters: std::array::from_fn(|_| Meter::new()),
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
                } else if entry.kind.manual() {
                    engine.resolve_module_control(id)?
                } else {
                    engine.resolve_module_lights(id)?
                },
                kind: entry.kind,
                release_epoch: 0,
            });
        }
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
    fn request_release(&self, entry: &mut Entry) {
        entry.action_epoch = entry.action_epoch.wrapping_add(1).max(1);
        self.shared.action_epochs[entry.slot].store(entry.action_epoch, Ordering::Release);
        self.shared.emergency_release_epochs[entry.slot].fetch_add(1, Ordering::Release);
        self.shared
            .emergency_release_requests
            .fetch_add(1, Ordering::Relaxed);
    }
    pub(super) fn close(&self) {
        // Serialize against producer acceptance. This mutex never reaches audio.
        let _control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        self.shared.closed.store(true, Ordering::Release);
        self.diagnostics.close();
    }
    fn load_saved_states(&self, states: &BTreeMap<String, bool>, kind: SavedModuleState) {
        let mut control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        if self.shared.closed.load(Ordering::Acquire) {
            return;
        }
        let revision = control.revision;
        for (id, value) in states {
            let Some(state) = control.state_entries.get_mut(id).filter(|state| {
                state
                    .desired_state
                    .is_some_and(|desired| desired.kind() == kind.kind())
            }) else {
                continue;
            };
            state.saved_generation = state.saved_generation.saturating_add(1);
            state.saved_revision = revision;
            state.desired_state = Some(kind.with_value(*value));
            if let Some(entry) = control.entries.get_mut(id) {
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
    fn inspect_scope(&self, id: &str, channel: usize) -> Result<Value, String> {
        if channel >= 16 {
            return Err("Scope channel is outside 1..16".into());
        }
        let Some(frame) = self.scope_frame(id) else {
            return Ok(json!({"available":false,"source":"live_rack_scope","id":id}));
        };
        let s = &frame.snapshot;
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
            "publication":{"maximum_hz":30,"budget":"one Scope snapshot per device callback, round-robin; busy racks reduce refresh cadence"},
            "x":bins(&s.x),"y":bins(&s.y)}))
    }
    fn load_module_running(&self, states: &BTreeMap<String, bool>) {
        self.load_saved_states(states, SavedModuleState::Seq3Run(false));
    }

    fn load_module_holding(&self, states: &BTreeMap<String, bool>) {
        self.load_saved_states(states, SavedModuleState::PushHold(false));
    }

    fn release_module_controls(&self, ids: &[String]) {
        let mut control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        if self.shared.closed.load(Ordering::Acquire) {
            return;
        }
        for id in ids {
            if let Some(entry) = control
                .entries
                .get_mut(id)
                .filter(|entry| entry.kind.manual())
            {
                self.request_release(entry);
            }
        }
    }

    fn initialize_modules(&self, ids: &[String]) {
        let mut control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        if self.shared.closed.load(Ordering::Acquire) {
            return;
        }
        let revision = control.revision;
        for id in ids {
            if let Some(entry) = control.state_entries.get_mut(id) {
                entry.initialize_generation = entry.initialize_generation.saturating_add(1);
                entry.initialize_revision = revision;
            }
            if let Some(entry) = control.entries.get_mut(id) {
                // Cancel older queued actions without changing the DSP token.
                // A held momentary button must also be released in both sides
                // of a graph crossfade, even if initialization is coalesced.
                if entry.kind.manual() {
                    self.request_release(entry);
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
                        if entry.kind.manual() {
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
                self.shared.action_epochs[slot].store(1, Ordering::Release);
                self.shared.desired[slot].store(token, Ordering::Release);
            }
        }
        control.revision = revision;
    }
    fn trigger(&self, id: &str, action: ModuleAction) -> Result<Value, String> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| "Module event registry poisoned")?;
        if self.shared.closed.load(Ordering::Acquire) {
            self.shared.rejected.fetch_add(1, Ordering::Relaxed);
            return Err("Audio device closed; module action was not queued".into());
        }
        let Some(entry) = control.entries.get(id).copied() else {
            self.shared.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(format!("Module '{id}' has no live sequencer instance"));
        };
        if !entry.kind.accepts(action) {
            self.shared.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(format!(
                "Module '{id}' ({}) has read-only telemetry or does not support this module action",
                entry.kind.name()
            ));
        }
        if entry.bypassed && !is_release(action) {
            self.shared.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(format!(
                "Module '{id}' is bypassed; module action was not queued"
            ));
        }
        let sequence = control.next_sequence;
        control.next_sequence = control.next_sequence.wrapping_add(1).max(1);
        let revision = control.revision;
        let event = Event {
            slot: entry.slot,
            token: entry.token,
            action_epoch: entry.action_epoch,
            action,
            sequence,
            revision,
            deadline: self
                .shared
                .frame
                .load(Ordering::Acquire)
                .saturating_add(u64::from(self.rate) * FUTURE_SECONDS),
        };
        // Count acceptance before publishing so the consumer cannot report an
        // applied event before its queued counter. A rejected push rolls back.
        self.shared.queued.fetch_add(1, Ordering::Release);
        if control.queue.push(event).is_err() {
            self.shared.queued.fetch_sub(1, Ordering::Release);
            self.shared.backpressure.fetch_add(1, Ordering::Relaxed);
            if is_release(action) {
                self.request_release(control.entries.get_mut(id).unwrap());
                return Ok(
                    json!({"queued":false,"applied":false,"safety_pending":true,"emergency_release":true,"sequence":sequence,
                    "id":id,"action":action,"minimum_revision":revision}),
                );
            }
            return Err("Module action queue is full; action was not queued".into());
        }
        Ok(
            json!({"queued":true,"applied":false,"sequence":sequence,"id":id,"action":action,"minimum_revision":revision,"future_timeout_seconds":FUTURE_SECONDS}),
        )
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
                    if let Some(SavedModuleState::PushHold(holding)) = snapshot.saved_state {
                        Some(json!({"holding":holding}))
                    } else {
                        snapshot.state.map(|state|json!({"step":state.step,"length":state.length,"running":state.running}))
                    }
                }),
                "controls":actual.and_then(|snapshot|snapshot.controls),"lights":actual.and_then(|snapshot|snapshot.lights)}))
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
        if !binding.kind.manual()
            || self.shared.desired[slot].load(Ordering::Acquire) != binding.token
        {
            return;
        }
        let epoch = self.shared.emergency_release_epochs[slot].load(Ordering::Acquire);
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
                            lights: if !binding.kind.sequential() {
                                engine.module_lights(binding.handle)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
