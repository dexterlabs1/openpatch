use super::*;

fn patch(version: u32) -> Patch {
    let value = if version == 1 {
        json!({"version":1,"nodes":[
            {"id":"external","kind":"constant","params":{"value":2}},
            {"id":"a","kind":"gain","params":{"gain":0.5},"bypassed":true},
            {"id":"b","kind":"gain","params":{"gain":2}},
            {"id":"sink","kind":"output"}],
            "cables":[{"from":"external","to":"a","input":"signal"},
                {"from":"a","to":"b","input":"signal"},
                {"from":"b","to":"sink","input":"signal"}],"output":"b"})
    } else {
        json!({"version":2,"nodes":[
            {"id":"external","kind":"poly_constant","params":{"channels":2,"v1":2,"v2":-3}},
            {"id":"a","kind":"mult","bypassed":true},
            {"id":"b","kind":"mid_side","params":{"encode_width":1.5,"decode_width":0.25}},
            {"id":"sink","kind":"mix6"}],
            "cables":[{"from":"external","to":"b","input":"right"},
                {"from":"a","output":"out2","to":"b","input":"left"},
                {"from":"b","output":"side","to":"a","input":"signal"},
                {"from":"b","output":"left","to":"sink","input":"in1"}],
            "output":"b","output_port":"side"})
    };
    serde_json::from_value(value).unwrap()
}

fn session(version: u32) -> Session {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session.replace_patch(patch(version), Some(0)).unwrap();
    session
}

fn mappings() -> Value {
    json!([{"id":"b","new_id":"b_copy"},{"id":"a","new_id":"a_copy"}])
}

#[test]
fn duplicate_modes_preserve_nodes_named_ports_feedback_and_selected_output() {
    for version in [1, 2] {
        for mode in [None, Some("none"), Some("internal"), Some("inputs")] {
            let mut session = session(version);
            let before = session.snapshot();
            let mut arguments = json!({"modules":mappings()});
            if let Some(mode) = mode {
                arguments["cable_mode"] = json!(mode);
            }
            let edits = json!([{"tool":"duplicate_modules","arguments":arguments}]);
            let preview = session
                .execute("preview_edits", json!({"edits":edits}))
                .unwrap();
            assert_eq!(session.snapshot(), before);
            let original = session.patch();
            let proposed: Patch = serde_json::from_value(preview["patch"].clone()).unwrap();
            assert_eq!(proposed.nodes.len(), 6);
            for (source, destination) in [("a", "a_copy"), ("b", "b_copy")] {
                let mut expected = original
                    .nodes
                    .iter()
                    .find(|node| node.id == source)
                    .unwrap()
                    .clone();
                expected.id = destination.into();
                assert_eq!(
                    serde_json::to_value(expected).unwrap(),
                    serde_json::to_value(
                        proposed
                            .nodes
                            .iter()
                            .find(|node| node.id == destination)
                            .unwrap()
                    )
                    .unwrap()
                );
            }
            assert_eq!(proposed.nodes[4].id, "b_copy");
            assert_eq!(proposed.nodes[5].id, "a_copy");
            assert_eq!(proposed.output, original.output);
            assert_eq!(proposed.output_port, original.output_port);
            let copied = &proposed.cables[original.cables.len()..];
            let internal = if version == 1 { 1 } else { 2 };
            let expected_count = match mode {
                Some("internal") => internal,
                Some("inputs") => internal + 1,
                _ => 0,
            };
            assert_eq!(copied.len(), expected_count);
            assert!(!copied.iter().any(|cable| cable.to == "sink"));
            if mode == Some("inputs") {
                assert!(copied.iter().any(|cable| cable.from == "external"));
            }
            if version == 2 && expected_count > 0 {
                assert!(copied.iter().any(|cable| cable.from == "a_copy"
                    && cable.output == "out2"
                    && cable.to == "b_copy"
                    && cable.input == "left"));
                assert!(copied.iter().any(|cable| cable.from == "b_copy"
                    && cable.output == "side"
                    && cable.to == "a_copy"));
            }
            let committed = session
                .execute("apply_edits", json!({"edits":edits,"expected_revision":1}))
                .unwrap();
            assert_eq!(committed["patch"], preview["patch"]);
            assert_eq!(committed["revision"], 2);
            assert_eq!(committed["history"]["undo_steps"], 2);
            session
                .execute("undo", json!({"expected_revision":2}))
                .unwrap();
            assert_eq!(session.snapshot()["patch"], before["patch"]);
            session
                .execute("redo", json!({"expected_revision":3}))
                .unwrap();
            assert_eq!(session.snapshot()["patch"], committed["patch"]);
        }
    }
}

