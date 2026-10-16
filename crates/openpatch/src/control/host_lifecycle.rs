//! Ordered control-thread port intent, independent of final graph coalescing.
use super::*;
use openpatch_core::{InputLifecycleTransform, OutputLifecycleTransform};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// Named transforms are resolved to fixed Core ports before audio publication.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostModuleLifecycleDelta {
    pub reset: bool,
    pub inputs: BTreeMap<String, InputLifecycleTransform>,
    pub outputs: BTreeMap<String, OutputLifecycleTransform>,
}

#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostLifecycleDelta {
    pub modules: BTreeMap<String, HostModuleLifecycleDelta>,
}
impl HostLifecycleDelta {
    pub fn is_empty(&self) -> bool {
        self.modules
            .values()
            .all(|module| !module.reset && module.inputs.is_empty() && module.outputs.is_empty())
    }

    /// Compose later intent after this intent. A retirement fence erases all
    /// earlier port operations for the old incarnation, never later operations.
    pub fn append(&mut self, later: &Self) {
        for (id, change) in &later.modules {
            let current = self.modules.entry(id.clone()).or_default();
            if change.reset {
                *current = HostModuleLifecycleDelta {
                    reset: true,
                    ..Default::default()
                };
            }
            for (port, transform) in &change.inputs {
                let value = current.inputs.entry(port.clone()).or_default();
                *value = value.then(*transform);
            }
            for (port, transform) in &change.outputs {
                let value = current.outputs.entry(port.clone()).or_default();
                *value = value.then(*transform);
            }
        }
    }

    pub fn then(mut self, later: Self) -> Self {
        self.append(&later);
        self
    }
}

/// Session retains the same Arc until its exact id is acknowledged. A compile
/// retry therefore cannot mix an old delta with a newer patch or wave bank.
#[doc(hidden)]
#[derive(Clone)]
pub struct HostLifecycleTicket {
    pub id: u64,
    pub revision: u64,
    pub patch: Patch,
    pub banks: Arc<PreparedWaveBanks>,
    pub delta: HostLifecycleDelta,
}

#[derive(Default)]
pub(super) struct Publication {
    pub tail: HostLifecycleDelta,
    sealed: Option<Arc<HostLifecycleTicket>>,
    baseline_ids: BTreeSet<String>,
}
impl Publication {
    pub fn at_patch(patch: &Patch) -> Self {
        Self {
            baseline_ids: patch.nodes.iter().map(|node| node.id.clone()).collect(),
            ..Default::default()
        }
    }

    pub fn prune(&mut self, patch: &Patch) {
        let mut live = self.baseline_ids.clone();
        live.extend(patch.nodes.iter().map(|node| node.id.clone()));
        if let Some(ticket) = &self.sealed {
            live.extend(ticket.patch.nodes.iter().map(|node| node.id.clone()));
        }
        // A transient never-published incarnation cannot be adopted. If its ID
        // is recreated later, creation itself installs a fresh reset fence.
        self.tail.modules.retain(|id, _| live.contains(id));
    }
}
static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);

impl Session {
    #[doc(hidden)]
    pub fn audio_snapshot(&mut self) -> Result<Arc<HostLifecycleTicket>, String> {
        if let Some(ticket) = &self.host_lifecycle.sealed {
            return Ok(ticket.clone());
        }
        let mut id = NEXT_TICKET.load(Ordering::Relaxed);
        loop {
            let next = id
                .checked_add(1)
                .ok_or("audio lifecycle ticket ids exhausted")?;
            match NEXT_TICKET.compare_exchange_weak(id, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => break,
                Err(current) => id = current,
            }
        }
        let ticket = Arc::new(HostLifecycleTicket {
            id,
            revision: self.revision,
            patch: self.patch.clone(),
            banks: self.prepared_wave_banks.clone(),
            delta: std::mem::take(&mut self.host_lifecycle.tail),
        });
        self.host_lifecycle.sealed = Some(ticket.clone());
        Ok(ticket)
    }

    #[doc(hidden)]
    pub fn acknowledge_audio_snapshot(&mut self, id: u64) -> bool {
        if self
            .host_lifecycle
            .sealed
            .as_ref()
            .is_some_and(|ticket| ticket.id == id)
        {
            let ticket = self.host_lifecycle.sealed.take().unwrap();
            self.host_lifecycle.baseline_ids = ticket
                .patch
                .nodes
                .iter()
                .map(|node| node.id.clone())
                .collect();
            self.host_lifecycle.prune(&self.patch);
            true
        } else {
            false
        }
    }

    /// Only for a new audio engine compiled from this exact current patch while
    /// holding the same Session lock. Ordinary retries must acknowledge tickets.
    #[doc(hidden)]
    pub fn discard_audio_lifecycle_through_current(&mut self) {
        self.host_lifecycle = Publication::at_patch(&self.patch);
    }
}

#[derive(Clone, Default)]
struct ModuleTopology {
    kind: String,
    bypassed: bool,
    inputs: BTreeMap<String, usize>,
    outputs: BTreeMap<String, usize>,
}

