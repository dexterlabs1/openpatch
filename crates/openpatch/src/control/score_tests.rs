use super::*;

#[test]
fn score_edits_validate_atomically_roundtrip_and_participate_in_history() {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    let score = json!({"length":4,"tracks":[{"voices":2,"steps":[{"step":0,"notes":[-12,7]}]}],"lanes":[{"points":[{"bar":0,"value":0},{"bar":4,"value":1}]}]});
    s.execute(
        "add_module",
        json!({"id":"score","kind":"score_sequencer","score":score}),
    )
    .unwrap();
    let initial = serde_json::to_value(&s.patch).unwrap();
    let mut changed = score.clone();
    changed["tracks"][0]["steps"][0]["notes"] = json!([0, 12]);
    s.execute("set_score", json!({"id":"score","score":changed}))
        .unwrap();
    let modified = serde_json::to_value(&s.patch).unwrap();
    assert_ne!(initial, modified);
    s.execute("undo", json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.patch).unwrap(), initial);
    s.execute("redo", json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.patch).unwrap(), modified);
    let revision = s.revision;
    for bad in [
        json!({"length":0}),
        json!({"tracks":[{"voices":17}]}),
        json!({"tracks":[{"steps":[{"step":64,"notes":[0]}]}]}),
        json!({"lanes":[{"points":[{"bar":0,"value":100000}]}]}),
        json!({"unknown":0}),
    ] {
        assert!(s
            .execute("set_score", json!({"id":"score","score":bad}))
            .is_err());
        assert_eq!(s.revision, revision);
        assert_eq!(serde_json::to_value(&s.patch).unwrap(), modified);
    }
    assert!(s
        .execute(
            "set_score",
            json!({"id":"score","score":score,"expected_revision":revision-1})
        )
        .is_err());
    let preset = crate::presets::capture(&s.patch.nodes[0], None).unwrap();
    let mut target: Node =
        serde_json::from_value(json!({"id":"copy","kind":"score_sequencer"})).unwrap();
    crate::presets::apply(&mut target, &preset).unwrap();
    assert_eq!(target.score, s.patch.nodes[0].score);
    s.execute(
        "duplicate_modules",
        json!({"modules":[{"id":"score","new_id":"copy"}]}),
    )
    .unwrap();
    assert_eq!(s.patch.nodes[0].score, s.patch.nodes[1].score);
}

#[test]
fn score_data_cannot_hide_on_other_modules_or_invalid_batches() {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    let revision = s.revision;
    assert!(s
        .execute(
            "add_module",
            json!({"id":"wrong","kind":"constant","score":{}})
        )
        .is_err());
    assert!(s
        .execute(
            "apply_edits",
            json!({"edits":[
                {"tool":"add_module","arguments":{"id":"score","kind":"score_sequencer"}},
                {"tool":"set_score","arguments":{"id":"score","score":{"length":255}}}
            ]})
        )
        .is_err());
    assert!(s.patch.nodes.is_empty());
    assert_eq!(s.revision, revision);
}
