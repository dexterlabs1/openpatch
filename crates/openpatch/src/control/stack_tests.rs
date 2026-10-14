use super::*;

#[derive(Default)]
struct Live {
    retired: Mutex<Vec<String>>,
    retired_modules: Mutex<Vec<String>>,
    synced: Mutex<Vec<u64>>,
}
impl LiveModuleEvents for Live {
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        unreachable!()
    }
    fn snapshot(&self) -> Value {
        json!({})
    }
    fn retire_modules(&self, ids: &[String]) {
        self.retired_modules.lock().unwrap().extend_from_slice(ids);
    }
    fn retire_cables(&self, ids: &[String]) {
        self.retired.lock().unwrap().extend_from_slice(ids);
    }
    fn sync_patch(&self, _: &Patch, revision: u64) {
        self.synced.lock().unwrap().push(revision);
    }
}
fn raw_patch(version: u32) -> Patch {
    serde_json::from_value(json!({"version":version,"nodes":[
        {"id":"a","kind":"constant","params":{"value":2}},
        {"id":"b","kind":"constant","params":{"value":3}},
        {"id":"sum","kind":"gain"}],"cables":[{"from":"a","to":"sum","input":"signal"}],"output":"sum"})).unwrap()
}
fn make_session(version: u32) -> Session {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session.replace_patch(raw_patch(version), Some(0)).unwrap();
    session
}
fn ids(patch: &Patch) -> Vec<&str> {
    patch
        .cables
        .iter()
        .map(|cable| cable.id.as_deref().unwrap())
        .collect()
}

#[test]
fn canonicalization_reserves_explicit_ids_preserves_order_and_matches_preview_commit() {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    let mut patch = raw_patch(2);
    let mut extra = patch.cables[0].clone();
    extra.from = "b".into();
    extra.id = Some("cable_1".into());
    patch.cables.push(extra);
    session.replace_patch(patch, None).unwrap();
    assert_eq!(ids(session.patch()), vec!["cable_2", "cable_1"]);
    let before = session.snapshot();
    let edits = json!([
        {"tool":"duplicate_modules","arguments":{"modules":[{"id":"sum","new_id":"copy"}],"cable_mode":"inputs"}},
        {"tool":"disconnect","arguments":{"cable_id":"cable_3"}}
    ]);
    let preview = session
        .execute("preview_edits", json!({"edits":edits}))
        .unwrap();
    assert_eq!(session.snapshot(), before);
    assert_eq!(preview["patch"]["cables"][2]["id"], "cable_4");
    assert_eq!(preview["patch"]["cables"][2]["from"], "b");
    let committed = session
        .execute("apply_edits", json!({"edits":edits,"expected_revision":1}))
        .unwrap();
    assert_eq!(committed["patch"], preview["patch"]);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], committed["patch"]);
}

#[test]
fn stacking_is_explicit_v2_only_and_duplicate_routes_contribute_separately() {
    let mut session = make_session(2);
    let before = session.snapshot();
    assert!(session
        .execute("connect", json!({"from":"b","to":"sum","input":"signal"}))
        .is_err());
    assert_eq!(session.snapshot(), before);
    session
        .execute(
            "connect",
            json!({"from":"b","to":"sum","input":"signal","mode":"stack","cable_id":"other"}),
        )
        .unwrap();
    let preview = session.execute("preview_edits", json!({"edits":[
        {"tool":"connect","arguments":{"from":"a","to":"sum","input":"signal","mode":"stack"}}
    ]})).unwrap();
    assert_eq!(
        preview["changes"]["added_cables"].as_array().unwrap().len(),
        1
    );
    assert_eq!(preview["changes"]["added_cables"][0]["id"], "cable_2");
    session
        .execute(
            "connect",
            json!({"from":"a","to":"sum","input":"signal","mode":"stack"}),
        )
        .unwrap();
    assert_eq!(ids(session.patch()), vec!["cable_1", "other", "cable_2"]);
    let captured = session
        .execute("capture_signal", json!({"id":"sum","seconds":0.001}))
        .unwrap();
    assert_eq!(
        captured["bins"].as_array().unwrap().last().unwrap()["mean"].as_f64(),
        Some(7.0)
    );
    let mut legacy = make_session(1);
    let before = legacy.snapshot();
    assert!(legacy
        .execute(
            "connect",
            json!({"from":"b","to":"sum","input":"cv","mode":"stack"})
        )
        .unwrap_err()
        .contains("version 2"));
    assert_eq!(legacy.snapshot(), before);
}

