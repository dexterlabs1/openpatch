use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct MockLive {
    queued: AtomicUsize,
    synced: Mutex<Vec<u64>>,
    retired: Mutex<Vec<String>>,
}
impl LiveModuleEvents for MockLive {
    fn trigger(&self, id: &str, action: ModuleAction) -> Result<Value, String> {
        let sequence = self.queued.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(json!({"status":"queued","id":id,"action":action,"sequence":sequence}))
    }
    fn snapshot(&self) -> Value {
        json!({"queued":self.queued.load(Ordering::Relaxed),"applied":0,"dropped":0,"backpressure":0})
    }
    fn retire_modules(&self, ids: &[String]) {
        self.retired.lock().unwrap().extend_from_slice(ids);
    }
    fn sync_patch(&self, _patch: &Patch, revision: u64) {
        self.synced.lock().unwrap().push(revision);
    }
}
fn session() -> Session {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    s.execute(
        "add_module",
        json!({"id":"seq","kind":"seq3","params":{"tempo":15,"row1_1":1,"row1_2":2,"row1_3":3}}),
    )
    .unwrap();
    s
}

#[test]
fn live_actions_are_queue_receipts_and_leave_patch_and_history_unchanged() {
    let mut s = session();
    let before = s.snapshot();
    assert!(s
        .execute("trigger_module", json!({"id":"seq","action":"clock"}))
        .unwrap_err()
        .contains("unsupported"));
    assert_eq!(
        s.execute("inspect_performance", json!({})).unwrap()["supported"],
        false
    );
    let backend = Arc::new(MockLive::default());
    s.set_live_module_events(Some(backend.clone()));
    let queued = s
        .execute(
            "trigger_module",
            json!({"id":"seq","action":"clock","expected_revision":1}),
        )
        .unwrap();
    assert_eq!(queued["receipt"]["status"], "queued");
    assert_eq!(queued["receipt"]["sequence"], 1);
    let telemetry = s
        .execute("inspect_performance", json!({"expected_revision":1}))
        .unwrap();
    assert_eq!(telemetry["performance"]["queued"], 1);
    assert_eq!(telemetry["performance"]["applied"], 0);
    for args in [
        json!({"id":"seq","action":"clock","expected_revision":0}),
        json!({"id":"missing","action":"clock"}),
        json!({"id":"seq","action":"start"}),
        json!({"id":"seq","action":1}),
    ] {
        assert!(s.execute("trigger_module", args).is_err());
    }
    assert!(s
        .execute("inspect_performance", json!({"expected_revision":0}))
        .is_err());
    assert_eq!(backend.queued.load(Ordering::Relaxed), 1);
    assert_eq!(s.snapshot(), before);
    s.seal_for_close(1).unwrap();
    assert!(s
        .execute("trigger_module", json!({"id":"seq","action":"reset"}))
        .unwrap_err()
        .contains("closing"));
    assert!(s.execute("inspect_performance", json!({})).is_ok());
    assert_eq!(backend.queued.load(Ordering::Relaxed), 1);
    s.set_live_module_events(None);
    assert_eq!(
        s.execute("inspect_performance", json!({})).unwrap()["supported"],
        false
    );
}

