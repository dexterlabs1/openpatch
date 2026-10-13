use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "openpatch-presets-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Live {
    revision: AtomicU64,
    available: AtomicBool,
    running: Mutex<BTreeMap<String, bool>>,
    calls: Mutex<Vec<Value>>,
}
impl Default for Live {
    fn default() -> Self {
        Self {
            revision: AtomicU64::new(0),
            available: AtomicBool::new(true),
            running: Mutex::new(BTreeMap::new()),
            calls: Mutex::new(Vec::new()),
        }
    }
}
impl LiveModuleEvents for Live {
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        unreachable!()
    }
    fn snapshot(&self) -> Value {
        let modules: serde_json::Map<_, _> = self.running.lock().unwrap().iter().map(|(id, value)| {
            (id.clone(), json!({"available":self.available.load(Ordering::Relaxed),"revision":self.revision.load(Ordering::Relaxed),"frame":123,"state":{"running":value}}))
        }).collect();
        json!({"modules":modules})
    }
    fn sync_patch(&self, patch: &Patch, revision: u64) {
        let mut running = self.running.lock().unwrap();
        running.retain(|id, _| {
            patch
                .nodes
                .iter()
                .any(|node| node.id == *id && node.kind == "seq3")
        });
        for node in patch.nodes.iter().filter(|node| node.kind == "seq3") {
            running
                .entry(node.id.clone())
                .or_insert_with(|| running::initial_running(node));
        }
        self.revision.store(revision, Ordering::Relaxed);
        self.calls.lock().unwrap().push(json!({"sync":revision}));
    }
    fn retire_modules(&self, ids: &[String]) {
        for id in ids {
            self.running.lock().unwrap().remove(id);
        }
        self.calls.lock().unwrap().push(json!({"retire":ids}));
    }
    fn initialize_modules(&self, ids: &[String]) {
        self.calls.lock().unwrap().push(json!({"initialize":ids}));
    }
    fn load_module_running(&self, states: &BTreeMap<String, bool>) {
        self.running.lock().unwrap().extend(states.clone());
        self.calls.lock().unwrap().push(json!({"load":states}));
    }
    fn release_module_controls(&self, ids: &[String]) {
        self.calls.lock().unwrap().push(json!({"release":ids}));
    }
}

fn preset(kind: &str, params: Value) -> Value {
    json!({"format":"openpatch-module-preset","version":1,"kind":kind,"params":params})
}
fn session() -> (Directory, Session, Arc<Live>) {
    let directory = Directory::new();
    let mut session = Session::new(&directory.0).unwrap();
    session
        .execute(
            "replace_patch",
            json!({"patch":{"version":2,"nodes":[
        {"id":"seq","kind":"seq3","params":{"running":1,"row1_1":3}},
        {"id":"vco","kind":"multi_vco","bypassed":true,"params":{"pulse_width":0.99}},
        {"id":"logic","kind":"logic"}],"cables":[
        {"id":"route","from":"seq","output":"cv1","to":"vco","input":"pitch"}],
        "output":"vco","output_port":"sine"}}),
        )
        .unwrap();
    let live = Arc::new(Live::default());
    session.set_live_module_events(Some(live.clone()));
    live.running.lock().unwrap().insert("seq".into(), false);
    live.calls.lock().unwrap().clear();
    (directory, session, live)
}

