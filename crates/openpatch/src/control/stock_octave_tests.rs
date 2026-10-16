use super::*;

fn session() -> Session {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .execute(
            "replace_patch",
            json!({"patch":{"version":2,"nodes":[
        {"id":"stock","kind":"stock_octave","params":{"octaves":20.5}},
        {"id":"legacy","kind":"octave","params":{"octaves":1}}
    ],"output":"stock"}}),
        )
        .unwrap();
    session
}

fn octaves(session: &Session, id: &str) -> f32 {
    session
        .patch()
        .nodes
        .iter()
        .find(|node| node.id == id)
        .unwrap()
        .params["octaves"]
}

#[test]
fn interactive_stock_bounds_accept_fractional_values_without_changing_old_octave() {
    let mut session = session();
    let modules = catalog();
    let stock = modules["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "stock_octave")
        .unwrap();
    assert_eq!(stock["inputs"], json!(["pitch", "shift_cv"]));
    assert_eq!(stock["outputs"], json!(["out"]));
    assert_eq!(stock["params"]["octaves"]["min"], -4.0);
    assert_eq!(stock["params"]["octaves"]["max"], 4.0);
    assert_eq!(stock["params"]["octaves"]["step"], 1.0);
    assert_eq!(octaves(&session, "stock"), 20.5);
    for invalid in [-4.001, 4.001, 40.0, f64::MAX] {
        let before = session.snapshot();
        assert!(session
            .execute(
                "set_parameter",
                json!({"id":"stock","parameter":"octaves",
            "value":invalid,"expected_revision":session.revision()})
            )
            .is_err());
        assert_eq!(session.snapshot(), before);
    }
    session
        .execute(
            "set_parameter",
            json!({"id":"stock","parameter":"octaves",
        "value":-3.5,"expected_revision":session.revision()}),
        )
        .unwrap();
    assert_eq!(octaves(&session, "stock"), -3.5);
    assert_eq!(octaves(&session, "legacy"), 1.0);
    let before = session.snapshot();
    assert!(session
        .execute(
            "set_parameter",
            json!({"id":"legacy","parameter":"octaves",
        "value":1.5,"expected_revision":session.revision()})
        )
        .is_err());
    assert_eq!(session.snapshot(), before);
    assert!(session
        .execute(
            "set_parameter",
            json!({"id":"stock","parameter":"octaves",
        "value":2.0,"expected_revision":session.revision()-1})
        )
        .is_err());
    assert_eq!(session.snapshot(), before);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(octaves(&session, "stock"), 20.5);
}

#[test]
fn invalid_stock_edit_is_atomic_in_preview_and_committed_batches() {
    let mut session = session();
    let before = session.snapshot();
    let request = json!({"expected_revision":session.revision(),"edits":[
        {"tool":"set_parameter","arguments":{"id":"stock","parameter":"octaves","value":2.5}},
        {"tool":"set_parameter","arguments":{"id":"stock","parameter":"octaves","value":4.25}}
    ]});
    assert!(session.execute("preview_edits", request.clone()).is_err());
    assert_eq!(session.snapshot(), before);
    assert!(session.execute("apply_edits", request).is_err());
    assert_eq!(session.snapshot(), before);
    let preview = session
        .execute(
            "preview_edits",
            json!({"edits":[
        {"tool":"set_parameter","arguments":{"id":"stock","parameter":"octaves","value":2.5}}
    ],"expected_revision":session.revision()}),
        )
        .unwrap();
    assert_eq!(preview["committed"], false);
    assert_eq!(session.snapshot(), before);
}

#[test]
fn raw_project_and_preset_values_survive_capture_apply_and_undo() {
    let mut session = session();
    let project = session
        .capture_project_patch(Some(session.revision()))
        .unwrap();
    assert_eq!(project.patch.nodes[0].params["octaves"], 20.5);
    let portable = serde_json::to_value(&project.patch).unwrap();
    let preset = session
        .execute("capture_module_preset", json!({"id":"stock"}))
        .unwrap()["preset"]
        .clone();
    assert_eq!(preset["params"]["octaves"], 20.5);
    session
        .execute(
            "set_parameter",
            json!({"id":"stock","parameter":"octaves","value":0.5}),
        )
        .unwrap();
    let before_apply = session.snapshot();
    session
        .execute("apply_module_preset", json!({"id":"stock","preset":preset}))
        .unwrap();
    assert_eq!(octaves(&session, "stock"), 20.5);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before_apply["patch"]);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(octaves(&session, "stock"), 20.5);
    session
        .execute("replace_patch", json!({"patch":portable}))
        .unwrap();
    assert_eq!(octaves(&session, "stock"), 20.5);
    let fractional_raw = json!({"format":"openpatch-module-preset","version":1,
        "kind":"stock_octave","params":{"octaves":-9.25}});
    session
        .execute(
            "apply_module_preset",
            json!({"id":"stock","preset":fractional_raw}),
        )
        .unwrap();
    assert_eq!(octaves(&session, "stock"), -9.25);
    assert_eq!(octaves(&session, "legacy"), 1.0);
}