#[test]
fn exact_disconnect_preserves_siblings_and_legacy_disconnect_removes_the_group() {
    let mut session = make_session(2);
    session.execute("apply_edits", json!({"edits":[
        {"tool":"connect","arguments":{"from":"b","to":"sum","input":"signal","mode":"stack","cable_id":"second"}},
        {"tool":"connect","arguments":{"from":"a","to":"sum","input":"signal","mode":"stack","cable_id":"third"}}
    ]})).unwrap();
    let before = session.snapshot();
    session
        .execute("disconnect", json!({"cable_id":"second"}))
        .unwrap();
    assert_eq!(ids(session.patch()), vec!["cable_1", "third"]);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
    session
        .execute("disconnect", json!({"to":"sum","input":"signal"}))
        .unwrap();
    assert!(session.patch.cables.is_empty());
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
}

#[test]
fn strict_selectors_modes_ids_and_failed_batches_leave_live_history_untouched() {
    let mut session = make_session(2);
    let live = Arc::new(Live::default());
    session.set_live_module_events(Some(live.clone()));
    let before = session.snapshot();
    for arguments in [
        json!({}),
        json!({"to":"sum"}),
        json!({"input":"signal"}),
        json!({"cable_id":"cable_1","to":"sum"}),
        json!({"cable_id":"cable_1","to":"sum","input":"signal"}),
        json!({"cable_id":null}),
        json!({"cable_id":"missing"}),
        json!({"cable_id":"cable_1","unknown":true}),
    ] {
        assert!(session.execute("disconnect", arguments).is_err());
        assert_eq!(session.snapshot(), before);
    }
    for extra in [
        json!({"mode":"all"}),
        json!({"mode":true}),
        json!({"cable_id":null}),
        json!({"cable_id":""}),
        json!({"cable_id":"bad id"}),
        json!({"cable_id":"cable_1"}),
        json!({"cable_id":"x".repeat(65)}),
        json!({"expected_revision":0}),
    ] {
        let mut args = json!({"from":"b","to":"sum","input":"cv"});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(session.execute("connect", args).is_err());
        assert_eq!(session.snapshot(), before);
    }
    for tool in ["preview_edits", "apply_edits"] {
        assert!(session.execute(tool, json!({"edits":[
            {"tool":"disconnect","arguments":{"cable_id":"cable_1"}},
            {"tool":"connect","arguments":{"from":"missing","to":"sum","input":"signal","cable_id":"cable_1"}}
        ]})).is_err());
        assert_eq!(session.snapshot(), before);
    }
    assert!(live.retired.lock().unwrap().is_empty());
    assert_eq!(*live.synced.lock().unwrap(), vec![1]);
}

#[test]
fn reusing_ids_retires_only_after_commit_and_document_replacement_retires_all() {
    let mut session = make_session(2);
    let live = Arc::new(Live::default());
    session.set_live_module_events(Some(live.clone()));
    let before = session.snapshot();
    let edits = json!([
        {"tool":"disconnect","arguments":{"cable_id":"cable_1"}},
        {"tool":"connect","arguments":{"from":"a","to":"sum","input":"signal","cable_id":"cable_1"}}
    ]);
    session
        .execute("preview_edits", json!({"edits":edits}))
        .unwrap();
    assert!(live.retired.lock().unwrap().is_empty());
    session
        .execute("apply_edits", json!({"edits":edits}))
        .unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
    assert_eq!(*live.retired.lock().unwrap(), vec!["cable_1"]);
    session.replace_patch(session.patch.clone(), None).unwrap();
    assert_eq!(*live.retired.lock().unwrap(), vec!["cable_1", "cable_1"]);
    session.execute("remove_module", json!({"id":"a"})).unwrap();
    assert_eq!(
        *live.retired.lock().unwrap(),
        vec!["cable_1", "cable_1", "cable_1"]
    );
}

