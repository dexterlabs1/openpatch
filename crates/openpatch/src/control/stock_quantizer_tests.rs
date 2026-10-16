use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Default)]
struct Live {
    states: Mutex<BTreeMap<String, SavedModuleState>>,
    revision: AtomicU64,
    hold_revision: AtomicBool,
    loads: Mutex<Vec<BTreeMap<String, SavedModuleState>>>,
}
impl LiveModuleEvents for Live {
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        Err("Stock Quantizer has no momentary actions".into())
    }
    fn snapshot(&self) -> Value {
        let modules: serde_json::Map<_, _> = self.states.lock().unwrap().iter().map(|(id, state)| {
            let SavedModuleState::StockQuantizerNotes(mask) = state else { unreachable!() };
            (id.clone(), json!({"kind":"stock_quantizer","available":true,
                "revision":self.revision.load(Ordering::Relaxed),"frame":512,
                "state":{"scale_mask":mask,"enabled_notes":std::array::from_fn::<_,12,_>(|i| mask & (1<<i) != 0)}}))
        }).collect();
        json!({"coherent":true,"frame":512,"modules":modules})
    }
    fn sync_patch(&self, patch: &Patch, revision: u64) {
        let mut states = self.states.lock().unwrap();
        states.retain(|id, _| {
            patch
                .nodes
                .iter()
                .any(|n| n.id == *id && n.kind == "stock_quantizer")
        });
        for node in patch.nodes.iter().filter(|n| n.kind == "stock_quantizer") {
            states
                .entry(node.id.clone())
                .or_insert_with(|| SavedModuleState::from_node(node).unwrap());
        }
        if !self.hold_revision.load(Ordering::Relaxed) {
            self.revision.store(revision, Ordering::Relaxed);
        }
    }
    fn load_module_saved_states(&self, states: &BTreeMap<String, SavedModuleState>) {
        self.loads.lock().unwrap().push(states.clone());
        self.states
            .lock()
            .unwrap()
            .extend(states.iter().map(|(id, state)| (id.clone(), *state)));
    }
    fn retire_modules(&self, ids: &[String]) {
        self.states
            .lock()
            .unwrap()
            .retain(|id, _| !ids.contains(id));
    }
}
fn fixture() -> (Session, Arc<Live>) {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session.execute("add_module", json!({"id":"scale","kind":"stock_quantizer","params":{"offset":0.25,"scale_mask":4095}})).unwrap();
    let live = Arc::new(Live::default());
    session.set_live_module_events(Some(live.clone())).unwrap();
    live.states.lock().unwrap().insert(
        "scale".into(),
        SavedModuleState::StockQuantizerNotes(0b1000_0000_0101),
    );
    (session, live)
}
fn mask(live: &Live) -> u16 {
    let SavedModuleState::StockQuantizerNotes(mask) = live.states.lock().unwrap()["scale"] else {
        unreachable!()
    };
    mask
}
#[test]
fn stock_quantizer_capture_projects_coherent_live_notes_without_editing() {
    let (mut session, live) = fixture();
    let before = session.snapshot();
    let captured = session
        .capture_project_patch(Some(session.revision()))
        .unwrap();
    assert_eq!(
        captured.patch.nodes[0].params["scale_mask"],
        f32::from(mask(&live))
    );
    assert_eq!(
        captured.saved_states["scale"],
        SavedModuleState::StockQuantizerNotes(mask(&live))
    );
    let preset = session
        .execute("capture_module_preset", json!({"id":"scale"}))
        .unwrap();
    assert_eq!(
        preset["preset"]["params"]["scale_mask"],
        f64::from(mask(&live))
    );
    assert_eq!(session.snapshot(), before);
    assert!(live.loads.lock().unwrap().is_empty());
}
#[test]
fn stock_quantizer_rotation_uses_actual_notes_and_one_history_step() {
    let (mut session, live) = fixture();
    let original = mask(&live);
    let revision = session.revision();
    session
        .execute(
            "rotate_scale",
            json!({"id":"scale","semitones":1,"expected_revision":revision}),
        )
        .unwrap();
    assert_eq!(mask(&live), 0b0000_0000_1011);
    assert_eq!(session.revision(), revision + 1);
    assert_eq!(session.patch.nodes[0].params["offset"], 0.25);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(mask(&live), original);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(mask(&live), 0b0000_0000_1011);
}
#[test]
fn stock_quantizer_offset_only_preset_does_not_assign_the_scale() {
    let (mut session, live) = fixture();
    let original = mask(&live);
    session.execute("apply_module_preset", json!({"id":"scale","preset":{
        "format":"openpatch-module-preset","version":1,"kind":"stock_quantizer","params":{"offset":-0.5}}})).unwrap();
    assert_eq!(mask(&live), original);
    assert_eq!(
        session.patch.nodes[0].params["scale_mask"],
        f32::from(original)
    );
    assert_eq!(session.patch.nodes[0].params["offset"], -0.5);
    assert!(live
        .loads
        .lock()
        .unwrap()
        .iter()
        .all(|states| !states.contains_key("scale")));
    session
        .execute("initialize_modules", json!({"ids":["scale"]}))
        .unwrap();
    assert_eq!(mask(&live), 4095);
    assert_eq!(session.patch.nodes[0].params["offset"], 0.0);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(mask(&live), original);
    assert_eq!(session.patch.nodes[0].params["offset"], -0.5);
}
#[test]
fn stock_quantizer_rotation_batches_preview_and_invalid_edits_are_atomic() {
    let (mut session, live) = fixture();
    let before = session.snapshot();
    let original = mask(&live);
    let edits = json!([{"tool":"rotate_scale","arguments":{"id":"scale","semitones":-1}},
        {"tool":"set_parameter","arguments":{"id":"scale","parameter":"offset","value":-0.25}}]);
    let preview = session
        .execute("preview_edits", json!({"edits":edits}))
        .unwrap();
    assert_eq!(preview["committed"], false);
    assert_eq!(mask(&live), original);
    assert_eq!(session.snapshot(), before);
    session
        .execute("apply_edits", json!({"edits":edits}))
        .unwrap();
    assert_eq!(mask(&live), 0b1100_0000_0010);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(mask(&live), original);
    for semitones in [json!(-13), json!(13), json!(0.5), json!("1")] {
        let before = session.snapshot();
        assert!(session
            .execute(
                "apply_edits",
                json!({"edits":[
            {"tool":"set_parameter","arguments":{"id":"scale","parameter":"offset","value":0.75}},
            {"tool":"rotate_scale","arguments":{"id":"scale","semitones":semitones}}]})
            )
            .is_err());
        assert_eq!(session.snapshot(), before);
        assert_eq!(mask(&live), original);
    }
}
#[test]
fn stock_quantizer_live_saved_rejects_stale_malformed_and_disagreeing_flags() {
    let (_, live) = fixture();
    let good = live.snapshot();
    let revision = live.revision.load(Ordering::Relaxed);
    let expected = SavedModuleState::StockQuantizerNotes(4095);
    assert!(running::live_saved(&good, "scale", revision, expected).is_ok());
    let bad_values = [
        ("kind", json!("quantizer")),
        ("kind", Value::Null),
        ("available", json!(false)),
        ("revision", json!(revision + 1)),
        ("frame", Value::Null),
    ];
    for (key, value) in bad_values {
        let mut bad = good.clone();
        bad["modules"]["scale"][key] = value;
        assert!(running::live_saved(&bad, "scale", revision, expected).is_err());
    }
    for value in [json!(-1), json!(4096), json!(1.5), Value::Null] {
        let mut bad = good.clone();
        bad["modules"]["scale"]["state"]["scale_mask"] = value;
        assert!(running::live_saved(&bad, "scale", revision, expected).is_err());
    }
    let mut bad = good.clone();
    bad["modules"]["scale"]["state"]["enabled_notes"][0] = json!(false);
    assert!(running::live_saved(&bad, "scale", revision, expected).is_err());
    let mut bad = good;
    bad["modules"]["scale"]["state"]["enabled_notes"] = json!([true]);
    assert!(running::live_saved(&bad, "scale", revision, expected).is_err());
}

