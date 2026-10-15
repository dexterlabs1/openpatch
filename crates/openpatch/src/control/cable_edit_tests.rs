use super::*;

#[derive(Default)]
struct Live {
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
    fn sync_patch(&self, _: &Patch, revision: u64) {
        self.revisions.lock().unwrap().push(revision);
    }
}

fn make_session(version: u32) -> Session {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .replace_patch(
            serde_json::from_value(json!({"version":version,"nodes":[
        {"id":"a","kind":"oscillator","params":{"frequency":137}},
        {"id":"b","kind":"oscillator","params":{"frequency":197}},
        {"id":"sink","kind":"gain"}, {"id":"other","kind":"gain"},
        {"id":"third","kind":"gain"}
    ],"cables":[
        {"id":"first","from":"a","to":"sink","input":"signal","color":[1,2,3]},
        {"id":"second","from":"b","to":"other","input":"signal","color":[4,5,6]}
    ],"output":"other"}))
            .unwrap(),
            None,
        )
        .unwrap();
    session
}

fn live(session: &mut Session) -> Arc<Live> {
    let live = Arc::new(Live::default());
    session.set_live_module_events(Some(live.clone())).unwrap();
    live
}

fn ids(session: &Session) -> Vec<&str> {
    session
        .patch
        .cables
        .iter()
        .map(|cable| cable.id.as_deref().unwrap())
        .collect()
}

#[test]
fn color_preview_history_noop_and_dsp_adoption_preserve_route_identity() {
    for version in [1, 2] {
        let mut session = make_session(version);
        let live = live(&mut session);
        let before = session.snapshot();
        let mut old = session.patch.compile().unwrap();
        old.process_block(&mut [0.0; 379]);
        let args = json!({"cable_id":"second","color":[0,128,255],"expected_revision":1});
        let preview = session
            .execute(
                "preview_edits",
                json!({"edits":[
                    {"tool":"set_cable_color","arguments":{"cable_id":"second","color":[0,128,255]}}
                ]}),
            )
            .unwrap();
        assert_eq!(session.snapshot(), before);
        assert_eq!(preview["changes"]["changed_cables"][0]["id"], "second");
        assert_eq!(preview["changes"]["added_cables"], json!([]));
        assert_eq!(preview["changes"]["removed_cables"], json!([]));
        let colored = session.execute("set_cable_color", args).unwrap();
        assert_eq!(colored["patch"], preview["patch"]);
        assert_eq!(ids(&session), ["first", "second"]);
        assert!(live.retired.lock().unwrap().is_empty());
        let mut updated = session.patch.compile().unwrap();
        updated.adopt_state_from(&old);
        let mut expected = [0.0; 257];
        let mut actual = [0.0; 257];
        old.process_block(&mut expected);
        updated.process_block(&mut actual);
        assert_eq!(actual, expected);
        let captured = session.capture_project_patch(None).unwrap();
        let restored: Patch =
            serde_json::from_slice(&serde_json::to_vec(&captured.patch).unwrap()).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), colored["patch"]);
        session.execute("undo", json!({})).unwrap();
        assert_eq!(session.snapshot()["patch"], before["patch"]);
        let undone = session.snapshot();
        let count = live.revisions.lock().unwrap().len();
        assert_eq!(session.execute("set_cable_color", json!({"cable_id":"second","color":[4,5,6],"expected_revision":undone["revision"]})).unwrap(), undone);
        assert_eq!(live.revisions.lock().unwrap().len(), count);
        assert!(session
            .execute(
                "set_cable_color",
                json!({"cable_id":"second","color":[4,5,6],"expected_revision":1})
            )
            .is_err());
        assert_eq!(session.snapshot(), undone);
        session.execute("redo", json!({})).unwrap();
        assert_eq!(session.snapshot()["patch"], colored["patch"]);
        assert!(live.retired.lock().unwrap().is_empty());
        assert!(live.modules.lock().unwrap().is_empty());
    }
}

