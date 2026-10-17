//! Two-phase save authority for a full, coherently frozen hosted document.
//! Tickets and receipts deliberately cannot be deserialized or edit the Session.
use super::*;
use crate::binary_host::runner::HostedCaptureAcknowledgment;
use crate::project_document::hosted::{HostedModule, HostedState};

#[derive(Clone)]
pub struct HostedProjectCaptureTicket {
    identity: Arc<()>,
    document: Arc<ProjectDocument>,
    document_generation: u64,
    revision: u64,
    state_id: u64,
    layout_generation: u64,
    module_id: String,
    native_patch: Patch,
    backend: Arc<dyn LiveModuleEvents>,
    audio_ticket: Arc<HostLifecycleTicket>,
}

impl HostedProjectCaptureTicket {
    pub fn document(&self) -> &Arc<ProjectDocument> {
        &self.document
    }
    pub fn document_generation(&self) -> u64 {
        self.document_generation
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn state_id(&self) -> u64 {
        self.state_id
    }
    pub fn layout_generation(&self) -> u64 {
        self.layout_generation
    }
    pub fn module_id(&self) -> &str {
        &self.module_id
    }
    pub fn native_patch(&self) -> &Patch {
        &self.native_patch
    }
    pub fn audio_ticket(&self) -> Arc<HostLifecycleTicket> {
        self.audio_ticket.clone()
    }
}

pub struct HostedProjectCaptureReceipt {
    ticket_identity: Arc<()>,
    native_patch: Patch,
    hosted_module: HostedModule,
    device_frame: u64,
}

impl HostedProjectCaptureReceipt {
    /// The runner acknowledgment is issued only after the exact endpoint and
    /// retained control reader have fully drained. The caller must additionally
    /// bind the child's successful finish result to that same execution fence.
    pub fn new(
        ticket: &HostedProjectCaptureTicket,
        acknowledgment: HostedCaptureAcknowledgment,
        native_patch: Patch,
        hosted_state: Value,
        device_frame: u64,
    ) -> Result<Self, String> {
        let seal = acknowledgment.seal();
        if seal.fence().generation != ticket.document_generation
            || seal.authored_revision() != ticket.revision
            || acknowledgment.applied_sequence() != seal.accepted_sequence()
        {
            return Err(
                "hosted capture acknowledgment does not match the sealed document cut".into(),
            );
        }
        validate_native_projection(&ticket.native_patch, &native_patch)?;
        let hosted_module = validate_hosted_state(ticket, hosted_state)?;
        Ok(Self {
            ticket_identity: ticket.identity.clone(),
            native_patch,
            hosted_module,
            device_frame,
        })
    }
}

impl Session {
    /// Seal the authored document without changing history or its saved baseline.
    #[doc(hidden)]
    pub fn begin_hosted_project_capture(
        &mut self,
        expected_revision: Option<u64>,
    ) -> Result<HostedProjectCaptureTicket, String> {
        self.check_open()?;
        self.check_revision(expected_revision)?;
        self.check_midi_cc_capture()?;
        if self.layout_pending {
            return Err(
                "host layout placement is pending; retry hosted capture after placement".into(),
            );
        }
        let backend = self
            .live_module_events
            .as_ref()
            .filter(|backend| backend.requires_hosted_capture_barrier())
            .ok_or("hosted capture requires an attached live hosted backend")?
            .clone();
        let document_generation = self.project_save.generation();
        let grant = self
            .hosted_execution_grant
            .as_ref()
            .ok_or("hosted capture requires explicit delayed hosted execution permission")?;
        if !grant.matches_document(&self.document, document_generation)? {
            return Err("hosted capture permission no longer matches this document".into());
        }
        let mut hosted = self
            .document
            .graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                DocumentNode::Hosted { id, module, .. } => Some((id, module)),
                _ => None,
            });
        let (module_id, module) = hosted
            .next()
            .ok_or("hosted capture requires one OFFSET module")?;
        module.validate_supported_offset()?;
        if hosted.next().is_some() {
            return Err("hosted capture requires exactly one OFFSET module".into());
        }
        let module_id = module_id.clone();
        let audio_ticket = self.capture_resume_snapshot()?;
        Ok(HostedProjectCaptureTicket {
            identity: Arc::new(()),
            document: Arc::new(self.document.clone()),
            document_generation,
            revision: self.revision,
            state_id: self.state_id,
            layout_generation: self.layout_generation,
            module_id,
            native_patch: self.patch.clone(),
            backend,
            audio_ticket,
        })
    }

    /// Merge only into the sealed clone. The caller registers the checkpoint
    /// only after persistence succeeds; completion itself never cleans a project.
    #[doc(hidden)]
    pub fn complete_hosted_project_capture(
        &self,
        ticket: &HostedProjectCaptureTicket,
        receipt: HostedProjectCaptureReceipt,
    ) -> Result<ActiveProjectSave, String> {
        self.validate_hosted_capture_ticket(ticket)?;
        if !Arc::ptr_eq(&ticket.identity, &receipt.ticket_identity) {
            return Err("hosted capture receipt belongs to another ticket".into());
        }
        // Overlay nodes only: every source reference, asset and route is taken
        // verbatim from the sealed full document, including hosted incident routes.
        let mut document = ticket.document.as_ref().clone();
        let native: BTreeMap<_, _> = receipt
            .native_patch
            .nodes
            .into_iter()
            .map(|node| (node.id.clone(), node))
            .collect();
        for node in &mut document.graph.nodes {
            match node {
                DocumentNode::Native { node, .. } => {
                    *node = native
                        .get(&node.id)
                        .ok_or("captured native node missing")?
                        .clone();
                }
                DocumentNode::Hosted { id, module, .. } if id == &ticket.module_id => {
                    *module = receipt.hosted_module.clone();
                }
                _ => {}
            }
        }
        document.validate_structure()?;
        let states = configured_document_states(&document);
        let edit_content = saved_content(
            &ticket.document,
            serde_json::to_value(&ticket.document.layout).map_err(|e| e.to_string())?,
            &states,
        );
        let frame = Some(receipt.device_frame);
        Ok(ActiveProjectSave {
            snapshot: ProjectDocumentSnapshot {
                document,
                revision: ticket.revision,
                state_id: ticket.state_id,
                layout_generation: ticket.layout_generation,
                state_source: "live_hosted_barrier",
                frame,
            },
            checkpoint: ProjectSaveCheckpoint {
                generation: ticket.document_generation,
                revision: ticket.revision,
                frame,
                edit_content,
            },
        })
    }

    fn validate_hosted_capture_ticket(
        &self,
        ticket: &HostedProjectCaptureTicket,
    ) -> Result<(), String> {
        self.check_open()?;
        self.check_midi_cc_capture()?;
        if self.layout_pending
            || self.project_save.generation() != ticket.document_generation
            || self.revision != ticket.revision
            || self.state_id != ticket.state_id
            || self.layout_generation != ticket.layout_generation
            || !self.live_module_events.as_ref().is_some_and(|backend| {
                Arc::ptr_eq(backend, &ticket.backend) && backend.requires_hosted_capture_barrier()
            })
            || !self.hosted_execution_grant.as_ref().is_some_and(|grant| {
                grant
                    .matches_document(&self.document, ticket.document_generation)
                    .unwrap_or(false)
            })
            || serde_json::to_value(&self.document).map_err(|e| e.to_string())?
                != serde_json::to_value(ticket.document.as_ref()).map_err(|e| e.to_string())?
        {
            return Err(
                "hosted capture ticket is stale; the active document or backend changed".into(),
            );
        }
        Ok(())
    }
}

