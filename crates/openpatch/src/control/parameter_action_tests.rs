use super::*;

#[derive(Default)]
struct Live {
    calls: Mutex<Vec<Value>>,
    current: Mutex<Option<(Patch, u64)>>,
}

impl LiveModuleEvents for Live {
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        unreachable!()
    }
    fn snapshot(&self) -> Value {
        let current = self.current.lock().unwrap();
        let (patch, revision) = current.as_ref().unwrap();
        let modules: serde_json::Map<String,Value> = patch.nodes.iter().filter(|node| node.kind == "seq3").map(|node| {
            (node.id.clone(),json!({"available":true,"revision":revision,"frame":0,"state":{"running":running::initial_running(node)}}))
        }).collect();
        json!({"modules":modules})
    }
    fn load_module_running(&self, states: &BTreeMap<String, bool>) {
        self.calls.lock().unwrap().push(json!({"load":states}));
    }
    fn retire_modules(&self, ids: &[String]) {
        self.calls.lock().unwrap().push(json!({"retire":ids}));
    }
    fn retire_cables(&self, ids: &[String]) {
        self.calls
            .lock()
            .unwrap()
            .push(json!({"retire_cables":ids}));
    }
    fn sync_patch(&self, patch: &Patch, revision: u64) {
        *self.current.lock().unwrap() = Some((patch.clone(), revision));
        self.calls
            .lock()
            .unwrap()
            .push(json!({"sync":revision,"patch":patch}));
    }
    fn initialize_modules(&self, ids: &[String]) {
        self.calls.lock().unwrap().push(json!({"initialize":ids}));
    }
}

fn session() -> (Session, Arc<Live>) {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session.execute("replace_patch", json!({"patch":{"version":2,"nodes":[
        {"id":"vco","kind":"multi_vco","bypassed":true,"params":{"frequency":440,"fm_amount":0.25,"pwm_amount":-0.5,"pulse_width":0.75}},
        {"id":"lfo","kind":"multi_lfo","params":{"frequency":8,"fm_amount":-0.25,"pwm_amount":0.75}},
        {"id":"seq","kind":"seq3","params":{"tempo":240,"tempo_amount":0.5,"steps":4,"steps_amount":0.25,"passthrough":1,"row1_1":3,"gate1":0}},
        {"id":"switch","kind":"switch_1to4","params":{"steps":2,"declick":1}}
    ],"cables":[{"id":"route","from":"vco","output":"sine","to":"lfo","input":"fm"}],"output":"vco","output_port":"triangle"}})).unwrap();
    let live = Arc::new(Live::default());
    session.set_live_module_events(Some(live.clone())).unwrap();
    live.calls.lock().unwrap().clear();
    (session, live)
}

fn node<'a>(session: &'a Session, id: &str) -> &'a Node {
    session
        .patch()
        .nodes
        .iter()
        .find(|node| node.id == id)
        .unwrap()
}

#[test]
fn initialize_preview_commit_and_history_preserve_structure_and_hook_order() {
    let (mut session, live) = session();
    let before = session.snapshot();
    let request = json!({"edits":[{"tool":"initialize_modules","arguments":{"ids":["vco","switch","seq","lfo"]}}],"expected_revision":1});
    let preview = session.execute("preview_edits", request.clone()).unwrap();
    assert_eq!(session.snapshot(), before);
    assert!(live.calls.lock().unwrap().is_empty());
    let committed = session.execute("apply_edits", request).unwrap();
    assert_eq!(committed["patch"], preview["patch"]);
    assert_eq!(committed["revision"], 2);
    assert_eq!(committed["history"]["undo_steps"], 2);
    assert_eq!(committed["patch"]["cables"], before["patch"]["cables"]);
    assert_eq!(committed["patch"]["output"], before["patch"]["output"]);
    assert_eq!(
        committed["patch"]["output_port"],
        before["patch"]["output_port"]
    );
    assert!(node(&session, "vco").bypassed);
    assert_eq!(node(&session, "vco").params["fm_amount"], 0.0);
    assert_eq!(node(&session, "switch").params["steps"], 4.0);
    assert_eq!(node(&session, "switch").params["declick"], 0.0);
    assert_eq!(node(&session, "seq").params["gate1"], 1.0);
    assert_eq!(node(&session, "seq").params["row1_1"], 0.0);
    assert!(!node(&session, "seq").params.contains_key("running"));
    let calls = live.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["sync"], 2);
    assert_eq!(calls[0]["patch"], committed["patch"]);
    assert_eq!(calls[1], json!({"initialize":["lfo","seq","switch","vco"]}));
    live.calls.lock().unwrap().clear();
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], committed["patch"]);
    let calls = live.calls.lock().unwrap();
    assert_eq!(calls.len(), 4);
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.get("sync").is_some())
            .count(),
        2
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.get("load").is_some())
            .count(),
        2
    );
}

