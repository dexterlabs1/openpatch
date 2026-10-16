use super::*;

fn session() -> Session {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .execute(
            "apply_edits",
            json!({"edits":[
                {"tool":"add_module","arguments":{"id":"scope","kind":"scope","params":{"auto_range":1.0,"x_offset":0.25,"y_offset":-0.5}}},
                {"tool":"add_module","arguments":{"id":"other","kind":"scope"}}
            ]}),
        )
        .unwrap();
    session
}

fn parameter(id: &str, parameter: &str, value: f32) -> Value {
    json!({"tool":"set_parameter","arguments":{"id":id,"parameter":parameter,"value":value}})
}

fn first_frame() -> Value {
    json!({"history_group":"scope-drag","history_parameter":{"id":"scope","parameter":"x_offset"},"edits":[
        parameter("scope", "auto_range", 0.0),
        parameter("scope", "x_offset", 0.5)
    ]})
}

fn next_frame(session: &mut Session, value: f32) {
    session
        .execute(
            "set_parameter",
            json!({"id":"scope","parameter":"x_offset","value":value,"history_group":"scope-drag"}),
        )
        .unwrap();
}

fn assert_rejected_unchanged(session: &mut Session, name: &str, args: Value) {
    let before = session.snapshot();
    let group = session.history_group.clone();
    assert!(
        session.execute(name, args.clone()).is_err(),
        "{name}: {args}"
    );
    assert_eq!(session.snapshot(), before, "{name}: {args}");
    assert_eq!(session.history_group, group, "{name}: {args}");
}

#[test]
fn scope_manual_transition_and_three_changed_frames_share_exact_undo_redo() {
    let mut session = session();
    let original = session.patch.reference_view();
    let original_state_id = session.state_id();
    let undo_steps = session.undo.len();
    let revision = session.revision();
    let mut first = first_frame();
    first["expected_revision"] = json!(revision);
    session.execute("apply_edits", first).unwrap();
    let gesture_state_id = session.state_id();
    for value in [0.75, 1.0, 1.25] {
        next_frame(&mut session, value);
    }
    assert_eq!(session.revision(), revision + 4);
    assert_eq!(session.undo.len(), undo_steps + 1);
    assert_eq!(session.state_id(), gesture_state_id);
    assert_eq!(session.patch.nodes[0].params["auto_range"], 0.0);
    assert_eq!(session.patch.nodes[0].params["x_offset"], 1.25);
    assert_eq!(session.patch.nodes[0].params["y_offset"], -0.5);
    let final_patch = session.patch.reference_view();
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.patch.reference_view(), original);
    assert_eq!(session.state_id(), original_state_id);
    assert_eq!(session.undo.len(), undo_steps);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.patch.reference_view(), final_patch);
    assert_eq!(session.state_id(), gesture_state_id);
    // History traversal ends the gesture even if a client reuses its key.
    next_frame(&mut session, 1.5);
    assert_eq!(session.undo.len(), undo_steps + 2);
}

#[test]
fn grouped_batches_join_each_other_and_direct_primary_edits() {
    let mut session = session();
    let original = session.patch.reference_view();
    let undo_steps = session.undo.len();
    next_frame(&mut session, 0.375);
    session.execute("apply_edits", first_frame()).unwrap();
    let mut second = first_frame();
    second["edits"] = json!([
        parameter("scope", "y_offset", 0.75),
        parameter("scope", "x_offset", 1.0)
    ]);
    session.execute("apply_edits", second).unwrap();
    next_frame(&mut session, 1.25);
    assert_eq!(session.undo.len(), undo_steps + 1);
    let final_patch = session.patch.reference_view();
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.patch.reference_view(), original);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.patch.reference_view(), final_patch);
}

