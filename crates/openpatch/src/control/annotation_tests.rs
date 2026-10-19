use super::*;

fn session() -> Session {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .execute(
            "replace_patch",
            json!({"patch":{"version":2,"nodes":[
        {"id":"notes","kind":"notes","content":{"type":"notes","text":"original\n🎛️"}},
        {"id":"blank","kind":"blank","content":{"type":"blank","width_hp":17}},
        {"id":"tone","kind":"constant","params":{"value":1}}
    ],"cables":[],"output":"tone"}}),
        )
        .unwrap();
    session
}

fn node<'a>(session: &'a Session, id: &str) -> &'a Node {
    session
        .patch
        .nodes
        .iter()
        .find(|node| node.id == id)
        .unwrap()
}

fn content_edit(id: &str, content: Value) -> Value {
    json!({"tool":"set_module_content","arguments":{"id":id,"content":content}})
}

#[test]
fn mixed_content_preview_commit_and_history_are_atomic() {
    let mut session = session();
    let before = session.snapshot();
    let history = session.undo.len();
    let edits = json!([
        content_edit("notes", json!({"type":"notes","text":"Unicode\n\t日本語 😀"})),
        content_edit("blank", json!({"type":"blank","width_hp":96})),
        {"tool":"set_parameter","arguments":{"id":"tone","parameter":"value","value":2}}
    ]);
    let preview = session
        .execute(
            "preview_edits",
            json!({"edits":edits,"expected_revision":1}),
        )
        .unwrap();
    assert_eq!(session.snapshot(), before);
    assert_eq!(session.undo.len(), history);
    assert_eq!(
        preview["changes"]["changed_modules"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let committed = session
        .execute("apply_edits", json!({"edits":edits,"expected_revision":1}))
        .unwrap();
    assert_eq!(committed["patch"], preview["patch"]);
    assert_eq!(session.revision, 2);
    assert_eq!(session.undo.len(), history + 1);
    let after = session.snapshot();
    session
        .execute("undo", json!({"expected_revision":2}))
        .unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
    session
        .execute("redo", json!({"expected_revision":3}))
        .unwrap();
    assert_eq!(session.snapshot()["patch"], after["patch"]);
}

#[test]
fn stale_and_invalid_mixed_content_edits_leave_revision_history_and_redo_unchanged() {
    let mut session = session();
    session
        .execute(
            "set_module_content",
            json!({"id":"notes","content":{"type":"notes","text":"temporary"}}),
        )
        .unwrap();
    session.execute("undo", json!({})).unwrap();
    let before = session.snapshot();
    let counts = (session.undo.len(), session.redo.len());
    let valid = content_edit("notes", json!({"type":"notes","text":"must roll back"}));
    let invalid = [
        content_edit("blank", json!({"type":"blank","width_hp":2})),
        content_edit("blank", json!({"type":"blank","width_hp":97})),
        content_edit("notes", json!({"type":"notes","text":"é".repeat(4097)})),
        content_edit("tone", json!({"type":"notes","text":"wrong kind"})),
        content_edit("notes", json!({"type":"blank","width_hp":10})),
        content_edit("notes", json!({"type":"notes","text":"x","extra":true})),
        content_edit("notes", Value::Null),
        content_edit("missing", json!({"type":"notes","text":"x"})),
    ];
    for bad in invalid {
        for tool in ["preview_edits", "apply_edits"] {
            assert!(session
                .execute(
                    tool,
                    json!({"edits":[valid,bad],"expected_revision":session.revision})
                )
                .is_err());
            assert_eq!(session.snapshot(), before);
            assert_eq!((session.undo.len(), session.redo.len()), counts);
        }
    }
    for tool in ["set_module_content", "apply_edits", "preview_edits"] {
        let args = if tool == "set_module_content" {
            json!({"id":"notes","content":{"type":"notes","text":"stale"},"expected_revision":0})
        } else {
            json!({"edits":[valid],"expected_revision":0})
        };
        assert!(session.execute(tool, args).is_err());
        assert_eq!(session.snapshot(), before);
    }
}

#[test]
fn content_add_duplicate_and_initialize_preserve_or_reset_intentionally() {
    let mut session = session();
    session
        .execute(
            "add_module",
            json!({"id":"extra","kind":"notes","content":{"type":"notes","text":"𝄞".repeat(2048)}}),
        )
        .unwrap();
    assert_eq!(
        node(&session, "extra")
            .effective_notes_text()
            .unwrap()
            .len(),
        8192
    );
    session
        .execute(
            "duplicate_modules",
            json!({"modules":[
                {"id":"notes","new_id":"notes_copy"},{"id":"blank","new_id":"blank_copy"}
            ]}),
        )
        .unwrap();
    assert_eq!(
        node(&session, "notes").content,
        node(&session, "notes_copy").content
    );
    assert_eq!(
        node(&session, "blank").content,
        node(&session, "blank_copy").content
    );
    let before = session.snapshot()["patch"].clone();
    session
        .execute("initialize_modules", json!({"ids":["notes","blank"]}))
        .unwrap();
    assert_eq!(node(&session, "notes").effective_notes_text(), Some(""));
    assert_eq!(node(&session, "blank").effective_blank_width_hp(), Some(10));
    assert_eq!(
        node(&session, "blank_copy").effective_blank_width_hp(),
        Some(17)
    );
    let after = session.snapshot()["patch"].clone();
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], after);
    let before = session.snapshot();
    assert!(session
        .execute(
            "randomize_modules",
            json!({"ids":["tone","notes"],"seed":3})
        )
        .is_err());
    assert_eq!(session.snapshot(), before);
}