#[test]
fn initialization_at_defaults_still_commits_and_preserves_running_presence() {
    for running in [None, Some(0.0), Some(1.0)] {
        let (mut session, live) = session();
        let seq = session
            .patch
            .nodes
            .iter_mut()
            .find(|node| node.id == "seq")
            .unwrap();
        if let Some(value) = running {
            seq.params.insert("running".into(), value);
        }
        for _ in 0..2 {
            let revision = session.revision();
            let history = session.undo.len();
            session
                .execute(
                    "initialize_modules",
                    json!({"ids":["seq"],"expected_revision":revision}),
                )
                .unwrap();
            assert_eq!(session.revision(), revision + 1);
            assert_eq!(session.undo.len(), history + 1);
            assert_eq!(
                node(&session, "seq").params.get("running").copied(),
                running
            );
        }
        assert_eq!(
            live.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| call.get("initialize").is_some())
                .count(),
            2
        );
    }
}

#[test]
fn invalid_targets_seeds_revisions_and_batches_are_atomic_without_hooks() {
    let (mut session, live) = session();
    let before = session.snapshot();
    let oversized: Vec<_> = (0..257).map(|index| format!("id{index}")).collect();
    for args in [
        json!({}),
        json!({"ids":[]}),
        json!({"ids":null}),
        json!({"ids":"vco"}),
        json!({"ids":[null]}),
        json!({"ids":[1]}),
        json!({"ids":["vco","vco"]}),
        json!({"ids":["vco","missing"]}),
        json!({"ids":oversized}),
        json!({"ids":["vco"],"unknown":true}),
        json!({"ids":["vco"],"expected_revision":0}),
    ] {
        assert!(
            session.execute("initialize_modules", args.clone()).is_err(),
            "{args}"
        );
        assert_eq!(session.snapshot(), before);
    }
    for seed in [
        json!(-1),
        json!(4294967296_u64),
        json!(0.5),
        json!(1.0),
        json!(true),
        json!(null),
        json!("1"),
    ] {
        assert!(
            session
                .execute("randomize_modules", json!({"ids":["vco"],"seed":seed}))
                .is_err(),
            "{seed}"
        );
        assert_eq!(session.snapshot(), before);
    }
    assert!(session
        .execute("randomize_modules", json!({"ids":["vco"]}))
        .is_err());
    for tool in ["preview_edits", "apply_edits"] {
        for invalid in [
            json!({"tool":"set_parameter","arguments":{"id":"missing","parameter":"frequency","value":1}}),
            json!({"tool":"initialize_modules","arguments":{"ids":["lfo"],"expected_revision":1}}),
            json!({"tool":"randomize_modules","arguments":{"ids":["lfo"],"seed":1,"extra":true}}),
        ] {
            let edits = json!([{"tool":"initialize_modules","arguments":{"ids":["vco"]}},invalid]);
            assert!(session.execute(tool, json!({"edits":edits})).is_err());
            assert_eq!(session.snapshot(), before);
        }
        // Successful local initialization cannot leak through final graph failure.
        assert!(session
            .execute(
                tool,
                json!({"edits":[
                    {"tool":"initialize_modules","arguments":{"ids":["vco"]}},
                    {"tool":"connect","arguments":{"from":"missing","to":"lfo","input":"pwm"}}
                ]})
            )
            .is_err());
        assert_eq!(session.snapshot(), before);
    }
    assert!(live.calls.lock().unwrap().is_empty());
}

#[test]
fn initialization_tracks_final_incarnations_and_keeps_later_parameter_edits() {
    for initialize_replacement in [false, true] {
        let (mut session, live) = session();
        let mut edits = vec![
            json!({"tool":"initialize_modules","arguments":{"ids":["vco","lfo"]}}),
            json!({"tool":"remove_module","arguments":{"id":"vco"}}),
            json!({"tool":"add_module","arguments":{"id":"vco","kind":"multi_vco","params":{"frequency":880}}}),
        ];
        if initialize_replacement {
            edits.push(json!({"tool":"initialize_modules","arguments":{"ids":["vco"]}}));
        }
        edits.push(json!({"tool":"set_parameter","arguments":{"id":"lfo","parameter":"frequency","value":16}}));
        session
            .execute("apply_edits", json!({"edits":edits}))
            .unwrap();
        assert_eq!(node(&session, "lfo").params["frequency"], 16.0);
        if !initialize_replacement {
            assert_eq!(node(&session, "vco").params["frequency"], 880.0);
        }
        let calls = live.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[0], json!({"retire":["vco"]}));
        assert_eq!(calls[1], json!({"retire_cables":["route"]}));
        assert_eq!(calls[2]["sync"], 2);
        assert_eq!(
            calls[3],
            if initialize_replacement {
                json!({"initialize":["lfo","vco"]})
            } else {
                json!({"initialize":["lfo"]})
            }
        );
    }
}