#[test]
fn rotations_move_all_columns_in_one_history_entry_and_sync_every_committed_state() {
    let mut s = session();
    let backend = Arc::new(MockLive::default());
    s.set_live_module_events(Some(backend.clone()));
    let mut patch = s.patch.clone();
    for row in 1..=3 {
        for step in 1..=8 {
            patch.nodes[0]
                .params
                .insert(format!("row{row}_{step}"), row as f32 + step as f32 / 10.0);
        }
    }
    patch.nodes[0].params.insert("gate1".into(), 0.0);
    patch.nodes[0].params.insert("gate4".into(), 0.0);
    s.replace_patch(patch, Some(1)).unwrap();
    let before = s.snapshot();
    let undo_count = s.undo.len();
    s.execute(
        "rotate_sequence",
        json!({"id":"seq","direction":"left","expected_revision":2}),
    )
    .unwrap();
    assert_eq!(s.undo.len(), undo_count + 1);
    for row in 1..=3 {
        for step in 1..=8 {
            assert_eq!(
                s.patch.nodes[0].params[&format!("row{row}_{step}")],
                row as f32 + (step % 8 + 1) as f32 / 10.0
            );
        }
    }
    assert_eq!(s.patch.nodes[0].params["gate8"], 0.0);
    assert_eq!(s.patch.nodes[0].params["gate3"], 0.0);
    assert_eq!(s.patch.nodes[0].params["gate1"], 1.0);
    s.execute("undo", json!({"expected_revision":3})).unwrap();
    assert_eq!(s.snapshot()["patch"], before["patch"]);
    s.execute("redo", json!({"expected_revision":4})).unwrap();
    s.execute(
        "rotate_sequence",
        json!({"id":"seq","direction":"right","expected_revision":5}),
    )
    .unwrap();
    let restored = &s.patch.nodes[0].params;
    for row in 1..=3 {
        for step in 1..=8 {
            assert_eq!(
                restored[&format!("row{row}_{step}")],
                row as f32 + step as f32 / 10.0
            );
        }
    }
    assert_eq!(restored["gate1"], 0.0);
    assert_eq!(restored["gate4"], 0.0);
    assert_eq!(
        backend.synced.lock().unwrap().as_slice(),
        &[1, 2, 3, 4, 5, 6]
    );
    let unchanged = s.snapshot();
    for args in [
        json!({"id":"seq","direction":"up"}),
        json!({"id":"seq","direction":"left","expected_revision":0}),
        json!({"id":"missing","direction":"left"}),
    ] {
        assert!(s.execute("rotate_sequence", args).is_err());
        assert_eq!(s.snapshot(), unchanged);
    }
    s.seal_for_close(s.revision).unwrap();
    assert!(s
        .execute("rotate_sequence", json!({"id":"seq","direction":"left"}))
        .is_err());
}

#[test]
fn omitted_rotation_cells_use_catalog_defaults_and_non_sequence_targets_are_rejected() {
    let mut s = session();
    s.execute("rotate_sequence", json!({"id":"seq","direction":"right"}))
        .unwrap();
    assert_eq!(s.patch.nodes[0].params["row1_1"], 0.0);
    assert_eq!(s.patch.nodes[0].params["row1_2"], 1.0);
    assert_eq!(s.patch.nodes[0].params["row3_8"], 0.0);
    assert_eq!(s.patch.nodes[0].params["gate8"], 1.0);
    s.execute("add_module", json!({"id":"dc","kind":"constant"}))
        .unwrap();
    for (tool, args) in [
        ("rotate_sequence", json!({"id":"dc","direction":"left"})),
        ("trigger_module", json!({"id":"dc","action":"clock"})),
    ] {
        assert!(s.execute(tool, args).unwrap_err().contains("seq3"));
    }
}

#[test]
fn module_actions_capture_exact_frames_and_analysis_matches_bins_without_mutation() {
    let mut s = session();
    let before = s.snapshot();
    let trace = json!([
        {"frame":10,"id":"seq","action":"clock"},
        {"frame":20,"id":"seq","action":"reset"}
    ]);
    let args = json!({"id":"seq","port":"cv1","seconds":0.001,"points":64,"module_events":trace,"expected_revision":1});
    let capture = s.execute("capture_signal", args.clone()).unwrap();
    assert_eq!(capture["module_event_count"], 2);
    assert_eq!(capture["midi_event_count"], 0);
    assert_eq!(capture["points"], 48);
    for (frame, bin) in capture["bins"].as_array().unwrap().iter().enumerate() {
        assert_eq!(
            bin["min"],
            if (10..20).contains(&frame) { 2.0 } else { 1.0 }
        );
        assert_eq!(bin["max"], bin["min"]);
    }
    let analysis = s.execute("analyze_signal", json!({"id":"seq","port":"cv1","seconds":0.001,"module_events":trace,"expected_revision":1})).unwrap();
    assert_eq!(analysis["channels"][0]["mean"], capture["summary"]["dc"]);
    assert_eq!(analysis["channels"][0]["rms"], capture["summary"]["rms"]);
    assert_eq!(analysis["module_event_count"], 2);
    assert_eq!(s.execute("capture_signal", args).unwrap(), capture);
    assert_eq!(s.snapshot(), before);
}