#[test]
fn captures_current_live_run_and_effective_defaults_without_mutating_state() {
    let (_directory, mut session, live) = session();
    let before = session.snapshot();
    let captured = session
        .execute(
            "capture_module_preset",
            json!({"id":"seq","expected_revision":1}),
        )
        .unwrap();
    assert_eq!(captured["state_source"], "live");
    assert_eq!(captured["frame"], 123);
    assert_eq!(captured["preset"]["params"]["running"], 0.0);
    assert_eq!(captured["preset"]["params"]["gate8"], 1.0);
    assert_eq!(captured["preset"]["params"]["row3_8"], 0.0);
    assert_eq!(captured["preset"]["bypassed"], false);
    assert!(captured["preset"].get("id").is_none());
    assert_eq!(session.snapshot(), before);
    assert!(live.calls.lock().unwrap().is_empty());
    live.available.store(false, Ordering::Relaxed);
    assert!(session
        .execute("capture_module_preset", json!({"id":"seq"}))
        .unwrap_err()
        .contains("unavailable"));
    live.available.store(true, Ordering::Relaxed);
    live.revision.store(0, Ordering::Relaxed);
    assert!(session
        .execute("capture_module_preset", json!({"id":"seq"}))
        .is_err());
    assert!(session
        .execute(
            "capture_module_preset",
            json!({"id":"seq","expected_revision":0})
        )
        .is_err());
    session.set_live_module_events(None);
    let offline = session
        .execute("capture_module_preset", json!({"id":"seq"}))
        .unwrap();
    assert_eq!(offline["state_source"], "offline_reset");
    assert_eq!(offline["preset"]["params"]["running"], 1.0);
    assert_eq!(offline["frame"], Value::Null);
    session.patch.nodes[0].params.remove("running");
    assert_eq!(
        session
            .execute("capture_module_preset", json!({"id":"seq"}))
            .unwrap()["preset"]["params"]["running"],
        1.0
    );
}

#[test]
fn explicit_equal_run_load_is_ordered_after_sync_and_recorded_for_history() {
    let (_directory, mut session, live) = session();
    let before = session.snapshot();
    let request =
        json!({"id":"seq","preset":preset("seq3",json!({"running":1})),"expected_revision":1});
    let preview = session.execute("preview_edits", json!({"edits":[{"tool":"apply_module_preset","arguments":{"id":"seq","preset":request["preset"]}}]})).unwrap();
    assert!(live.calls.lock().unwrap().is_empty());
    let after = session.execute("apply_module_preset", request).unwrap();
    assert_eq!(after["patch"], before["patch"]);
    assert_eq!(after["patch"], preview["patch"]);
    assert_eq!(after["revision"], 2);
    assert_eq!(
        *live.calls.lock().unwrap(),
        vec![
            json!({"sync":2}),
            json!({"release":["seq"]}),
            json!({"load":{"seq":true}})
        ]
    );
    assert!(live.running.lock().unwrap()["seq"]);
    live.calls.lock().unwrap().clear();
    session.execute("undo", json!({})).unwrap();
    assert!(!live.running.lock().unwrap()["seq"]);
    assert_eq!(
        *live.calls.lock().unwrap(),
        vec![
            json!({"sync":3}),
            json!({"release":["seq"]}),
            json!({"load":{"seq":false}})
        ]
    );
    live.running.lock().unwrap().insert("seq".into(), false);
    session.execute("redo", json!({})).unwrap();
    assert!(live.running.lock().unwrap()["seq"]);
    assert!(live
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|call| call.get("initialize").is_none()));
}

#[test]
fn partial_preset_preserves_omitted_run_bypass_and_structure() {
    let (_directory, mut session, live) = session();
    let before = session.snapshot();
    session
        .execute(
            "apply_module_preset",
            json!({"id":"seq","preset":preset("seq3",json!({"row1_1":7}))}),
        )
        .unwrap();
    assert!(!live.running.lock().unwrap()["seq"]);
    assert!(live
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|call| call.get("load").is_none()));
    assert_eq!(session.patch.nodes[0].params["running"], 1.0);
    assert_eq!(session.patch.nodes[0].params["row1_1"], 7.0);
    assert_eq!(
        session.snapshot()["patch"]["cables"],
        before["patch"]["cables"]
    );
    assert_eq!(session.patch.output, Some("vco".into()));
    session
        .execute(
            "apply_module_preset",
            json!({"id":"vco","preset":preset("multi_vco",json!({"frequency":440}))}),
        )
        .unwrap();
    assert!(session.patch.nodes[1].bypassed);
    assert_eq!(session.patch.nodes[1].params["pulse_width"], 0.99);
    let mut unbypass = preset("multi_vco", json!({}));
    unbypass["bypassed"] = json!(false);
    session
        .execute("apply_module_preset", json!({"id":"vco","preset":unbypass}))
        .unwrap();
    assert!(!session.patch.nodes[1].bypassed);
}