#[test]
fn history_replays_identity_resets_for_byte_identical_cable_module_and_document_transitions() {
    let mut session = make_session(2);
    let live = Arc::new(Live::default());
    session.set_live_module_events(Some(live.clone()));
    let original = session.snapshot()["patch"].clone();
    session.execute("apply_edits", json!({"edits":[
        {"tool":"disconnect","arguments":{"cable_id":"cable_1"}},
        {"tool":"connect","arguments":{"from":"a","to":"sum","input":"signal","cable_id":"cable_1"}}
    ]})).unwrap();
    session.execute("undo", json!({})).unwrap();
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], original);
    assert_eq!(*live.retired.lock().unwrap(), vec!["cable_1"; 3]);
    assert!(live.retired_modules.lock().unwrap().is_empty());
    session.execute("apply_edits", json!({"edits":[
        {"tool":"remove_module","arguments":{"id":"sum"}},
        {"tool":"add_module","arguments":{"id":"sum","kind":"gain"}},
        {"tool":"connect","arguments":{"from":"a","to":"sum","input":"signal","cable_id":"cable_1"}},
        {"tool":"set_output","arguments":{"id":"sum"}}
    ]})).unwrap();
    session.execute("undo", json!({})).unwrap();
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], original);
    assert_eq!(*live.retired.lock().unwrap(), vec!["cable_1"; 6]);
    assert_eq!(*live.retired_modules.lock().unwrap(), vec!["sum"; 3]);
    let patch = session.patch.clone();
    session.replace_patch(patch, None).unwrap();
    session.execute("undo", json!({})).unwrap();
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], original);
    assert_eq!(*live.retired.lock().unwrap(), vec!["cable_1"; 9]);
    assert_eq!(
        &live.retired_modules.lock().unwrap()[3..],
        ["a", "b", "sum", "a", "b", "sum", "a", "b", "sum"]
    );
}

