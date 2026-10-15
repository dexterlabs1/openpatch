use super::*;
use openpatch_core::{MidiLearnCompletion, MidiLearnKind};
use std::sync::atomic::{AtomicBool, Ordering};

struct Backend {
    states: Mutex<BTreeMap<String, SavedModuleState>>,
    kinds: Mutex<BTreeMap<String, MidiLearnKind>>,
    revision: Mutex<u64>,
    ready: Mutex<Vec<MidiLearnReceipt>>,
    armed: Mutex<BTreeSet<String>>,
    fail_ack: AtomicBool,
    loads: Mutex<Vec<(String, SavedModuleState)>>,
    actions: Mutex<Vec<ModuleAction>>,
}
impl Backend {
    fn complete(&self, id: &str, cell: u8, note: i16, sequence: u64) {
        let kind = self.kinds.lock().unwrap()[id];
        let mut states = self.states.lock().unwrap();
        let state = states.get_mut(id).unwrap();
        let (before, after) = match state {
            SavedModuleState::MidiGateMappings(maps)
            | SavedModuleState::MidiCcMappings(maps)
            | SavedModuleState::MidiCc {
                assignments: maps, ..
            } => {
                let before = *maps;
                for (index, old) in maps.iter_mut().enumerate() {
                    if index != usize::from(cell) && *old == note {
                        *old = -1;
                    }
                }
                maps[usize::from(cell)] = note;
                (before, *maps)
            }
            _ => unreachable!(),
        };
        self.armed.lock().unwrap().remove(id);
        self.ready.lock().unwrap().push(MidiLearnReceipt {
            kind,
            backend: 41,
            id: id.into(),
            instance: 7,
            epoch: 3,
            frame: 128,
            sequence,
            completion: MidiLearnCompletion {
                serial: sequence,
                cell,
                before,
                after,
            },
        });
    }
}
impl LiveModuleEvents for Backend {
    fn trigger(&self, _: &str, action: ModuleAction) -> Result<Value, String> {
        self.actions.lock().unwrap().push(action);
        Ok(json!({"queued":true}))
    }
    fn snapshot(&self) -> Value {
        let states = self.states.lock().unwrap();
        let revision = *self.revision.lock().unwrap();
        let modules:serde_json::Map<String,Value>=states.iter().map(|(id,state)| {
            let state=match *state {
                SavedModuleState::MidiGateMappings(maps)|SavedModuleState::MidiCcMappings(maps)=>json!({"assignments":maps}),
                SavedModuleState::MidiCc {assignments,values}=>json!({"assignments":assignments,"values":values.to_vec()}),
                _=>unreachable!(),
            };
            (id.clone(),json!({"kind":self.kinds.lock().unwrap()[id].module_kind(),"revision":revision,"frame":256,"available":true,"state":state}))
        }).collect();
        json!({"coherent":true,"modules":modules})
    }
    fn sync_patch(&self, patch: &Patch, revision: u64) {
        *self.revision.lock().unwrap() = revision;
        for node in &patch.nodes {
            if let Some(kind) = MidiLearnKind::from_module_kind(&node.kind) {
                self.kinds.lock().unwrap().insert(node.id.clone(), kind);
                self.states
                    .lock()
                    .unwrap()
                    .entry(node.id.clone())
                    .or_insert_with(|| SavedModuleState::from_node(node).unwrap());
            }
        }
    }
    fn learn_midi(&self, kind: MidiLearnKind, id: &str, _: u8) -> Result<Value, String> {
        if self.kinds.lock().unwrap().get(id) != Some(&kind) {
            return Err("wrong kind".into());
        }
        self.armed.lock().unwrap().insert(id.into());
        Ok(json!({"kind":kind,"backend":41}))
    }
    fn cancel_midi_learn(&self, kind: MidiLearnKind, id: &str) -> Result<Value, String> {
        if self.kinds.lock().unwrap().get(id) != Some(&kind) {
            return Err("wrong kind".into());
        }
        self.prepare_midi_learn_edit(&[id.into()])?;
        Ok(json!({"cancelled":true}))
    }
    fn midi_learn_ready(&self) -> Vec<MidiLearnReceipt> {
        self.ready.lock().unwrap().clone()
    }
    fn acknowledge_midi_learn(&self, receipt: &MidiLearnReceipt) -> Result<(), String> {
        if self.fail_ack.load(Ordering::Acquire) {
            return Err("ack retry".into());
        }
        if receipt.backend != 41
            || self.kinds.lock().unwrap().get(&receipt.id) != Some(&receipt.kind)
        {
            return Err("stale identity".into());
        }
        self.ready.lock().unwrap().retain(|other| {
            !(other.id == receipt.id
                && other.kind == receipt.kind
                && other.completion.serial == receipt.completion.serial)
        });
        Ok(())
    }
    fn prepare_midi_learn_edit(&self, ids: &[String]) -> Result<(), String> {
        if self
            .ready
            .lock()
            .unwrap()
            .iter()
            .any(|receipt| ids.contains(&receipt.id))
        {
            return Err("decision awaits reconciliation".into());
        }
        for id in ids {
            self.armed.lock().unwrap().remove(id);
        }
        Ok(())
    }
    fn midi_learning_active(&self) -> bool {
        !self.armed.lock().unwrap().is_empty() || self.midi_learning_uncommitted()
    }
    fn midi_learning_uncommitted(&self) -> bool {
        !self.ready.lock().unwrap().is_empty()
    }
    fn load_module_saved_states(&self, states: &BTreeMap<String, SavedModuleState>) {
        for (id, state) in states {
            self.loads.lock().unwrap().push((id.clone(), *state));
            let mut current = self.states.lock().unwrap();
            if let SavedModuleState::MidiCcMappings(maps) = *state {
                if let Some(SavedModuleState::MidiCc { assignments, .. }) = current.get_mut(id) {
                    *assignments = maps;
                }
            } else {
                current.insert(id.clone(), *state);
            }
        }
    }
}
fn setup() -> (Session, Arc<Backend>) {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    s.execute("apply_edits",json!({"edits":[{"tool":"add_module","arguments":{"id":"cc","kind":"midi_cc"}},{"tool":"add_module","arguments":{"id":"gate","kind":"midi_gate"}}]})).unwrap();
    let b = Arc::new(Backend {
        states: Mutex::new(BTreeMap::new()),
        kinds: Mutex::new(BTreeMap::new()),
        revision: Mutex::new(0),
        ready: Mutex::new(Vec::new()),
        armed: Mutex::new(BTreeSet::new()),
        fail_ack: AtomicBool::new(false),
        loads: Mutex::new(Vec::new()),
        actions: Mutex::new(Vec::new()),
    });
    s.set_live_module_events(Some(b.clone())).unwrap();
    (s, b)
}
#[test]
fn simultaneous_cc_gate_completions_share_sequence_order_and_one_history_step_each() {
    let (mut s, b) = setup();
    s.execute("learn_midi_cc", json!({"id":"cc","cell":1}))
        .unwrap();
    s.execute("learn_midi_gate", json!({"id":"gate","cell":1}))
        .unwrap();
    b.complete("cc", 0, 80, 2);
    b.complete("gate", 0, 81, 1);
    s.reconcile_midi_learn().unwrap();
    assert_eq!(s.revision(), 3);
    assert_eq!(s.patch.nodes[0].params["cc1"], 80.0);
    assert_eq!(s.patch.nodes[1].params["note1"], 81.0);
    s.execute("undo", json!({})).unwrap();
    assert_eq!(midi_cc::node_mappings(&s.patch.nodes[0]).unwrap()[0], 1);
    assert_eq!(s.patch.nodes[1].params["note1"], 81.0);
    s.execute("undo", json!({})).unwrap();
    assert_eq!(midi_cc::node_mappings(&s.patch.nodes[1]).unwrap()[0], 48);
    assert!(b.loads.lock().unwrap().iter().all(|(_, state)| matches!(
        state,
        SavedModuleState::MidiCcMappings(_) | SavedModuleState::MidiGateMappings(_)
    )));
}
#[test]
fn generic_ack_retry_is_idempotent_and_wrong_kind_cannot_target_replacement() {
    let (mut s, b) = setup();
    b.complete("gate", 0, 75, 1);
    b.fail_ack.store(true, Ordering::Release);
    assert!(s.reconcile_midi_learn().is_err());
    let revision = s.revision();
    assert!(s.reconcile_midi_learn().is_err());
    assert_eq!(s.revision(), revision);
    b.fail_ack.store(false, Ordering::Release);
    s.reconcile_midi_learn().unwrap();
    assert_eq!(s.revision(), revision);
    let maps = midi_cc::node_mappings(&s.patch.nodes[1]).unwrap();
    let receipt = MidiLearnReceipt {
        kind: MidiLearnKind::Cc,
        backend: 41,
        id: "gate".into(),
        instance: 7,
        epoch: 3,
        frame: 128,
        sequence: 2,
        completion: MidiLearnCompletion {
            serial: 2,
            cell: 0,
            before: maps,
            after: maps,
        },
    };
    b.ready.lock().unwrap().push(receipt);
    let before = s.snapshot();
    assert!(s.reconcile_midi_learn().is_err());
    assert_eq!(s.snapshot(), before);
    let gate_receipt = MidiLearnReceipt {
        kind: MidiLearnKind::Gate,
        backend: 42,
        ..b.ready.lock().unwrap()[0].clone()
    };
    assert!(b.acknowledge_midi_learn(&gate_receipt).is_err());
}
#[test]
fn direct_collision_preset_capture_duplicate_and_undo_use_normalized_note_maps() {
    let (mut s, b) = setup();
    s.execute(
        "set_parameter",
        json!({"id":"gate","parameter":"note16","value":48}),
    )
    .unwrap();
    assert_eq!(s.patch.nodes[1].params["note1"], -1.0);
    assert_eq!(s.patch.nodes[1].params["note16"], 48.0);
    let capture = s
        .execute("capture_module_preset", json!({"id":"gate"}))
        .unwrap();
    assert_eq!(capture["preset"]["params"]["note1"], -1.0);
    s.execute("apply_module_preset",json!({"id":"gate","preset":{"format":"openpatch-module-preset","version":1,"kind":"midi_gate","params":{"note1":60,"note2":60}}})).unwrap();
    assert_eq!(s.patch.nodes[1].params["note1"], -1.0);
    assert_eq!(s.patch.nodes[1].params["note2"], 60.0);
    s.execute(
        "duplicate_modules",
        json!({"modules":[{"id":"gate","new_id":"copy"}]}),
    )
    .unwrap();
    assert_eq!(s.patch.nodes[2].params["note1"], -1.0);
    assert_eq!(s.patch.nodes[2].params["note2"], 60.0);
    let saved = s.capture_project_patch(None).unwrap();
    assert_eq!(saved.patch.nodes[2].params["note2"], 60.0);
    s.execute("initialize_modules", json!({"ids":["gate"]}))
        .unwrap();
    assert_eq!(midi_cc::node_mappings(&s.patch.nodes[1]).unwrap()[0], 48);
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.patch.nodes[1].params["note2"], 60.0);
    assert!(b
        .loads
        .lock()
        .unwrap()
        .iter()
        .filter(|(id, _)| id == "gate")
        .all(|(_, state)| matches!(state, SavedModuleState::MidiGateMappings(_))));
}
#[test]
fn invalid_stale_wrong_kind_edits_preserve_learn_and_panic_works_bypassed() {
    let (mut s, b) = setup();
    s.execute("learn_midi_gate", json!({"id":"gate","cell":1}))
        .unwrap();
    for (name, args) in [
        (
            "set_parameter",
            json!({"id":"gate","parameter":"note1","value":128}),
        ),
        (
            "set_parameter",
            json!({"id":"gate","parameter":"note1","value":50,"expected_revision":0}),
        ),
        ("learn_midi_gate", json!({"id":"cc","cell":1})),
        ("learn_midi_gate", json!({"id":"gate","cell":17})),
    ] {
        assert!(s.execute(name, args).is_err());
        assert!(b.armed.lock().unwrap().contains("gate"));
    }
    s.execute(
        "set_parameter",
        json!({"id":"cc","parameter":"cc1","value":40}),
    )
    .unwrap();
    assert!(b.armed.lock().unwrap().contains("gate"));
    s.execute("set_bypass", json!({"id":"gate","bypassed":true}))
        .unwrap();
    assert!(!b.armed.lock().unwrap().contains("gate"));
    let revision = s.revision();
    s.execute("trigger_module", json!({"id":"gate","action":"midi_panic"}))
        .unwrap();
    assert_eq!(s.revision(), revision);
    assert_eq!(*b.actions.lock().unwrap(), [ModuleAction::MidiPanic]);
    assert!(s
        .execute("trigger_module", json!({"id":"cc","action":"midi_panic"}))
        .is_err());
}
#[test]
fn one_shared_worker_reconciles_gate_and_cc_without_gui() {
    let (s, b) = setup();
    let shared = s.into_shared();
    execute_shared(&shared, "learn_midi_gate", json!({"id":"gate","cell":1})).unwrap();
    execute_shared(&shared, "learn_midi_cc", json!({"id":"cc","cell":1})).unwrap();
    b.complete("gate", 0, 90, 1);
    b.complete("cc", 0, 91, 2);
    let deadline = Instant::now() + std::time::Duration::from_secs(2);
    while shared.lock().unwrap().revision() != 3 && Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(shared.lock().unwrap().revision(), 3);
    assert!(b.ready.lock().unwrap().is_empty());
}

