use super::*;

fn state() -> Value {
    json!({"data":{"selectedPattern":"A","patterns":[{"name":"A","length":4,"tracks":[{"voices":2,
        "steps":[{"step":0,"notes":[0,7],"accent":true},{"step":2,"notes":[12,19]}]}]}],
        "lanes":[{"moduleId":"42","paramId":0,"points":[{"bar":0,"value":0.25}]}]},
        "targets":[{"source_module_id":"42","source_param_id":0,"module":"vca","parameter":"level"}]})
}

#[test]
fn rackmcp_sequence_creation_history_and_patch_roundtrip_keep_typed_source_data() {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .execute(
            "add_module",
            json!({"id":"seq","kind":"rackmcp_sequencer",
        "rackmcp_sequence":state(),"params":{"running":1}}),
        )
        .unwrap();
    let authored = session.patch.nodes[0].rackmcp_sequence.clone();
    assert!(session.patch.nodes[0].score.is_none());
    session
        .execute(
            "set_parameter",
            json!({"id":"seq","parameter":"bpm","value":128}),
        )
        .unwrap();
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.patch.nodes[0].rackmcp_sequence, authored);
    session.execute("redo", json!({})).unwrap();
    let reopened: Patch =
        serde_json::from_value(serde_json::to_value(&session.patch).unwrap()).unwrap();
    reopened.validate().unwrap();
    assert_eq!(reopened.nodes[0].rackmcp_sequence, authored);
}

#[test]
fn rackmcp_sequence_presets_retain_native_targets_and_validate_before_assignment() {
    let original: Node = serde_json::from_value(json!({"id":"seq","kind":"rackmcp_sequencer",
        "rackmcp_sequence":state(),"params":{"bpm":128}}))
    .unwrap();
    let preset =
        crate::presets::capture_state(&original, SavedModuleState::from_node(&original)).unwrap();
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .execute(
            "add_module",
            json!({"id":"copy","kind":"rackmcp_sequencer"}),
        )
        .unwrap();
    session
        .execute("apply_module_preset", json!({"id":"copy","preset":preset}))
        .unwrap();
    assert_eq!(
        session.patch.nodes[0].rackmcp_sequence,
        original.rackmcp_sequence
    );
    session.execute("undo", json!({})).unwrap();
    let mut defaults: Node =
        serde_json::from_value(json!({"id":"copy","kind":"rackmcp_sequencer"})).unwrap();
    SavedModuleState::from_node(&defaults)
        .unwrap()
        .write_node(&mut defaults);
    assert_eq!(
        session.patch.nodes[0].rackmcp_sequence,
        defaults.rackmcp_sequence
    );
    session.execute("redo", json!({})).unwrap();
    let revision = session.revision;
    let mut invalid = preset.clone();
    invalid["rackmcp_sequence"]["data"]["patterns"][0]["length"] = json!(0);
    assert!(session
        .execute("apply_module_preset", json!({"id":"copy","preset":invalid}))
        .is_err());
    assert_eq!(session.revision, revision);
    assert_eq!(
        session.patch.nodes[0].rackmcp_sequence,
        original.rackmcp_sequence
    );
    let default_node: Node =
        serde_json::from_value(json!({"id":"default","kind":"rackmcp_sequencer"})).unwrap();
    crate::presets::validate(
        &crate::presets::capture_state(&default_node, SavedModuleState::from_node(&default_node))
            .unwrap(),
    )
    .unwrap();
}

#[test]
fn rackmcp_data_on_other_kinds_and_invalid_batches_are_atomic_rejections() {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    let revision = session.revision;
    for kind in ["constant", "score_sequencer"] {
        assert!(session
            .execute(
                "add_module",
                json!({"id":"wrong","kind":kind,"rackmcp_sequence":state()})
            )
            .is_err());
    }
    assert!(session.execute("apply_edits", json!({"edits":[
        {"tool":"add_module","arguments":{"id":"seq","kind":"rackmcp_sequencer","rackmcp_sequence":state()}},
        {"tool":"set_parameter","arguments":{"id":"seq","parameter":"bpm","value":301}}
    ]})).is_err());
    assert_eq!(session.revision, revision);
    assert!(session.patch.nodes.is_empty());
}