#[test]
fn exact_disconnect_validates_every_id_then_restores_order_colors_and_incarnations() {
    let mut session = make_session(2);
    let live = live(&mut session);
    let before = session.snapshot();
    let edits = json!([{"tool":"disconnect_cables","arguments":{"cable_ids":["second","first"]}}]);
    let preview = session
        .execute("preview_edits", json!({"edits":edits}))
        .unwrap();
    assert_eq!(session.snapshot(), before);
    assert_eq!(
        preview["changes"]["removed_cables"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(live.retired.lock().unwrap().is_empty());
    let committed = session
        .execute("apply_edits", json!({"edits":edits}))
        .unwrap();
    assert_eq!(committed["patch"], preview["patch"]);
    assert_eq!(committed["revision"], 2);
    assert_eq!(*live.retired.lock().unwrap(), ["first", "second"]);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], committed["patch"]);
    assert_eq!(live.retired.lock().unwrap().len(), 6);
    assert!(live.modules.lock().unwrap().is_empty());
}

#[test]
fn moves_preserve_positions_ids_colors_and_only_retire_changed_members() {
    for end in ["input", "output"] {
        let mut session = make_session(2);
        let live = live(&mut session);
        let before = session.snapshot();
        let args = if end == "input" {
            json!({"cable_ids":["first"],"end":end,"node":"third","port":"signal"})
        } else {
            json!({"cable_ids":["first"],"end":end,"node":"b","port":"out"})
        };
        let preview = session
            .execute(
                "preview_edits",
                json!({"edits":[{"tool":"move_cables","arguments":args}]}),
            )
            .unwrap();
        assert_eq!(session.snapshot(), before);
        let moved = session.execute("move_cables", args).unwrap();
        assert_eq!(moved["patch"], preview["patch"]);
        assert_eq!(ids(&session), ["first", "second"]);
        assert_eq!(session.patch.cables[0].color, Some([1, 2, 3]));
        let mut expected_sibling = before["patch"]["cables"][1].clone();
        if end == "output" {
            expected_sibling["output_order"] = moved["patch"]["cables"][1]["output_order"].clone();
        }
        assert_eq!(moved["patch"]["cables"][1], expected_sibling);
        assert_eq!(*live.retired.lock().unwrap(), ["first"]);
        assert!(live.modules.lock().unwrap().is_empty());
        session.execute("undo", json!({})).unwrap();
        assert_eq!(session.snapshot()["patch"], before["patch"]);
        session.execute("redo", json!({})).unwrap();
        assert_eq!(session.snapshot()["patch"], moved["patch"]);
        assert_eq!(*live.retired.lock().unwrap(), ["first", "first", "first"]);
    }
}

#[test]
fn moved_edge_resets_but_unselected_delayed_route_and_source_state_continue() {
    let mut session = make_session(2);
    let mut old = session.patch.compile().unwrap();
    old.process_block(&mut [0.0; 379]);
    let live = live(&mut session);
    session
        .execute(
            "move_cables",
            json!({"cable_ids":["first"],"end":"output","node":"b","port":"out"}),
        )
        .unwrap();
    let mut updated = session.patch.compile().unwrap();
    let exclusions: Vec<_> = live
        .retired
        .lock()
        .unwrap()
        .iter()
        .map(|id| updated.resolve_cable_state(id).unwrap())
        .collect();
    updated.adopt_state_from_excluding_routes(&old, &[], &exclusions);
    let mut expected = [0.0; 257];
    let mut actual = [0.0; 257];
    old.process_block(&mut expected);
    updated.process_block(&mut actual);
    assert_eq!(
        actual, expected,
        "untouched second route and source phase must continue"
    );
}

