use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

struct Backend {
    state: Mutex<SavedModuleState>,
    revision: Mutex<u64>,
    ready: Mutex<Vec<MidiCcLearnReceipt>>,
    armed: AtomicBool,
    processing: AtomicBool,
    fail_ack: AtomicBool,
    prepared: Mutex<Vec<String>>,
    loads: Mutex<Vec<SavedModuleState>>,
    releases: Mutex<Vec<Vec<String>>>,
}
impl Backend {
    fn new() -> Self {
        Self {
            state: Mutex::new(SavedModuleState::MidiCc {
                assignments: std::array::from_fn(|i| (i + 1) as i16),
                values: [0; 128],
            }),
            revision: Mutex::new(0),
            ready: Mutex::new(Vec::new()),
            armed: AtomicBool::new(false),
            processing: AtomicBool::new(false),
            fail_ack: AtomicBool::new(false),
            prepared: Mutex::new(Vec::new()),
            loads: Mutex::new(Vec::new()),
            releases: Mutex::new(Vec::new()),
        }
    }
    fn complete(&self, cell: u8, cc: i16, serial: u64) {
        let mut state = self.state.lock().unwrap();
        let SavedModuleState::MidiCc { assignments, .. } = &mut *state else {
            unreachable!()
        };
        let before = *assignments;
        for (i, old) in assignments.iter_mut().enumerate() {
            if i != cell as usize && *old == cc {
                *old = -1;
            }
        }
        assignments[cell as usize] = cc;
        self.ready.lock().unwrap().push(MidiCcLearnReceipt {
            id: "cc".into(),
            instance: 7,
            epoch: 3,
            frame: 128,
            sequence: serial,
            completion: openpatch_core::MidiCcLearnCompletion {
                serial,
                cell,
                before,
                after: *assignments,
            },
        });
        self.armed.store(false, Ordering::Release);
    }
    fn movement(&self, cc: usize, value: u8) {
        let SavedModuleState::MidiCc { values, .. } = &mut *self.state.lock().unwrap() else {
            unreachable!()
        };
        values[cc] = value;
    }
}
impl LiveModuleEvents for Backend {
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        unreachable!()
    }
    fn snapshot(&self) -> Value {
        let SavedModuleState::MidiCc {
            assignments,
            values,
        } = *self.state.lock().unwrap()
        else {
            unreachable!()
        };
        json!({"coherent":true,"modules":{"cc":{"kind":"midi_cc","available":true,"revision":*self.revision.lock().unwrap(),"frame":256,"state":{"assignments":assignments,"values":values.to_vec()}}}})
    }
    fn sync_patch(&self, _: &Patch, revision: u64) {
        *self.revision.lock().unwrap() = revision;
    }
    fn learn_midi_cc(&self, _: &str, cell: u8) -> Result<Value, String> {
        self.armed.store(true, Ordering::Release);
        Ok(json!({"cell":cell}))
    }
    fn cancel_midi_cc_learn(&self, _: &str) -> Result<Value, String> {
        self.prepare_midi_cc_edit(&["cc".into()])?;
        Ok(json!({"cancelled":true}))
    }
    fn midi_cc_learn_ready(&self) -> Vec<MidiCcLearnReceipt> {
        self.ready.lock().unwrap().clone()
    }
    fn acknowledge_midi_cc_learn(&self, r: &MidiCcLearnReceipt) -> Result<(), String> {
        if self.fail_ack.load(Ordering::Acquire) {
            return Err("ack retry".into());
        }
        self.ready
            .lock()
            .unwrap()
            .retain(|v| v.completion.serial != r.completion.serial);
        Ok(())
    }
    fn prepare_midi_cc_edit(&self, ids: &[String]) -> Result<(), String> {
        if self.processing.load(Ordering::Acquire) || !self.ready.lock().unwrap().is_empty() {
            return Err("decision busy; retry".into());
        }
        self.prepared.lock().unwrap().extend_from_slice(ids);
        self.armed.store(false, Ordering::Release);
        Ok(())
    }
    fn midi_cc_learning_active(&self) -> bool {
        self.armed.load(Ordering::Acquire) || self.midi_cc_learning_uncommitted()
    }
    fn midi_cc_learning_uncommitted(&self) -> bool {
        self.processing.load(Ordering::Acquire) || !self.ready.lock().unwrap().is_empty()
    }
    fn load_module_saved_states(&self, states: &BTreeMap<String, SavedModuleState>) {
        if let Some(state) = states.get("cc") {
            self.loads.lock().unwrap().push(*state);
            match *state {
                SavedModuleState::MidiCc { .. } => *self.state.lock().unwrap() = *state,
                SavedModuleState::MidiCcMappings(mappings) => {
                    let SavedModuleState::MidiCc { assignments, .. } =
                        &mut *self.state.lock().unwrap()
                    else {
                        unreachable!()
                    };
                    *assignments = mappings;
                }
                _ => unreachable!(),
            }
        }
    }
    fn release_module_controls(&self, ids: &[String]) {
        self.releases.lock().unwrap().push(ids.to_vec());
    }
}
fn setup() -> (Session, Arc<Backend>) {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    s.execute("add_module", json!({"id":"cc","kind":"midi_cc"}))
        .unwrap();
    let backend = Arc::new(Backend::new());
    s.set_live_module_events(Some(backend.clone())).unwrap();
    backend.prepared.lock().unwrap().clear();
    (s, backend)
}