#[derive(Default)]
struct SequenceLoadObserver(Mutex<Vec<(u64, Vec<String>)>>, Mutex<Value>);
impl SequenceLoadObserver {
    fn publish(&self, patch: &Patch, revision: u64) {
        let modules: serde_json::Map<_, _> = patch.nodes.iter().filter(|n| n.kind == "rackmcp_sequencer").map(|n| {
            let selected = match SavedModuleState::from_node(n).unwrap() {
                SavedModuleState::RackMcpSelection(name) => name, _ => unreachable!(),
            };
            (n.id.clone(), json!({"kind":n.kind,"available":true,"revision":revision,"frame":64,"state":{"selected_pattern":selected.as_str()}}))
        }).collect();
        *self.1.lock().unwrap() = json!({"coherent":true,"modules":modules});
    }
}
impl LiveModuleEvents for SequenceLoadObserver {
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        Err("not used by this control-only observer".into())
    }
    fn snapshot(&self) -> Value {
        self.1.lock().unwrap().clone()
    }
    fn sync_patch_with_intents(
        &self,
        patch: &Patch,
        revision: u64,
        _: &BTreeMap<String, BTreeMap<String, f32>>,
        loads: &[String],
    ) {
        self.publish(patch, revision);
        self.0.lock().unwrap().push((revision, loads.to_vec()));
    }
}

#[test]
fn explicit_equal_sequence_loads_and_history_emit_intent_without_unrelated_reloads() {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .execute(
            "add_module",
            json!({"id":"seq","kind":"rackmcp_sequencer","rackmcp_sequence":state()}),
        )
        .unwrap();
    let backend = Arc::new(SequenceLoadObserver::default());
    backend.publish(&session.patch, session.revision);
    session
        .set_live_module_events(Some(backend.clone()))
        .unwrap();
    let before = session.snapshot();
    session.execute("preview_edits", json!({"edits":[{"tool":"set_rackmcp_sequence","arguments":{"id":"seq","rackmcp_sequence":state()}}]})).unwrap();
    assert_eq!(session.snapshot(), before);
    assert!(backend.0.lock().unwrap().is_empty());
    for (tool, args, load) in [
        (
            "set_rackmcp_sequence",
            json!({"id":"seq","rackmcp_sequence":state()}),
            true,
        ),
        ("undo", json!({}), true),
        ("redo", json!({}), true),
        (
            "set_parameter",
            json!({"id":"seq","parameter":"bpm","value":128}),
            false,
        ),
        ("undo", json!({}), false),
        ("redo", json!({}), false),
    ] {
        session.execute(tool, args).unwrap();
        let observed = backend.0.lock().unwrap();
        let (revision, loads) = observed.last().unwrap();
        assert_eq!(*revision, session.revision);
        assert_eq!(
            loads,
            &if load {
                vec!["seq".to_owned()]
            } else {
                Vec::new()
            }
        );
    }
    let before = session.snapshot();
    let calls = backend.0.lock().unwrap().len();
    let mut invalid = state();
    invalid["data"]["patterns"][0]["length"] = json!(0);
    assert!(session
        .execute(
            "set_rackmcp_sequence",
            json!({"id":"seq","rackmcp_sequence":invalid})
        )
        .is_err());
    assert!(session
        .execute(
            "set_rackmcp_sequence",
            json!({"id":"seq","rackmcp_sequence":state(),"expected_revision":0})
        )
        .is_err());
    assert_eq!(session.snapshot(), before);
    assert_eq!(backend.0.lock().unwrap().len(), calls);
}

#[test]
fn sequence_preset_intent_depends_on_explicit_data_and_batches_commit_one_final_load() {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .execute(
            "add_module",
            json!({"id":"seq","kind":"rackmcp_sequencer","rackmcp_sequence":state()}),
        )
        .unwrap();
    session
        .execute("add_module", json!({"id":"score","kind":"score_sequencer"}))
        .unwrap();
    let backend = Arc::new(SequenceLoadObserver::default());
    backend.publish(&session.patch, session.revision);
    session
        .set_live_module_events(Some(backend.clone()))
        .unwrap();
    let preset = crate::presets::capture_state(
        &session.patch.nodes[0],
        SavedModuleState::from_node(&session.patch.nodes[0]),
    )
    .unwrap();
    session.execute("apply_module_preset", json!({"id":"seq","preset":{"format":"openpatch-module-preset","version":1,"kind":"rackmcp_sequencer","params":{"bpm":130}}})).unwrap();
    assert!(backend.0.lock().unwrap().last().unwrap().1.is_empty());
    session
        .execute("apply_module_preset", json!({"id":"seq","preset":preset}))
        .unwrap();
    assert_eq!(
        backend.0.lock().unwrap().last().unwrap().1,
        vec!["seq".to_owned()]
    );
    let before = backend.0.lock().unwrap().len();
    let mut final_state = state();
    final_state["data"]["patterns"][0]["length"] = json!(8);
    session.execute("apply_edits", json!({"edits":[
        {"tool":"set_rackmcp_sequence","arguments":{"id":"seq","rackmcp_sequence":state()}},
        {"tool":"set_rackmcp_sequence","arguments":{"id":"seq","rackmcp_sequence":final_state}}
    ]})).unwrap();
    assert_eq!(backend.0.lock().unwrap().len(), before + 1);
    assert_eq!(
        session.patch.nodes[0]
            .rackmcp_sequence
            .as_ref()
            .unwrap()
            .data
            .patterns[0]
            .length,
        8
    );
    assert_eq!(
        backend.0.lock().unwrap().last().unwrap().1,
        vec!["seq".to_owned()]
    );
    let before = session.snapshot();
    assert!(session
        .execute(
            "set_rackmcp_sequence",
            json!({"id":"score","rackmcp_sequence":state()})
        )
        .is_err());
    assert_eq!(session.snapshot(), before);
}