#[test]
fn transient_moves_reset_only_selected_delay_even_when_final_routes_match() {
    let mut session = make_session(2);
    // Pre-existing ranks keep the selected plug top at the temporary origin,
    // so this test isolates retirement from any independent order edit.
    session.patch.cables[0].output_order = Some(2);
    session.patch.cables[1].output_order = Some(1);
    let original = session.snapshot()["patch"].clone();
    let mut old = session.patch.compile().unwrap();
    old.process_block(&mut [0.0; 379]);
    let live = live(&mut session);
    session.execute("apply_edits", json!({"edits":[
        {"tool":"move_cables","arguments":{"cable_ids":["first"],"end":"output","node":"b","port":"out"}},
        {"tool":"move_cables","arguments":{"cable_ids":["first"],"end":"output","node":"a","port":"out"}}
    ]})).unwrap();
    assert_eq!(session.snapshot()["patch"], original);
    assert_eq!(*live.retired.lock().unwrap(), ["first", "first"]);
    let mut updated = session.patch.compile().unwrap();
    let handle = updated.resolve_cable_state("first").unwrap();
    updated.adopt_state_from_excluding_routes(&old, &[], &[handle]);
    let taps = |engine: &mut Engine| {
        let handles = [
            Some(engine.resolve_tap("sink", "out", 0).unwrap()),
            Some(engine.resolve_tap("other", "out", 0).unwrap()),
        ];
        let mut samples = [[openpatch_core::TapSample::default(); 2]; 3];
        engine.process_audio_block_with_taps(&mut [[0.0; 2]; 3], &handles, &mut samples);
        samples
    };
    let expected = taps(&mut old);
    let actual = taps(&mut updated);
    assert_eq!(actual[0][0].value, 0.0);
    assert_ne!(actual[0][0], expected[0][0]);
    for index in 0..3 {
        assert_eq!(actual[index][1], expected[index][1]);
    }
    assert_eq!(actual[1][0], expected[1][0]);
    assert_eq!(actual[2][0], expected[2][0]);
    assert!(live.modules.lock().unwrap().is_empty());
}

#[test]
fn discard_keeps_destination_siblings_and_accepts_mixed_success_in_one_step() {
    let mut session = make_session(2);
    session
        .execute(
            "connect",
            json!({"from":"a","to":"third","input":"signal","cable_id":"existing","color":[7,8,9]}),
        )
        .unwrap();
    let live = live(&mut session);
    let before = session.snapshot();
    let args = json!({"cable_ids":["first","second"],"end":"input","node":"third","port":"signal","duplicate_mode":"discard"});
    let preview = session
        .execute(
            "preview_edits",
            json!({"edits":[{"tool":"move_cables","arguments":args}]}),
        )
        .unwrap();
    let moved = session.execute("move_cables", args).unwrap();
    assert_eq!(moved["patch"], preview["patch"]);
    assert_eq!(ids(&session), ["second", "existing"]);
    assert_eq!(session.patch.cables[0].to, "third");
    assert_eq!(session.patch.cables[0].color, Some([4, 5, 6]));
    assert_eq!(moved["patch"]["cables"][1], before["patch"]["cables"][2]);
    assert_eq!(*live.retired.lock().unwrap(), ["first", "second"]);
    assert_eq!(
        moved["history"]["undo_steps"].as_u64(),
        before["history"]["undo_steps"].as_u64().map(|n| n + 1)
    );
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], moved["patch"]);
}

#[test]
fn supplied_id_order_controls_discard_winner_without_reordering_survivors() {
    for order in [json!(["first", "copy"]), json!(["copy", "first"])] {
        let mut session = make_session(2);
        session.execute("connect", json!({"from":"a","to":"sink","input":"signal","mode":"stack","cable_id":"copy","color":[9,9,9]})).unwrap();
        session.execute("move_cables", json!({"cable_ids":order,"end":"input","node":"third","port":"signal","duplicate_mode":"discard"})).unwrap();
        let winner = order[0].as_str().unwrap();
        let loser = order[1].as_str().unwrap();
        assert!(ids(&session).contains(&winner));
        assert!(!ids(&session).contains(&loser));
        if winner == "first" {
            assert_eq!(ids(&session), ["first", "second"]);
        } else {
            assert_eq!(ids(&session), ["second", "copy"]);
        }
    }
}

