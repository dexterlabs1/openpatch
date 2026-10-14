use super::*;
use openpatch_core::{cable_endpoint_order, CableEndpoint};

struct Live {
    engine: Mutex<Engine>,
    retired: Mutex<Vec<String>>,
    modules: Mutex<Vec<String>>,
    revisions: Mutex<Vec<u64>>,
}
impl LiveModuleEvents for Live {
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        unreachable!()
    }
    fn snapshot(&self) -> Value {
        json!({})
    }
    fn retire_cables(&self, ids: &[String]) {
        self.retired.lock().unwrap().extend_from_slice(ids);
    }
    fn retire_modules(&self, ids: &[String]) {
        self.modules.lock().unwrap().extend_from_slice(ids);
    }
    fn sync_patch(&self, patch: &Patch, revision: u64) {
        let mut old = self.engine.lock().unwrap();
        let mut new = patch.compile().unwrap();
        new.adopt_state_from(&old);
        *old = new;
        self.revisions.lock().unwrap().push(revision);
    }
}

fn session(version: u32) -> Session {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    let patch = json!({"version":version,"nodes":[
        {"id":"source","kind":"oscillator","params":{"frequency":137}},
        {"id":"sink","kind":"gain"},{"id":"other","kind":"gain"},{"id":"third","kind":"gain"}],
        "cables":[
            {"id":"first","from":"source","to":"sink","input":"signal","color":[1,2,3],"input_order":50,"output_order":1},
            {"id":"second","from":"source","to":"other","input":"signal","color":[4,5,6],"input_order":40,"output_order":2},
            {"id":"third","from":"source","to":"third","input":"signal","input_order":30,"output_order":3}],"output":"sink"});
    session
        .replace_patch(serde_json::from_value(patch).unwrap(), None)
        .unwrap();
    session
}

fn order(session: &Session, end: CableEndpoint, node: &str, port: &str) -> Vec<String> {
    cable_endpoint_order(&session.patch.cables, end, node, port)
        .into_iter()
        .map(|i| session.patch.cables[i].id.clone().unwrap())
        .collect()
}

#[test]
fn metadata_promote_preview_undo_redo_adopt_live_source_and_delayed_cable_state() {
    for version in [1, 2] {
        let mut session = session(version);
        let before = session.snapshot();
        let mut expected = session.patch.compile().unwrap();
        expected.process_block(&mut [0.0; 379]);
        let mut warmed = session.patch.compile().unwrap();
        warmed.process_block(&mut [0.0; 379]);
        let live = Arc::new(Live {
            engine: Mutex::new(warmed),
            retired: Mutex::new(vec![]),
            modules: Mutex::new(vec![]),
            revisions: Mutex::new(vec![]),
        });
        session.set_live_module_events(Some(live.clone()));
        let assert_continuity = |reference: &mut Engine| {
            let mut want = [0.0; 257];
            let mut actual = [0.0; 257];
            reference.process_block(&mut want);
            live.engine.lock().unwrap().process_block(&mut actual);
            assert_eq!(
                actual, want,
                "module phase and one-sample frame must survive metadata adoption"
            );
        };
        let edits =
            json!([{"tool":"promote_cables","arguments":{"cable_ids":["first"],"end":"output"}}]);
        let preview = session
            .execute("preview_edits", json!({"edits":edits}))
            .unwrap();
        assert_eq!(session.snapshot(), before);
        assert_eq!(preview["changes"]["added_cables"], json!([]));
        assert_eq!(preview["changes"]["removed_cables"], json!([]));
        assert_eq!(preview["changes"]["changed_cables"], json!([]));
        assert_eq!(preview["changes"]["cable_order_changed"], false);
        assert!(!preview["changes"]["changed_cable_order"]
            .as_array()
            .unwrap()
            .is_empty());
        let promoted = session
            .execute("apply_edits", json!({"edits":edits}))
            .unwrap();
        assert_eq!(promoted["patch"], preview["patch"]);
        assert_eq!(
            order(&session, CableEndpoint::Output, "source", "out"),
            ["second", "third", "first"]
        );
        assert_eq!(
            session
                .patch
                .cables
                .iter()
                .map(|c| c.input_order)
                .collect::<Vec<_>>(),
            [Some(50), Some(40), Some(30)]
        );
        assert_continuity(&mut expected);
        session.execute("undo", json!({})).unwrap();
        assert_eq!(session.snapshot()["patch"], before["patch"]);
        assert_continuity(&mut expected);
        session.execute("redo", json!({})).unwrap();
        assert_eq!(session.snapshot()["patch"], promoted["patch"]);
        assert_continuity(&mut expected);
        assert!(live.retired.lock().unwrap().is_empty());
        assert!(live.modules.lock().unwrap().is_empty());
    }
}