#[test]
fn module_change_history_restores_captured_run_but_bypass_and_cables_do_not() {
    for (tool, args) in [
        ("initialize_modules", json!({"ids":["seq"]})),
        ("randomize_modules", json!({"ids":["seq"],"seed":1})),
    ] {
        let (_directory, mut session, live) = session();
        session.execute(tool, args).unwrap();
        live.running.lock().unwrap().insert("seq".into(), true);
        live.calls.lock().unwrap().clear();
        session.execute("undo", json!({})).unwrap();
        assert!(!live.running.lock().unwrap()["seq"], "{tool}");
        live.running.lock().unwrap().insert("seq".into(), true);
        session.execute("redo", json!({})).unwrap();
        assert!(!live.running.lock().unwrap()["seq"], "{tool}");
        assert!(live
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|call| call.get("initialize").is_none()));
    }
    let (_directory, mut session, live) = session();
    for (tool, args) in [
        (
            "set_parameter",
            json!({"id":"seq","parameter":"row1_1","value":5}),
        ),
        ("rotate_sequence", json!({"id":"seq","direction":"left"})),
        ("set_bypass", json!({"id":"seq","bypassed":true})),
        ("disconnect", json!({"cable_id":"route"})),
    ] {
        session.execute(tool, args).unwrap();
        live.calls.lock().unwrap().clear();
        session.execute("undo", json!({})).unwrap();
        session.execute("redo", json!({})).unwrap();
        assert!(live
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|call| call.get("load").is_none()));
    }
}

#[test]
fn duplicate_and_removal_history_retain_actual_run_without_restoring_source() {
    let (_directory, mut session, live) = session();
    session
        .execute(
            "duplicate_modules",
            json!({"modules":[{"id":"seq","new_id":"copy"}]}),
        )
        .unwrap();
    assert_eq!(
        session
            .patch
            .nodes
            .iter()
            .find(|node| node.id == "copy")
            .unwrap()
            .params["running"],
        0.0
    );
    live.running.lock().unwrap().insert("seq".into(), true);
    live.running.lock().unwrap().insert("copy".into(), true);
    session.execute("undo", json!({})).unwrap();
    assert!(live.running.lock().unwrap()["seq"]);
    session.execute("redo", json!({})).unwrap();
    assert!(live.running.lock().unwrap()["seq"]);
    assert!(!live.running.lock().unwrap()["copy"]);
    session
        .execute("remove_module", json!({"id":"copy"}))
        .unwrap();
    session.execute("undo", json!({})).unwrap();
    assert!(!live.running.lock().unwrap()["copy"]);
}

#[test]
fn batch_load_order_and_replacement_clear_old_incarnation_requests() {
    let (_directory, mut session, live) = session();
    session.execute("apply_edits", json!({"edits":[
        {"tool":"apply_module_preset","arguments":{"id":"seq","preset":preset("seq3",json!({"running":0}))}},
        {"tool":"set_parameter","arguments":{"id":"seq","parameter":"running","value":1}}
    ]})).unwrap();
    assert!(live.running.lock().unwrap()["seq"]);
    live.calls.lock().unwrap().clear();
    session.execute("apply_edits", json!({"edits":[
        {"tool":"apply_module_preset","arguments":{"id":"seq","preset":preset("seq3",json!({"running":0}))}},
        {"tool":"remove_module","arguments":{"id":"seq"}},
        {"tool":"add_module","arguments":{"id":"seq","kind":"seq3","params":{"running":1}}}
    ]})).unwrap();
    assert!(live.running.lock().unwrap()["seq"]);
    assert!(live
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|call| call.get("load").is_none() && call.get("release").is_none()));
    session.execute("undo", json!({})).unwrap();
    assert!(live.running.lock().unwrap()["seq"]);
}