fn grouped_mapping(mapping: i16) -> Value {
    json!({"id":"cc","parameter":"cc1","value":mapping,"history_group":"saved-state-gesture"})
}

fn grouped_mapping_and_value(mapping: i16, value: u8) -> Value {
    json!({"history_group":"saved-state-gesture","history_parameter":{"id":"cc","parameter":"cc1"},"edits":[
        {"tool":"set_parameter","arguments":{"id":"cc","parameter":"cc1","value":mapping}},
        {"tool":"set_parameter","arguments":{"id":"cc","parameter":"cc_value7","value":value}}
    ]})
}

fn fill_controller_bank(backend: &Backend) -> SavedModuleState {
    for controller in 0..128 {
        backend.movement(controller, controller as u8);
    }
    *backend.state.lock().unwrap()
}

#[test]
fn grouped_mapping_then_full_state_keeps_exact_undo_and_redo_checkpoints() {
    let (mut s, b) = setup();
    let original = fill_controller_bank(&b);
    let undo_steps = s.undo.len();
    s.execute("set_parameter", grouped_mapping(20)).unwrap();
    let mapped = *b.state.lock().unwrap();
    let mapping_state_id = s.state_id();
    s.execute("apply_edits", grouped_mapping_and_value(21, 99))
        .unwrap();
    let final_state = *b.state.lock().unwrap();
    assert_ne!(s.state_id(), mapping_state_id);
    assert_eq!(s.undo.len(), undo_steps + 2);
    for expected in [mapped, original] {
        s.execute("undo", json!({})).unwrap();
        assert_eq!(*b.state.lock().unwrap(), expected);
    }
    for expected in [mapped, final_state] {
        s.execute("redo", json!({})).unwrap();
        assert_eq!(*b.state.lock().unwrap(), expected);
    }
}

#[test]
fn grouped_full_state_then_mapping_keeps_exact_undo_and_redo_checkpoints() {
    let (mut s, b) = setup();
    let original = fill_controller_bank(&b);
    let undo_steps = s.undo.len();
    s.execute("apply_edits", grouped_mapping_and_value(20, 99))
        .unwrap();
    let full = *b.state.lock().unwrap();
    let full_state_id = s.state_id();
    s.execute("set_parameter", grouped_mapping(21)).unwrap();
    let final_state = *b.state.lock().unwrap();
    assert_ne!(s.state_id(), full_state_id);
    assert_eq!(s.undo.len(), undo_steps + 2);
    for expected in [full, original] {
        s.execute("undo", json!({})).unwrap();
        assert_eq!(*b.state.lock().unwrap(), expected);
    }
    for expected in [full, final_state] {
        s.execute("redo", json!({})).unwrap();
        assert_eq!(*b.state.lock().unwrap(), expected);
    }
}