#[test]
fn same_frame_action_order_is_preserved_and_reset_suppresses_clock_for_one_millisecond() {
    let mut s = session();
    for (actions, expected) in [
        (vec!["run", "clock"], 1.0),
        (vec!["clock", "run"], 2.0),
        (vec!["reset", "clock"], 1.0),
        (vec!["clock", "reset"], 1.0),
    ] {
        let events: Vec<_> = actions
            .into_iter()
            .map(|action| json!({"frame":0,"id":"seq","action":action}))
            .collect();
        let result = s
            .execute(
                "capture_signal",
                json!({"id":"seq","port":"cv1","seconds":0.001,"module_events":events}),
            )
            .unwrap();
        assert_eq!(result["bins"][0]["mean"], expected);
    }
    let result = s
        .execute(
            "capture_signal",
            json!({"id":"seq","port":"cv1","seconds":0.002,"points":2048,"module_events":[
                {"frame":0,"id":"seq","action":"reset"},
                {"frame":47,"id":"seq","action":"clock"},
                {"frame":48,"id":"seq","action":"clock"}
            ]}),
        )
        .unwrap();
    assert_eq!(result["bins"][47]["mean"], 1.0);
    assert_eq!(result["bins"][48]["mean"], 2.0);
}

#[test]
fn binned_sequence_trace_preserves_single_step_extrema_and_combined_midi_clock() {
    let mut s = session();
    s.execute("apply_edits", json!({"edits":[
        {"tool":"add_module","arguments":{"id":"midi","kind":"midi_cv"}},
        {"tool":"connect","arguments":{"from":"midi","output":"clock","to":"seq","input":"clock"}}
    ]})).unwrap();
    let result = s
        .execute(
            "capture_signal",
            json!({"id":"seq","port":"cv1","seconds":0.01,"points":64,"midi_events":[
                {"frame":100,"event":{"type":"clock"}}
            ],"module_events":[
                {"frame":99,"id":"seq","action":"clock"},
                {"frame":100,"id":"seq","action":"reset"}
            ]}),
        )
        .unwrap();
    let bin = result["bins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|bin| {
            bin["start_frame"].as_u64().unwrap() <= 99 && bin["end_frame"].as_u64().unwrap() > 99
        })
        .unwrap();
    assert_eq!(bin["max"], 2.0);
    assert_eq!(bin["min"], 1.0);
    // MIDI's rising clock arrives through its delayed cable at frame 101,
    // while the reset guard is active; the only 2 V sample is frame 99.
    assert_eq!(result["summary"]["dc"], 481.0 / 480.0);
    assert_eq!(result["midi_event_count"], 1);
    assert_eq!(result["module_event_count"], 2);
}