#[test]
fn same_origin_non_top_commits_only_chosen_rank_but_top_and_unique_input_preserve_redo() {
    for version in [1, 2] {
        let mut session = session(version);
        session
            .execute(
                "set_parameter",
                json!({"id":"source","parameter":"amplitude","value":0.3}),
            )
            .unwrap();
        session.execute("undo", json!({})).unwrap();
        let before = session.snapshot();
        assert_eq!(session.execute("promote_cables",json!({"cable_ids":["first"],"end":"input","expected_revision":before["revision"]})).unwrap(),before);
        assert_eq!(
            session
                .execute(
                    "promote_cables",
                    json!({"cable_ids":["third"],"end":"output"})
                )
                .unwrap(),
            before
        );
        assert_eq!(
            session
                .execute(
                    "move_cables",
                    json!({"cable_ids":["first"],"end":"output","node":"source","port":"out"})
                )
                .unwrap(),
            before
        );
        let promoted=session.execute("move_cables",json!({"cable_ids":["first"],"end":"output","node":"source","port":"out","promote":true,"plug_order":["first"]})).unwrap();
        assert_eq!(
            promoted["revision"].as_u64(),
            before["revision"].as_u64().map(|n| n + 1)
        );
        assert_eq!(promoted["history"]["redo_steps"], 0);
        assert_eq!(
            order(&session, CableEndpoint::Output, "source", "out")
                .last()
                .unwrap(),
            "first"
        );
        let top = session.snapshot();
        assert_eq!(session.execute("move_cables",json!({"cable_ids":["first"],"end":"output","node":"source","port":"out","promote":true})).unwrap(),top);
        session.execute("undo", json!({})).unwrap();
        assert_eq!(session.snapshot()["patch"], before["patch"]);
        let undone = session.snapshot();
        assert_eq!(
            session
                .execute(
                    "promote_cables",
                    json!({"cable_ids":["third"],"end":"output"})
                )
                .unwrap(),
            undone
        );
        assert_eq!(session.snapshot()["history"]["redo_steps"], 1);
        assert!(session
            .execute(
                "promote_cables",
                json!({"cable_ids":["third"],"end":"output","expected_revision":0})
            )
            .is_err());
        assert_eq!(session.snapshot(), undone);
    }
}

#[test]
fn independent_input_and_output_tops_survive_recolor_and_history() {
    let mut session = session(2);
    session.execute("move_cables",json!({"cable_ids":["second","third"],"end":"input","node":"sink","port":"signal","duplicate_mode":"stack","plug_order":["third","second"]})).unwrap();
    let opposite: Vec<_> = session
        .patch
        .cables
        .iter()
        .map(|c| c.output_order)
        .collect();
    assert_eq!(
        order(&session, CableEndpoint::Input, "sink", "signal"),
        ["first", "third", "second"]
    );
    assert_eq!(
        order(&session, CableEndpoint::Output, "source", "out"),
        ["first", "second", "third"]
    );
    session
        .execute(
            "promote_cables",
            json!({"cable_ids":["first"],"end":"input"}),
        )
        .unwrap();
    assert_eq!(
        order(&session, CableEndpoint::Input, "sink", "signal"),
        ["third", "second", "first"]
    );
    assert_eq!(
        session
            .patch
            .cables
            .iter()
            .map(|c| c.output_order)
            .collect::<Vec<_>>(),
        opposite
    );
    let before = session.snapshot();
    session
        .execute(
            "set_cable_color",
            json!({"cable_id":"first","color":[255,128,0]}),
        )
        .unwrap();
    assert_eq!(
        order(&session, CableEndpoint::Input, "sink", "signal"),
        ["third", "second", "first"]
    );
    assert_eq!(
        session
            .patch
            .cables
            .iter()
            .map(|c| c.output_order)
            .collect::<Vec<_>>(),
        opposite
    );
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
}

