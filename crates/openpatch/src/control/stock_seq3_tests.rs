use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

#[derive(Default)]
struct Live {
    states: Mutex<BTreeMap<String, SavedModuleState>>,
    revision: AtomicU64,
    reads: AtomicUsize,
    loads: AtomicUsize,
    triggered: Mutex<Vec<ModuleAction>>,
    invalid: Mutex<Option<Value>>,
    edge_after_snapshot: AtomicBool,
}
impl LiveModuleEvents for Live {
    fn trigger(&self, _: &str, action: ModuleAction) -> Result<Value, String> {
        self.triggered.lock().unwrap().push(action);
        Ok(json!({"status":"queued","action":action}))
    }
    fn snapshot(&self) -> Value {
        self.reads.fetch_add(1, Ordering::Relaxed);
        if let Some(value) = self.invalid.lock().unwrap().as_ref() {
            return value.clone();
        }
        let states = self.states.lock().unwrap();
        let modules: serde_json::Map<_, _> = states.iter().map(|(id,state)| {
            let SavedModuleState::StockSeq3 { running,gates,passthrough } = state else { unreachable!() };
            (id.clone(),json!({"kind":"stock_seq3","available":true,"revision":self.revision.load(Ordering::Relaxed),"frame":512,"state":{
                "running":running,"gates":(0..8).map(|i| gates & (1 << i) != 0).collect::<Vec<_>>(),"passthrough":passthrough
            }}))
        }).collect();
        drop(states);
        if self.edge_after_snapshot.swap(false, Ordering::Relaxed) {
            let mut states = self.states.lock().unwrap();
            let SavedModuleState::StockSeq3 {
                running,
                gates,
                passthrough,
            } = states["seq"]
            else {
                unreachable!()
            };
            states.insert("seq".into(), flags(!running, gates ^ 2, passthrough));
        }
        json!({"coherent":true,"modules":modules})
    }
    fn sync_patch(&self, patch: &Patch, revision: u64) {
        let mut states = self.states.lock().unwrap();
        states.retain(|id, _| {
            patch
                .nodes
                .iter()
                .any(|n| n.id == *id && n.kind == "stock_seq3")
        });
        for node in patch.nodes.iter().filter(|n| n.kind == "stock_seq3") {
            states
                .entry(node.id.clone())
                .or_insert_with(|| SavedModuleState::from_node(node).unwrap());
        }
        self.revision.store(revision, Ordering::Relaxed);
    }
    fn load_module_saved_states(&self, values: &BTreeMap<String, SavedModuleState>) {
        self.loads.fetch_add(values.len(), Ordering::Relaxed);
        let mut states = self.states.lock().unwrap();
        for (id, value) in values {
            match value {
                SavedModuleState::StockSeq3 { .. } => {
                    states.insert(id.clone(), *value);
                }
                SavedModuleState::StockSeq3Fields {
                    running,
                    gates,
                    gate_mask,
                    passthrough,
                } => {
                    let SavedModuleState::StockSeq3 {
                        running: old_run,
                        gates: old_gates,
                        passthrough: old_pass,
                    } = states[id]
                    else {
                        unreachable!()
                    };
                    states.insert(
                        id.clone(),
                        flags(
                            running.unwrap_or(old_run),
                            (old_gates & !gate_mask) | (gates & gate_mask),
                            passthrough.unwrap_or(old_pass),
                        ),
                    );
                }
                _ => unreachable!(),
            }
        }
    }
    fn retire_modules(&self, ids: &[String]) {
        for id in ids {
            self.states.lock().unwrap().remove(id);
        }
    }
}
fn flags(running: bool, gates: u8, passthrough: bool) -> SavedModuleState {
    SavedModuleState::StockSeq3 {
        running,
        gates,
        passthrough,
    }
}
fn fixture() -> (Session, Arc<Live>) {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    s.execute("add_module",json!({"id":"seq","kind":"stock_seq3","params":{"row1_1":1,"row1_2":2,"row1_8":8,"tempo":1}})).unwrap();
    let live = Arc::new(Live::default());
    s.set_live_module_events(Some(live.clone())).unwrap();
    live.states
        .lock()
        .unwrap()
        .insert("seq".into(), flags(false, 0b0101_0010, true));
    (s, live)
}
fn state(live: &Live, id: &str) -> SavedModuleState {
    live.states.lock().unwrap()[id]
}
fn preset(params: Value) -> Value {
    json!({"format":"openpatch-module-preset","version":1,"kind":"stock_seq3","params":params})
}