#[test]
fn temporary_duplicate_add_then_remove_restores_original_actual_run_on_undo() {
    let (_directory, mut session, live) = session();
    let before = session.snapshot()["patch"].clone();
    assert_eq!(session.patch.nodes[0].params["running"], 1.0);
    assert!(!live.running.lock().unwrap()["seq"]);
    session
        .execute(
            "apply_edits",
            json!({"edits":[
                {"tool":"add_module","arguments":{"id":"seq","kind":"seq3","params":{"running":1}}},
                {"tool":"remove_module","arguments":{"id":"seq"}}
            ]}),
        )
        .unwrap();
    assert!(!session.patch.nodes.iter().any(|node| node.id == "seq"));
    assert!(!live.running.lock().unwrap().contains_key("seq"));
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before);
    assert!(!live.running.lock().unwrap()["seq"]);
    session.execute("redo", json!({})).unwrap();
    assert!(!live.running.lock().unwrap().contains_key("seq"));
}

#[test]
fn failed_or_unavailable_live_operations_are_atomic_without_hooks() {
    let (_directory, mut session, live) = session();
    let before = session.snapshot();
    live.available.store(false, Ordering::Relaxed);
    for (tool, args) in [
        ("initialize_modules", json!({"ids":["seq"]})),
        ("randomize_modules", json!({"ids":["seq"],"seed":1})),
        (
            "apply_module_preset",
            json!({"id":"seq","preset":preset("seq3",json!({"running":0}))}),
        ),
        ("remove_module", json!({"id":"seq"})),
    ] {
        assert!(session.execute(tool, args).is_err());
        assert_eq!(session.snapshot(), before);
    }
    live.available.store(true, Ordering::Relaxed);
    for tool in ["preview_edits", "apply_edits"] {
        assert!(session.execute(tool,json!({"edits":[
            {"tool":"apply_module_preset","arguments":{"id":"seq","preset":preset("seq3",json!({"running":0}))}},
            {"tool":"connect","arguments":{"from":"missing","to":"vco","input":"fm"}}
        ]})).is_err());
        assert_eq!(session.snapshot(), before);
    }
    assert!(live.calls.lock().unwrap().is_empty());
}

#[test]
fn exact_reviewed_file_contents_survive_source_and_disk_changes() {
    let (directory, mut session, _live) = session();
    let captured = session
        .execute("capture_module_preset", json!({"id":"seq"}))
        .unwrap()["preset"]
        .clone();
    let snapshot = session.snapshot();
    session
        .execute(
            "save_module_preset",
            json!({"filename":"paused.opreset","preset":captured}),
        )
        .unwrap();
    assert_eq!(session.snapshot(), snapshot);
    let read = session
        .execute("read_module_preset", json!({"filename":"paused.opreset"}))
        .unwrap();
    assert_eq!(read["preset"], captured);
    let disk = preset("seq3", json!({"running":1}));
    fs::write(
        directory.0.join("paused.opreset"),
        serde_json::to_vec(&disk).unwrap(),
    )
    .unwrap();
    session
        .execute(
            "apply_module_preset",
            json!({"id":"seq","preset":read["preset"]}),
        )
        .unwrap();
    assert_eq!(session.patch.nodes[0].params["running"], 0.0);
    assert!(session
        .execute(
            "save_module_preset",
            json!({"filename":"paused.opreset","preset":captured})
        )
        .is_err());
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(directory.0.join("paused.opreset")).unwrap())
            .unwrap(),
        disk
    );
}