#[test]
fn compatible_mapping_or_full_saved_state_gestures_still_coalesce() {
    for full in [false, true] {
        let (mut s, b) = setup();
        let original = fill_controller_bank(&b);
        let undo_steps = s.undo.len();
        for (mapping, value) in [(20, 99), (21, 111)] {
            if full {
                s.execute("apply_edits", grouped_mapping_and_value(mapping, value))
                    .unwrap();
            } else {
                s.execute("set_parameter", grouped_mapping(mapping))
                    .unwrap();
            }
        }
        let final_state = *b.state.lock().unwrap();
        assert_eq!(s.undo.len(), undo_steps + 1);
        s.execute("undo", json!({})).unwrap();
        assert_eq!(*b.state.lock().unwrap(), original);
        s.execute("redo", json!({})).unwrap();
        assert_eq!(*b.state.lock().unwrap(), final_state);
    }
}

#[test]
fn adding_or_removing_saved_module_coverage_starts_another_checkpoint() {
    for mapping_first in [false, true] {
        let (mut s, b) = setup();
        let original = fill_controller_bank(&b);
        let undo_steps = s.undo.len();
        let mut states = Vec::new();
        let mut patches = Vec::new();
        for (first, smooth) in [(true, 0), (false, 1)] {
            let mut request = json!({"history_group":"saved-state-gesture","history_parameter":{"id":"cc","parameter":"smooth"},"edits":[
                {"tool":"set_parameter","arguments":{"id":"cc","parameter":"smooth","value":smooth}}
            ]});
            if first == mapping_first {
                request["edits"].as_array_mut().unwrap().push(json!({
                    "tool":"set_parameter","arguments":{"id":"cc","parameter":"cc1","value":20}
                }));
            }
            s.execute("apply_edits", request).unwrap();
            states.push(*b.state.lock().unwrap());
            patches.push(s.patch.reference_view());
        }
        assert_eq!(s.undo.len(), undo_steps + 2);
        s.execute("undo", json!({})).unwrap();
        assert_eq!(*b.state.lock().unwrap(), states[0]);
        assert_eq!(s.patch.reference_view(), patches[0]);
        s.execute("undo", json!({})).unwrap();
        assert_eq!(*b.state.lock().unwrap(), original);
        for (expected_state, expected_patch) in states.into_iter().zip(patches) {
            s.execute("redo", json!({})).unwrap();
            assert_eq!(*b.state.lock().unwrap(), expected_state);
            assert_eq!(s.patch.reference_view(), expected_patch);
        }
    }
}

#[test]
fn changing_release_obligations_keeps_both_history_transitions() {
    let (mut s, b) = setup();
    s.execute("add_module", json!({"id":"other","kind":"constant"}))
        .unwrap();
    let original = s.patch.reference_view();
    let undo_steps = s.undo.len();
    let group = Some(("release-gesture".into(), "cc".into(), "smooth".into()));
    let mut patches = Vec::new();
    // Supply explicit effect receipts to exercise the generic commit boundary;
    // ordinary parameter gestures currently do not request control releases.
    for (id, value) in [("cc", 0), ("other", 1)] {
        let mut candidate = s.patch.clone();
        let mut effects = EditEffects::default();
        s.apply_session_edit(
            &mut candidate,
            "set_parameter",
            &json!({"id":"cc","parameter":"smooth","value":value}),
            &mut effects,
        )
        .unwrap();
        effects.release_controls.insert(id.into());
        s.commit(candidate, None, group.clone(), &effects).unwrap();
        patches.push(s.patch.reference_view());
    }
    assert_eq!(s.undo.len(), undo_steps + 2);
    b.releases.lock().unwrap().clear();
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.patch.reference_view(), patches[0]);
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.patch.reference_view(), original);
    s.execute("redo", json!({})).unwrap();
    assert_eq!(s.patch.reference_view(), patches[0]);
    s.execute("redo", json!({})).unwrap();
    assert_eq!(s.patch.reference_view(), patches[1]);
    assert_eq!(
        *b.releases.lock().unwrap(),
        vec![
            vec!["other".to_owned()],
            vec!["cc".to_owned()],
            vec!["cc".to_owned()],
            vec!["other".to_owned()]
        ]
    );
}

