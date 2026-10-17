use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Live {
    value: Mutex<Value>,
    reads: AtomicUsize,
}
impl LiveModuleEvents for Live {
    fn supports_live_parameters(&self) -> bool {
        true
    }
    fn snapshot(&self) -> Value {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.value.lock().unwrap().clone()
    }
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        Err("unexpected trigger".into())
    }
}

fn native_session() -> Session {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .execute(
            "replace_patch",
            json!({"patch":{"version":2,"nodes":[
                {"id":"dc","kind":"constant","params":{"value":1}}
            ]}}),
        )
        .unwrap();
    session
}
fn mark_initial(session: &Session) {
    session
        .project_save_baseline()
        .mark_initial(session.project_edit_content(None).unwrap())
        .unwrap();
}
fn live_session() -> (Session, Arc<Live>) {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .execute(
            "replace_patch",
            json!({"patch":{"version":2,"nodes":[
                {"id":"seq","kind":"seq3","params":{"running":1}},
                {"id":"vca","kind":"stock_vca","params":{"level":1}}
            ]}}),
        )
        .unwrap();
    let live = Arc::new(Live::default());
    *live.value.lock().unwrap() = json!({"coherent":true,"modules":{
        "seq":{"kind":"seq3","available":true,"revision":session.revision(),"frame":128,"state":{"running":false}},
        "vca":{"kind":"stock_vca","available":true,"revision":session.revision(),"frame":128,"parameters":{"level":0.25}}
    }});
    session.set_live_module_events(Some(live.clone())).unwrap();
    (session, live)
}

#[test]
fn authored_edits_undo_and_layout_share_one_content_baseline() {
    let mut session = native_session();
    mark_initial(&session);
    assert_eq!(session.project_save_status()["status"], "untitled");
    assert_eq!(session.project_save_status()["dirty"], false);
    session
        .execute(
            "set_parameter",
            json!({"id":"dc","parameter":"value","value":2}),
        )
        .unwrap();
    assert_eq!(session.project_save_status()["dirty"], true);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.project_save_status()["dirty"], false);
    session.attach_layout_host().unwrap();
    assert_eq!(session.project_save_status()["status"], "unknown");
    let mut layout = session.document.layout.clone();
    layout.positions.insert("dc".into(), [0.0, 0.0]);
    session
        .publish_host_layout(
            session.revision(),
            session.state_id(),
            session.layout_generation(),
            layout,
            LayoutIntent::Placement,
        )
        .unwrap();
    mark_initial(&session);
    let mut layout = session.document.layout.clone();
    layout.pan = [45.0, -30.0];
    session
        .publish_host_layout(
            session.revision(),
            session.state_id(),
            session.layout_generation(),
            layout,
            LayoutIntent::Camera,
        )
        .unwrap();
    assert_eq!(session.project_save_status()["dirty"], true);
}

#[test]
fn effective_parameters_stay_clean_but_full_save_samples_them_with_the_same_durable_frame() {
    let (session, live) = live_session();
    mark_initial(&session);
    live.value.lock().unwrap()["modules"]["vca"]["parameters"]["level"] = json!(0.75);
    assert_eq!(session.project_save_status()["dirty"], false);
    live.reads.store(0, Ordering::Relaxed);
    let captured = session
        .capture_active_project_save(Some(session.revision()))
        .unwrap();
    assert_eq!(
        live.reads.load(Ordering::Relaxed),
        1,
        "save must not sample durable state a second time"
    );
    let DocumentNode::Native { node, .. } = &captured.snapshot.document.graph.nodes[1] else {
        panic!("native vca")
    };
    assert_eq!(node.params["level"], 0.75);
    assert_eq!(captured.checkpoint.frame, Some(128));
    assert_eq!(
        captured.checkpoint.edit_content,
        session.project_edit_content(None).unwrap()
    );
    assert_eq!(
        session.patch.nodes[1].params["level"], 1.0,
        "save leaves authored controls intact"
    );
    session
        .project_save_baseline()
        .register(&captured.checkpoint, Some("live.json".into()))
        .unwrap();
    assert_eq!(session.project_save_status()["status"], "saved");
    live.value.lock().unwrap()["modules"]["seq"]["state"]["running"] = json!(true);
    assert_eq!(session.project_save_status()["dirty"], true);
    live.value.lock().unwrap()["modules"]["seq"]["available"] = json!(false);
    assert_eq!(session.project_save_status()["status"], "unknown");
    assert!(session.project_save_status()["dirty"].is_null());
    assert!(session.capture_active_project_save(None).is_err());
}