#[test]
fn stock_quantizer_acknowledged_paint_bank_continues_before_callback_publication() {
    let (mut session, live) = fixture();
    let original = mask(&live);
    let before_undo = session.execute("inspect_history", json!({})).unwrap()["history"]
        ["undo_steps"]
        .as_u64()
        .unwrap();
    live.hold_revision.store(true, Ordering::Relaxed);
    for value in [2052, 2048, 0] {
        let revision = session.revision();
        session
            .execute(
                "set_parameter",
                json!({"id":"scale","parameter":"scale_mask", "value":value,
            "history_group":"paint-one","expected_revision":revision}),
            )
            .unwrap();
        assert_eq!(mask(&live), value);
    }
    assert_eq!(
        session.execute("inspect_history", json!({})).unwrap()["history"]["undo_steps"],
        before_undo + 1
    );
    let before = session.snapshot();
    assert!(session
        .execute(
            "set_parameter",
            json!({"id":"scale","parameter":"scale_mask", "value":1,
        "history_group":"another-gesture","expected_revision":session.revision()})
        )
        .is_err());
    assert!(session
        .execute(
            "rotate_scale",
            json!({"id":"scale","semitones":1,"expected_revision":session.revision()})
        )
        .is_err());
    assert!(session
        .capture_project_patch(Some(session.revision()))
        .is_err());
    assert_eq!(session.snapshot(), before);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(mask(&live), original);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(mask(&live), 0);
}