#[test]
fn assignment_collision_and_undo_preserve_controller_motion() {
    let (mut s, b) = setup();
    b.movement(7, 31);
    s.execute(
        "set_parameter",
        json!({"id":"cc","parameter":"cc1","value":2}),
    )
    .unwrap();
    assert_eq!(s.patch.nodes[0].params["cc1"], 2.0);
    assert_eq!(s.patch.nodes[0].params["cc2"], -1.0);
    b.movement(7, 99);
    s.execute("undo", json!({})).unwrap();
    assert!(matches!(
        b.loads.lock().unwrap().last(),
        Some(SavedModuleState::MidiCcMappings(_))
    ));
    let SavedModuleState::MidiCc {
        assignments,
        values,
    } = *b.state.lock().unwrap()
    else {
        unreachable!()
    };
    assert_eq!(assignments[0..2], [1, 2]);
    assert_eq!(values[7], 99);
    s.execute("redo", json!({})).unwrap();
    assert_eq!(
        s.capture_project_patch(None).unwrap().patch.nodes[0].params["cc_value7"],
        99.0
    );
}
#[test]
fn invalid_and_stale_edits_preserve_armed_and_unrelated_edits_do_not_prepare() {
    let (mut s, b) = setup();
    s.execute("learn_midi_cc", json!({"id":"cc","cell":1}))
        .unwrap();
    for args in [
        json!({"id":"cc","parameter":"cc1","value":128}),
        json!({"id":"cc","parameter":"cc1","value":2,"expected_revision":0}),
    ] {
        assert!(s.execute("set_parameter", args).is_err());
        assert!(b.armed.load(Ordering::Acquire));
    }
    s.execute("add_module", json!({"id":"other","kind":"constant"}))
        .unwrap();
    assert!(b.armed.load(Ordering::Acquire));
    assert!(b.prepared.lock().unwrap().is_empty());
    assert!(s
        .execute("learn_midi_cc", json!({"id":"cc","cell":17}))
        .is_err());
    s.execute(
        "set_parameter",
        json!({"id":"cc","parameter":"cc1","value":33}),
    )
    .unwrap();
    assert!(!b.armed.load(Ordering::Acquire));
}
#[test]
fn durable_ack_retry_records_once_and_capture_waits_for_reconciliation() {
    let (mut s, b) = setup();
    b.complete(0, 45, 1);
    b.fail_ack.store(true, Ordering::Release);
    assert!(s.capture_project_patch(None).is_err());
    assert!(s.reconcile_midi_cc_learn().is_err());
    let revision = s.revision();
    assert!(s.reconcile_midi_cc_learn().is_err());
    assert_eq!(s.revision(), revision);
    assert_eq!(s.patch.nodes[0].params["cc1"], 45.0);
    b.fail_ack.store(false, Ordering::Release);
    s.reconcile_midi_cc_learn().unwrap();
    assert_eq!(s.revision(), revision);
    assert!(b.ready.lock().unwrap().is_empty());
    b.movement(45, 100);
    s.execute("undo", json!({})).unwrap();
    let SavedModuleState::MidiCc {
        assignments,
        values,
    } = *b.state.lock().unwrap()
    else {
        unreachable!()
    };
    assert_eq!(assignments[0], 1);
    assert_eq!(values[45], 100);
}
#[test]
fn shared_worker_reconciles_without_gui_and_releases_weak_session() {
    let (s, b) = setup();
    let shared = s.into_shared();
    execute_shared(&shared, "learn_midi_cc", json!({"id":"cc","cell":1})).unwrap();
    b.complete(0, 70, 1);
    let deadline = Instant::now() + std::time::Duration::from_secs(2);
    while shared.lock().unwrap().revision() == 1 && Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(shared.lock().unwrap().patch.nodes[0].params["cc1"], 70.0);
    assert!(b.ready.lock().unwrap().is_empty());
    let weak = Arc::downgrade(&shared);
    drop(shared);
    // Unlocking the Session and dropping the worker's temporary Arc are
    // separate operations. Another thread can acquire the mutex between them.
    // Check eventual release, rather than requiring that scheduling race to end
    // before the caller drops its own Arc.
    let deadline = Instant::now() + std::time::Duration::from_secs(2);
    while weak.upgrade().is_some() {
        assert!(
            Instant::now() < deadline,
            "worker retained a dropped session"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
#[test]
fn processing_blocks_close_and_backend_replacement_without_erasing_decisions() {
    let (mut s, b) = setup();
    b.processing.store(true, Ordering::Release);
    assert!(s.set_live_module_events(None).is_err());
    assert!(s.live_module_events.is_some());
    assert!(s.seal_for_close(s.revision()).is_err());
    assert!(!s.closing);
    b.processing.store(false, Ordering::Release);
    b.complete(0, 50, 1);
    s.set_live_module_events(None).unwrap();
    assert_eq!(s.patch.nodes[0].params["cc1"], 50.0);
    assert!(b.ready.lock().unwrap().is_empty());
    assert!(s.live_module_events.is_none());
}
#[test]
fn full_preset_duplicate_initialize_and_shared_save_capture_live_values() {
    let (mut s, b) = setup();
    b.movement(17, 88);
    let preset = s
        .execute("capture_module_preset", json!({"id":"cc"}))
        .unwrap()["preset"]
        .clone();
    assert_eq!(preset["params"]["cc_value17"], 88.0);
    s.execute(
        "duplicate_modules",
        json!({"modules":[{"id":"cc","new_id":"copy"}]}),
    )
    .unwrap();
    assert_eq!(s.patch.nodes[1].params["cc_value17"], 88.0);
    b.movement(17, 99);
    s.execute("apply_module_preset", json!({"id":"cc","preset":preset}))
        .unwrap();
    let SavedModuleState::MidiCc { values, .. } = *b.state.lock().unwrap() else {
        unreachable!()
    };
    assert_eq!(values[17], 88);
    s.execute("initialize_modules", json!({"ids":["cc"]}))
        .unwrap();
    let SavedModuleState::MidiCc { values, .. } = *b.state.lock().unwrap() else {
        unreachable!()
    };
    assert_eq!(values, [0; 128]);
}
#[test]
fn batch_maps_duplicate_current_mapping_and_controller_values_without_full_restore_on_original() {
    let (mut s, b) = setup();
    b.movement(90, 60);
    s.execute(
        "apply_edits",
        json!({"edits":[
            {"tool":"set_parameter","arguments":{"id":"cc","parameter":"cc1","value":90}},
            {"tool":"duplicate_modules","arguments":{"modules":[{"id":"cc","new_id":"copy"}]}},
            {"tool":"set_parameter","arguments":{"id":"cc","parameter":"cc2","value":91}}
        ]}),
    )
    .unwrap();
    assert_eq!(s.patch.nodes[1].params["cc1"], 90.0);
    assert_eq!(s.patch.nodes[1].params["cc_value90"], 60.0);
    b.movement(90, 77);
    s.execute("undo", json!({})).unwrap();
    let SavedModuleState::MidiCc { values, .. } = *b.state.lock().unwrap() else {
        unreachable!()
    };
    assert_eq!(values[90], 77);
    s.execute("redo", json!({})).unwrap();
    let SavedModuleState::MidiCc { values, .. } = *b.state.lock().unwrap() else {
        unreachable!()
    };
    assert_eq!(values[90], 77);
}
#[test]
fn shared_save_reconciles_original_and_writes_live_values() {
    let workspace = std::env::temp_dir().join(format!(
        "openpatch-midi-cc-save-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut s = Session::new(&workspace).unwrap();
    s.execute("add_module", json!({"id":"cc","kind":"midi_cc"}))
        .unwrap();
    let b = Arc::new(Backend::new());
    s.set_live_module_events(Some(b.clone())).unwrap();
    b.movement(77, 101);
    b.complete(0, 77, 1);
    let shared = s.into_shared();
    execute_shared(&shared, "save_patch", json!({"filename":"saved.json"})).unwrap();
    assert_eq!(shared.lock().unwrap().revision(), 2);
    assert_eq!(shared.lock().unwrap().patch.nodes[0].params["cc1"], 77.0);
    let saved: Patch =
        serde_json::from_slice(&fs::read(workspace.join("saved.json")).unwrap()).unwrap();
    assert_eq!(saved.nodes[0].params["cc_value77"], 101.0);
    assert_eq!(saved.nodes[0].params["cc1"], 77.0);
    drop(shared);
    fs::remove_dir_all(workspace).unwrap();
}
#[test]
fn failed_commit_keeps_ready_and_history_unchanged_until_retry() {
    let (mut s, b) = setup();
    s.revision = u64::MAX;
    b.complete(0, 55, 1);
    let before = s.snapshot();
    assert!(s
        .reconcile_midi_cc_learn()
        .unwrap_err()
        .contains("revision exhausted"));
    assert_eq!(s.snapshot(), before);
    assert_eq!(b.ready.lock().unwrap().len(), 1);
    s.revision = 1;
    s.reconcile_midi_cc_learn().unwrap();
    assert_eq!(s.patch.nodes[0].params["cc1"], 55.0);
    assert!(b.ready.lock().unwrap().is_empty());
}
#[test]
fn live_state_parser_rejects_malformed_controller_banks_and_map_arrays() {
    let (s, b) = setup();
    let node = &s.patch.nodes[0];
    let expected = SavedModuleState::from_node(node).unwrap();
    let baseline = b.snapshot();
    for (field, value) in [("values", json!([0, 1])), ("assignments", json!([1, 2]))] {
        let mut snapshot = baseline.clone();
        snapshot["modules"]["cc"]["state"][field] = value;
        assert!(running::live_saved(&snapshot, "cc", 1, expected).is_err());
    }
    for (field, index, value) in [
        ("values", 0, json!(128)),
        ("values", 1, json!(-1)),
        ("values", 2, json!(1.5)),
        ("assignments", 0, json!(128)),
        ("assignments", 1, json!(1)),
    ] {
        let mut snapshot = baseline.clone();
        snapshot["modules"]["cc"]["state"][field][index] = value;
        assert!(running::live_saved(&snapshot, "cc", 1, expected).is_err());
    }
}

#[test]
fn disabled_randomization_preserves_bank_and_history() {
    let (mut s, b) = setup();
    b.movement(17, 80);
    let before = s.snapshot();
    s.execute("randomize_modules", json!({"ids":["cc"],"seed":42}))
        .unwrap();
    assert_eq!(s.snapshot(), before);
    assert!(b.loads.lock().unwrap().is_empty());
    assert_eq!(
        s.capture_project_patch(None).unwrap().patch.nodes[0].params["cc_value17"],
        80.0
    );
}