#[test]
fn coherent_capture_and_preset_use_all_live_flags_without_mutation() {
    let (mut s, live) = fixture();
    let before = s.snapshot();
    let captured = s.capture_project_patch(Some(s.revision())).unwrap();
    assert!(!captured.running["seq"]);
    assert_eq!(captured.saved_states["seq"], state(&live, "seq"));
    assert_eq!(captured.patch.nodes[0].params["gate2"], 1.0);
    assert_eq!(captured.patch.nodes[0].params["gate1"], 0.0);
    assert_eq!(captured.patch.nodes[0].params["passthrough"], 1.0);
    let p = s
        .execute("capture_module_preset", json!({"id":"seq"}))
        .unwrap();
    assert_eq!(p["preset"]["params"]["running"], 0.0);
    assert_eq!(p["preset"]["params"]["gate7"], 1.0);
    assert_eq!(p["preset"]["params"]["gate8"], 0.0);
    assert_eq!(p["frame"], 512);
    assert_eq!(s.snapshot(), before);
    assert_eq!(live.loads.load(Ordering::Relaxed), 0);
}
#[test]
fn rotation_uses_live_gates_and_restores_one_atomic_history_entry() {
    let (mut s, live) = fixture();
    let before = state(&live, "seq");
    let undo = s.undo.len();
    s.execute(
        "rotate_sequence",
        json!({"id":"seq","direction":"right","expected_revision":1}),
    )
    .unwrap();
    assert_eq!(s.undo.len(), undo + 1);
    assert_eq!(s.revision(), 2);
    assert_eq!(state(&live, "seq"), flags(false, 0b1010_0100, true));
    assert_eq!(s.patch.nodes[0].params["row1_1"], 8.0);
    assert_eq!(s.patch.nodes[0].params["row1_2"], 1.0);
    assert_eq!(live.reads.load(Ordering::Relaxed), 1);
    s.execute("undo", json!({})).unwrap();
    assert_eq!(state(&live, "seq"), before);
    s.execute("redo", json!({})).unwrap();
    assert_eq!(state(&live, "seq"), flags(false, 0b1010_0100, true));
    s.execute("rotate_sequence", json!({"id":"seq","direction":"left"}))
        .unwrap();
    assert_eq!(state(&live, "seq"), before);
}
#[test]
fn durable_edits_change_only_requested_fields_even_when_equal_to_stale_params() {
    let (mut s, live) = fixture();
    s.execute(
        "set_parameter",
        json!({"id":"seq","parameter":"gate1","value":1}),
    )
    .unwrap();
    assert_eq!(state(&live, "seq"), flags(false, 0b0101_0011, true));
    s.execute(
        "set_parameter",
        json!({"id":"seq","parameter":"running","value":1}),
    )
    .unwrap();
    assert_eq!(state(&live, "seq"), flags(true, 0b0101_0011, true));
    s.execute(
        "set_parameter",
        json!({"id":"seq","parameter":"passthrough","value":0}),
    )
    .unwrap();
    assert_eq!(state(&live, "seq"), flags(true, 0b0101_0011, false));
    s.execute("undo", json!({})).unwrap();
    assert_eq!(state(&live, "seq"), flags(true, 0b0101_0011, true));
    let loads = live.loads.load(Ordering::Relaxed);
    s.execute(
        "set_parameter",
        json!({"id":"seq","parameter":"row2_8","value":7.5}),
    )
    .unwrap();
    assert_eq!(live.loads.load(Ordering::Relaxed), loads);
}
#[test]
fn partial_presets_duplicate_and_initialize_preserve_source_state_ownership() {
    let (mut s, live) = fixture();
    s.execute(
        "apply_module_preset",
        json!({"id":"seq","preset":preset(json!({"row2_5":9,"gate8":1}))}),
    )
    .unwrap();
    assert_eq!(state(&live, "seq"), flags(false, 0b1101_0010, true));
    s.execute(
        "duplicate_modules",
        json!({"modules":[{"id":"seq","new_id":"copy"}]}),
    )
    .unwrap();
    assert_eq!(state(&live, "copy"), state(&live, "seq"));
    s.execute("initialize_modules", json!({"ids":["seq"]}))
        .unwrap();
    assert_eq!(state(&live, "seq"), flags(false, 255, false));
    assert_eq!(s.patch.nodes[0].params["row2_5"], 0.0);
    s.execute("undo", json!({})).unwrap();
    assert_eq!(state(&live, "seq"), flags(false, 0b1101_0010, true));
}
#[test]
fn ordered_batch_state_edits_and_failed_edits_are_transactional() {
    let (mut s, live) = fixture();
    s.execute(
        "apply_edits",
        json!({"edits":[
            {"tool":"set_parameter","arguments":{"id":"seq","parameter":"gate8","value":1}},
            {"tool":"rotate_sequence","arguments":{"id":"seq","direction":"right"}},
            {"tool":"duplicate_modules","arguments":{"modules":[{"id":"seq","new_id":"copy"}]}},
            {"tool":"set_parameter","arguments":{"id":"seq","parameter":"running","value":1}}
        ]}),
    )
    .unwrap();
    assert_eq!(state(&live, "copy"), flags(false, 0b1010_0101, true));
    assert_eq!(state(&live, "seq"), flags(true, 0b1010_0101, true));
    let before = s.snapshot();
    let runtime = live.states.lock().unwrap().clone();
    let loads = live.loads.load(Ordering::Relaxed);
    assert!(s
        .execute(
            "apply_edits",
            json!({"edits":[
                {"tool":"rotate_sequence","arguments":{"id":"seq","direction":"left"}},
                {"tool":"set_parameter","arguments":{"id":"seq","parameter":"gate1","value":0.5}}
            ]})
        )
        .is_err());
    assert_eq!(s.snapshot(), before);
    assert_eq!(*live.states.lock().unwrap(), runtime);
    assert_eq!(live.loads.load(Ordering::Relaxed), loads);
}
#[test]
fn stale_or_incomplete_flag_banks_reject_rotation_and_capture_without_effects() {
    let (mut s, live) = fixture();
    let good = live.snapshot();
    let before = s.snapshot();
    for bad in 0..6 {
        let mut value = good.clone();
        match bad {
            0 => value["coherent"] = json!(false),
            1 => value["modules"]["seq"]["revision"] = json!(0),
            2 => value["modules"]["seq"]["state"]["gates"] = json!([true]),
            3 => value["modules"]["seq"]["state"]["gates"][3] = json!(1),
            4 => value["modules"]["seq"]["state"]["passthrough"] = Value::Null,
            _ => value["modules"]["seq"]["kind"] = json!("seq3"),
        }
        *live.invalid.lock().unwrap() = Some(value);
        assert!(s.capture_project_patch(None).is_err());
        assert!(s
            .execute("rotate_sequence", json!({"id":"seq","direction":"right"}))
            .is_err());
        assert_eq!(s.snapshot(), before);
    }
    assert_eq!(live.loads.load(Ordering::Relaxed), 0);
}
#[test]
fn physical_buttons_have_queue_receipts_and_releases_remain_available_in_bypass() {
    let (mut s, live) = fixture();
    let before = s.snapshot();
    for action in [
        "clock_press",
        "clock_release",
        "run_press",
        "run_release",
        "reset_press",
        "reset_release",
        "seq_gate1_press",
        "seq_gate1_release",
        "seq_gate8_press",
        "seq_gate8_release",
    ] {
        assert_eq!(
            s.execute("trigger_module", json!({"id":"seq","action":action}))
                .unwrap()["receipt"]["status"],
            "queued"
        );
    }
    assert_eq!(s.snapshot(), before);
    s.execute("set_bypass", json!({"id":"seq","bypassed":true}))
        .unwrap();
    for action in [
        "clock_release",
        "run_release",
        "reset_release",
        "seq_gate1_release",
    ] {
        assert!(s
            .execute("trigger_module", json!({"id":"seq","action":action}))
            .is_ok());
    }
    for action in ["clock_press", "run_press", "seq_gate1_press", "clock"] {
        assert!(s
            .execute("trigger_module", json!({"id":"seq","action":action}))
            .is_err());
    }
    assert_eq!(live.triggered.lock().unwrap().len(), 14);
}