#[test]
fn same_origin_preserves_identical_stacks_and_redo_but_batch_noop_commits() {
    let mut session = make_session(2);
    session
        .execute(
            "connect",
            json!({"from":"a","to":"sink","input":"signal","mode":"stack","cable_id":"copy"}),
        )
        .unwrap();
    session
        .execute(
            "set_parameter",
            json!({"id":"a","parameter":"amplitude","value":0.3}),
        )
        .unwrap();
    session.execute("undo", json!({})).unwrap();
    let live = live(&mut session);
    let before = session.snapshot();
    for mode in ["reject", "discard", "stack"] {
        let args = json!({"cable_ids":["copy","first"],"end":"input","node":"sink","port":"signal","duplicate_mode":mode,"expected_revision":before["revision"]});
        assert_eq!(session.execute("move_cables", args).unwrap(), before);
    }
    assert!(live.retired.lock().unwrap().is_empty());
    assert_eq!(live.revisions.lock().unwrap().len(), 1);
    let transaction = session.execute("apply_edits", json!({"edits":[
        {"tool":"move_cables","arguments":{"cable_ids":["first","copy"],"end":"input","node":"sink","port":"signal","duplicate_mode":"discard"}}
    ]})).unwrap();
    assert_eq!(transaction["patch"], before["patch"]);
    assert_eq!(
        transaction["revision"].as_u64(),
        before["revision"].as_u64().map(|n| n + 1)
    );
    assert_eq!(transaction["history"]["redo_steps"], 0);
}

#[test]
fn malformed_selectors_colors_targets_and_failed_batches_are_atomic() {
    let mut session = make_session(2);
    let live = live(&mut session);
    let before = session.snapshot();
    let oversized = json!(vec!["first"; openpatch_core::MAX_CABLES + 1]);
    for selected in [
        json!([]),
        json!(null),
        json!({}),
        json!("first"),
        json!([1]),
        json!([null]),
        json!(["first", "first"]),
        json!(["first", "missing"]),
        oversized,
    ] {
        for tool in ["disconnect_cables", "move_cables"] {
            let args = if tool == "disconnect_cables" {
                json!({"cable_ids":selected})
            } else {
                json!({"cable_ids":selected,"end":"input","node":"third","port":"signal","duplicate_mode":"discard"})
            };
            assert!(session.execute(tool, args).is_err());
            assert_eq!(session.snapshot(), before);
        }
    }
    for extra in [
        json!({"end":"both"}),
        json!({"end":null}),
        json!({"node":"missing"}),
        json!({"node":null}),
        json!({"port":"out"}),
        json!({"port":false}),
        json!({"duplicate_mode":"replace"}),
        json!({"duplicate_mode":false}),
        json!({"expected_revision":0}),
        json!({"to":"sink"}),
    ] {
        let mut args = json!({"cable_ids":["first"],"end":"input","node":"third","port":"signal","duplicate_mode":"discard"});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(session.execute("move_cables", args).is_err());
        assert_eq!(session.snapshot(), before);
    }
    for color in [
        json!(null),
        json!([]),
        json!([0, 1]),
        json!([0, 1, 2, 3]),
        json!([-1, 0, 0]),
        json!([0, 256, 0]),
        json!([0, 0.5, 0]),
        json!([0, true, 0]),
    ] {
        assert!(session
            .execute("set_cable_color", json!({"cable_id":"first","color":color}))
            .is_err());
        assert_eq!(session.snapshot(), before);
    }
    for id in [json!(null), json!(1), json!("missing")] {
        assert!(session
            .execute("set_cable_color", json!({"cable_id":id,"color":[1,2,3]}))
            .is_err());
        assert_eq!(session.snapshot(), before);
    }
    for tool in ["set_cable_color", "disconnect_cables", "move_cables"] {
        let args = match tool {
            "set_cable_color" => json!({"cable_id":"first","color":[1,2,3],"expected_revision":0}),
            "disconnect_cables" => json!({"cable_ids":["first"],"expected_revision":0}),
            _ => {
                json!({"cable_ids":["first"],"end":"output","node":"a","port":"out","expected_revision":0})
            }
        };
        assert!(session.execute(tool, args).is_err());
        assert_eq!(session.snapshot(), before);
    }
    for tool in ["preview_edits", "apply_edits"] {
        assert!(session.execute(tool, json!({"edits":[
            {"tool":"set_cable_color","arguments":{"cable_id":"first","color":[255,0,0]}},
            {"tool":"disconnect_cables","arguments":{"cable_ids":["second"]}},
            {"tool":"move_cables","arguments":{"cable_ids":["first"],"end":"input","node":"third","port":"missing","duplicate_mode":"discard"}}
        ]})).is_err());
        assert_eq!(session.snapshot(), before);
    }
    assert!(live.retired.lock().unwrap().is_empty());
    assert!(live.modules.lock().unwrap().is_empty());
    assert_eq!(*live.revisions.lock().unwrap(), [1]);
}