#[test]
fn library_is_sorted_filtered_paged_and_keeps_invalid_entries_visible() {
    let (directory, mut session, _live) = session();
    for (filename, kind) in [("z.opreset", "seq3"), ("a.opreset", "multi_vco")] {
        session
            .execute(
                "save_module_preset",
                json!({"filename":filename,"preset":preset(kind,json!({}))}),
            )
            .unwrap();
    }
    fs::write(directory.0.join("broken.opreset"), b"not json").unwrap();
    let listing = session
        .execute("list_module_presets", json!({"limit":2}))
        .unwrap();
    assert_eq!(listing["total"], 3);
    assert_eq!(listing["presets"][0]["filename"], "a.opreset");
    assert_eq!(listing["presets"][1]["filename"], "broken.opreset");
    assert_eq!(listing["presets"][1]["valid"], false);
    let filtered = session
        .execute("list_module_presets", json!({"kind":"seq3"}))
        .unwrap();
    assert_eq!(filtered["total"], 2);
    assert_eq!(filtered["presets"][1]["filename"], "z.opreset");
    assert_eq!(
        session
            .execute("list_module_presets", json!({"offset":2,"limit":2}))
            .unwrap()["presets"][0]["filename"],
        "z.opreset"
    );
    let search = session
        .execute(
            "list_module_presets",
            json!({"query":"Z.OPRESET","limit":1}),
        )
        .unwrap();
    assert_eq!(search["total"], 1);
    assert_eq!(search["presets"][0]["filename"], "z.opreset");
    for args in [
        json!({"limit":0}),
        json!({"limit":101}),
        json!({"offset":-1}),
        json!({"limit":true}),
        json!({"kind":"unknown"}),
        json!({"extra":true}),
    ] {
        assert!(session.execute("list_module_presets", args).is_err());
    }
}

#[test]
fn preset_file_names_limits_and_unsafe_files_are_rejected() {
    let (directory, mut session, _live) = session();
    let value = preset("multi_vco", json!({}));
    for filename in [
        "../escape.opreset",
        ".hidden.opreset",
        "bad name.opreset",
        "wrong.json",
        "",
        "é.opreset",
    ] {
        assert!(session
            .execute(
                "save_module_preset",
                json!({"filename":filename,"preset":value})
            )
            .is_err());
    }
    fs::create_dir(directory.0.join("directory.opreset")).unwrap();
    assert!(session
        .execute(
            "read_module_preset",
            json!({"filename":"directory.opreset"})
        )
        .is_err());
    fs::write(
        directory.0.join("oversized.opreset"),
        vec![b' '; crate::presets::MAX_BYTES + 1],
    )
    .unwrap();
    assert!(session
        .execute(
            "read_module_preset",
            json!({"filename":"oversized.opreset"})
        )
        .is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            directory.0.join("oversized.opreset"),
            directory.0.join("link.opreset"),
        )
        .unwrap();
        assert!(session
            .execute("read_module_preset", json!({"filename":"link.opreset"}))
            .is_err());
        assert!(session
            .execute(
                "save_module_preset",
                json!({"filename":"link.opreset","preset":value})
            )
            .is_err());
    }
    assert!(fs::read_dir(&directory.0).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".openpatch-preset-")));
}

#[test]
fn malformed_presets_and_target_failures_do_not_change_state() {
    let (_directory, mut session, live) = session();
    let before = session.snapshot();
    let mut invalid = vec![
        json!(null),
        preset("unknown", json!({})),
        preset("multi_vco", json!({"frequency":-1})),
        preset("multi_vco", json!({"soft_sync":0.5})),
        preset("multi_vco", json!({"frequency":"220"})),
        preset("multi_vco", json!({"unknown":1})),
    ];
    let mut extra = preset("multi_vco", json!({}));
    extra["id"] = json!("forbidden");
    invalid.push(extra);
    let mut bypass = preset("multi_vco", json!({}));
    bypass["bypassed"] = json!(1);
    invalid.push(bypass);
    let mut version = preset("multi_vco", json!({}));
    version["version"] = json!(2);
    invalid.push(version);
    for preset in invalid {
        assert!(session
            .execute("apply_module_preset", json!({"id":"vco","preset":preset}))
            .is_err());
        assert_eq!(session.snapshot(), before);
    }
    assert!(session
        .execute(
            "apply_module_preset",
            json!({"id":"vco","preset":preset("seq3",json!({}))})
        )
        .is_err());
    assert!(session
        .execute(
            "apply_module_preset",
            json!({"id":"vco","preset":preset("multi_vco",json!({})),"expected_revision":0})
        )
        .is_err());
    assert!(live.calls.lock().unwrap().is_empty());
}
