//! UI-thread child ownership. Only prepared graph parts cross into audio.
//!
//! This controller intentionally inherits `HostProcess`'s !Send/!Sync property.
//! All launch, health observation, transport draining and capture happen here.

use openpatch::binary_host::control::HostedControl;
use openpatch::binary_host::process::{HostProcess, LaunchConfig};
use openpatch::binary_host::runner::HostedRunner;
use openpatch::binary_host::{Fence, OutputBlock, OwnedEndpoint, Status};
use openpatch::control::{DocumentExecutionRequest, HostLifecycleTicket};
use openpatch_core::Engine;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const OWNER_LIMIT: usize = 16;
const RECEIVE_BUDGET: usize = 4;
const RETIRE_TIMEOUT: Duration = Duration::from_secs(2);
static NEXT_EXECUTION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub(super) struct HostedAudioConfig {
    pub(super) companion_executable: PathBuf,
    pub(super) sdk_dir: PathBuf,
    pub(super) plugin_dir: PathBuf,
    pub(super) workspace_parent: PathBuf,
}

impl HostedAudioConfig {
    /// Explicit paths only: no discovery, import or execution side effects.
    pub(super) fn from_environment() -> Result<Self, String> {
        fn required(name: &str) -> Result<PathBuf, String> {
            std::env::var_os(name)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| format!("hosted audio requires explicit {name}"))
        }
        Ok(Self {
            companion_executable: required("OPENPATCH_HOSTED_COMPANION")?,
            sdk_dir: required("OPENPATCH_HOSTED_SDK")?,
            plugin_dir: required("OPENPATCH_HOSTED_PLUGIN")?,
            workspace_parent: std::env::var_os("OPENPATCH_HOSTED_WORKSPACE")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir),
        })
    }
}

pub(super) struct HostedPreparation {
    pub(super) engine: Engine,
    pub(super) runner: HostedRunner,
    pub(super) control: Arc<HostedControl>,
    pub(super) fence: Fence,
    pub(super) proof: openpatch::binary_host::graph::HostedCompilationProof,
}

#[derive(Debug)]
pub(super) struct HostedRetirementReceipt {
    pub(super) fence: Fence,
    pub(super) captured: Option<Value>,
    pub(super) error: Option<String>,
}

impl HostedRetirementReceipt {
    fn failed(fence: Fence, error: impl Into<String>) -> Self {
        Self {
            fence,
            captured: None,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug)]
pub(super) struct HostedOwnerStatus {
    pub(super) fence: Fence,
    pub(super) process_id: u32,
    pub(super) retiring: bool,
    pub(super) health_error: Option<String>,
}

struct Owner {
    process: HostProcess,
    control: Arc<HostedControl>,
    health_error: Option<String>,
}

impl Owner {
    fn fence(&self) -> Fence {
        self.process.fence()
    }

    fn observe_health(&mut self) -> Option<String> {
        if self.health_error.is_some() {
            return None;
        }
        let error = match self.process.poll_health() {
            Ok(Status::Ready) => return None,
            Ok(status) => format!("hosted companion left READY: {status:?}"),
            Err(error) => error.to_string(),
        };
        self.control.close();
        self.health_error = Some(error.clone());
        Some(error)
    }

    fn status(&self, retiring: bool) -> HostedOwnerStatus {
        HostedOwnerStatus {
            fence: self.fence(),
            process_id: self.process.id(),
            retiring,
            health_error: self.health_error.clone(),
        }
    }
}

struct Retirement {
    owner: Owner,
    endpoint: OwnedEndpoint,
    deadline: Instant,
}

#[derive(Default)]
pub(super) struct HostedAudioController {
    config: Option<HostedAudioConfig>,
    owners: Vec<Owner>,
    retirements: Vec<Retirement>,
}

impl HostedAudioController {
    pub(super) fn abort_capture(&mut self, fence: Fence) {
        if let Some(index) = self.owners.iter().position(|owner| owner.fence() == fence) {
            let owner = self.owners.swap_remove(index);
            owner.control.close();
            drop(owner);
        }
    }

    pub(super) fn finish_capture(
        &mut self,
        capture: openpatch::binary_host::runner::QuiescedHostedRunner,
    ) -> Result<
        (
            openpatch::binary_host::runner::HostedCaptureAcknowledgment,
            Value,
        ),
        String,
    > {
        let fence = capture.seal().fence();
        let index = self
            .owners
            .iter()
            .position(|owner| owner.fence() == fence)
            .ok_or("capture endpoint has no matching process owner")?;
        let mut owner = self.owners.swap_remove(index);
        if let Some(error) = owner
            .observe_health()
            .or_else(|| owner.health_error.clone())
        {
            drop(capture);
            drop(owner);
            return Err(error);
        }
        let (endpoint, acknowledgment) = capture.finish()?;
        let captured = owner
            .process
            .finish(endpoint)
            .map_err(|error| error.to_string())?;
        if captured.fence != fence {
            return Err("stopped child capture has a different execution fence".into());
        }
        Ok((acknowledgment, captured.state))
    }
    pub(super) fn set_config(&mut self, config: HostedAudioConfig) {
        self.config = Some(config);
    }

