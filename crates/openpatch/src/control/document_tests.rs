use super::*;
use crate::project_document::{
    MappingFidelity, ObservedPort, PortDirection, SourceBlob, SourceMapping, SourceModule,
    SourcePackage,
};

fn imported() -> ProjectDocument {
    let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"dc","kind":"constant","params":{"value":2.0}}],"cables":[],"output":"dc"}))
    .unwrap();
    let mut document = ProjectDocument::from_patch(patch, ProjectLayout::default()).unwrap();
    let blob = SourceBlob::new(vec![0, 255, 1, 23]).unwrap();
    let blob_id = blob.id();
    let package = SourcePackage {
        patch: Arc::new(json!({"version":"2.6.4","unknown-global":{"keep":true},
            "modules":[{"id":1,"plugin":"Test","model":"DC","data":{"unknown":[1,null]}},
                {"id":2,"plugin":"Absent","model":"Resonator","data":{"sample":"modules/2/raw.bin"}}],
            "cables":[{"id":12,"unknown-cable":"keep"}]})),
        assets: BTreeMap::from([("modules/2/raw.bin".into(), blob_id.clone())]),
        missing_assets: BTreeSet::new(),
    };
    let package_id = package.id().unwrap();
    if let DocumentNode::Native { source, .. } = &mut document.graph.nodes[0] {
        *source = Some(SourceModule {
            package: package_id.clone(),
            index: 0,
            mapping: Some(SourceMapping {
                id: "test/dc".into(),
                version: 1,
                fidelity: MappingFidelity::SourceBehavior,
            }),
        });
    }
    document.graph.nodes.push(DocumentNode::Unresolved {
        id: "unknown".into(),
        source: SourceModule {
            package: package_id.clone(),
            index: 1,
            mapping: None,
        },
        ports: vec![ObservedPort {
            direction: PortDirection::Input,
            index: 0,
        }],
    });
    document
        .graph
        .cables
        .push(crate::project_document::DocumentCable {
            route: serde_json::from_value(
                json!({"id":"opaque-route","from":"dc","to":"unknown","input":"vcv_in_0"}),
            )
            .unwrap(),
            source: Some(crate::project_document::SourceCable {
                package: package_id.clone(),
                index: 0,
            }),
        });
    document
        .layout
        .positions
        .insert("unknown".into(), [30., 380.]);
    document.sources.insert(package_id, Arc::new(package));
    document.source_blobs.insert(blob_id, blob);
    document.validate_structure().unwrap();
    document
}
fn session() -> Session {
    Session::new(&std::env::temp_dir()).unwrap()
}
fn value(document: &ProjectDocument) -> Value {
    serde_json::to_value(document).unwrap()
}

