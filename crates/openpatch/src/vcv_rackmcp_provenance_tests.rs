use crate::control::{execute_shared, LiveModuleEvents, Session};
use crate::project_document::{
    AutomationOmissionReason, DocumentNode, ExecutionDisposition, MappingFidelity, PlaybackMode,
    ProjectDocument, SourceModule,
};
use openpatch_core::rackmcp_sequence::SequenceState;
use openpatch_core::{ModuleAction, Node, Patch};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

fn imported(nonce: &str) -> ProjectDocument {
    let patch = json!({"version":"2.6.6","retain":{"nonce":nonce,"opaque":[null,17]},
    "modules":[
        {"id":11,"plugin":"RackMCP","model":"Sequencer","version":"2.0.0",
            "params":[{"id":0,"value":128},{"id":2,"value":1}],
            "data":{"seed":1729,"selectedPattern":"A","patterns":[
                {"name":"A","length":4,"tracks":[{"voices":2,"glideMs":125,"steps":[
                    {"step":0,"notes":[0,7],"vel":0.7,"gate":0.6,"prob":0.8,"ratchet":2,"accent":true,"slide":true},
                    {"step":2,"notes":[]}]}]}],
                "lanes":[{"moduleId":"42","paramId":0,"mode":"raw","loop":false,
                    "lengthBars":2,"points":[{"bar":0,"value":0.2,"curve":"hold"},{"bar":2,"value":0.8,"curve":"easeInOut"}]}]}},
        {"id":42,"plugin":"Fundamental","model":"VCA-1","version":"2.6.4","params":[{"id":0,"value":0.9}]},
        {"id":43,"plugin":"Fundamental","model":"VCA-1","version":"2.6.4"}
    ]});
    crate::vcv_import::import(&serde_json::to_vec(&patch).unwrap())
        .unwrap()
        .document
}

fn native_mut(document: &mut ProjectDocument, index: usize) -> &mut Node {
    match &mut document.graph.nodes[index] {
        DocumentNode::Native { node, .. } => node,
        _ => panic!("expected native node"),
    }
}

fn source_mut(document: &mut ProjectDocument, index: usize) -> &mut Option<SourceModule> {
    match &mut document.graph.nodes[index] {
        DocumentNode::Native { source, .. } => source,
        _ => panic!("expected native node"),
    }
}

fn state(document: &ProjectDocument) -> &SequenceState {
    match &document.graph.nodes[0] {
        DocumentNode::Native { node, .. } => node.rackmcp_sequence.as_deref().unwrap(),
        _ => panic!("expected sequencer"),
    }
}

fn state_mut(document: &mut ProjectDocument) -> &mut SequenceState {
    native_mut(document, 0)
        .rackmcp_sequence
        .as_deref_mut()
        .unwrap()
}

fn value(document: &ProjectDocument) -> Value {
    serde_json::to_value(document).unwrap()
}

fn assert_rejected(document: &ProjectDocument, case: &str) {
    let error = document.validate_structure().unwrap_err();
    assert!(error.contains("imported automation"), "{case}: {error}");
    // Serialize directly so encode's own validation cannot hide a decode hole.
    assert!(
        ProjectDocument::decode(&serde_json::to_vec(document).unwrap()).is_err(),
        "{case}"
    );
}

#[test]
fn imported_bindings_reject_existing_destinations_without_exact_source_authority() {
    let baseline = imported("baseline");
    baseline.project(PlaybackMode::Exact).unwrap();
    for case in [
        "source_none",
        "cross_package",
        "original_id",
        "mapping_id",
        "mapping_version",
        "mapping_fidelity",
        "unsupported_parameter",
        "native_kind",
        "sequencer_mapping",
    ] {
        let mut document = baseline.clone();
        match case {
            "source_none" => *source_mut(&mut document, 1) = None,
            "cross_package" => {
                let foreign = imported("different-package-same-source-ids");
                let foreign_source = foreign.graph.nodes[1].source().unwrap().clone();
                document.sources.extend(foreign.sources);
                document.source_blobs.extend(foreign.source_blobs);
                *source_mut(&mut document, 1) = Some(foreign_source);
            }
            "original_id" => source_mut(&mut document, 1).as_mut().unwrap().index = 2,
            "mapping_id" => {
                source_mut(&mut document, 1)
                    .as_mut()
                    .unwrap()
                    .mapping
                    .as_mut()
                    .unwrap()
                    .id = "vcv/Fundamental/Other/2.6.4".into()
            }
            "mapping_version" => {
                source_mut(&mut document, 1)
                    .as_mut()
                    .unwrap()
                    .mapping
                    .as_mut()
                    .unwrap()
                    .version = 2
            }
            "mapping_fidelity" => {
                source_mut(&mut document, 1)
                    .as_mut()
                    .unwrap()
                    .mapping
                    .as_mut()
                    .unwrap()
                    .fidelity = MappingFidelity::UserSubstitution
            }
            "unsupported_parameter" => {
                let sequence = state_mut(&mut document);
                sequence.data.lanes[0].param_id = 1;
                sequence.targets[0].source_param_id = 1;
            }
            "native_kind" => {
                let destination = native_mut(&mut document, 1);
                destination.kind = "rack264_vcf".into();
                destination.params.clear();
            }
            "sequencer_mapping" => {
                source_mut(&mut document, 0)
                    .as_mut()
                    .unwrap()
                    .mapping
                    .as_mut()
                    .unwrap()
                    .id = "vcv/RackMCP/Other/2.0.0".into()
            }
            _ => unreachable!(),
        }
        assert_rejected(&document, case);
    }
}

