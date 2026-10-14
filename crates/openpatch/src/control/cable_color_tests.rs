use super::*;

fn session() -> Session {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .replace_patch(
            serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"a","kind":"constant","params":{"value":2}},
        {"id":"b","kind":"constant","params":{"value":3}},
        {"id":"sink","kind":"gain"},
        {"id":"end","kind":"gain"}
    ],"output":"sink"}))
            .unwrap(),
            None,
        )
        .unwrap();
    session
}

#[test]
fn malformed_colors_leave_patch_history_revision_and_id_assignment_unchanged() {
    let mut session = session();
    session
        .execute(
            "set_parameter",
            json!({"id":"a","parameter":"value","value":4}),
        )
        .unwrap();
    session.execute("undo", json!({})).unwrap();
    let before = session.snapshot();
    for color in [
        json!(null),
        json!(false),
        json!("#ff0000"),
        json!({}),
        json!([]),
        json!([0, 1]),
        json!([0, 1, 2, 3]),
        json!([-1, 0, 0]),
        json!([0, 256, 0]),
        json!([0, 0.5, 0]),
        json!([0, "1", 0]),
        json!([0, true, 0]),
    ] {
        let args = json!({"from":"a","to":"sink","input":"signal","color":color});
        assert!(session
            .execute("connect", args.clone())
            .unwrap_err()
            .contains("color"));
        assert_eq!(session.snapshot(), before);
        // The valid first subedit allocates a candidate ID before the malformed
        // second one; rollback must also preserve redo and future ID allocation.
        for tool in ["preview_edits", "apply_edits"] {
            assert!(session.execute(tool, json!({"edits":[
                {"tool":"connect","arguments":{"from":"b","to":"sink","input":"cv","color":[0,128,255]}},
                {"tool":"connect","arguments":args}
            ]})).is_err());
            assert_eq!(session.snapshot(), before);
        }
    }
    session
        .execute(
            "connect",
            json!({"from":"a","to":"sink","input":"signal","color":[255,0,128]}),
        )
        .unwrap();
    assert_eq!(session.patch().cables[0].id.as_deref(), Some("cable_1"));
    assert_eq!(session.patch().cables[0].color, Some([255, 0, 128]));
}

#[test]
fn colored_connect_preview_commit_undo_redo_and_project_capture_match() {
    let mut session = session();
    let before = session.snapshot();
    let edits = json!([
        {"tool":"connect","arguments":{"from":"a","to":"sink","input":"signal","color":[0,128,255]}},
        {"tool":"connect","arguments":{"from":"b","to":"sink","input":"signal","mode":"stack"}}
    ]);
    let preview = session
        .execute("preview_edits", json!({"edits":edits}))
        .unwrap();
    assert_eq!(session.snapshot(), before);
    assert_eq!(
        preview["changes"]["added_cables"][0]["color"],
        json!([0, 128, 255])
    );
    assert!(preview["patch"]["cables"][1].get("color").is_none());
    let committed = session
        .execute("apply_edits", json!({"edits":edits}))
        .unwrap();
    assert_eq!(committed["patch"], preview["patch"]);
    let captured = session
        .capture_project_patch(Some(session.revision()))
        .unwrap();
    // Native Save and recovery both serialize this captured patch.
    let restored: Patch =
        serde_json::from_slice(&serde_json::to_vec(&captured.patch).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(restored).unwrap(), committed["patch"]);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], committed["patch"]);
}

#[test]
fn duplication_preserves_incoming_internal_and_outgoing_colors_with_fresh_ids() {
    for mode in ["internal", "inputs", "all"] {
        let mut session = session();
        session.execute("apply_edits", json!({"edits":[
            {"tool":"connect","arguments":{"from":"a","to":"sink","input":"signal","color":[255,0,0]}},
            {"tool":"connect","arguments":{"from":"b","to":"sink","input":"cv","color":[0,255,0]}},
            {"tool":"connect","arguments":{"from":"sink","to":"end","input":"signal","color":[0,0,255]}}
        ]})).unwrap();
        let originals = session.patch().cables.clone();
        let before = session.snapshot();
        let arguments = json!({"modules":[{"id":"a","new_id":"a_copy"},{"id":"sink","new_id":"sink_copy"}],"cable_mode":mode});
        let preview = session
            .execute(
                "preview_edits",
                json!({"edits":[{"tool":"duplicate_modules","arguments":arguments}]}),
            )
            .unwrap();
        let committed = session.execute("duplicate_modules", arguments).unwrap();
        assert_eq!(committed["patch"], preview["patch"]);
        let count = match mode {
            "internal" => 1,
            "inputs" => 2,
            _ => 3,
        };
        let copies = &session.patch().cables[originals.len()..];
        assert_eq!(copies.len(), count);
        for (original, copy) in originals.iter().zip(copies) {
            assert_eq!(copy.color, original.color);
            assert_ne!(copy.id, original.id);
            assert_eq!(copy.output, original.output);
            assert_eq!(copy.input, original.input);
        }
        session.execute("undo", json!({})).unwrap();
        assert_eq!(session.snapshot()["patch"], before["patch"]);
        session.execute("redo", json!({})).unwrap();
        assert_eq!(session.snapshot()["patch"], committed["patch"]);
    }
}

#[test]
fn json_save_and_load_preserve_color_and_connect_schema_is_strict_rgb() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let workspace = std::env::temp_dir().join(format!(
        "openpatch-cable-colors-{}-{unique}",
        std::process::id()
    ));
    let mut session = session();
    session.workspace = workspace.clone();
    fs::create_dir_all(&workspace).unwrap();
    session
        .execute(
            "connect",
            json!({"from":"a","to":"sink","input":"signal","color":[17,34,255]}),
        )
        .unwrap();
    let saved = session.snapshot()["patch"].clone();
    session
        .execute("save_patch", json!({"filename":"colored.json"}))
        .unwrap();
    session.execute("clear_cables", json!({})).unwrap();
    session
        .execute("load_patch", json!({"filename":"colored.json"}))
        .unwrap();
    assert_eq!(session.snapshot()["patch"], saved);
    fs::remove_dir_all(workspace).unwrap();

    let schemas = tools();
    let connect = schemas
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "connect")
        .unwrap();
    let schema = &connect["inputSchema"]["properties"]["color"];
    assert_eq!(schema["type"], "array");
    assert_eq!(schema["minItems"], 3);
    assert_eq!(schema["maxItems"], 3);
    assert_eq!(
        schema["items"],
        json!({"type":"integer","minimum":0,"maximum":255})
    );
    assert!(!connect["inputSchema"]["required"]
        .as_array()
        .unwrap()
        .contains(&json!("color")));
}
