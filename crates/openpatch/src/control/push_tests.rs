use super::*;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[derive(Default)]
struct Live {
    states: Mutex<BTreeMap<String, SavedModuleState>>,
    revision: AtomicU64,
    reads: AtomicUsize,
    loads: Mutex<Vec<(String, SavedModuleState)>>,
    override_snapshot: Mutex<Option<Value>>,
}
impl LiveModuleEvents for Live {
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        unreachable!()
    }
    fn snapshot(&self) -> Value {
        self.reads.fetch_add(1, Ordering::Relaxed);
        if let Some(value) = self.override_snapshot.lock().unwrap().as_ref() {
            return value.clone();
        }
        let modules: serde_json::Map<String, Value> = self.states.lock().unwrap().iter().map(|(id, state)| {
            (id.clone(), json!({"kind":state.kind(),"available":true,"revision":self.revision.load(Ordering::Relaxed),"frame":512,"state":{state.parameter():state.value()}}))
        }).collect();
        json!({"coherent":true,"frame":512,"modules":modules})
    }
    fn sync_patch(&self, patch: &Patch, revision: u64) {
        let mut states = self.states.lock().unwrap();
        states.retain(|id, state| {
            patch
                .nodes
                .iter()
                .any(|n| n.id == *id && n.kind == state.kind())
        });
        for node in &patch.nodes {
            if let Some(state) = SavedModuleState::from_node(node) {
                states.entry(node.id.clone()).or_insert(state);
            }
        }
        self.revision.store(revision, Ordering::Relaxed);
    }
    fn retire_modules(&self, ids: &[String]) {
        let mut states = self.states.lock().unwrap();
        for id in ids {
            states.remove(id);
        }
    }
    fn load_module_running(&self, values: &BTreeMap<String, bool>) {
        self.load(values, SavedModuleState::Seq3Run(false));
    }
    fn load_module_holding(&self, values: &BTreeMap<String, bool>) {
        self.load(values, SavedModuleState::PushHold(false));
    }
}
impl Live {
    fn load(&self, values: &BTreeMap<String, bool>, tag: SavedModuleState) {
        let mut states = self.states.lock().unwrap();
        for (id, value) in values {
            if let Some(current) = states.get_mut(id).filter(|s| s.kind() == tag.kind()) {
                *current = tag.with_value(*value);
                self.loads.lock().unwrap().push((id.clone(), *current));
            }
        }
    }
    fn holding(&self, id: &str) -> bool {
        assert_eq!(self.states.lock().unwrap()[id].kind(), "push");
        self.states.lock().unwrap()[id].value()
    }
    fn hold(&self, id: &str, value: bool) {
        self.states
            .lock()
            .unwrap()
            .insert(id.into(), SavedModuleState::PushHold(value));
    }
}
fn fixture() -> (Session, Arc<Live>) {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    s.execute(
        "replace_patch",
        json!({"patch":{"version":2,"nodes":[
            {"id":"push","kind":"push","params":{"holding":0}},
            {"id":"seq","kind":"seq3","params":{"running":1}},
            {"id":"tone","kind":"analog_vco"}
        ]}}),
    )
    .unwrap();
    let live = Arc::new(Live::default());
    s.set_live_module_events(Some(live.clone()));
    live.hold("push", true);
    live.states
        .lock()
        .unwrap()
        .insert("seq".into(), SavedModuleState::Seq3Run(false));
    (s, live)
}
#[test]
fn mixed_project_capture_uses_one_frame_and_projects_actual_flags_without_mutation() {
    let (mut s, live) = fixture();
    let before = s.snapshot();
    let reads = live.reads.load(Ordering::Relaxed);
    let capture = s.capture_project_patch(Some(s.revision())).unwrap();
    assert_eq!(live.reads.load(Ordering::Relaxed), reads + 1);
    assert_eq!(capture.running, BTreeMap::from([("seq".into(), false)]));
    assert_eq!(capture.holding, BTreeMap::from([("push".into(), true)]));
    assert_eq!(capture.patch.nodes[0].params["holding"], 1.0);
    assert_eq!(capture.patch.nodes[1].params["running"], 0.0);
    assert_eq!(capture.frame, Some(512));
    assert_eq!(s.snapshot(), before);
    let wire = s.execute("capture_project_patch", json!({})).unwrap();
    assert_eq!(wire["running"], json!({"seq":false}));
    assert_eq!(wire["holding"], json!({"push":true}));
    let original = live.snapshot();
    for bad in [
        "unavailable",
        "stale",
        "missing",
        "mixed_frame",
        "incoherent",
        "wrong_kind",
    ] {
        let mut invalid = original.clone();
        match bad {
            "unavailable" => invalid["modules"]["push"]["available"] = json!(false),
            "stale" => invalid["modules"]["push"]["revision"] = json!(0),
            "missing" => {
                invalid["modules"]["push"]["state"] = json!({});
            }
            "mixed_frame" => invalid["modules"]["push"]["frame"] = json!(511),
            "incoherent" => invalid["coherent"] = json!(false),
            _ => invalid["modules"]["push"]["kind"] = json!("seq3"),
        }
        *live.override_snapshot.lock().unwrap() = Some(invalid);
        assert!(s.capture_project_patch(None).is_err(), "{bad}");
        assert_eq!(s.snapshot(), before);
    }
    *live.override_snapshot.lock().unwrap() = None;
    s.set_live_module_events(None);
    let offline = s.capture_project_patch(None).unwrap();
    assert_eq!(offline.state_source, "offline_reset");
    assert_eq!(offline.frame, None);
    assert!(!offline.holding["push"]);
    assert!(offline.running["seq"]);
}
#[test]
fn equal_configured_assignment_and_grouped_history_restore_actual_hold() {
    let (mut s, live) = fixture();
    s.execute(
        "set_parameter",
        json!({"id":"push","parameter":"holding","value":0,"history_group":"gesture"}),
    )
    .unwrap();
    assert!(!live.holding("push"));
    assert_eq!(live.loads.lock().unwrap().len(), 1);
    s.execute(
        "set_parameter",
        json!({"id":"push","parameter":"holding","value":1,"history_group":"gesture"}),
    )
    .unwrap();
    s.execute(
        "set_parameter",
        json!({"id":"push","parameter":"holding","value":0,"history_group":"gesture"}),
    )
    .unwrap();
    s.execute("undo", json!({})).unwrap();
    assert!(live.holding("push"));
    s.execute("redo", json!({})).unwrap();
    assert!(!live.holding("push"));
    assert_eq!(
        live.states.lock().unwrap()["seq"],
        SavedModuleState::Seq3Run(false)
    );
}
#[test]
fn initialize_resets_hold_randomize_preserves_it_and_history_keeps_seq_policy() {
    let (mut s, live) = fixture();
    s.execute(
        "randomize_modules",
        json!({"ids":["push","seq"],"seed":123}),
    )
    .unwrap();
    assert!(live.holding("push"));
    s.execute("initialize_modules", json!({"ids":["push","seq"]}))
        .unwrap();
    assert!(!live.holding("push"));
    assert_eq!(
        live.states.lock().unwrap()["seq"],
        SavedModuleState::Seq3Run(false)
    );
    s.execute("undo", json!({})).unwrap();
    assert!(live.holding("push"));
    s.execute("redo", json!({})).unwrap();
    assert!(!live.holding("push"));
}
#[test]
fn presets_duplicate_and_ordered_transactions_capture_actual_hold() {
    let (mut s, live) = fixture();
    let preset = s
        .execute("capture_module_preset", json!({"id":"push"}))
        .unwrap();
    assert_eq!(preset["preset"]["params"]["holding"], 1.0);
    s.execute("apply_module_preset",json!({"id":"push","preset":{"format":"openpatch-module-preset","version":1,"kind":"push","params":{}}})).unwrap();
    assert!(live.holding("push"));
    assert!(live.loads.lock().unwrap().is_empty());
    s.execute(
        "duplicate_modules",
        json!({"modules":[{"id":"push","new_id":"copy"}]}),
    )
    .unwrap();
    assert!(live.holding("copy"));
    assert_eq!(
        s.patch
            .nodes
            .iter()
            .find(|n| n.id == "copy")
            .unwrap()
            .params["holding"],
        1.0
    );
    s.execute(
        "apply_edits",
        json!({"edits":[
            {"tool":"set_parameter","arguments":{"id":"push","parameter":"holding","value":0}},
            {"tool":"duplicate_modules","arguments":{"modules":[{"id":"push","new_id":"after"}]}},
            {"tool":"set_parameter","arguments":{"id":"push","parameter":"holding","value":1}}
        ]}),
    )
    .unwrap();
    assert!(live.holding("push"));
    assert!(!live.holding("after"));
    s.execute("undo", json!({})).unwrap();
    assert!(live.holding("push"));
    s.execute("redo", json!({})).unwrap();
    assert!(live.holding("push"));
    assert!(!live.holding("after"));
    let mut explicit = preset["preset"].clone();
    explicit["params"]["holding"] = json!(0);
    s.execute(
        "apply_module_preset",
        json!({"id":"push","preset":explicit}),
    )
    .unwrap();
    assert!(!live.holding("push"));
    s.execute("undo", json!({})).unwrap();
    assert!(live.holding("push"));
}
#[test]
fn remove_readd_and_failed_batch_do_not_reuse_latch_or_apply_effects() {
    let (mut s, live) = fixture();
    let before = s.snapshot();
    let loads = live.loads.lock().unwrap().clone();
    assert!(s
        .execute(
            "apply_edits",
            json!({"edits":[
                {"tool":"set_parameter","arguments":{"id":"push","parameter":"holding","value":0}},
                {"tool":"set_parameter","arguments":{"id":"push","parameter":"holding","value":0.5}}
            ]})
        )
        .is_err());
    assert_eq!(s.snapshot(), before);
    assert!(live.holding("push"));
    assert_eq!(*live.loads.lock().unwrap(), loads);
    s.execute(
        "apply_edits",
        json!({"edits":[
            {"tool":"remove_module","arguments":{"id":"push"}},
            {"tool":"add_module","arguments":{"id":"push","kind":"push"}}
        ]}),
    )
    .unwrap();
    assert!(!live.holding("push"));
    s.execute("undo", json!({})).unwrap();
    assert!(live.holding("push"));
    s.execute("redo", json!({})).unwrap();
    assert!(!live.holding("push"));
}