#[test]
fn randomization_is_deterministic_order_independent_and_respects_exclusions() {
    assert_eq!(module_random_seed("hello", 0), 0x4f9f_2cab);
    assert_eq!(module_random_seed("hello", 123), 0x4f9f_2cab ^ 123);
    let (mut a, live) = session();
    let (mut b, _) = session();
    let before = a.snapshot();
    let edits = json!([{"tool":"randomize_modules","arguments":{"ids":["vco","lfo","seq","switch"],"seed":4294967295_u64}}]);
    let preview = a.execute("preview_edits", json!({"edits":edits})).unwrap();
    assert_eq!(a.snapshot(), before);
    assert!(live.calls.lock().unwrap().is_empty());
    let changed = a.execute("apply_edits", json!({"edits":edits})).unwrap();
    assert_eq!(changed["patch"], preview["patch"]);
    b.execute(
        "randomize_modules",
        json!({"ids":["switch","seq","lfo","vco"],"seed":4294967295_u64}),
    )
    .unwrap();
    assert_eq!(a.snapshot()["patch"], b.snapshot()["patch"]);
    assert_ne!(changed["patch"]["nodes"], before["patch"]["nodes"]);
    assert_eq!(changed["patch"]["cables"], before["patch"]["cables"]);
    assert!(node(&a, "vco").bypassed);
    for (id, fm, pwm) in [("vco", 0.25, -0.5), ("lfo", -0.25, 0.75)] {
        assert_eq!(node(&a, id).params["fm_amount"], fm);
        assert_eq!(node(&a, id).params["pwm_amount"], pwm);
    }
    for (key, value) in [
        ("tempo", 240.0),
        ("tempo_amount", 0.5),
        ("steps", 4.0),
        ("steps_amount", 0.25),
        ("passthrough", 1.0),
    ] {
        assert_eq!(node(&a, "seq").params[key], value);
    }
    assert!(!node(&a, "seq").params.contains_key("running"));
    for step in 1..=8 {
        assert!([0.0, 1.0].contains(&node(&a, "seq").params[&format!("gate{step}")]));
    }
    let (mut only_vco, _) = session();
    only_vco
        .execute(
            "randomize_modules",
            json!({"ids":["vco"],"seed":4294967295_u64}),
        )
        .unwrap();
    assert_eq!(node(&a, "vco").params, node(&only_vco, "vco").params);
    a.execute("undo", json!({})).unwrap();
    assert_eq!(a.snapshot()["patch"], before["patch"]);
    a.execute("redo", json!({})).unwrap();
    assert_eq!(a.snapshot()["patch"], changed["patch"]);
    assert!(live
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|call| call.get("initialize").is_none()));
}

#[test]
fn parameter_action_schemas_are_strict_and_advertise_batch_support() {
    let schemas = tools();
    for name in ["initialize_modules", "randomize_modules"] {
        let tool = schemas
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap();
        let schema = &tool["inputSchema"];
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["ids"]["uniqueItems"], true);
        assert_eq!(schema["properties"]["ids"]["minItems"], 1);
        assert_eq!(schema["properties"]["ids"]["maxItems"], 256);
        assert!(EDIT_TOOLS.contains(&name));
    }
}

#[test]
fn parameter_actions_support_version_one_and_the_full_selection_limit() {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .execute(
            "replace_patch",
            json!({"patch":{"version":1,
        "nodes":[{"id":"source","kind":"constant","params":{"value":2}},
            {"id":"gain","kind":"gain","params":{"gain":0.5}}],
        "cables":[{"id":"route","from":"source","to":"gain","input":"signal"}],
        "output":"gain"}}),
        )
        .unwrap();
    let cables = session.snapshot()["patch"]["cables"].clone();
    for (tool, args) in [
        ("initialize_modules", json!({"ids":["gain","source"]})),
        (
            "randomize_modules",
            json!({"ids":["gain","source"],"seed":0}),
        ),
    ] {
        session.execute(tool, args).unwrap();
        assert_eq!(session.patch().version, 1);
        assert_eq!(session.snapshot()["patch"]["cables"], cables);
        assert_eq!(session.patch().output.as_deref(), Some("gain"));
    }
    let nodes: Vec<_> = (0..256)
        .map(|index| json!({"id":format!("node{index:03}"),"kind":"constant","params":{"value":2}}))
        .collect();
    session
        .execute(
            "replace_patch",
            json!({"patch":{"version":2,"nodes":nodes,"cables":[]}}),
        )
        .unwrap();
    let live = Arc::new(Live::default());
    session.set_live_module_events(Some(live.clone())).unwrap();
    live.calls.lock().unwrap().clear();
    let ids: Vec<_> = session
        .patch()
        .nodes
        .iter()
        .rev()
        .map(|node| node.id.clone())
        .collect();
    session
        .execute("initialize_modules", json!({"ids":ids}))
        .unwrap();
    let calls = live.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    let initialized = calls[1]["initialize"].as_array().unwrap();
    assert_eq!(initialized.len(), 256);
    assert_eq!(initialized[0], "node000");
    assert_eq!(initialized[255], "node255");
}