#[test]
fn invalid_module_traces_fail_all_offline_tools_before_creating_a_file() {
    let path = std::env::temp_dir().join(format!(
        "openpatch-seq-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut s = Session::new(&path).unwrap();
    s.execute("add_module", json!({"id":"seq","kind":"seq3"}))
        .unwrap();
    s.execute("add_module", json!({"id":"dc","kind":"constant"}))
        .unwrap();
    s.execute("set_output", json!({"id":"seq","port":"cv1"}))
        .unwrap();
    let before = s.snapshot();
    let mut invalid = vec![
        json!([{"frame":48,"id":"seq","action":"clock"}]),
        json!([{"frame":1,"id":"seq","action":"clock"},{"frame":0,"id":"seq","action":"reset"}]),
        json!([{"frame":0,"id":"missing","action":"clock"}]),
        json!([{"frame":0,"id":"dc","action":"clock"}]),
        json!([{"frame":0,"id":"seq","action":"clock","extra":1}]),
        json!([{"frame":0,"id":"seq","action":"start"}]),
        json!([{"frame":0,"id":"seq","action":"clock"},{"frame":0,"id":"bad id","action":"clock"}]),
    ];
    invalid.push(Value::Array(vec![
        json!({"frame":0,"id":"seq","action":"clock"});
        65
    ]));
    invalid.push(Value::Array(vec![
        json!({"frame":0,"id":"seq","action":"clock"});
        65537
    ]));
    for trace in invalid {
        for tool in ["capture_signal", "analyze_signal", "render_audio"] {
            let args = if tool == "render_audio" {
                json!({"filename":"invalid.wav","seconds":0.001,"module_events":trace})
            } else {
                json!({"id":"seq","port":"cv1","seconds":0.001,"module_events":trace})
            };
            assert!(
                s.execute(tool, args).is_err(),
                "{tool} accepted invalid trace"
            );
            assert_eq!(s.snapshot(), before);
            assert!(!path.join("invalid.wav").exists());
        }
    }
    let events = json!([{ "frame":10,"id":"seq","action":"clock" }]);
    s.execute(
        "render_audio",
        json!({"filename":"one.wav","seconds":0.001,"module_events":events}),
    )
    .unwrap();
    s.execute(
        "render_audio",
        json!({"filename":"two.wav","seconds":0.001,"module_events":events}),
    )
    .unwrap();
    assert_eq!(
        fs::read(path.join("one.wav")).unwrap(),
        fs::read(path.join("two.wav")).unwrap()
    );
    assert_eq!(s.snapshot(), before);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn explicit_replacement_retires_identity_only_after_successful_commit() {
    let mut s = session();
    let backend = Arc::new(MockLive::default());
    s.set_live_module_events(Some(backend.clone()));
    let before = s.snapshot();
    let node = s
        .patch
        .nodes
        .iter()
        .find(|n| n.id == "seq")
        .unwrap()
        .clone();
    let edits = json!([
        {"tool":"remove_module","arguments":{"id":"seq"}},
        {"tool":"add_module","arguments":{"id":node.id,"kind":node.kind,"params":node.params}}
    ]);
    s.execute("preview_edits", json!({"edits":edits})).unwrap();
    assert!(backend.retired.lock().unwrap().is_empty());
    assert!(s
        .execute("apply_edits", json!({"edits":edits,"expected_revision":0}))
        .is_err());
    let invalid = json!([
        {"tool":"remove_module","arguments":{"id":"seq"}},
        {"tool":"add_module","arguments":{"id":"seq","kind":"unknown_model"}}
    ]);
    assert!(s.execute("apply_edits", json!({"edits":invalid})).is_err());
    assert!(backend.retired.lock().unwrap().is_empty());
    assert_eq!(s.snapshot(), before);
    s.execute("apply_edits", json!({"edits":edits,"expected_revision":1}))
        .unwrap();
    assert_eq!(s.revision, 2);
    assert_eq!(s.snapshot()["patch"], before["patch"]);
    assert_eq!(*backend.retired.lock().unwrap(), vec!["seq"]);
    assert_eq!(*backend.synced.lock().unwrap(), vec![1, 2]);
}

#[test]
fn document_replacements_retire_sequences_but_rejected_loads_do_not() {
    let root = std::env::temp_dir().join(format!(
        "openpatch-seq-load-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut s = Session::new(&root).unwrap();
    s.execute("add_module", json!({"id":"seq","kind":"seq3"}))
        .unwrap();
    let patch = s.patch.clone();
    let backend = Arc::new(MockLive::default());
    s.set_live_module_events(Some(backend.clone()));
    s.execute(
        "set_parameter",
        json!({"id":"seq","parameter":"tempo","value":90}),
    )
    .unwrap();
    assert!(backend.retired.lock().unwrap().is_empty());
    let mut invalid = patch.clone();
    invalid.nodes[0].kind = "invalid".into();
    assert!(s.replace_patch(invalid, None).is_err());
    assert!(s
        .execute("load_patch", json!({"filename":"missing.json"}))
        .is_err());
    assert!(s
        .execute(
            "replace_patch",
            json!({"patch":patch,"expected_revision":0})
        )
        .is_err());
    assert!(backend.retired.lock().unwrap().is_empty());
    s.replace_patch(patch.clone(), Some(2)).unwrap();
    s.execute("replace_patch", json!({"patch":patch})).unwrap();
    fs::write(
        root.join("sequence.json"),
        serde_json::to_vec(&patch).unwrap(),
    )
    .unwrap();
    s.execute("load_patch", json!({"filename":"sequence.json"}))
        .unwrap();
    assert_eq!(*backend.retired.lock().unwrap(), vec!["seq", "seq", "seq"]);
    assert_eq!(*backend.synced.lock().unwrap(), vec![1, 2, 3, 4, 5]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn switch_actions_remain_seq3_only_and_replacements_retire_all_telemetry_modules() {
    let root = std::env::temp_dir().join(format!(
        "openpatch-switch-load-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut s = Session::new(&root).unwrap();
    for (id, kind) in [
        ("seq", "seq3"),
        ("split", "switch_1to4"),
        ("join", "switch_4to1"),
    ] {
        s.execute("add_module", json!({"id":id,"kind":kind}))
            .unwrap();
    }
    let backend = Arc::new(MockLive::default());
    s.set_live_module_events(Some(backend.clone()));
    let patch = s.patch.clone();
    let before = s.snapshot();
    for id in ["split", "join"] {
        for action in ["clock", "reset", "run"] {
            assert!(s
                .execute("trigger_module", json!({"id":id,"action":action}))
                .unwrap_err()
                .contains("requires a seq3"));
        }
    }
    assert_eq!(backend.queued.load(Ordering::Relaxed), 0);
    assert_eq!(s.snapshot(), before);
    s.execute(
        "set_parameter",
        json!({"id":"split","parameter":"steps","value":3}),
    )
    .unwrap();
    assert!(backend.retired.lock().unwrap().is_empty());
    let mut invalid = patch.clone();
    invalid.nodes[1].params.insert("steps".into(), 5.0);
    assert!(s.replace_patch(invalid, None).is_err());
    assert!(backend.retired.lock().unwrap().is_empty());
    s.replace_patch(patch.clone(), None).unwrap();
    s.execute("replace_patch", json!({"patch":patch})).unwrap();
    fs::write(
        root.join("switches.json"),
        serde_json::to_vec(&patch).unwrap(),
    )
    .unwrap();
    s.execute("load_patch", json!({"filename":"switches.json"}))
        .unwrap();
    assert_eq!(
        *backend.retired.lock().unwrap(),
        vec!["seq", "split", "join", "seq", "split", "join", "seq", "split", "join"]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bypass_is_a_strict_atomic_graph_edit_and_seq3_actions_are_unavailable() {
    let mut s = session();
    let backend = Arc::new(MockLive::default());
    s.set_live_module_events(Some(backend.clone()));
    let before = s.snapshot();
    for bypassed in [json!(0), json!(1), json!("true"), Value::Null] {
        assert!(s
            .execute("set_bypass", json!({"id":"seq","bypassed":bypassed}))
            .is_err());
        assert_eq!(s.snapshot(), before);
    }
    let edits = json!([{"tool":"set_bypass","arguments":{"id":"seq","bypassed":true}}]);
    let preview = s.execute("preview_edits", json!({"edits":edits})).unwrap();
    assert_eq!(
        preview["changes"]["changed_modules"][0]["after"]["bypassed"],
        true
    );
    assert_eq!(s.snapshot(), before);
    s.execute("apply_edits", json!({"edits":edits,"expected_revision":1}))
        .unwrap();
    assert!(s.patch.nodes[0].bypassed);
    assert_eq!(s.revision, 2);
    assert_eq!(
        s.snapshot()["history"]["undo_steps"],
        before["history"]["undo_steps"].as_u64().unwrap() + 1
    );
    for action in ["clock", "run", "reset"] {
        assert!(s
            .execute("trigger_module", json!({"id":"seq","action":action}))
            .unwrap_err()
            .contains("bypassed"));
    }
    assert_eq!(backend.queued.load(Ordering::Relaxed), 0);
    s.execute(
        "set_parameter",
        json!({"id":"seq","parameter":"row1_1","value":4}),
    )
    .unwrap();
    s.execute("rotate_sequence", json!({"id":"seq","direction":"right"}))
        .unwrap();
    assert!(s.patch.nodes[0].bypassed);
    let before_invalid = s.snapshot();
    assert!(s
        .execute(
            "apply_edits",
            json!({"edits":[
                {"tool":"set_bypass","arguments":{"id":"seq","bypassed":false}},
                {"tool":"set_bypass","arguments":{"id":"missing","bypassed":true}}
            ]})
        )
        .is_err());
    assert_eq!(s.snapshot(), before_invalid);
    s.execute("set_bypass", json!({"id":"seq","bypassed":false}))
        .unwrap();
    assert!(!s.patch.nodes[0].bypassed);
    assert!(s.snapshot()["patch"]["nodes"][0].get("bypassed").is_none());
    s.execute("undo", json!({})).unwrap();
    assert!(s.patch.nodes[0].bypassed);
    s.execute("redo", json!({})).unwrap();
    assert!(!s.patch.nodes[0].bypassed);
    s.execute("trigger_module", json!({"id":"seq","action":"clock"}))
        .unwrap();
    assert_eq!(backend.queued.load(Ordering::Relaxed), 1);
}

#[test]
fn invert_mutes_inverts_all_defaults_and_batches_with_bypass_in_one_undo() {
    let mut s = session();
    s.execute(
        "add_module",
        json!({"id":"mutes","kind":"mutes10","params":{"mute2":1,"mute10":1}}),
    )
    .unwrap();
    let before = s.snapshot();
    let edits = json!([
        {"tool":"set_bypass","arguments":{"id":"mutes","bypassed":true}},
        {"tool":"invert_mutes","arguments":{"id":"mutes"}}
    ]);
    let preview = s.execute("preview_edits", json!({"edits":edits})).unwrap();
    assert_eq!(
        preview["patch"]["nodes"][1]["params"]["mute1"].as_f64(),
        Some(1.0)
    );
    assert_eq!(
        preview["patch"]["nodes"][1]["params"]["mute2"].as_f64(),
        Some(0.0)
    );
    assert_eq!(s.snapshot(), before);
    s.execute("apply_edits", json!({"edits":edits,"expected_revision":2}))
        .unwrap();
    assert_eq!(s.revision, 3);
    assert_eq!(
        s.snapshot()["history"]["undo_steps"],
        before["history"]["undo_steps"].as_u64().unwrap() + 1
    );
    let node = &s.patch.nodes[1];
    assert!(node.bypassed);
    for channel in 1..=10 {
        assert_eq!(
            node.params[&format!("mute{channel}")],
            if [2, 10].contains(&channel) { 0.0 } else { 1.0 }
        );
    }
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.snapshot()["patch"], before["patch"]);
    s.execute("redo", json!({})).unwrap();
    let before_invalid = s.snapshot();
    assert!(s
        .execute("invert_mutes", json!({"id":"seq"}))
        .unwrap_err()
        .contains("mutes10"));
    assert!(s.execute("invert_mutes", json!({"id":"missing"})).is_err());
    assert!(s
        .execute(
            "apply_edits",
            json!({"edits":[
                {"tool":"invert_mutes","arguments":{"id":"mutes"}},
                {"tool":"set_parameter","arguments":{"id":"mutes","parameter":"mute1","value":0.5}}
            ]})
        )
        .is_err());
    assert_eq!(s.snapshot(), before_invalid);
    s.execute("invert_mutes", json!({"id":"mutes"})).unwrap();
    for channel in 1..=10 {
        assert_eq!(
            s.patch.nodes[1].params[&format!("mute{channel}")],
            if [2, 10].contains(&channel) { 1.0 } else { 0.0 }
        );
    }
}