#[test]
fn bundle_plug_order_does_not_change_discard_winner_or_dsp_array_order() {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session.replace_patch(serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"a","kind":"constant"},{"id":"b","kind":"constant"},{"id":"c","kind":"constant"},
        {"id":"origin","kind":"gain"},{"id":"target","kind":"gain"}],
        "cables":[
            {"id":"first","from":"a","to":"origin","input":"signal","input_order":30,"output_order":1},
            {"id":"copy","from":"a","to":"origin","input":"signal","input_order":10,"output_order":2},
            {"id":"second","from":"b","to":"origin","input":"signal","input_order":20,"output_order":3},
            {"id":"existing","from":"c","to":"target","input":"signal","input_order":65535,"output_order":4}]})).unwrap(),None).unwrap();
    let edits = json!([{"tool":"move_cables","arguments":{"cable_ids":["first","copy","second"],"end":"input","node":"target","port":"signal","duplicate_mode":"discard","plug_order":["second","copy","first"]}}]);
    let preview = session
        .execute("preview_edits", json!({"edits":edits}))
        .unwrap();
    let committed = session
        .execute("apply_edits", json!({"edits":edits}))
        .unwrap();
    assert_eq!(committed["patch"], preview["patch"]);
    assert_eq!(
        session
            .patch
            .cables
            .iter()
            .map(|c| c.id.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["first", "second", "existing"]
    );
    assert_eq!(
        order(&session, CableEndpoint::Input, "target", "signal"),
        ["existing", "second", "first"]
    );
    assert_eq!(
        session
            .patch
            .cables
            .iter()
            .map(|c| c.output_order)
            .collect::<Vec<_>>(),
        [Some(1), Some(3), Some(4)]
    );
}

#[test]
fn fresh_connect_duplicate_and_reused_id_allocate_new_top_ranks_at_both_ends() {
    let mut session = session(2);
    session.patch.cables[0].input_order = Some(65535);
    session.patch.cables[0].output_order = Some(65535);
    let old_output = order(&session, CableEndpoint::Output, "source", "out");
    session
        .execute(
            "connect",
            json!({"from":"source","to":"sink","input":"signal","mode":"stack","cable_id":"fresh"}),
        )
        .unwrap();
    let current = order(&session, CableEndpoint::Output, "source", "out");
    assert_eq!(&current[..3], &old_output);
    assert_eq!(current.last().unwrap(), "fresh");
    assert_eq!(
        order(&session, CableEndpoint::Input, "sink", "signal")
            .last()
            .unwrap(),
        "fresh"
    );
    session
        .execute(
            "duplicate_modules",
            json!({"modules":[{"id":"sink","new_id":"sink_copy"}],"cable_mode":"inputs"}),
        )
        .unwrap();
    let copies = &session.patch.cables[4..];
    assert_eq!(copies.len(), 2);
    assert!(copies
        .iter()
        .all(|c| c.input_order.is_some() && c.output_order.is_some()));
    let expected_last = copies.last().unwrap().id.clone().unwrap();
    assert_eq!(
        order(&session, CableEndpoint::Output, "source", "out")
            .last()
            .unwrap(),
        &expected_last
    );
    session.execute("apply_edits",json!({"edits":[
        {"tool":"disconnect_cables","arguments":{"cable_ids":["fresh"]}},
        {"tool":"connect","arguments":{"from":"source","to":"sink","input":"signal","mode":"stack","cable_id":"fresh"}}
    ]})).unwrap();
    assert_eq!(
        order(&session, CableEndpoint::Output, "source", "out")
            .last()
            .unwrap(),
        "fresh"
    );
    assert_eq!(
        order(&session, CableEndpoint::Input, "sink", "signal")
            .last()
            .unwrap(),
        "fresh"
    );
}