#[test]
fn preview_reports_pure_cable_reordering_separately_from_additions_and_removals() {
    let mut session = make_session(2);
    session
        .execute(
            "connect",
            json!({"from":"b","to":"sum","input":"signal","mode":"stack","cable_id":"second"}),
        )
        .unwrap();
    let before = session.snapshot();
    let preview = session.execute("preview_edits", json!({"edits":[
        {"tool":"disconnect","arguments":{"cable_id":"cable_1"}},
        {"tool":"connect","arguments":{"from":"a","to":"sum","input":"signal","mode":"stack","cable_id":"cable_1"}}
    ]})).unwrap();
    assert_eq!(preview["changes"]["cable_order_changed"], true);
    assert_eq!(preview["changes"]["added_cables"], json!([]));
    assert_eq!(preview["changes"]["removed_cables"], json!([]));
    assert_eq!(preview["patch"]["cables"][0]["id"], "second");
    assert_eq!(session.snapshot(), before);
    let addition = session.execute("preview_edits", json!({"edits":[
        {"tool":"connect","arguments":{"from":"a","to":"sum","input":"signal","mode":"stack"}}
    ]})).unwrap();
    assert_eq!(addition["changes"]["cable_order_changed"], false);
    assert_eq!(
        addition["changes"]["added_cables"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        patch_changes(session.patch(), session.patch())["cable_order_changed"],
        false
    );
}

#[test]
fn clear_cables_retires_exact_ids_once_and_preserves_modules_in_both_versions() {
    for version in [1, 2] {
        let mut patch = raw_patch(version);
        patch.cables[0].id = Some("first".into());
        let mut second = patch.cables[0].clone();
        second.id = Some("second".into());
        if version == 1 {
            second.from = "b".into();
            second.input = "cv".into();
        }
        patch.cables.push(second);
        let mut session = Session::new(&std::env::temp_dir()).unwrap();
        session.replace_patch(patch, None).unwrap();
        let live = Arc::new(Live::default());
        session.set_live_module_events(Some(live.clone()));
        let before = session.snapshot();
        let request = json!({"expected_revision":session.revision(),"edits":[
            {"tool":"clear_cables","arguments":{}}
        ]});
        let preview = session.execute("preview_edits", request).unwrap();
        assert_eq!(session.snapshot(), before);
        for tool in ["preview_edits", "apply_edits"] {
            assert!(session.execute(tool, json!({"edits":[
                {"tool":"clear_cables","arguments":{}},
                {"tool":"connect","arguments":{"from":"missing","to":"sum","input":"signal"}}
            ]})).is_err());
            assert_eq!(session.snapshot(), before);
        }
        assert!(live.retired.lock().unwrap().is_empty());
        assert!(live.retired_modules.lock().unwrap().is_empty());
        assert_eq!(*live.synced.lock().unwrap(), vec![1]);
        let result = session
            .execute("clear_cables", json!({"expected_revision":1}))
            .unwrap();
        assert_eq!(result["patch"], preview["patch"]);
        let mut expected = before["patch"].clone();
        expected["cables"] = json!([]);
        assert_eq!(result["patch"], expected);
        assert_eq!(result["revision"], 2);
        assert_eq!(
            result["history"]["undo_steps"],
            before["history"]["undo_steps"].as_u64().unwrap() + 1
        );
        assert_eq!(*live.retired.lock().unwrap(), vec!["first", "second"]);
        assert!(live.retired_modules.lock().unwrap().is_empty());
        assert_eq!(*live.synced.lock().unwrap(), vec![1, 2]);
        session.execute("undo", json!({})).unwrap();
        assert_eq!(session.snapshot()["patch"], before["patch"]);
        session.execute("redo", json!({})).unwrap();
        assert_eq!(session.snapshot()["patch"], expected);
        assert_eq!(
            *live.retired.lock().unwrap(),
            vec!["first", "second", "first", "second", "first", "second"]
        );
        assert!(live.retired_modules.lock().unwrap().is_empty());
        assert_eq!(*live.synced.lock().unwrap(), vec![1, 2, 3, 4]);
    }
}

#[test]
fn clear_and_reconnect_identical_ids_replays_retirement_for_each_history_transition() {
    let mut session = make_session(2);
    session
        .execute(
            "connect",
            json!({"from":"a","to":"sum","input":"signal","mode":"stack","cable_id":"second"}),
        )
        .unwrap();
    let original = session.snapshot()["patch"].clone();
    let live = Arc::new(Live::default());
    session.set_live_module_events(Some(live.clone()));
    let edits = json!([
        {"tool":"clear_cables","arguments":{}},
        {"tool":"connect","arguments":{"from":"a","to":"sum","input":"signal","cable_id":"cable_1"}},
        {"tool":"connect","arguments":{"from":"a","to":"sum","input":"signal","mode":"stack","cable_id":"second"}}
    ]);
    session
        .execute("preview_edits", json!({"edits":edits}))
        .unwrap();
    assert!(live.retired.lock().unwrap().is_empty());
    assert_eq!(*live.synced.lock().unwrap(), vec![2]);
    session
        .execute("apply_edits", json!({"edits":edits}))
        .unwrap();
    session.execute("undo", json!({})).unwrap();
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], original);
    assert_eq!(
        *live.retired.lock().unwrap(),
        vec!["cable_1", "second", "cable_1", "second", "cable_1", "second"]
    );
    assert!(live.retired_modules.lock().unwrap().is_empty());
    assert_eq!(*live.synced.lock().unwrap(), vec![2, 3, 4, 5]);
}

