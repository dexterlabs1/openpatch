//! Shared precommit hosted-parameter intent for native UI, MCP and history.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub struct HostedParameterAssignment {
    pub module_id: String,
    pub parameter: u16,
    pub value: f32,
}

/// A backend may reserve bounded live capacity, but cannot edit this document.
pub struct HostedEditProposal<'a> {
    pub before: &'a ProjectDocument,
    pub after: &'a ProjectDocument,
    pub document_generation: u64,
    pub base_revision: u64,
    pub execution_request: DocumentExecutionRequest,
    pub assignments: &'a [HostedParameterAssignment],
}

/// Dropping a reservation cancels it. Publication happens only after the shared
/// document/history commit and must not fail or discard an accepted command.
pub trait HostedEditAdmission: Send {
    fn publish(self: Box<Self>, authored_revision: u64);
}

#[derive(Default)]
pub(super) struct ParameterEffects {
    pub before: Vec<HostedParameterAssignment>,
    pub after: Vec<HostedParameterAssignment>,
}

pub(super) fn same_coverage(
    a: &[HostedParameterAssignment],
    b: &[HostedParameterAssignment],
) -> bool {
    a.iter()
        .map(|p| (&p.module_id, p.parameter))
        .eq(b.iter().map(|p| (&p.module_id, p.parameter)))
}

fn module<'a>(
    document: &'a ProjectDocument,
    id: &str,
) -> Option<&'a crate::project_document::hosted::HostedModule> {
    document.graph.nodes.iter().find_map(|node| match node {
        DocumentNode::Hosted {
            id: node_id,
            module,
            ..
        } if node_id == id => Some(module),
        _ => None,
    })
}

fn key(parameter: u16) -> Result<&'static str, String> {
    match parameter {
        0 => Ok("offset"),
        1 => Ok("scale"),
        _ => Err("unsupported hosted parameter assignment".into()),
    }
}

pub(super) fn for_commit(
    before: &ProjectDocument,
    after: &ProjectDocument,
    effects: &EditEffects,
) -> Result<ParameterEffects, String> {
    let mut assignments = effects.hosted_parameters.clone();
    // A document load retires the old executor. Its restored state belongs to a
    // fresh plan, not live events addressed to the old child incarnation.
    if effects.next_document_generation.is_none() && effects.retired.is_empty() {
        for node in &after.graph.nodes {
            let DocumentNode::Hosted {
                id, module: next, ..
            } = node
            else {
                continue;
            };
            let Some(old) = module(before, id) else {
                continue;
            };
            if old.validate_supported_offset().is_err() || next.validate_supported_offset().is_err()
            {
                continue;
            }
            for parameter in 0..=1 {
                let name = key(parameter)?;
                let value = next.parameter(name)?;
                if old.parameter(name)?.to_bits() != value.to_bits()
                    && !assignments
                        .iter()
                        .any(|p| p.module_id == *id && p.parameter == parameter)
                {
                    assignments.push(HostedParameterAssignment {
                        module_id: id.clone(),
                        parameter,
                        value,
                    });
                }
            }
        }
    }
    let mut previous = Vec::with_capacity(assignments.len());
    for assignment in &assignments {
        let name = key(assignment.parameter)?;
        let old = module(before, &assignment.module_id)
            .ok_or("hosted assignment has no previous incarnation")?;
        let next = module(after, &assignment.module_id)
            .ok_or("hosted assignment has no target incarnation")?;
        if next.parameter(name)?.to_bits() != assignment.value.to_bits() {
            return Err("hosted assignment differs from committed document".into());
        }
        previous.push(HostedParameterAssignment {
            module_id: assignment.module_id.clone(),
            parameter: assignment.parameter,
            value: old.parameter(name)?,
        });
    }
    Ok(ParameterEffects {
        before: previous,
        after: assignments,
    })
}

impl Session {
    pub(super) fn reserve_hosted_edit(
        &self,
        after: &ProjectDocument,
        assignments: &[HostedParameterAssignment],
        retired: bool,
    ) -> Result<Option<Box<dyn HostedEditAdmission>>, String> {
        if assignments.is_empty() || retired {
            return Ok(None);
        }
        let Some(backend) = &self.live_module_events else {
            return Ok(None);
        };
        backend.reserve_hosted_parameters(HostedEditProposal {
            before: &self.document,
            after,
            document_generation: self.project_save.generation(),
            base_revision: self.revision,
            execution_request: self.document_execution_request(),
            assignments,
        })
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
impl HostedEditProposal<'_> {
    pub fn reserve_with_control(
        self,
        control: &Arc<crate::binary_host::control::HostedControl>,
    ) -> Result<Box<dyn HostedEditAdmission>, String> {
        use crate::binary_host::control::{HostedReservation, ParameterAssignment};
        struct Admission(HostedReservation);
        impl HostedEditAdmission for Admission {
            fn publish(self: Box<Self>, revision: u64) {
                self.0.publish(revision);
            }
        }
        let DocumentExecutionRequest::HostedDelayed(grant) = self.execution_request else {
            return Err("active hosted edits require the current full-document grant".into());
        };
        if grant.document_generation() != self.document_generation
            || !grant.matches_document(self.before, self.document_generation)?
            || !grant.matches_document(self.after, self.document_generation)?
            || !control.accepts_document(self.before, self.document_generation)?
            || !control.accepts_document(self.after, self.document_generation)?
        {
            return Err("active hosted edit does not match its document and execution plan".into());
        }
        if control
            .snapshot()
            .queued
            .is_some_and(|receipt| receipt.authored_revision > self.base_revision)
        {
            return Err("active hosted control belongs to a newer authored revision".into());
        }
        let assignments = self
            .assignments
            .iter()
            .map(|edit| {
                if edit.module_id != control.module_id() {
                    return Err("assignment targets another hosted incarnation".to_owned());
                }
                Ok(ParameterAssignment {
                    parameter: edit.parameter,
                    value: edit.value,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Box::new(Admission(control.reserve(&assignments)?)))
    }
}