#[test]
fn legacy_cc_default_adapter_rejects_generic_wrong_kind_and_backend_without_ack() {
    struct Legacy(std::sync::atomic::AtomicU64);
    impl LiveModuleEvents for Legacy {
        fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
            unreachable!()
        }
        fn snapshot(&self) -> Value {
            json!({})
        }
        fn acknowledge_midi_cc_learn(&self, _: &MidiCcLearnReceipt) -> Result<(), String> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }
    let legacy = Legacy(std::sync::atomic::AtomicU64::new(0));
    let receipt = MidiLearnReceipt {
        kind: MidiLearnKind::Cc,
        backend: 0,
        id: "cc".into(),
        instance: 1,
        epoch: 1,
        frame: 0,
        sequence: 1,
        completion: MidiLearnCompletion {
            serial: 1,
            cell: 0,
            before: [-1; 16],
            after: [-1; 16],
        },
    };
    legacy.acknowledge_midi_learn(&receipt).unwrap();
    assert_eq!(legacy.0.load(Ordering::Relaxed), 1);
    assert!(legacy
        .acknowledge_midi_learn(&MidiLearnReceipt {
            backend: 1,
            ..receipt.clone()
        })
        .is_err());
    assert!(legacy
        .acknowledge_midi_learn(&MidiLearnReceipt {
            kind: MidiLearnKind::Gate,
            ..receipt
        })
        .is_err());
    assert_eq!(legacy.0.load(Ordering::Relaxed), 1);
}