#[test]
fn sequence_load_order_tracks_last_action_and_reverses_with_history() {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    for id in ["z", "a"] {
        session
            .execute(
                "add_module",
                json!({"id":id,"kind":"rackmcp_sequencer","rackmcp_sequence":state()}),
            )
            .unwrap();
    }
    let backend = Arc::new(SequenceLoadObserver::default());
    backend.publish(&session.patch, session.revision);
    session
        .set_live_module_events(Some(backend.clone()))
        .unwrap();
    session
        .execute(
            "apply_edits",
            json!({"edits":[
                {"tool":"set_rackmcp_sequence","arguments":{"id":"z","rackmcp_sequence":state()}},
                {"tool":"set_rackmcp_sequence","arguments":{"id":"a","rackmcp_sequence":state()}},
                {"tool":"set_rackmcp_sequence","arguments":{"id":"z","rackmcp_sequence":state()}}
            ]}),
        )
        .unwrap();
    assert_eq!(backend.0.lock().unwrap().last().unwrap().1, ["a", "z"]);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(backend.0.lock().unwrap().last().unwrap().1, ["z", "a"]);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(backend.0.lock().unwrap().last().unwrap().1, ["a", "z"]);
    session
        .execute(
            "apply_edits",
            json!({"edits":[
                {"tool":"set_rackmcp_sequence","arguments":{"id":"a","rackmcp_sequence":state()}},
                {"tool":"remove_module","arguments":{"id":"a"}},
                {"tool":"set_rackmcp_sequence","arguments":{"id":"z","rackmcp_sequence":state()}}
            ]}),
        )
        .unwrap();
    assert_eq!(backend.0.lock().unwrap().last().unwrap().1, ["z"]);
}

#[test]
fn creation_and_duplication_loads_preserve_final_order_without_undo_ghosts() {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    let backend = Arc::new(SequenceLoadObserver::default());
    backend.publish(&session.patch, session.revision);
    session
        .set_live_module_events(Some(backend.clone()))
        .unwrap();
    session.execute("apply_edits", json!({"edits":[
        {"tool":"add_module","arguments":{"id":"z","kind":"rackmcp_sequencer","rackmcp_sequence":state()}},
        {"tool":"add_module","arguments":{"id":"a","kind":"rackmcp_sequencer"}},
        {"tool":"set_rackmcp_sequence","arguments":{"id":"z","rackmcp_sequence":state()}}
    ]})).unwrap();
    assert_eq!(backend.0.lock().unwrap().last().unwrap().1, ["a", "z"]);
    session.execute("undo", json!({})).unwrap();
    assert!(backend.0.lock().unwrap().last().unwrap().1.is_empty());
    session.execute("redo", json!({})).unwrap();
    assert_eq!(backend.0.lock().unwrap().last().unwrap().1, ["a", "z"]);
    session.execute("apply_edits", json!({"edits":[
        {"tool":"duplicate_modules","arguments":{"modules":[{"id":"z","new_id":"copy_z"},{"id":"a","new_id":"copy_a"}]}},
        {"tool":"set_rackmcp_sequence","arguments":{"id":"copy_z","rackmcp_sequence":state()}}
    ]})).unwrap();
    assert_eq!(
        backend.0.lock().unwrap().last().unwrap().1,
        ["copy_a", "copy_z"]
    );
    session.execute("undo", json!({})).unwrap();
    assert!(backend.0.lock().unwrap().last().unwrap().1.is_empty());
    session.execute("redo", json!({})).unwrap();
    assert_eq!(
        backend.0.lock().unwrap().last().unwrap().1,
        ["copy_a", "copy_z"]
    );
}
