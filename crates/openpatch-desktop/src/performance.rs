//! Control-thread telemetry registry and bounded sequencer performance events. The only
//! producer is behind a control-only mutex; the audio consumer never locks.
use openpatch::control::LiveModuleEvents;
use openpatch_core::{
    Cable, CableHandle, Engine, ModuleAction, ModuleControlState, ModuleHandle, ModuleLightState,
    ModuleTelemetry, Patch, MAX_CABLES, MAX_NODES,
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
        }
    }
    fn manual(self) -> bool {
        matches!(self, Self::Logic | Self::Process)
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
            _ => false,
        }
    }
}
fn is_release(action: ModuleAction) -> bool {
    matches!(action, ModuleAction::BRelease | ModuleAction::GateRelease)
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
    revision: u64,
    next_token: u64,
    next_sequence: u64,
}
#[derive(Clone, Copy)]
struct ModuleSnapshot {
    revision: u64,
    state: Option<ModuleTelemetry>,
    controls: Option<ModuleControlState>,
    lights: Option<ModuleLightState>,
}
struct Meter {
    serial: AtomicU64,
    token: AtomicU64,
    revision: AtomicU64,
    value: AtomicU32,
    flags: AtomicU32,
    brightness: [AtomicU32; 16],
}
impl Meter {
    fn new() -> Self {
        Self {
            serial: AtomicU64::new(0),
            token: AtomicU64::new(0),
            revision: AtomicU64::new(0),
            value: AtomicU32::new(0),
            flags: AtomicU32::new(0),
            brightness: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }
    fn write(&self, token: u64, snapshot: ModuleSnapshot) {
        self.serial.fetch_add(1, Ordering::SeqCst);
        self.token.store(token, Ordering::SeqCst);
        self.revision.store(snapshot.revision, Ordering::SeqCst);
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
        if let Some(controls) = snapshot.controls {
            flags |= 2 | (u32::from(controls.pressed) << 3);
        }
        if let Some(lights) = snapshot.lights {
            flags |= 4 | (u32::from(lights.manual) << 4) | (u32::from(lights.count) << 8);
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
            let value = self.value.load(Ordering::SeqCst);
            let flags = self.flags.load(Ordering::SeqCst);
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
                    state: (flags & 1 != 0).then_some(ModuleTelemetry {
                        step: value as u8,
                        length: (value >> 8) as u8,
                        running: value & (1 << 16) != 0,
                    }),
                    controls: (flags & 2 != 0).then_some(ModuleControlState {
                        pressed: flags & 8 != 0,
                    }),
                    lights: (flags & 4 != 0).then_some(ModuleLightState {
                        count: (flags >> 8) as u8,
                        brightness,
                        manual: flags & 16 != 0,
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
pub(super) struct Performance {
    control: Mutex<Control>,
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
}
#[derive(Clone, Copy, Debug)]
pub(super) struct CableBinding {
    pub token: u64,
    pub handle: CableHandle,
}
pub(super) struct ModulePlan {
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
    /// Exclusions are fixed stack storage; no control registry reaches audio.
    pub(super) fn adopt_state(&self, engine: &mut Engine, previous: &Self, old: &Engine) {
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
    pub(super) fn new(rate: u32) -> (Arc<Self>, EventReader) {
        let (queue, input) = RingBuffer::new(EVENT_CAPACITY);
        let shared = Arc::new(Shared {
            desired: std::array::from_fn(|_| AtomicU64::new(0)),
            action_epochs: std::array::from_fn(|_| AtomicU64::new(0)),
            emergency_release_epochs: std::array::from_fn(|_| AtomicU64::new(0)),
            emergency_release_requests: AtomicU64::new(0),
            emergency_release_applied: AtomicU64::new(0),
            meters: std::array::from_fn(|_| Meter::new()),
            closed: AtomicBool::new(false),
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
                    revision: 0,
                    next_token: 1,
                    next_sequence: 1,
                }),
                shared,
                rate,
            }),
            reader,
        )
    }
    pub(super) fn plan(
        &self,
        patch: &Patch,
        engine: &Engine,
        revision: u64,
    ) -> Result<ModulePlan, String> {
        let control = self
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
        Ok(plan)
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
    fn retire_modules(&self, ids: &[String]) {
        let mut control = self.control.lock().unwrap_or_else(|e| e.into_inner());
        for id in ids {
            control.state_entries.remove(id);
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
        let modules: serde_json::Map<String, Value> = control.entries.iter().map(|(id, entry)| {
            let actual = available.then(|| shared.meters[entry.slot].read(*entry, control.revision)).flatten();
            (id.clone(), json!({"kind":entry.kind.name(),"bypassed":entry.bypassed,"available":actual.is_some(),
                "revision":actual.map(|snapshot| snapshot.revision),"state":actual.and_then(|snapshot|snapshot.state).map(|state|json!({"step":state.step,"length":state.length,"running":state.running})),
                "controls":actual.and_then(|snapshot|snapshot.controls),"lights":actual.and_then(|snapshot|snapshot.lights)}))
        }).collect();
        json!({"available":available,"modules":modules,"revision":control.revision,"emergency_release_requests":shared.emergency_release_requests.load(Ordering::Relaxed),"emergency_release_applied":shared.emergency_release_applied.load(Ordering::Relaxed),"queued":queued,"applied":applied,"dropped":dropped,
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
        self.shared.frame.store(frame, Ordering::Release);
        for (slot, binding) in plan.bindings.iter().enumerate() {
            if let Some(binding) = binding {
                self.shared.meters[slot].write(
                    binding.token,
                    ModuleSnapshot {
                        revision,
                        state: if binding.kind.sequential() {
                            engine.module_telemetry(binding.handle)
                        } else {
                            None
                        },
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
    }
}