#[test]
fn connect_then_clear_retires_new_candidate_id_even_when_it_never_reaches_live_patch() {
    let mut session = make_session(2);
    let live = Arc::new(Live::default());
    session.set_live_module_events(Some(live.clone()));
    let edits = json!([
        {"tool":"connect","arguments":{"from":"b","to":"sum","input":"signal","mode":"stack","cable_id":"candidate_only"}},
        {"tool":"clear_cables","arguments":{}}
    ]);
    session
        .execute("preview_edits", json!({"edits":edits}))
        .unwrap();
    assert!(live.retired.lock().unwrap().is_empty());
    session
        .execute("apply_edits", json!({"edits":edits}))
        .unwrap();
    assert!(session.patch().cables.is_empty());
    assert_eq!(
        *live.retired.lock().unwrap(),
        vec!["cable_1", "candidate_only"]
    );
    assert!(live.retired_modules.lock().unwrap().is_empty());
    assert_eq!(*live.synced.lock().unwrap(), vec![1, 2]);
}

#[test]
fn direct_empty_clear_preserves_group_state_identity_redo_and_backend_and_still_validates() {
    let mut session = make_session(2);
    session.execute("clear_cables", json!({})).unwrap();
    session
        .execute(
            "set_parameter",
            json!({"id":"a","parameter":"value","value":4,"history_group":"drag"}),
        )
        .unwrap();
    let live = Arc::new(Live::default());
    session.set_live_module_events(Some(live.clone()));
    let before = session.snapshot();
    let group = session.history_group.clone();
    let states: Vec<_> = session.history_state_ids().collect();
    assert_eq!(
        session
            .execute(
                "clear_cables",
                json!({"expected_revision":session.revision()})
            )
            .unwrap(),
        before
    );
    assert_eq!(session.history_group, group);
    assert_eq!(session.history_state_ids().collect::<Vec<_>>(), states);
    for args in [
        json!({"expected_revision":0}),
        json!({"expected_revision":false}),
        json!({"unknown":true}),
        json!({"history_group":"other"}),
    ] {
        assert!(session.execute("clear_cables", args).is_err());
        assert_eq!(session.snapshot(), before);
        assert_eq!(session.history_group, group);
    }
    assert_eq!(*live.synced.lock().unwrap(), vec![3]);
    assert!(live.retired.lock().unwrap().is_empty());
    assert!(live.retired_modules.lock().unwrap().is_empty());
    session
        .execute(
            "set_parameter",
            json!({"id":"a","parameter":"value","value":5,"history_group":"drag"}),
        )
        .unwrap();
    assert_eq!(session.snapshot()["history"], before["history"]);
    assert_eq!(session.state_id(), before["state_id"].as_u64().unwrap());
    session.execute("undo", json!({})).unwrap();
    let undone = session.snapshot();
    let synced = live.synced.lock().unwrap().clone();
    assert_eq!(undone["history"]["redo_steps"], 1);
    assert_eq!(session.execute("clear_cables", json!({})).unwrap(), undone);
    assert_eq!(*live.synced.lock().unwrap(), synced);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.patch.nodes[0].params["value"], 5.0);
    assert!(live.retired.lock().unwrap().is_empty());
    assert!(live.retired_modules.lock().unwrap().is_empty());
    session.seal_for_close(session.revision()).unwrap();
    let sealed = session.snapshot();
    let synced = live.synced.lock().unwrap().clone();
    assert!(session
        .execute("clear_cables", json!({}))
        .unwrap_err()
        .contains("closing"));
    assert_eq!(session.snapshot(), sealed);
    assert_eq!(*live.synced.lock().unwrap(), synced);
}
