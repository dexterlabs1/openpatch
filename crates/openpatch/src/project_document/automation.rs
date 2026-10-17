//! Imported automation is safe by document invariant, not by filtering saved
//! state or guessing a destination from its current native ID.
use super::{DocumentNode, ProjectDocument, SourceModule};
use openpatch_core::rackmcp_sequence::NativeTarget;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationOmissionReason {
    UnboundLane,
    MissingTarget,
    UnresolvedTarget,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct OmittedAutomation {
    pub sequencer_id: String,
    pub source_module_id: String,
    pub source_param_id: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_module: Option<String>,
    pub reason: AutomationOmissionReason,
}

impl ProjectDocument {
    /// Inspect every imported lane. A missing destination is dormant; an
    /// existing native destination must have the precise registered provenance.
    /// Callers must reject errors rather than execute a merely diagnosed graph.
    /// Pure native sequences have no source authority to validate here.
    pub fn automation_omissions(&self) -> Result<Vec<OmittedAutomation>, String> {
        let nodes: BTreeMap<_, _> = self
            .graph
            .nodes
            .iter()
            .map(|node| (node.id(), node))
            .collect();
        let mut omissions = Vec::new();
        for entry in &self.graph.nodes {
            let DocumentNode::Native {
                node,
                source: Some(source),
            } = entry
            else {
                continue;
            };
            if node.kind != "rackmcp_sequencer" {
                continue;
            }
            let Some(state) = &node.rackmcp_sequence else {
                continue;
            };
            for target in &state.targets {
                if let Some(DocumentNode::Native {
                    node: destination,
                    source: destination_source,
                }) = nodes.get(target.module.as_str()).copied()
                {
                    self.validate_automation_binding(
                        &node.id,
                        source,
                        target,
                        destination,
                        destination_source.as_ref(),
                    )?;
                }
            }
            for lane in &state.data.lanes {
                let binding = state.targets.iter().find(|target| {
                    target.source_module_id == lane.module_id
                        && target.source_param_id == lane.param_id
                });
                let reason = match binding.and_then(|target| nodes.get(target.module.as_str())) {
                    None if binding.is_none() => AutomationOmissionReason::UnboundLane,
                    None => AutomationOmissionReason::MissingTarget,
                    Some(DocumentNode::Unresolved { .. } | DocumentNode::Hosted { .. }) => {
                        AutomationOmissionReason::UnresolvedTarget
                    }
                    Some(DocumentNode::Native { .. }) => continue,
                };
                omissions.push(OmittedAutomation {
                    sequencer_id: node.id.clone(),
                    source_module_id: lane.module_id.clone(),
                    source_param_id: lane.param_id,
                    native_module: binding.map(|target| target.module.clone()),
                    reason,
                });
            }
        }
        Ok(omissions)
    }

    fn validate_automation_binding(
        &self,
        sequencer_id: &str,
        source: &SourceModule,
        target: &NativeTarget,
        destination: &openpatch_core::Node,
        destination_source: Option<&SourceModule>,
    ) -> Result<(), String> {
        let invalid = |reason| {
            format!(
                "imported automation {sequencer_id} -> {}.{}: {reason}; clear the binding or use a qualified source target",
                target.module, target.parameter
            )
        };
        let original = self.source_module(source)?;
        let expected = crate::vcv_import::registered_source_mapping(original)
            .ok_or_else(|| invalid("unregistered sequencer source"))?;
        let mapping = source
            .mapping
            .as_ref()
            .ok_or_else(|| invalid("missing sequencer mapping"))?;
        if original["plugin"] != "RackMCP"
            || original["model"] != "Sequencer"
            || mapping.id != expected.id
            || mapping.version != expected.version
            || mapping.fidelity != expected.fidelity
        {
            return Err(invalid(
                "sequencer source mapping does not match its registered adapter",
            ));
        }
        let destination_source = destination_source
            .filter(|destination| destination.package == source.package)
            .ok_or_else(|| invalid("target belongs to a different source identity"))?;
        let original_target = self.source_module(destination_source)?;
        if !original_target["id"]
            .as_u64()
            .is_some_and(|id| target.source_module_id.parse::<u64>().ok() == Some(id))
        {
            return Err(invalid("target original module ID does not match the lane"));
        }
        let parameter = destination_source.mapping.as_ref().and_then(|mapping| {
            crate::vcv_import::source_automation_parameter(
                original_target,
                mapping,
                &destination.kind,
                target.source_param_id,
            )
        });
        if parameter != Some(target.parameter.as_str()) {
            return Err(invalid(
                "target source parameter has no matching registered native control",
            ));
        }
        Ok(())
    }
}
