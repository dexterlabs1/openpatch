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

/// Full-document execution request, distinct from an audio activation receipt.
/// In particular, a hosted request never permits executing the native carrier.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub enum DocumentExecutionRequest {
    Native(ExecutionDisposition),
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    HostedDelayed(crate::binary_host::execution::HostedExecutionGrant),
}

/// Borrowed control-thread view of the complete authored execution request.
pub struct DocumentExecutionContext<'a> {
    pub document: &'a ProjectDocument,
    pub document_generation: u64,
    pub request: DocumentExecutionRequest,
    pub native_carrier: &'a Patch,
    pub revision: u64,
    pub fence_native: bool,
}
impl DocumentExecutionContext<'_> {
    pub fn native_disposition(&self) -> ExecutionDisposition {
        match self.request {
            DocumentExecutionRequest::Native(disposition) => disposition,
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            DocumentExecutionRequest::HostedDelayed(_) => ExecutionDisposition::SuspendedUnresolved,
        }
    }
}

/// Session retains the same Arc until its exact id is acknowledged. A retry
/// cannot mix document, execution permission, ordered effects or wave banks.
#[doc(hidden)]
#[derive(Clone)]
pub struct HostLifecycleTicket {
    pub id: u64,
    pub revision: u64,
    pub document_generation: u64,
    pub document: Arc<ProjectDocument>,
    pub execution: DocumentExecutionRequest,
    pub(super) patch: Patch,
    pub banks: Arc<PreparedWaveBanks>,
    pub delta: HostLifecycleDelta,
}

impl HostLifecycleTicket {
    /// Native preparation is permitted only with the sealed native disposition.
    /// Hosted dispatch must consume the complete document and its own grant.
    pub fn native_projection(&self) -> Result<(&Patch, ExecutionDisposition), String> {
        match &self.execution {
            DocumentExecutionRequest::Native(disposition) => Ok((&self.patch, *disposition)),
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            DocumentExecutionRequest::HostedDelayed(_) => Err(
                "delayed hosted execution requires full-document audio preparation; native projection refused".into(),
            ),
        }
    }
}

#[derive(Default)]
pub(super) struct Publication {
    pub tail: HostLifecycleDelta,
    sealed: Option<Arc<HostLifecycleTicket>>,
    baseline_ids: BTreeSet<String>,
}
impl Publication {
    /// Replace a permission decision without pairing its old grant with a new
    /// document. Retain all unacknowledged lifecycle intent in original order.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    pub(super) fn supersede_execution(&mut self) {
        if let Some(ticket) = self.sealed.take() {
            let mut combined = ticket.delta.clone();
            combined.append(&self.tail);
            self.tail = combined;
            // A control-side submission may be between preparation and its
            // Session acknowledgment. Keep those IDs until a newer exact ack.
            self.baseline_ids
                .extend(ticket.patch.nodes.iter().map(|node| node.id.clone()));
        }
    }

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
fn next_ticket_id() -> Result<u64, String> {
    NEXT_TICKET
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| "audio lifecycle ticket ids exhausted".into())
}

impl Session {
    pub(super) fn document_execution_context(&self) -> DocumentExecutionContext<'_> {
        DocumentExecutionContext {
            document: &self.document,
            document_generation: self.project_save.generation(),
            request: self.document_execution_request(),
            native_carrier: &self.patch,
            revision: self.revision,
            fence_native: self.document_execution,
        }
    }
    pub(super) fn document_execution_request(&self) -> DocumentExecutionRequest {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        if let Some(grant) = &self.hosted_execution_grant {
            return DocumentExecutionRequest::HostedDelayed(grant.clone());
        }
        DocumentExecutionRequest::Native(self.execution_disposition())
    }

    #[doc(hidden)]
    pub fn audio_snapshot(&mut self) -> Result<Arc<HostLifecycleTicket>, String> {
        if let Some(ticket) = &self.host_lifecycle.sealed {
            return Ok(ticket.clone());
        }
        let id = next_ticket_id()?;
        let ticket = Arc::new(HostLifecycleTicket {
            id,
            revision: self.revision,
            document_generation: self.project_save.generation(),
            document: Arc::new(self.document.clone()),
            execution: self.document_execution_request(),
            patch: self.patch.clone(),
            banks: self.prepared_wave_banks.clone(),
            delta: std::mem::take(&mut self.host_lifecycle.tail),
        });
        self.host_lifecycle.sealed = Some(ticket.clone());
        Ok(ticket)
    }

    /// Capture preparation does not publish audio intent, take lifecycle edges,
    /// or leave a sealed publication that later edits would need to acknowledge.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    pub(super) fn capture_resume_snapshot(&self) -> Result<Arc<HostLifecycleTicket>, String> {
        Ok(Arc::new(HostLifecycleTicket {
            id: next_ticket_id()?,
            revision: self.revision,
            document_generation: self.project_save.generation(),
            document: Arc::new(self.document.clone()),
            execution: self.document_execution_request(),
            patch: self.patch.clone(),
            banks: self.prepared_wave_banks.clone(),
            delta: HostLifecycleDelta::default(),
        }))
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