#[test]
fn save_completion_registers_captured_content_and_rejects_old_document_generations() {
    let mut session = native_session();
    mark_initial(&session);
    let captured = session.capture_active_project_save(None).unwrap();
    session
        .execute(
            "set_parameter",
            json!({"id":"dc","parameter":"value","value":3}),
        )
        .unwrap();
    let baseline = session.project_save_baseline();
    baseline
        .register(&captured.checkpoint, Some("original.json".into()))
        .unwrap();
    assert_eq!(session.project_save_status()["dirty"], true);
    assert_eq!(
        session.project_save_status()["saved_revision"],
        captured.checkpoint.revision
    );
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.project_save_status()["dirty"], false);
    let clone = session.clone();
    clone.project_save_baseline().mark_unsaved().unwrap();
    assert_eq!(
        session.project_save_status()["dirty"],
        false,
        "preflight Session clone must have detached authority"
    );
    let next = baseline.next_generation().unwrap();
    baseline.begin_document(next).unwrap();
    assert!(baseline
        .register(&captured.checkpoint, Some("obsolete.json".into()))
        .is_err());
    assert_eq!(baseline.generation(), next);
    assert!(
        baseline.status_from_content(Some(&captured.checkpoint.edit_content))["target"].is_null()
    );
    assert_eq!(
        baseline.status_from_content(Some(&captured.checkpoint.edit_content))["dirty"],
        true
    );
}

#[test]
fn snapshot_exports_and_failed_writes_do_not_mark_the_active_project_saved() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::new(dir.path()).unwrap();
    session
        .execute(
            "replace_patch",
            json!({"patch":{"version":2,"nodes":[{"id":"dc","kind":"constant"}]}}),
        )
        .unwrap();
    mark_initial(&session);
    session
        .execute(
            "set_parameter",
            json!({"id":"dc","parameter":"value","value":4}),
        )
        .unwrap();
    let before = session.project_save_status();
    session
        .execute("save_document", json!({"filename":"document-export.json"}))
        .unwrap();
    session
        .execute("save_patch", json!({"filename":"patch-export.json"}))
        .unwrap();
    assert_eq!(session.project_save_status(), before);
    let captured = session.capture_active_project_save(None).unwrap();
    let failed = std::fs::write(dir.path(), captured.snapshot.document.encode().unwrap());
    assert!(failed.is_err());
    assert_eq!(
        session.project_save_status(),
        before,
        "a failed write never registers its checkpoint"
    );
}

#[test]
fn canonical_reference_tracks_source_metadata_and_exact_persistent_voltage_bits() {
    let import = |metadata| {
        crate::vcv_import::import(
            &serde_json::to_vec(&json!({"modules":[
        {"id":1,"plugin":"unknown","model":"future","version":"1","future":metadata}
    ],"cables":[]}))
            .unwrap(),
        )
        .unwrap()
        .document
    };
    let original = import(1);
    let changed = import(2);
    let canonical = |document: &ProjectDocument, states: &BTreeMap<String, SavedModuleState>| {
        saved_content(
            document,
            serde_json::to_value(&document.layout).unwrap(),
            states,
        )
    };
    let baseline = SharedProjectSave::default();
    baseline
        .mark_initial(canonical(&original, &BTreeMap::new()))
        .unwrap();
    assert!(!baseline.matches_content(&canonical(&changed, &BTreeMap::new())));
    let mut zero = [0_u32; 8];
    let positive = BTreeMap::from([(
        "register".into(),
        SavedModuleState::ShiftRegister {
            values: zero,
            rng: 1,
        },
    )]);
    zero[0] = (-0.0f32).to_bits();
    let negative = BTreeMap::from([(
        "register".into(),
        SavedModuleState::ShiftRegister {
            values: zero,
            rng: 1,
        },
    )]);
    let changed_rng = BTreeMap::from([(
        "register".into(),
        SavedModuleState::ShiftRegister {
            values: zero,
            rng: 2,
        },
    )]);
    assert_ne!(
        canonical(&original, &positive),
        canonical(&original, &negative)
    );
    assert_ne!(
        canonical(&original, &negative),
        canonical(&original, &changed_rng)
    );
}