#[test]
fn duplicate_rejects_malformed_mappings_and_stale_edits_atomically() {
    let mut session = session(2);
    session
        .execute(
            "set_parameter",
            json!({"id":"b","parameter":"encode_width","value":1}),
        )
        .unwrap();
    session.execute("undo", json!({})).unwrap();
    let before = session.snapshot();
    let oversized: Vec<_> = (0..257)
        .map(|index| json!({"id":"a","new_id":format!("copy{index}")}))
        .collect();
    for arguments in [
        json!({"modules":[]}),
        json!({"modules":null}),
        json!({"modules":{}}),
        json!({"modules":[null]}),
        json!({"modules":[{"id":"a"}]}),
        json!({"modules":[{"id":1,"new_id":"copy"}]}),
        json!({"modules":[{"id":"a","new_id":"copy","unexpected":true}]}),
        json!({"modules":[{"id":"a","new_id":""}]}),
        json!({"modules":[{"id":"a","new_id":"bad id"}]}),
        json!({"modules":[{"id":"a","new_id":"é"}]}),
        json!({"modules":[{"id":"a","new_id":"x".repeat(65)}]}),
        json!({"modules":[{"id":"missing","new_id":"copy"}]}),
        json!({"modules":[{"id":"a","new_id":"b"}]}),
        json!({"modules":[{"id":"a","new_id":"copy"},{"id":"a","new_id":"other"}]}),
        json!({"modules":[{"id":"a","new_id":"copy"},{"id":"b","new_id":"copy"}]}),
        json!({"modules":oversized}),
        json!({"modules":mappings(),"cable_mode":true}),
        json!({"modules":mappings(),"cable_mode":"all"}),
        json!({"modules":mappings(),"cable_mode":null}),
        json!({"modules":mappings(),"include_inputs":true}),
        json!({"modules":mappings(),"expected_revision":1}),
    ] {
        assert!(
            session
                .execute("duplicate_modules", arguments.clone())
                .is_err(),
            "{arguments}"
        );
        assert_eq!(session.snapshot(), before);
    }
    for tool in ["preview_edits", "apply_edits"] {
        assert!(session.execute(tool, json!({"edits":[
            {"tool":"duplicate_modules","arguments":{"modules":mappings(),"cable_mode":"inputs"}},
            {"tool":"set_parameter","arguments":{"id":"missing","parameter":"gain","value":1}}
        ]})).is_err());
        assert_eq!(session.snapshot(), before);
    }
}

#[test]
fn duplicate_snapshots_each_operations_sources_and_never_copies_external_outputs() {
    let mut session = session(2);
    session.execute("apply_edits", json!({"expected_revision":1,"edits":[
        {"tool":"duplicate_modules","arguments":{"modules":[{"id":"external","new_id":"bus.copy-1"}],"cable_mode":"inputs"}},
        {"tool":"set_parameter","arguments":{"id":"bus.copy-1","parameter":"v2","value":9}},
        {"tool":"duplicate_modules","arguments":{"modules":[{"id":"bus.copy-1","new_id":"second"}]}}
    ]})).unwrap();
    assert_eq!(session.revision, 2);
    assert_eq!(session.patch.cables.len(), 4);
    let second = session
        .patch
        .nodes
        .iter()
        .find(|node| node.id == "second")
        .unwrap();
    assert_eq!(second.params["v2"], 9.0);
    assert_eq!(session.patch.nodes[0].params["v2"], -3.0);
    assert_eq!(session.patch.output.as_deref(), Some("b"));
}

fn large_patch(count: usize, wired: bool) -> Patch {
    let nodes: Vec<_> = (0..count)
        .map(|index| json!({"id":format!("n{index}"),"kind":"mid_side"}))
        .collect();
    let mut cables = Vec::new();
    if wired {
        for index in 0..count {
            for input in ["left", "right", "mid", "side", "encode_cv", "decode_cv"] {
                cables.push(json!({"from":format!("n{index}"),"output":"mid","to":format!("n{index}"),"input":input}));
            }
        }
    }
    serde_json::from_value(json!({"version":2,"nodes":nodes,"cables":cables})).unwrap()
}

#[test]
fn duplicate_limits_validate_final_graph_and_allow_batches_to_remove_temporary_overflow() {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .replace_patch(large_patch(256, false), None)
        .unwrap();
    let before = session.snapshot();
    let args = json!({"modules":[{"id":"n0","new_id":"copy"}]});
    assert!(session
        .execute("duplicate_modules", args.clone())
        .unwrap_err()
        .contains("nodes"));
    assert_eq!(session.snapshot(), before);
    session
        .execute(
            "apply_edits",
            json!({"edits":[
                {"tool":"duplicate_modules","arguments":args},
                {"tool":"remove_module","arguments":{"id":"n255"}}
            ]}),
        )
        .unwrap();
    assert_eq!(session.patch.nodes.len(), 256);
    session.replace_patch(large_patch(100, true), None).unwrap();
    let before = session.snapshot();
    let modules: Vec<_> = (0..100)
        .map(|index| json!({"id":format!("n{index}"),"new_id":format!("c{index}")}))
        .collect();
    let args = json!({"modules":modules,"cable_mode":"internal"});
    assert!(session
        .execute("duplicate_modules", args.clone())
        .unwrap_err()
        .contains("cables"));
    assert_eq!(session.snapshot(), before);
    let mut edits = vec![json!({"tool":"duplicate_modules","arguments":args})];
    edits.extend(
        (0..30).map(|index| json!({"tool":"remove_module","arguments":{"id":format!("n{index}")}})),
    );
    session
        .execute("apply_edits", json!({"edits":edits}))
        .unwrap();
    assert_eq!(session.patch.cables.len(), 1020);
    assert_eq!(session.patch.nodes.len(), 170);
}

#[test]
fn duplicate_roundtrips_bypass_polyphony_and_named_ports_through_save_load() {
    let root = std::env::temp_dir().join(format!(
        "openpatch-duplicate-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut session = Session::new(&root).unwrap();
    session.replace_patch(patch(2), None).unwrap();
    session
        .execute(
            "duplicate_modules",
            json!({"modules":mappings(),"cable_mode":"inputs"}),
        )
        .unwrap();
    let expected = session.snapshot()["patch"].clone();
    session
        .execute("save_patch", json!({"filename":"duplicates.json"}))
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(root.join("duplicates.json")).unwrap()).unwrap(),
        expected
    );
    session
        .execute("remove_module", json!({"id":"a_copy"}))
        .unwrap();
    session
        .execute("load_patch", json!({"filename":"duplicates.json"}))
        .unwrap();
    assert_eq!(session.snapshot()["patch"], expected);
    fs::remove_dir_all(root).unwrap();
}