#[test]
fn grouped_parameter_errors_and_stale_revisions_preserve_history_and_redo() {
    let mut session = session();
    let original = session.patch.reference_view();
    let undo_steps = session.undo.len();
    session.execute("apply_edits", first_frame()).unwrap();
    let mut stale = first_frame();
    stale["expected_revision"] = json!(session.revision() - 1);
    assert_rejected_unchanged(&mut session, "apply_edits", stale);
    for invalid in [
        parameter("scope", "x_offset", 11.0),
        parameter("scope", "auto_range", 0.5),
        parameter("scope", "not_a_parameter", 0.0),
        json!({"tool":"set_parameter","arguments":{"id":"scope","parameter":"x_offset","value":"bad"}}),
        json!({"tool":"set_parameter","arguments":{"id":"scope","parameter":"x_offset","value":1e300}}),
    ] {
        let mut request = first_frame();
        request["edits"].as_array_mut().unwrap().push(invalid);
        assert_rejected_unchanged(&mut session, "apply_edits", request);
    }
    next_frame(&mut session, 1.0);
    assert_eq!(session.undo.len(), undo_steps + 1);
    let final_patch = session.patch.reference_view();
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.patch.reference_view(), original);
    let mut invalid = first_frame();
    invalid["edits"][1] = parameter("scope", "x_offset", 11.0);
    assert_rejected_unchanged(&mut session, "apply_edits", invalid);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.patch.reference_view(), final_patch);
}

#[test]
fn grouped_parameter_fields_are_paired_strict_and_bounded_in_utf8_bytes() {
    let mut session = session();
    session.execute("apply_edits", first_frame()).unwrap();
    for field in ["history_group", "history_parameter"] {
        let mut request = first_frame();
        request.as_object_mut().unwrap().remove(field);
        assert_rejected_unchanged(&mut session, "apply_edits", request);
    }
    for group in [
        json!(null),
        json!(false),
        json!(7),
        json!({}),
        json!(""),
        json!("x".repeat(129)),
        json!("é".repeat(65)),
    ] {
        let mut request = first_frame();
        request["history_group"] = group;
        assert_rejected_unchanged(&mut session, "apply_edits", request);
    }
    for primary in [
        json!(null),
        json!(false),
        json!("x_offset"),
        json!([]),
        json!({}),
        json!({"id":"scope"}),
        json!({"parameter":"x_offset"}),
        json!({"id":7,"parameter":"x_offset"}),
        json!({"id":"scope","parameter":false}),
        json!({"id":"scope","parameter":"x_offset","extra":0}),
        json!({"id":"scope","parameter":"missing"}),
    ] {
        let mut request = first_frame();
        request["history_parameter"] = primary;
        assert_rejected_unchanged(&mut session, "apply_edits", request);
    }
    for group in ["x".repeat(128), "é".repeat(64)] {
        let mut request = first_frame();
        request["history_group"] = json!(group);
        session.execute("apply_edits", request).unwrap();
    }
    let mut too_many = first_frame();
    too_many["edits"] = json!(vec![parameter("scope", "x_offset", 1.0); MAX_EDITS + 1]);
    assert_rejected_unchanged(&mut session, "apply_edits", too_many);
    let mut empty = first_frame();
    empty["edits"] = json!([]);
    assert_rejected_unchanged(&mut session, "apply_edits", empty);
    let mut maximum = first_frame();
    maximum["edits"] = json!(vec![parameter("scope", "x_offset", 1.0); MAX_EDITS]);
    session.execute("apply_edits", maximum).unwrap();
}