#[test]
fn imported_document_edits_duplicates_deletes_and_history_preserve_originals() {
    let mut s = session();
    let original = imported();
    let source_bytes = original.encode().unwrap();
    s.replace_project_document(original.clone(), Some(0))
        .unwrap();
    assert_eq!(
        s.execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    assert_eq!(s.snapshot()["patch_is_native_state_carrier"], true);
    s.execute(
        "set_parameter",
        json!({"id":"dc","parameter":"value","value":3.0}),
    )
    .unwrap();
    assert_eq!(
        s.project_document().sources.keys().collect::<Vec<_>>(),
        original.sources.keys().collect::<Vec<_>>()
    );
    assert_eq!(s.project_document().graph.cables.len(), 1);
    let edited = value(s.project_document());
    s.execute(
        "duplicate_modules",
        json!({"modules":[{"id":"dc","new_id":"dc2"}]}),
    )
    .unwrap();
    let copied = s
        .project_document()
        .graph
        .nodes
        .iter()
        .find(|node| node.id() == "dc2")
        .unwrap();
    assert_eq!(
        serde_json::to_value(copied.source()).unwrap(),
        serde_json::to_value(original.graph.nodes[0].source()).unwrap()
    );
    s.execute("remove_module", json!({"id":"dc"})).unwrap();
    assert!(s.project_document().graph.cables.is_empty());
    assert!(s
        .project_document()
        .graph
        .nodes
        .iter()
        .any(|node| node.id() == "unknown"));
    s.execute("undo", json!({})).unwrap();
    s.execute("undo", json!({})).unwrap();
    assert_eq!(value(s.project_document()), edited);
    s.execute("undo", json!({})).unwrap();
    assert_eq!(value(s.project_document()), value(&original));
    s.execute("redo", json!({})).unwrap();
    assert_eq!(value(s.project_document()), edited);
    assert_eq!(
        original.encode().unwrap(),
        source_bytes,
        "history never mutates original source payload"
    );
}

#[test]
fn source_loss_and_implicit_partial_execution_are_rejected_atomically() {
    let mut s = session();
    s.replace_project_document(imported(), None).unwrap();
    let before = s.snapshot();
    assert!(s
        .replace_patch(Patch::new_v2(), None)
        .unwrap_err()
        .contains("source data"));
    assert!(s.capture_project_patch(None).is_err());
    assert!(s.execute("benchmark", json!({"seconds":0.1})).is_err());
    assert!(s
        .execute("save_patch", json!({"filename":"must-not-write.json"}))
        .is_err());
    assert!(s
        .execute(
            "set_parameter",
            json!({"id":"dc","parameter":"value","value":4.0,"expected_revision":0})
        )
        .is_err());
    assert!(s
        .execute("add_module", json!({"id":"unknown","kind":"gain"}))
        .is_err());
    assert_eq!(s.snapshot(), before);
    assert_eq!(
        s.capture_project_document(None).unwrap().state_source,
        "suspended_persisted"
    );
    s.set_document_playback(PlaybackMode::Partial, Some(s.revision()))
        .unwrap();
    assert_eq!(
        s.execution_disposition(),
        ExecutionDisposition::RunnablePartial
    );
    assert_eq!(
        s.document_execution_status()["omitted_routes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    s.execute("undo", json!({})).unwrap();
    assert_eq!(
        s.execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    s.execute("redo", json!({})).unwrap();
    assert_eq!(
        s.execution_disposition(),
        ExecutionDisposition::RunnablePartial
    );
}

#[test]
fn suspended_full_save_reopen_preserves_assets_and_defaults_to_exact() {
    let unique = format!(
        "openpatch-document-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(unique);
    let mut s = Session::new(&dir).unwrap();
    let document = imported();
    s.replace_project_document(document.clone(), None).unwrap();
    let saved = s
        .execute("save_document", json!({"filename":"retained.json"}))
        .unwrap();
    assert_eq!(saved["state_source"], "suspended_persisted");
    assert_eq!(
        ProjectDocument::decode(&fs::read(dir.join("retained.json")).unwrap())
            .map(|d| value(&d))
            .unwrap(),
        value(&document)
    );
    s.set_document_playback(PlaybackMode::Partial, None)
        .unwrap();
    s.execute("load_document", json!({"filename":"retained.json"}))
        .unwrap();
    assert_eq!(
        s.execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    assert_eq!(value(s.project_document()), value(&document));
    assert!(
        s.execute("save_document", json!({"filename":"retained.json"}))
            .is_err(),
        "no overwrite"
    );
    s.new_document(Patch::new_v2(), None).unwrap();
    assert!(!s.can_undo());
    assert!(!s.has_source_document());
    assert_eq!(
        s.execution_disposition(),
        ExecutionDisposition::RunnableExact
    );
    fs::remove_dir_all(dir).unwrap();
}

#[derive(Default)]
struct Backend {
    requests: Mutex<Vec<(u64, ExecutionDisposition)>>,
    syncs: Mutex<Vec<u64>>,
    capable: bool,
    observation: Mutex<Value>,
    reads: std::sync::atomic::AtomicUsize,
}
impl LiveModuleEvents for Backend {
    fn snapshot(&self) -> Value {
        self.reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.observation.lock().unwrap().clone()
    }
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        unreachable!()
    }
    fn supports_document_execution(&self) -> bool {
        self.capable
    }
    fn request_document_execution(&self, r: u64, d: ExecutionDisposition) {
        self.requests.lock().unwrap().push((r, d));
    }
    fn sync_patch(&self, _: &Patch, r: u64) {
        if self.capable {
            assert_eq!(
                self.requests.lock().unwrap().last().map(|pair| pair.0),
                Some(r)
            );
        }
        self.syncs.lock().unwrap().push(r);
    }
}
#[test]
fn backend_capability_and_revision_fences_cover_edits_history_and_native_return() {
    let mut s = session();
    let unsupported = Arc::new(Backend::default());
    s.set_live_module_events(Some(unsupported)).unwrap();
    let before = s.snapshot();
    assert!(s
        .replace_project_document(imported(), None)
        .unwrap_err()
        .contains("cannot fence"));
    assert_eq!(s.snapshot(), before);
    s.set_live_module_events(None).unwrap();
    s.replace_project_document(imported(), None).unwrap();
    let backend = Arc::new(Backend {
        capable: true,
        ..Default::default()
    });
    s.set_live_module_events(Some(backend.clone())).unwrap();
    s.execute(
        "set_parameter",
        json!({"id":"dc","parameter":"value","value":3.0}),
    )
    .unwrap();
    s.execute("undo", json!({})).unwrap();
    assert_eq!(
        backend.requests.lock().unwrap().last().copied(),
        Some((s.revision(), ExecutionDisposition::SuspendedUnresolved))
    );
    s.new_document(Patch::new_v2(), None).unwrap();
    assert_eq!(
        backend.requests.lock().unwrap().last().copied(),
        Some((s.revision(), ExecutionDisposition::RunnableExact))
    );
}

#[test]
fn native_legacy_documents_and_offline_waveforms_remain_unchanged() {
    let mut s = session();
    let patch = imported().state_carrier().unwrap().patch;
    s.replace_patch(patch.clone(), None).unwrap();
    assert!(s.snapshot().get("document").is_none());
    assert!(s.capture_project_patch(None).is_ok());
    assert_eq!(
        value(s.project_document()),
        value(&ProjectDocument::from_patch(patch, ProjectLayout::default()).unwrap())
    );
}
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn pausing_captures_current_persistent_state_and_suspended_edits_do_not_read_signal() {
    let mut document = imported();
    let node: Node =
        serde_json::from_value(json!({"id":"seq","kind":"seq3","params":{"running":1}})).unwrap();
    document
        .graph
        .nodes
        .push(DocumentNode::Native { node, source: None });
    let mut s = session();
    s.replace_project_document(document, None).unwrap();
    s.set_document_playback(PlaybackMode::Partial, None)
        .unwrap();
    let backend = Arc::new(Backend {
        capable: true,
        ..Default::default()
    });
    s.set_live_module_events(Some(backend.clone())).unwrap();
    let before = s.snapshot();
    let requests = backend.requests.lock().unwrap().len();
    assert!(
        s.set_document_playback(PlaybackMode::Exact, None).is_err(),
        "unavailable live state must not become a reset snapshot"
    );
    assert_eq!(s.snapshot(), before);
    assert_eq!(backend.requests.lock().unwrap().len(), requests);
    *backend.observation.lock().unwrap() = json!({"coherent":true,"modules":{
        "seq":{"kind":"seq3","available":true,"revision":s.revision(),"frame":128,"state":{"running":false}}
    }});
    s.set_document_playback(PlaybackMode::Exact, None).unwrap();
    let reads = backend.reads.load(std::sync::atomic::Ordering::Relaxed);
    *backend.observation.lock().unwrap() = json!({"coherent":false});
    let captured = s.capture_project_document(None).unwrap();
    assert_eq!(captured.state_source, "suspended_persisted");
    let carrier = captured.document.state_carrier().unwrap().patch;
    assert_eq!(
        carrier
            .nodes
            .iter()
            .find(|node| node.id == "seq")
            .unwrap()
            .params["running"],
        0.
    );
    s.execute(
        "set_parameter",
        json!({"id":"seq","parameter":"running","value":1}),
    )
    .unwrap();
    s.execute(
        "duplicate_modules",
        json!({"modules":[{"id":"seq","new_id":"copy"}]}),
    )
    .unwrap();
    assert_eq!(
        backend.reads.load(std::sync::atomic::Ordering::Relaxed),
        reads,
        "suspended edits use persisted fields without fabricating live observations"
    );
    assert_eq!(
        s.patch()
            .nodes
            .iter()
            .find(|node| node.id == "copy")
            .unwrap()
            .params["running"],
        1.
    );
}

#[test]
fn undo_into_suspension_captures_surviving_live_state_before_history_mutation() {
    let mut document = imported();
    document.graph.nodes.push(DocumentNode::Native {
        node: serde_json::from_value(json!({"id":"seq","kind":"seq3","params":{"running":1}}))
            .unwrap(),
        source: None,
    });
    let mut s = session();
    s.replace_project_document(document, None).unwrap();
    s.set_document_playback(PlaybackMode::Partial, None)
        .unwrap();
    let backend = Arc::new(Backend {
        capable: true,
        ..Default::default()
    });
    s.set_live_module_events(Some(backend.clone())).unwrap();
    let before = s.snapshot();
    let requests = backend.requests.lock().unwrap().len();
    assert!(
        s.execute("undo", json!({})).is_err(),
        "unavailable capture cannot consume a history entry or fence audio"
    );
    assert_eq!(s.snapshot(), before);
    assert_eq!(backend.requests.lock().unwrap().len(), requests);
    *backend.observation.lock().unwrap() = json!({"coherent":true,"modules":{
        "seq":{"kind":"seq3","available":true,"revision":s.revision(),"frame":128,"state":{"running":false}}
    }});
    s.execute("undo", json!({})).unwrap();
    assert_eq!(
        s.execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    let captured = s.capture_project_document(None).unwrap();
    assert_eq!(
        captured
            .document
            .state_carrier()
            .unwrap()
            .patch
            .nodes
            .iter()
            .find(|node| node.id == "seq")
            .unwrap()
            .params["running"],
        0.
    );
    s.execute("redo", json!({})).unwrap();
    assert_eq!(
        s.execution_disposition(),
        ExecutionDisposition::RunnablePartial
    );
    assert_eq!(
        s.patch()
            .nodes
            .iter()
            .find(|node| node.id == "seq")
            .unwrap()
            .params["running"],
        0.,
        "redo must not restore stale authored Run over the surviving incarnation"
    );
}

#[test]
fn clear_source_routes_and_generated_cable_ids_use_the_full_document_namespace() {
    let mut document = imported();
    document.graph.cables[0].route.id = Some("cable_1".into());
    document.graph.nodes.push(DocumentNode::Native {
        node: serde_json::from_value(json!({"id":"amp","kind":"gain","params":{}})).unwrap(),
        source: None,
    });
    let mut s = session();
    s.replace_project_document(document, None).unwrap();
    assert_eq!(
        s.history_asset_bytes(),
        4,
        "source bytes share the history asset budget"
    );
    let before = s.snapshot();
    assert!(s
        .execute(
            "duplicate_modules",
            json!({"modules":[{"id":"dc","new_id":"copy"}],"cable_mode":"all"})
        )
        .unwrap_err()
        .contains("unresolved routes"));
    assert_eq!(s.snapshot(), before);
    s.execute("connect", json!({"from":"dc","to":"amp","input":"signal"}))
        .unwrap();
    assert_eq!(s.patch().cables[0].id.as_deref(), Some("cable_2"));
    assert_eq!(s.project_document().graph.cables.len(), 2);
    assert_eq!(
        s.history_asset_bytes(),
        4,
        "shared source payloads counted once across edits/history"
    );
    s.execute("clear_cables", json!({})).unwrap();
    assert!(s.project_document().graph.cables.is_empty());
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.project_document().graph.cables.len(), 2);
    s.execute(
        "apply_edits",
        json!({"edits":[
            {"tool":"remove_module","arguments":{"id":"dc"}},
            {"tool":"add_module","arguments":{"id":"dc","kind":"constant"}}
        ]}),
    )
    .unwrap();
    assert!(s
        .project_document()
        .graph
        .nodes
        .iter()
        .find(|node| node.id() == "dc")
        .unwrap()
        .source()
        .is_none());
    assert!(
        s.project_document().graph.cables.is_empty(),
        "recreated native IDs cannot inherit opaque incident cables"
    );
}