#[test]
fn partial_presets_and_gate_edits_preserve_edges_after_the_coherent_capture() {
    let (mut s, live) = fixture();
    live.edge_after_snapshot.store(true, Ordering::Relaxed);
    s.execute(
        "apply_module_preset",
        json!({"id":"seq","preset":preset(json!({"row1_1":7}))}),
    )
    .unwrap();
    assert_eq!(state(&live, "seq"), flags(true, 0b0101_0000, true));
    assert_eq!(
        live.loads.load(Ordering::Relaxed),
        0,
        "row-only preset cannot restore a stale bank"
    );
    live.edge_after_snapshot.store(true, Ordering::Relaxed);
    s.execute(
        "set_parameter",
        json!({"id":"seq","parameter":"gate1","value":1}),
    )
    .unwrap();
    assert_eq!(state(&live, "seq"), flags(false, 0b0101_0011, true));
}

#[test]
fn seeded_randomization_changes_rows_and_gates_without_rewinding_live_run_or_mode() {
    let (mut s, live) = fixture();
    live.edge_after_snapshot.store(true, Ordering::Relaxed);
    s.execute("randomize_modules", json!({"ids":["seq"],"seed":421}))
        .unwrap();
    let first = state(&live, "seq");
    let SavedModuleState::StockSeq3 {
        running,
        gates,
        passthrough,
    } = first
    else {
        unreachable!()
    };
    assert!(running && passthrough);
    assert_eq!(s.patch.nodes[0].params["tempo"], 1.0);
    s.execute("randomize_modules", json!({"ids":["seq"],"seed":421}))
        .unwrap();
    assert_eq!(state(&live, "seq"), flags(true, gates, true));
}

#[test]
fn a_partial_callback_assignment_is_not_a_complete_preset_capture() {
    let (session, _) = fixture();
    let node = &session.patch.nodes[0];
    assert!(crate::presets::capture_state(
        node,
        Some(SavedModuleState::StockSeq3Fields {
            running: None,
            gates: 0,
            gate_mask: 1,
            passthrough: None,
        })
    )
    .unwrap_err()
    .contains("requires current persistent state"));
}