/// No parameters, assets, note text or complete Patch clones per graph edit.
pub(super) struct Topology {
    rate: u32,
    version: u32,
    modules: BTreeMap<String, ModuleTopology>,
}
type PortNames = (BTreeSet<String>, BTreeSet<String>);
fn known_ports() -> &'static BTreeMap<String, PortNames> {
    static PORTS: OnceLock<BTreeMap<String, PortNames>> = OnceLock::new();
    PORTS.get_or_init(|| {
        let catalog = catalog();
        catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|module| {
                let names = |field: &str| {
                    module[field]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                };
                (
                    module["kind"].as_str().unwrap().to_owned(),
                    (names("inputs"), names("outputs")),
                )
            })
            .collect()
    })
}

impl Topology {
    pub fn capture(patch: &Patch) -> Self {
        let mut modules: BTreeMap<_, _> = patch
            .nodes
            .iter()
            .map(|node| {
                (
                    node.id.clone(),
                    ModuleTopology {
                        kind: node.kind.clone(),
                        bypassed: node.bypassed,
                        ..Default::default()
                    },
                )
            })
            .collect();
        let ports = known_ports();
        for cable in &patch.cables {
            let valid_from = modules
                .get(&cable.from)
                .and_then(|m| ports.get(&m.kind))
                .is_some_and(|(_, outputs)| outputs.contains(&cable.output));
            let valid_to = modules
                .get(&cable.to)
                .and_then(|m| ports.get(&m.kind))
                .is_some_and(|(inputs, _)| inputs.contains(&cable.input));
            // Invalid transient routes have no actual host port. A later edit
            // making both endpoints valid will capture the real connection.
            if !valid_from || !valid_to {
                continue;
            }
            if let Some(module) = modules.get_mut(&cable.to) {
                *module.inputs.entry(cable.input.clone()).or_default() += 1;
            }
            if let Some(module) = modules.get_mut(&cable.from) {
                *module.outputs.entry(cable.output.clone()).or_default() += 1;
            }
        }
        if let Some(id) = &patch.output {
            if let Some(module) = modules.get_mut(id) {
                if ports
                    .get(&module.kind)
                    .is_some_and(|(_, outputs)| outputs.contains(&patch.output_port))
                {
                    *module.outputs.entry(patch.output_port.clone()).or_default() += 1;
                }
            }
        }
        Self {
            rate: patch.sample_rate,
            version: patch.version,
            modules,
        }
    }

    pub fn transition(&self, after: &Self, retired: &[String]) -> HostLifecycleDelta {
        let mut delta = HostLifecycleDelta::default();
        let ids: BTreeSet<_> = self.modules.keys().chain(after.modules.keys()).collect();
        for id in ids {
            let old = self.modules.get(id);
            let new = after.modules.get(id);
            let reset = old.is_none()
                || new.is_none()
                || old.zip(new).is_some_and(|(old, new)| old.kind != new.kind)
                || self.rate != after.rate
                || self.version != after.version
                || retired.iter().any(|retired| retired == id);
            let mut change = HostModuleLifecycleDelta {
                reset,
                ..Default::default()
            };
            let empty = ModuleTopology::default();
            // A reset fences off the old incarnation. New connections are
            // applied to fresh state; deleted modules retain only the fence.
            let before = if reset { &empty } else { old.unwrap_or(&empty) };
            let next = new.unwrap_or(&empty);
            let inputs: BTreeSet<_> = before.inputs.keys().chain(next.inputs.keys()).collect();
            for port in inputs {
                let was = before.inputs.get(port).copied().unwrap_or(0) > 0;
                let now = next.inputs.get(port).copied().unwrap_or(0) > 0;
                if was != now {
                    change.inputs.insert(
                        port.clone(),
                        if now {
                            InputLifecycleTransform::connect()
                        } else {
                            InputLifecycleTransform::disconnect()
                        },
                    );
                }
            }
            let outputs: BTreeSet<_> = before.outputs.keys().chain(next.outputs.keys()).collect();
            for port in outputs {
                let was = before.outputs.get(port).copied().unwrap_or(0) > 0;
                let now = next.outputs.get(port).copied().unwrap_or(0) > 0;
                if was != now {
                    change.outputs.insert(
                        port.clone(),
                        if now {
                            OutputLifecycleTransform::connect()
                        } else {
                            OutputLifecycleTransform::disconnect()
                        },
                    );
                }
            }
            if before.bypassed != next.bypassed {
                // Disconnected output.setChannels(0) is identity. Consumer names
                // suffice; no catalog lookup or fabricated port list is needed.
                for port in next.outputs.keys() {
                    let transform = change.outputs.entry(port.clone()).or_default();
                    *transform = transform.then(OutputLifecycleTransform::bypass_edge());
                }
            }
            if change.reset || !change.inputs.is_empty() || !change.outputs.is_empty() {
                delta.modules.insert(id.clone(), change);
            }
        }
        delta
    }
}

pub(super) fn may_change_topology(name: &str) -> bool {
    matches!(
        name,
        "connect"
            | "disconnect"
            | "disconnect_cables"
            | "move_cables"
            | "remove_module"
            | "add_module"
            | "clear_cables"
            | "duplicate_modules"
            | "set_bypass"
            | "set_output"
            | "apply_module_preset"
            | "initialize_modules"
            | "randomize_modules"
    )
}