    pub(super) fn ensure_config(&mut self) -> Result<(), String> {
        if self.config.is_none() {
            self.set_config(HostedAudioConfig::from_environment()?);
        }
        Ok(())
    }

    pub(super) fn prepare(
        &mut self,
        ticket: &HostLifecycleTicket,
    ) -> Result<HostedPreparation, String> {
        let DocumentExecutionRequest::HostedDelayed(grant) = &ticket.execution else {
            return Err("hosted preparation requires the full delayed execution grant".into());
        };
        if ticket.document_generation == 0 {
            return Err("hosted preparation requires a nonzero document generation".into());
        }
        let config = self
            .config
            .as_ref()
            .ok_or("hosted audio has no explicit launch configuration")?;
        // Validate the full grant/document pair and compile before launching.
        let plan = grant.prepare(&ticket.document, ticket.document_generation)?;
        let (engine, handle, proof) = plan.compile_attested(&ticket.banks)?;
        if self.owner_count() >= OWNER_LIMIT {
            return Err("hosted audio owner/retirement limit reached".into());
        }
        let fence = Fence {
            incarnation: next_id(&NEXT_EXECUTION_ID)?,
            generation: ticket.document_generation,
            // This is an execution plan epoch, never an authored revision.
            revision: next_id(&NEXT_EXECUTION_ID)?,
        };
        let (control, reader) =
            HostedControl::new(grant.clone(), &ticket.document, fence, ticket.revision)?;
        let module = plan.module();
        let prepared = LaunchConfig {
            companion_executable: config.companion_executable.clone(),
            sdk_dir: config.sdk_dir.clone(),
            plugin_dir: config.plugin_dir.clone(),
            workspace_parent: config.workspace_parent.clone(),
            expected_plugin_sha256: module.identity.plugin_sha256.clone(),
            expected_runtime_sha256: module.identity.runtime_sha256.clone(),
            fence,
            input_mask: plan.input_mask(),
            snapshot: module.state.as_ref().clone(),
            startup_timeout: Duration::from_secs(5),
        }
        .launch()
        .map_err(|error| format!("hosted companion preparation failed: {error}"))?;
        // On runner rejection the local process owner stops/reaps automatically.
        let runner = HostedRunner::prepare_with_control(prepared.endpoint, handle, reader)?;
        self.owners.push(Owner {
            process: prepared.process,
            control: Arc::clone(&control),
            health_error: None,
        });
        Ok(HostedPreparation {
            engine,
            runner,
            control,
            fence,
            proof,
        })
    }

    /// Consume a runner only after it has left every callback/queued graph path.
    /// Immediate failures return a receipt; successful admission finishes in service.
    pub(super) fn retire(&mut self, runner: HostedRunner) -> Option<HostedRetirementReceipt> {
        let (endpoint, tail, awaiting_inputs, fault) = runner.retire();
        let fence = endpoint.fence();
        let Some(index) = self
            .owners
            .iter()
            .position(|owner| owner.fence().incarnation == fence.incarnation)
        else {
            return Some(HostedRetirementReceipt::failed(
                fence,
                "returned hosted endpoint has no live process owner",
            ));
        };
        if self.owners[index].fence() != fence {
            // A stale endpoint must not remove a different generation/epoch.
            return Some(HostedRetirementReceipt::failed(
                fence,
                "returned hosted endpoint has a mismatched execution fence",
            ));
        }
        let mut owner = self.owners.swap_remove(index);
        owner.control.close();
        owner.observe_health();
        let error = retirement_error(tail, awaiting_inputs, fault.is_some())
            .map(str::to_owned)
            .or_else(|| owner.health_error.clone())
            .or_else(|| {
                (owner.control.snapshot().outstanding_commands != 0)
                    .then(|| "unacknowledged hosted edits prevent persistent capture".into())
            })
            .or_else(|| {
                endpoint
                    .fault()
                    .map(|fault| format!("hosted endpoint fault: {fault:?}"))
            });
        if let Some(error) = error {
            // Dropping the owner stops/reaps; no state is read or published.
            return Some(HostedRetirementReceipt::failed(fence, error));
        }
        self.retirements.push(Retirement {
            owner,
            endpoint,
            deadline: Instant::now() + RETIRE_TIMEOUT,
        });
        None
    }