#[test]
fn grouped_parameters_reject_crossed_modules_nested_metadata_and_non_parameter_edits() {
    let mut session = session();
    session.execute("apply_edits", first_frame()).unwrap();
    for forbidden in [
        parameter("other", "x_offset", 0.5),
        json!({"tool":"set_parameter","arguments":{"id":"scope","parameter":"x_offset","value":1.0,"history_group":"nested"}}),
        json!({"tool":"set_parameter","arguments":{"id":"scope","parameter":"x_offset","value":1.0,"history_parameter":{"id":"scope","parameter":"x_offset"}}}),
        json!({"tool":"set_parameter","arguments":{"id":"scope","parameter":"x_offset","value":1.0,"expected_revision":2}}),
        json!({"tool":"add_module","arguments":{"id":"new","kind":"constant"}}),
        json!({"tool":"remove_module","arguments":{"id":"other"}}),
        json!({"tool":"connect","arguments":{"from":"scope","output":"x","to":"other","input":"x"}}),
        json!({"tool":"set_module_content","arguments":{"id":"scope","content":{"type":"notes","text":"forbidden"}}}),
        json!({"tool":"initialize_modules","arguments":{"ids":["scope"]}}),
        json!({"tool":"randomize_modules","arguments":{"ids":["scope"],"seed":1}}),
        json!({"tool":"set_bypass","arguments":{"id":"scope","bypassed":true}}),
        json!({"tool":"set_output","arguments":{"id":"scope","port":"x"}}),
        json!({"tool":"apply_edits","arguments":{"edits":[]}}),
    ] {
        let mut request = first_frame();
        request["edits"].as_array_mut().unwrap().push(forbidden);
        assert_rejected_unchanged(&mut session, "apply_edits", request);
    }
    let mut no_primary = first_frame();
    no_primary["edits"] = json!([parameter("scope", "auto_range", 0.0)]);
    assert_rejected_unchanged(&mut session, "apply_edits", no_primary);
    for field in ["history_group", "history_parameter"] {
        let mut preview = json!({"edits":[parameter("scope", "x_offset", 1.0)]});
        preview[field] = first_frame()[field].clone();
        assert_rejected_unchanged(&mut session, "preview_edits", preview);
    }
    assert_rejected_unchanged(&mut session, "preview_edits", first_frame());
}

#[test]
fn another_group_module_primary_or_ordinary_commit_breaks_parameter_grouping() {
    for (tool, arguments) in [
        (
            "set_parameter",
            json!({"id":"scope","parameter":"x_offset","value":0.75,"history_group":"another-drag"}),
        ),
        (
            "set_parameter",
            json!({"id":"other","parameter":"x_offset","value":0.75,"history_group":"scope-drag"}),
        ),
        (
            "set_parameter",
            json!({"id":"scope","parameter":"y_offset","value":0.75,"history_group":"scope-drag"}),
        ),
        (
            "set_parameter",
            json!({"id":"scope","parameter":"x_offset","value":0.75}),
        ),
        (
            "apply_edits",
            json!({"history_group":"scope-drag","history_parameter":{"id":"scope","parameter":"y_offset"},"edits":[parameter("scope", "y_offset", 0.75)]}),
        ),
        (
            "apply_edits",
            json!({"edits":[parameter("scope", "x_offset", 0.75)]}),
        ),
        ("set_bypass", json!({"id":"scope","bypassed":true})),
    ] {
        let mut session = session();
        let undo_steps = session.undo.len();
        session.execute("apply_edits", first_frame()).unwrap();
        let first_patch = session.patch.reference_view();
        session.execute(tool, arguments.clone()).unwrap();
        let interrupted_patch = session.patch.reference_view();
        next_frame(&mut session, 1.0);
        assert_eq!(session.undo.len(), undo_steps + 3, "{tool}: {arguments}");
        session.execute("undo", json!({})).unwrap();
        assert_eq!(session.patch.reference_view(), interrupted_patch);
        session.execute("undo", json!({})).unwrap();
        assert_eq!(session.patch.reference_view(), first_patch);
    }
}

#[test]
fn grouping_schema_requires_explicit_primary_only_for_committed_grouped_batches() {
    let schemas = tools();
    let schema = |name: &str| {
        schemas
            .as_array()
            .unwrap()
            .iter()
            .find(|schema| schema["name"] == name)
            .unwrap()["inputSchema"]
            .clone()
    };
    let apply = schema("apply_edits");
    assert_eq!(apply["required"], json!(["edits"]));
    assert_eq!(
        apply["dependentRequired"]["history_group"],
        json!(["history_parameter"])
    );
    assert_eq!(
        apply["dependentRequired"]["history_parameter"],
        json!(["history_group"])
    );
    assert_eq!(
        apply["properties"]["history_parameter"]["required"],
        json!(["id", "parameter"])
    );
    assert_eq!(
        apply["properties"]["history_parameter"]["additionalProperties"],
        false
    );
    let preview = schema("preview_edits");
    assert!(preview["properties"].get("history_group").is_none());
    assert!(preview["properties"].get("history_parameter").is_none());
}
