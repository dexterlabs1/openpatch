use super::*;

fn state() -> Value {
    json!({"data":{"patterns":[{"name":"A","length":4,"tracks":[{"voices":2,
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
    let preset = crate::presets::capture(&original, None).unwrap();
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
    assert!(session.patch.nodes[0].rackmcp_sequence.is_none());
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
    crate::presets::validate(&crate::presets::capture(&default_node, None).unwrap()).unwrap();
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