#[test]
fn annotation_presets_roundtrip_unicode_bounds_and_missing_content_preserves() {
    let mut session = session();
    for id in ["notes", "blank"] {
        let captured = session
            .execute("capture_module_preset", json!({"id":id}))
            .unwrap();
        let preset = captured["preset"].clone();
        assert_eq!(
            preset["content"],
            serde_json::to_value(&node(&session, id).content).unwrap()
        );
        session
            .execute("initialize_modules", json!({"ids":[id]}))
            .unwrap();
        session
            .execute("apply_module_preset", json!({"id":id,"preset":preset}))
            .unwrap();
        assert_eq!(
            serde_json::to_value(&node(&session, id).content).unwrap(),
            preset["content"]
        );
        let mut omitted = preset.clone();
        omitted.as_object_mut().unwrap().remove("content");
        session
            .execute("apply_module_preset", json!({"id":id,"preset":omitted}))
            .unwrap();
        assert_eq!(
            serde_json::to_value(&node(&session, id).content).unwrap(),
            preset["content"]
        );
    }
    let before = session.snapshot();
    for (id, content) in [
        ("notes", json!({"type":"notes","text":"é".repeat(4097)})),
        ("notes", json!({"type":"blank","width_hp":10})),
        ("blank", json!({"type":"blank","width_hp":2})),
        ("blank", json!({"type":"blank","width_hp":97})),
        ("notes", json!({"type":"notes","text":"ok","unknown":1})),
        ("tone", json!({"type":"notes","text":"wrong kind"})),
    ] {
        let preset = json!({"format":"openpatch-module-preset","version":1,"kind":node(&session,id).kind,"params":{},"content":content});
        assert!(session
            .execute("apply_module_preset", json!({"id":id,"preset":preset}))
            .is_err());
        assert_eq!(session.snapshot(), before);
    }
}

#[test]
fn annotation_default_presets_capture_explicit_defaults_and_validate_boundary_content() {
    for (kind, content) in [
        ("notes", json!({"type":"notes","text":""})),
        ("blank", json!({"type":"blank","width_hp":10})),
    ] {
        let node: Node = serde_json::from_value(json!({"id":"default","kind":kind})).unwrap();
        let captured = crate::presets::capture(&node, None).unwrap();
        assert_eq!(captured["content"], content);
    }
    for (kind, content) in [
        ("notes", json!({"type":"notes","text":"é".repeat(4096)})),
        ("blank", json!({"type":"blank","width_hp":3})),
        ("blank", json!({"type":"blank","width_hp":96})),
    ] {
        crate::presets::validate(&json!({"format":"openpatch-module-preset","version":1,"kind":kind,"params":{},"content":content})).unwrap();
    }
}

#[test]
fn content_cannot_make_annotation_kinds_or_content_valid_in_v1() {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session.execute("replace_patch", json!({"patch":{"version":1,"nodes":[{"id":"tone","kind":"constant"}],"cables":[],"output":"tone"}})).unwrap();
    let before = session.snapshot();
    for args in [
        json!({"id":"notes","kind":"notes"}),
        json!({"id":"notes","kind":"notes","content":{"type":"notes","text":"x"}}),
        json!({"id":"blank","kind":"blank","content":{"type":"blank","width_hp":10}}),
        json!({"id":"wrong","kind":"constant","content":{"type":"notes","text":"x"}}),
    ] {
        assert!(session.execute("add_module", args).is_err());
        assert_eq!(session.snapshot(), before);
    }
}

#[test]
fn content_schema_is_strict_and_shared_by_add_edit_and_presets() {
    let schemas = tools();
    let schema = |name| {
        schemas
            .as_array()
            .unwrap()
            .iter()
            .find(|schema| schema["name"] == name)
            .unwrap()
    };
    let content = &schema("set_module_content")["inputSchema"]["properties"]["content"];
    assert_eq!(
        content,
        &schema("add_module")["inputSchema"]["properties"]["content"]
    );
    assert_eq!(
        content,
        &schema("apply_module_preset")["inputSchema"]["properties"]["preset"]["properties"]
            ["content"]
    );
    let variant = |kind| {
        content["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|schema| schema["properties"]["type"]["const"] == kind)
            .unwrap()
    };
    assert_eq!(variant("notes")["additionalProperties"], false);
    assert_eq!(variant("blank")["properties"]["width_hp"]["maximum"], 96);
    assert!(
        schema("apply_edits")["inputSchema"]["properties"]["edits"]["items"]["properties"]["tool"]
            ["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("set_module_content"))
    );
}

#[test]
fn project_capture_preserves_annotation_content_without_runtime_telemetry() {
    let session = session();
    let capture = session
        .capture_project_patch(Some(session.revision))
        .unwrap();
    assert_eq!(
        serde_json::to_value(capture.patch).unwrap(),
        session.snapshot()["patch"]
    );
}

#[test]
fn equal_content_is_a_revision_checked_noop_preserving_redo() {
    let mut session = session();
    session
        .execute(
            "set_module_content",
            json!({"id":"blank","content":{"type":"blank","width_hp":25}}),
        )
        .unwrap();
    session.execute("undo", json!({})).unwrap();
    let before = session.snapshot();
    let history = (session.undo.len(), session.redo.len());
    session.execute("set_module_content", json!({"id":"blank","content":{"type":"blank","width_hp":17},"expected_revision":session.revision})).unwrap();
    assert_eq!(session.snapshot(), before);
    assert_eq!((session.undo.len(), session.redo.len()), history);
    assert!(session
        .execute(
            "set_module_content",
            json!({"id":"blank","content":{"type":"blank","width_hp":17},"expected_revision":0})
        )
        .is_err());
    assert_eq!(session.snapshot(), before);
}