#[test]
fn duplicate_reject_stack_and_version_one_final_validation_roll_back() {
    let mut session = make_session(2);
    session
        .execute(
            "connect",
            json!({"from":"a","to":"third","input":"signal","cable_id":"existing"}),
        )
        .unwrap();
    let before = session.snapshot();
    assert!(session
        .execute(
            "move_cables",
            json!({"cable_ids":["first"],"end":"input","node":"third","port":"signal"})
        )
        .is_err());
    assert_eq!(session.snapshot(), before);
    session.execute("move_cables", json!({"cable_ids":["first"],"end":"input","node":"third","port":"signal","duplicate_mode":"stack"})).unwrap();
    assert_eq!(ids(&session), ["first", "second", "existing"]);
    let mut session = make_session(1);
    let before = session.snapshot();
    for args in [
        json!({"cable_ids":["first"],"end":"input","node":"other","port":"signal"}),
        json!({"cable_ids":["first"],"end":"input","node":"other","port":"signal","duplicate_mode":"discard"}),
        json!({"cable_ids":["first"],"end":"output","node":"sink","port":"out"}),
        json!({"cable_ids":["first"],"end":"input","node":"third","port":"signal","duplicate_mode":"stack"}),
    ] {
        assert!(session.execute("move_cables", args).is_err());
        assert_eq!(session.snapshot(), before);
    }
    session
        .execute(
            "move_cables",
            json!({"cable_ids":["first"],"end":"input","node":"third","port":"signal"}),
        )
        .unwrap();
    assert_eq!(session.patch.cables[0].to, "third");
}

#[test]
fn full_limit_bulk_operations_and_reused_ids_keep_exact_retirement_contract() {
    let mut session = make_session(2);
    let mut patch = session.patch.clone();
    patch.cables = (0..openpatch_core::MAX_CABLES)
        .map(|index| {
            let mut cable = patch.cables[0].clone();
            cable.id = Some(format!("route_{index}"));
            cable.color = Some([index as u8, 64, 255]);
            cable
        })
        .collect();
    session.replace_patch(patch, None).unwrap();
    let ids: Vec<_> = session
        .patch
        .cables
        .iter()
        .map(|c| c.id.clone().unwrap())
        .collect();
    let before = session.snapshot();
    session.execute("move_cables", json!({"cable_ids":ids,"end":"input","node":"third","port":"signal","duplicate_mode":"stack"})).unwrap();
    assert_eq!(session.patch.cables.len(), openpatch_core::MAX_CABLES);
    assert!(session.patch.cables.iter().all(|c| c.to == "third"));
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
    let live = live(&mut session);
    session
        .execute("disconnect_cables", json!({"cable_ids":ids}))
        .unwrap();
    assert!(session.patch.cables.is_empty());
    assert_eq!(
        live.retired.lock().unwrap().len(),
        openpatch_core::MAX_CABLES
    );
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
    live.retired.lock().unwrap().clear();
    session.execute("apply_edits", json!({"edits":[
        {"tool":"disconnect_cables","arguments":{"cable_ids":["route_0"]}},
        {"tool":"connect","arguments":{"from":"a","to":"sink","input":"signal","mode":"stack","cable_id":"route_0","color":[255,128,0]}},
        {"tool":"set_cable_color","arguments":{"cable_id":"route_0","color":[0,128,255]}}
    ]})).unwrap();
    assert_eq!(*live.retired.lock().unwrap(), ["route_0"]);
    assert_eq!(
        session.patch.cables.last().unwrap().id.as_deref(),
        Some("route_0")
    );
    assert_eq!(
        session.patch.cables.last().unwrap().color,
        Some([0, 128, 255])
    );
    assert_eq!(session.patch.cables.len(), openpatch_core::MAX_CABLES);
}