    /// Bounded nonblocking polling/draining; full capture itself is control work.
    pub(super) fn service(&mut self) -> Vec<HostedRetirementReceipt> {
        let mut receipts = Vec::new();
        for owner in &mut self.owners {
            if let Some(error) = owner.observe_health() {
                // Retain process/workspace until its exact runner is returned.
                receipts.push(HostedRetirementReceipt::failed(owner.fence(), error));
            }
        }
        let mut budget = RECEIVE_BUDGET;
        let mut index = 0;
        // One scratch output per service call, reused across bounded jobs.
        let mut output = OutputBlock::default();
        while index < self.retirements.len() {
            let job = &mut self.retirements[index];
            let mut error = job.owner.observe_health();
            while error.is_none() && !job.endpoint.is_drained() && budget > 0 {
                budget -= 1;
                match job.endpoint.try_receive(&mut output) {
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(fault) => {
                        error = Some(format!("hosted retirement drain failed: {fault:?}"))
                    }
                }
            }
            if error.is_none() && Instant::now() >= job.deadline {
                error = Some("hosted retirement drain timed out".into());
            }
            if error.is_none() && !job.endpoint.is_drained() {
                index += 1;
                continue;
            }
            let job = self.retirements.swap_remove(index);
            let fence = job.owner.fence();
            if let Some(error) = error {
                receipts.push(HostedRetirementReceipt::failed(fence, error));
                continue;
            }
            // finish independently verifies exact mapping identity, all fences,
            // healthy drain, processed acknowledgments and successful child exit.
            let receipt = match job.owner.process.finish(job.endpoint) {
                Ok(captured) if captured.fence == fence => HostedRetirementReceipt {
                    fence,
                    captured: Some(captured.state),
                    error: None,
                },
                Ok(_) => HostedRetirementReceipt::failed(fence, "hosted capture fence mismatch"),
                Err(error) => HostedRetirementReceipt::failed(
                    fence,
                    format!("hosted state capture failed: {error}"),
                ),
            };
            receipts.push(receipt);
        }
        receipts
    }

    /// Includes retiring jobs: a returned endpoint does not free a child slot.
    pub(super) fn owner_count(&self) -> usize {
        self.owners.len() + self.retirements.len()
    }

    pub(super) fn retiring_count(&self) -> usize {
        self.retirements.len()
    }

    pub(super) fn statuses(&self) -> Vec<HostedOwnerStatus> {
        self.owners
            .iter()
            .map(|owner| owner.status(false))
            .chain(self.retirements.iter().map(|job| job.owner.status(true)))
            .collect()
    }

    /// Caller must stop the stream and drop/drain all graph paths first.
    /// Shutdown cleanup never manufactures successful saved-state receipts.
    pub(super) fn shutdown(&mut self) {
        for owner in &self.owners {
            owner.control.close();
        }
        for job in &self.retirements {
            job.owner.control.close();
        }
        self.owners.clear();
        self.retirements.clear();
    }
}

impl Drop for HostedAudioController {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn next_id(counter: &AtomicU64) -> Result<u64, String> {
    counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            (current != 0).then(|| current.checked_add(1)).flatten()
        })
        .map_err(|_| "hosted execution incarnation/plan epochs exhausted".into())
}

fn retirement_error(tail: usize, awaiting_inputs: bool, faulted: bool) -> Option<&'static str> {
    if faulted {
        Some("faulted hosted runner cannot capture persistent state")
    } else if awaiting_inputs {
        Some("incomplete hosted frame cannot capture persistent state")
    } else if tail != 0 {
        Some("partial hosted quantum cannot capture persistent state")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incarnation_and_plan_epoch_never_reuse_or_wrap() {
        let counter = AtomicU64::new(1);
        assert_eq!(next_id(&counter).unwrap(), 1);
        assert_eq!(next_id(&counter).unwrap(), 2);
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(next_id(&counter).unwrap(), u64::MAX - 1);
        assert!(next_id(&counter).is_err());
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
        assert!(next_id(&AtomicU64::new(0)).is_err());
    }

    #[test]
    fn partial_frame_tail_and_fault_never_admit_capture() {
        assert!(retirement_error(0, false, false).is_none());
        assert!(retirement_error(1, false, false).is_some());
        assert!(retirement_error(0, true, false).is_some());
        assert!(retirement_error(0, false, true).is_some());
    }

    #[test]
    fn unconfigured_controller_has_no_process_or_retirement_side_effects() {
        let mut controller = HostedAudioController::default();
        assert_eq!(controller.owner_count(), 0);
        assert_eq!(controller.retiring_count(), 0);
        assert!(controller.statuses().is_empty());
        assert!(controller.service().is_empty());
        controller.shutdown();
        assert_eq!(controller.owner_count(), 0);
    }
}