#[test]
fn missing_binding_is_dormant_and_a_qualified_duplicate_requires_explicit_rebinding() {
    let mut document = imported("duplicate");
    let original_state = state(&document).clone();
    let mut duplicate = document.graph.nodes[1].clone();
    if let DocumentNode::Native { node, .. } = &mut duplicate {
        node.id = "qualified_copy".into();
    }
    document.graph.nodes.remove(1);
    document.graph.nodes.push(duplicate);
    document.validate_structure().unwrap();
    assert_eq!(
        state(&document),
        &original_state,
        "deletion must retain the explicit binding"
    );
    let omissions = document.automation_omissions().unwrap();
    assert_eq!(omissions.len(), 1);
    assert_eq!(omissions[0].reason, AutomationOmissionReason::MissingTarget);
    assert_eq!(omissions[0].native_module.as_deref(), Some("vcv_m_42"));
    assert!(document.project(PlaybackMode::Exact).is_err());
    let partial = document.project(PlaybackMode::Partial).unwrap();
    assert!(partial.unresolved_modules.is_empty());
    assert_eq!(partial.omitted_automation, omissions);
    assert_eq!(
        partial.patch.nodes[0].rackmcp_sequence.as_deref(),
        Some(&original_state)
    );
    let carrier = document.state_carrier().unwrap();
    assert_eq!(carrier.omitted_automation, omissions);
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .replace_project_document(document.clone(), None)
        .unwrap();
    assert_eq!(
        session.execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    assert_eq!(
        session.document_execution_status()["omitted_automation"][0]["reason"],
        "missing_target"
    );
    state_mut(&mut document).targets[0].module = "qualified_copy".into();
    document.validate_structure().unwrap();
    assert!(document.automation_omissions().unwrap().is_empty());
    document.project(PlaybackMode::Exact).unwrap();
}

#[test]
fn an_unbound_imported_lane_alone_suspends_exact_and_is_disclosed_in_partial() {
    let mut document = imported("unbound");
    let source_data = state(&document).data.clone();
    state_mut(&mut document).targets.clear();
    let partial = document.project(PlaybackMode::Partial).unwrap();
    assert!(partial.unresolved_modules.is_empty());
    assert!(partial.omitted_routes.is_empty());
    assert_eq!(partial.omitted_automation.len(), 1);
    assert_eq!(
        partial.omitted_automation[0].reason,
        AutomationOmissionReason::UnboundLane
    );
    assert_eq!(partial.omitted_automation[0].native_module, None);
    assert_eq!(state(&document).data, source_data);
    assert!(document.project(PlaybackMode::Exact).is_err());
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session.replace_project_document(document, None).unwrap();
    assert_eq!(
        session.execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    session
        .set_document_playback(PlaybackMode::Partial, None)
        .unwrap();
    assert_eq!(
        session.execution_disposition(),
        ExecutionDisposition::RunnablePartial
    );
    let status = session.document_execution_status();
    assert_eq!(status["omitted_automation"][0]["reason"], "unbound_lane");
    assert!(status["warning"].is_string());
    assert_eq!(state(session.project_document()).data, source_data);
}

#[test]
fn source_free_native_sequences_keep_their_explicit_binding_without_source_inference() {
    let mut document = imported("native");
    let before = state(&document).clone();
    *source_mut(&mut document, 0) = None;
    *source_mut(&mut document, 1) = None;
    document.validate_structure().unwrap();
    assert!(document.automation_omissions().unwrap().is_empty());
    let projection = document.project(PlaybackMode::Exact).unwrap();
    assert_eq!(
        projection.patch.nodes[0].rackmcp_sequence.as_deref(),
        Some(&before)
    );
    let reopened = ProjectDocument::decode(&document.encode().unwrap()).unwrap();
    assert_eq!(state(&reopened), &before);
    assert!(reopened.graph.nodes[0].source().is_none());
    assert!(reopened.graph.nodes[1].source().is_none());
}

#[derive(Default)]
struct PublicationObserver(Mutex<Vec<Value>>);

impl LiveModuleEvents for PublicationObserver {
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        Err("no live trigger in provenance test".into())
    }
    fn snapshot(&self) -> Value {
        json!({})
    }
    fn supports_document_execution(&self) -> bool {
        true
    }
    fn sync_patch(&self, patch: &Patch, revision: u64) {
        self.0
            .lock()
            .unwrap()
            .push(json!({"sync":revision,"patch":patch.reference_view()}));
    }
    fn request_document_execution(&self, revision: u64, disposition: ExecutionDisposition) {
        self.0
            .lock()
            .unwrap()
            .push(json!({"gate":revision,"disposition":disposition}));
    }
    fn retire_modules(&self, ids: &[String]) {
        self.0.lock().unwrap().push(json!({"retired":ids}));
    }
}

#[test]
fn same_id_remove_and_add_rejects_atomically_before_shared_session_or_backend_publication() {
    let shared = Session::new(&std::env::temp_dir()).unwrap().into_shared();
    execute_shared(
        &shared,
        "replace_document",
        json!({"document":imported("atomic")}),
    )
    .unwrap();
    execute_shared(
        &shared,
        "set_parameter",
        json!({"id":"vcv_m_42","parameter":"level","value":0.4}),
    )
    .unwrap();
    execute_shared(&shared, "undo", json!({})).unwrap();
    let backend = Arc::new(PublicationObserver::default());
    let (snapshot, document, patch, revision, state_id, history, can_undo, can_redo, ticket) = {
        let mut session = shared.lock().unwrap();
        session
            .set_live_module_events(Some(backend.clone()))
            .unwrap();
        (
            session.snapshot(),
            session.project_document().encode().unwrap(),
            serde_json::to_value(session.patch()).unwrap(),
            session.revision(),
            session.state_id(),
            session.history_state_ids().collect::<Vec<_>>(),
            session.can_undo(),
            session.can_redo(),
            session.audio_snapshot().unwrap(),
        )
    };
    let publication = backend.0.lock().unwrap().clone();
    let result = execute_shared(
        &shared,
        "apply_edits",
        json!({"expected_revision":revision,"edits":[
            {"tool":"remove_module","arguments":{"id":"vcv_m_42"}},
            {"tool":"add_module","arguments":{"id":"vcv_m_42","kind":"stock_vca","params":{"level":0.1}}}
        ]}),
    );
    assert!(result.unwrap_err().contains("imported automation"));
    let mut session = shared.lock().unwrap();
    assert_eq!(session.snapshot(), snapshot);
    assert_eq!(session.project_document().encode().unwrap(), document);
    assert_eq!(serde_json::to_value(session.patch()).unwrap(), patch);
    assert_eq!(
        (session.revision(), session.state_id()),
        (revision, state_id)
    );
    assert_eq!(session.history_state_ids().collect::<Vec<_>>(), history);
    assert_eq!(
        (session.can_undo(), session.can_redo()),
        (can_undo, can_redo)
    );
    assert!(
        can_redo,
        "the rejected batch must preserve an existing redo branch"
    );
    assert!(Arc::ptr_eq(&session.audio_snapshot().unwrap(), &ticket));
    assert_eq!(*backend.0.lock().unwrap(), publication);
}

#[test]
fn target_deletion_history_and_partial_save_reopen_preserve_source_and_dormant_bindings() {
    let directory = std::env::temp_dir().join(format!(
        "openpatch-rackmcp-provenance-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let original = imported("history");
    let original_state = state(&original).clone();
    let original_sources = value(&original)["sources"].clone();
    let original_blobs = value(&original)["source_blobs"].clone();
    let mut session = Session::new(&directory).unwrap();
    session.replace_project_document(original, None).unwrap();
    session
        .execute("remove_module", json!({"id":"vcv_m_42"}))
        .unwrap();
    let deleted = value(session.project_document());
    assert_eq!(
        session.execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    assert_eq!(state(session.project_document()), &original_state);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(
        session.execution_disposition(),
        ExecutionDisposition::RunnableExact
    );
    assert_eq!(state(session.project_document()), &original_state);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(value(session.project_document()), deleted);
    session
        .set_document_playback(PlaybackMode::Partial, None)
        .unwrap();
    assert_eq!(
        session.execution_disposition(),
        ExecutionDisposition::RunnablePartial
    );
    let captured = session.capture_project_document(None).unwrap();
    assert_eq!(state(&captured.document), &original_state);
    session
        .execute("save_document", json!({"filename":"dormant.json"}))
        .unwrap();
    let bytes = std::fs::read(directory.join("dormant.json")).unwrap();
    let saved = ProjectDocument::decode(&bytes).unwrap();
    assert_eq!(state(&saved), &original_state);
    assert_eq!(value(&saved)["sources"], original_sources);
    assert_eq!(value(&saved)["source_blobs"], original_blobs);
    assert_eq!(
        saved.automation_omissions().unwrap()[0].reason,
        AutomationOmissionReason::MissingTarget
    );
    session
        .execute("load_document", json!({"filename":"dormant.json"}))
        .unwrap();
    assert_eq!(session.playback_mode(), PlaybackMode::Exact);
    assert_eq!(
        session.execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    assert_eq!(state(session.project_document()), &original_state);
    assert_eq!(session.project_document().encode().unwrap(), bytes);
    assert_eq!(
        value(session.project_document())["sources"],
        original_sources
    );
    assert_eq!(
        value(session.project_document())["source_blobs"],
        original_blobs
    );
    std::fs::remove_dir_all(directory).unwrap();
}