fn validate_hosted_state(
    ticket: &HostedProjectCaptureTicket,
    state: Value,
) -> Result<HostedModule, String> {
    let mut module = ticket
        .document
        .graph
        .nodes
        .iter()
        .find_map(|node| match node {
            DocumentNode::Hosted { id, module, .. } if id == &ticket.module_id => {
                Some(module.clone())
            }
            _ => None,
        })
        .ok_or("sealed hosted module missing")?;
    let expected_state = module.state.clone();
    module.state = HostedState::new(state)?;
    module.validate_supported_offset()?;
    // OFFSET has no autonomous persistent state. Every parameter and option
    // must still equal the sealed authored request after its queue is drained.
    // Future capabilities must explicitly qualify their mutable state fields.
    if module.state.as_ref() != expected_state.as_ref() {
        return Err("captured OFFSET state differs from the sealed request".into());
    }
    Ok(module)
}

/// Reconstruct every permitted change using the existing typed capabilities,
/// then compare the complete patch. Unknown fields and graph changes fail closed.
fn validate_native_projection(authored: &Patch, captured: &Patch) -> Result<(), String> {
    if authored.nodes.len() != captured.nodes.len() {
        return Err("hosted capture changed the native graph".into());
    }
    let mut permitted = authored.clone();
    for (node, captured_node) in permitted.nodes.iter_mut().zip(&captured.nodes) {
        if node.id != captured_node.id || node.kind != captured_node.kind {
            return Err("hosted capture changed native module identity or ordering".into());
        }
        for &name in openpatch_core::runtime_parameter_names(&node.kind) {
            if node.params.contains_key(name) && !captured_node.params.contains_key(name) {
                return Err(format!(
                    "captured native parameter '{name}' missing for '{}'",
                    node.id
                ));
            }
            if let Some(&value) = captured_node.params.get(name) {
                let descriptor = openpatch_core::runtime_parameter_descriptor(&node.kind, name)
                    .ok_or("unqualified native runtime parameter")?;
                if !value.is_finite()
                    || value < descriptor.min
                    || value > descriptor.max
                    || (descriptor.snap && value.fract() != 0.0)
                {
                    return Err(format!(
                        "invalid captured native parameter '{name}' for '{}'",
                        node.id
                    ));
                }
                node.params.insert(name.into(), value);
            }
        }
        if SavedModuleState::from_node(node).is_some() {
            let state = SavedModuleState::from_node(captured_node)
                .ok_or("captured native persistent state is invalid")?;
            super::rackmcp_transport::validate_selection(node, state)?;
            state.write_node(node);
        }
    }
    if serde_json::to_value(&permitted).map_err(|e| e.to_string())?
        != serde_json::to_value(captured).map_err(|e| e.to_string())?
    {
        return Err("hosted capture changed native structure, routing or unsupported state".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_document::hosted::HostedIdentity;
    use crate::project_document::DocumentCable;

    struct Backend;
    impl LiveModuleEvents for Backend {
        fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
            Err("capture must not trigger runtime mutations".into())
        }
        fn snapshot(&self) -> Value {
            panic!("capture must use the frozen graph")
        }
        fn requires_hosted_capture_barrier(&self) -> bool {
            true
        }
        fn supports_document_execution(&self) -> bool {
            true
        }
    }

    fn session() -> Session {
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"sample_rate":48000,
            "nodes":[{"id":"dc","kind":"constant","params":{"value":0.5}},
                {"id":"amp","kind":"stock_vca","params":{"level":0.5}}],
            "cables":[],"output":"amp","output_port":"out"
        }))
        .unwrap();
        let mut document = ProjectDocument::from_patch(patch, ProjectLayout::default()).unwrap();
        document.graph.nodes.push(DocumentNode::Hosted {
            id: "offset".into(),
            module: HostedModule::offset(
                HostedIdentity {
                    plugin: "Bogaudio".into(),
                    model: "Bogaudio-Offset".into(),
                    version: "2.6.47".into(),
                    rack_version: "2.6.6".into(),
                    plugin_sha256: "1".repeat(64),
                    runtime_sha256: "2".repeat(64),
                },
                0.25,
                0.5,
                true,
                false,
            )
            .unwrap(),
            source: None,
        });
        for route in [
            json!({"id":"host-in","from":"dc","output":"out","to":"offset","input":"in"}),
            json!({"id":"host-out","from":"offset","output":"out","to":"amp","input":"signal"}),
        ] {
            document.graph.cables.push(DocumentCable {
                route: serde_json::from_value(route).unwrap(),
                source: None,
            });
        }
        let mut session = Session::new(&std::env::temp_dir()).unwrap();
        session.replace_project_document(document, None).unwrap();
        // Placement is a separate host responsibility, not under test here.
        session.layout_pending = false;
        session.request_delayed_hosted_execution(None).unwrap();
        session
            .set_live_module_events(Some(Arc::new(Backend)))
            .unwrap();
        session
    }

    fn receipt(ticket: &HostedProjectCaptureTicket) -> HostedProjectCaptureReceipt {
        let mut native_patch = ticket.native_patch.clone();
        native_patch
            .nodes
            .iter_mut()
            .find(|node| node.id == "amp")
            .unwrap()
            .params
            .insert("level".into(), 0.75);
        validate_native_projection(&ticket.native_patch, &native_patch).unwrap();
        let state = ticket
            .document
            .graph
            .nodes
            .iter()
            .find_map(|node| match node {
                DocumentNode::Hosted { module, .. } => Some(module.state.as_ref().clone()),
                _ => None,
            })
            .unwrap();
        HostedProjectCaptureReceipt {
            ticket_identity: ticket.identity.clone(),
            native_patch,
            hosted_module: validate_hosted_state(ticket, state).unwrap(),
            device_frame: 512,
        }
    }

    #[test]
    fn native_capture_rejects_forged_graph_routes_and_unknown_state() {
        let mut session = session();
        let ticket = session.begin_hosted_project_capture(None).unwrap();
        let original = ticket.native_patch();
        let mut valid = original.clone();
        valid.nodes[1].params.insert("level".into(), 0.75);
        validate_native_projection(original, &valid).unwrap();
        for fault in [
            "node",
            "order",
            "output",
            "version",
            "unknown",
            "bounds",
            "bypass",
            "missing_runtime",
        ] {
            let mut forged = valid.clone();
            match fault {
                "node" => {
                    forged.nodes.pop();
                }
                "order" => forged.nodes.swap(0, 1),
                "output" => forged.output = Some("dc".into()),
                "version" => forged.version = 1,
                "unknown" => {
                    forged.nodes[1].params.insert("unqualified".into(), 1.0);
                }
                "bounds" => {
                    forged.nodes[1].params.insert("level".into(), 2.0);
                }
                "bypass" => forged.nodes[1].bypassed = true,
                "missing_runtime" => {
                    forged.nodes[1].params.remove("level");
                }
                _ => unreachable!(),
            }
            assert!(
                validate_native_projection(original, &forged).is_err(),
                "{fault}"
            );
        }
        let mut rerouted = original.clone();
        rerouted.cables.push(
            serde_json::from_value(json!({
                "id":"forged","from":"dc","output":"out","to":"amp","input":"signal"
            }))
            .unwrap(),
        );
        assert!(validate_native_projection(original, &rerouted).is_err());
    }

    #[test]
    fn hosted_capture_rejects_wrong_identity_parameters_and_custom_state() {
        let mut session = session();
        let ticket = session.begin_hosted_project_capture(None).unwrap();
        let valid = receipt(&ticket).hosted_module.state.as_ref().clone();
        for fault in [
            "plugin",
            "id",
            "parameter",
            "missing",
            "custom",
            "extra",
            "wrong_offset",
            "wrong_scale",
            "wrong_offset_first",
            "wrong_output_limit",
        ] {
            let mut state = valid.clone();
            match fault {
                "plugin" => state["plugin"] = json!("Other"),
                "id" => state["id"] = json!(3),
                "parameter" => state["params"][0]["value"] = json!(2),
                "wrong_offset" => state["params"][0]["value"] = json!(0.5),
                "wrong_scale" => state["params"][1]["value"] = json!(0.25),
                "wrong_offset_first" => state["data"]["offset_first"] = json!(false),
                "wrong_output_limit" => state["data"]["disableOutputLimit"] = json!(true),
                "missing" => {
                    state["params"].as_array_mut().unwrap().pop();
                }
                "custom" => state["data"]["offset_first"] = json!(1),
                "extra" => state["extra"] = json!(true),
                _ => unreachable!(),
            }
            assert!(validate_hosted_state(&ticket, state).is_err(), "{fault}");
        }
    }

    #[test]
    fn native_capture_accepts_only_canonical_typed_persistent_fields() {
        let authored: Patch = serde_json::from_value(json!({
            "version":2,"sample_rate":48000,
            "nodes":[{"id":"seq","kind":"seq3","params":{"running":1,"tempo":120}}],
            "cables":[],"output":"seq","output_port":"cv1"
        }))
        .unwrap();
        let mut captured = authored.clone();
        captured.nodes[0].params.insert("running".into(), 0.0);
        validate_native_projection(&authored, &captured).unwrap();
        captured.nodes[0].params.insert("running".into(), 0.5);
        assert!(validate_native_projection(&authored, &captured).is_err());
    }

    #[test]
    fn completion_preserves_full_routes_and_authored_baseline_without_session_mutation() {
        let mut session = session();
        let before = session.snapshot();
        let history = session.history_state_ids().collect::<Vec<_>>();
        let baseline = session.project_save.status_from_content(None);
        let ticket = session.begin_hosted_project_capture(None).unwrap();
        let captured = session
            .complete_hosted_project_capture(&ticket, receipt(&ticket))
            .unwrap();
        assert_eq!(captured.snapshot.state_source, "live_hosted_barrier");
        assert_eq!(captured.snapshot.frame, Some(512));
        assert_eq!(
            serde_json::to_value(&captured.snapshot.document.graph.cables).unwrap(),
            serde_json::to_value(&ticket.document.graph.cables).unwrap()
        );
        let amp = captured
            .snapshot
            .document
            .graph
            .nodes
            .iter()
            .find_map(|node| match node {
                DocumentNode::Native { node, .. } if node.id == "amp" => Some(node),
                _ => None,
            })
            .unwrap();
        assert_eq!(amp.params["level"], 0.75);
        assert_eq!(
            captured.checkpoint.edit_content,
            saved_content(
                &ticket.document,
                serde_json::to_value(&ticket.document.layout).unwrap(),
                &configured_document_states(&ticket.document)
            )
        );
        assert_eq!(session.snapshot(), before);
        assert_eq!(session.history_state_ids().collect::<Vec<_>>(), history);
        assert_eq!(session.project_save.status_from_content(None), baseline);
    }

    #[test]
    fn stale_edit_load_layout_backend_and_full_document_are_rejected_without_baseline_changes() {
        for fault in ["edit", "load", "layout", "backend", "document", "state"] {
            let mut session = session();
            let ticket = session.begin_hosted_project_capture(None).unwrap();
            match fault {
                "edit" => {
                    session
                        .execute(
                            "set_parameter",
                            json!({"id":"amp","parameter":"level","value":0.6}),
                        )
                        .unwrap();
                }
                "load" => {
                    session
                        .replace_project_document(ticket.document.as_ref().clone(), None)
                        .unwrap();
                }
                "layout" => session.layout_generation += 1,
                "backend" => {
                    session
                        .set_live_module_events(Some(Arc::new(Backend)))
                        .unwrap();
                }
                "document" => session.document.graph.sample_rate = 44_100,
                "state" => session.state_id += 1,
                _ => unreachable!(),
            }
            let before = session.snapshot();
            let history = session.history_state_ids().collect::<Vec<_>>();
            let baseline = session.project_save.status_from_content(None);
            assert!(
                session
                    .complete_hosted_project_capture(&ticket, receipt(&ticket))
                    .is_err(),
                "{fault}"
            );
            assert_eq!(session.snapshot(), before);
            assert_eq!(session.history_state_ids().collect::<Vec<_>>(), history);
            assert_eq!(session.project_save.status_from_content(None), baseline);
        }
    }
}