#[test]
fn malformed_promotion_and_permutations_reject_before_any_publish_or_retirement() {
    let mut session = session(2);
    let before = session.snapshot();
    for args in [
        json!({"cable_ids":[],"end":"input"}),
        json!({"cable_ids":["first","first"],"end":"input"}),
        json!({"cable_ids":["first","missing"],"end":"input"}),
        json!({"cable_ids":["first"],"end":"both"}),
        json!({"cable_ids":["first"],"end":null}),
        json!({"cable_ids":["first"],"end":"output","expected_revision":0}),
    ] {
        assert!(session.execute("promote_cables", args).is_err());
        assert_eq!(session.snapshot(), before);
    }
    for extra in [
        json!({"promote":null}),
        json!({"promote":1}),
        json!({"plug_order":[]}),
        json!({"plug_order":["first"]}),
        json!({"plug_order":["first","first"]}),
        json!({"plug_order":["first","third"]}),
        json!({"plug_order":["first","missing"]}),
    ] {
        let mut args = json!({"cable_ids":["first","second"],"end":"input","node":"third","port":"signal","duplicate_mode":"stack"});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(session.execute("move_cables", args).is_err());
        assert_eq!(session.snapshot(), before);
    }
    for tool in ["preview_edits", "apply_edits"] {
        assert!(session.execute(tool,json!({"edits":[
            {"tool":"promote_cables","arguments":{"cable_ids":["first"],"end":"output"}},
            {"tool":"move_cables","arguments":{"cable_ids":["first"],"end":"input","node":"missing","port":"signal","promote":true}}
        ]})).is_err());
        assert_eq!(session.snapshot(), before);
    }
    let schemas = tools();
    let promote = schemas
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "promote_cables")
        .unwrap();
    assert_eq!(
        promote["inputSchema"]["properties"]["cable_ids"]["maxItems"],
        1024
    );
    assert_eq!(
        promote["inputSchema"]["properties"]["end"]["enum"],
        json!(["input", "output"])
    );
}

#[test]
fn full_cable_limit_promotion_preview_history_keeps_original_array_and_opposite_ranks() {
    let mut session = session(2);
    let mut patch = session.patch.clone();
    let prototype = patch.cables[0].clone();
    patch.cables = (0..openpatch_core::MAX_CABLES)
        .map(|index| {
            let mut cable = prototype.clone();
            cable.id = Some(format!("route_{index}"));
            cable.input_order = None;
            cable.output_order = Some((openpatch_core::MAX_CABLES - index) as u16);
            cable
        })
        .collect();
    session.replace_patch(patch, None).unwrap();
    let before = session.snapshot();
    let selected: Vec<_> = (0..openpatch_core::MAX_CABLES)
        .rev()
        .map(|i| format!("route_{i}"))
        .collect();
    assert_eq!(
        session
            .execute(
                "promote_cables",
                json!({"cable_ids":selected,"end":"input"})
            )
            .unwrap(),
        before
    );
    let edits =
        json!([{"tool":"promote_cables","arguments":{"cable_ids":["route_0"],"end":"input"}}]);
    let preview = session
        .execute("preview_edits", json!({"edits":edits}))
        .unwrap();
    let committed = session
        .execute("apply_edits", json!({"edits":edits}))
        .unwrap();
    assert_eq!(committed["patch"], preview["patch"]);
    assert_eq!(
        order(&session, CableEndpoint::Input, "sink", "signal")
            .last()
            .unwrap(),
        "route_0"
    );
    assert_eq!(session.patch.cables[0].input_order, Some(1024));
    for (index, cable) in session.patch.cables.iter().enumerate() {
        assert_eq!(cable.id.as_deref(), Some(format!("route_{index}").as_str()));
        assert_eq!(
            cable.output_order,
            Some((openpatch_core::MAX_CABLES - index) as u16)
        );
    }
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], committed["patch"]);
}
