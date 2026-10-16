use super::*;

fn session() -> Session {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session.execute("replace_patch", json!({"patch":{"version":2,"nodes":[
        {"id":"stock","kind":"stock_vca_mix","params":{"mix_level":-2.5,"level1":3.25,"channel_exp":1,"mix_exp":1}},
        {"id":"legacy","kind":"vca_mix","params":{"level1":0.75}}
    ],"output":"stock","output_port":"mix"}})).unwrap();
    session
}

fn value(session: &Session, id: &str, parameter: &str) -> f32 {
    session
        .patch()
        .nodes
        .iter()
        .find(|node| node.id == id)
        .unwrap()
        .params[parameter]
}

#[test]
fn stock_gain_editing_bounds_are_checked_before_f32_rounding() {
    let mut session = session();
    for parameter in ["mix_level", "level1", "level2", "level3", "level4"] {
        let maximum = if parameter == "mix_level" {
            2.0
        } else {
            f64::from(std::f32::consts::SQRT_2)
        };
        for invalid in [-f64::MIN_POSITIVE, -0.001, maximum + 1e-10, f64::MAX] {
            let before = session.snapshot();
            assert!(session
                .execute(
                    "set_parameter",
                    json!({"id":"stock","parameter":parameter,
                "value":invalid,"expected_revision":session.revision()})
                )
                .is_err());
            assert_eq!(
                session.snapshot(),
                before,
                "invalid {parameter} must not alter raw state/history"
            );
        }
        for valid in [0.0, 0.375, maximum] {
            session
                .execute(
                    "set_parameter",
                    json!({"id":"stock","parameter":parameter,"value":valid}),
                )
                .unwrap();
            assert_eq!(value(&session, "stock", parameter), valid as f32);
        }
    }
    for parameter in ["channel_exp", "mix_exp"] {
        for invalid in [-1.0, 0.5, 1.0 + 1e-10] {
            let before = session.snapshot();
            assert!(session
                .execute(
                    "set_parameter",
                    json!({"id":"stock","parameter":parameter,"value":invalid})
                )
                .is_err());
            assert_eq!(session.snapshot(), before);
        }
        for valid in [0.0, 1.0] {
            session
                .execute(
                    "set_parameter",
                    json!({"id":"stock","parameter":parameter,"value":valid}),
                )
                .unwrap();
            assert_eq!(value(&session, "stock", parameter), valid as f32);
        }
    }
    assert_eq!(value(&session, "legacy", "level1"), 0.75);
}

#[test]
fn stock_gain_batch_preview_failure_and_stale_revision_are_atomic() {
    let mut session = session();
    let before = session.snapshot();
    let edits = json!({"expected_revision":session.revision(),"edits":[
        {"tool":"set_parameter","arguments":{"id":"stock","parameter":"level1","value":0.5}},
        {"tool":"set_parameter","arguments":{"id":"stock","parameter":"mix_level","value":2.01}}
    ]});
    for method in ["preview_edits", "apply_edits"] {
        assert!(session.execute(method, edits.clone()).is_err());
        assert_eq!(session.snapshot(), before);
    }
    let valid = json!({"expected_revision":session.revision(),"edits":[
        {"tool":"set_parameter","arguments":{"id":"stock","parameter":"level1","value":0.5}},
        {"tool":"set_parameter","arguments":{"id":"stock","parameter":"mix_exp","value":0}}
    ]});
    let preview = session.execute("preview_edits", valid.clone()).unwrap();
    assert_eq!(session.snapshot(), before);
    let committed = session.execute("apply_edits", valid).unwrap();
    assert_eq!(committed["patch"], preview["patch"]);
    assert_eq!(
        committed["revision"],
        before["revision"].as_u64().unwrap() + 1
    );
    let after = session.snapshot();
    assert!(session
        .execute(
            "set_parameter",
            json!({"id":"stock","parameter":"level1","value":0.25,
        "expected_revision":before["revision"]})
        )
        .is_err());
    assert_eq!(session.snapshot(), after);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], after["patch"]);
}

#[test]
fn raw_gain_project_preset_and_initialize_preserve_their_distinct_contracts() {
    let mut session = session();
    let captured = session
        .capture_project_patch(Some(session.revision()))
        .unwrap();
    assert_eq!(captured.patch.nodes[0].params["mix_level"], -2.5);
    assert_eq!(captured.patch.nodes[0].params["level1"], 3.25);
    let preset = session
        .execute("capture_module_preset", json!({"id":"stock"}))
        .unwrap()["preset"]
        .clone();
    for (name, expected) in [
        ("mix_level", -2.5),
        ("level1", 3.25),
        ("channel_exp", 1.0),
        ("mix_exp", 1.0),
    ] {
        assert_eq!(preset["params"][name], expected);
    }
    session
        .execute(
            "set_parameter",
            json!({"id":"stock","parameter":"level1","value":0.5}),
        )
        .unwrap();
    session
        .execute("apply_module_preset", json!({"id":"stock","preset":preset}))
        .unwrap();
    assert_eq!(value(&session, "stock", "level1"), 3.25);
    let raw = json!({"format":"openpatch-module-preset","version":1,"kind":"stock_vca_mix",
        "params":{"mix_level":-f32::MAX,"level1":f32::MAX,"level2":-8.75,"channel_exp":1,"mix_exp":1}});
    session
        .execute("apply_module_preset", json!({"id":"stock","preset":raw}))
        .unwrap();
    assert_eq!(value(&session, "stock", "mix_level"), -f32::MAX);
    assert_eq!(value(&session, "stock", "level1"), f32::MAX);
    let before = session.snapshot();
    let project = session
        .capture_project_patch(Some(session.revision()))
        .unwrap();
    session
        .execute("replace_patch", json!({"patch":project.patch}))
        .unwrap();
    assert_eq!(value(&session, "stock", "level1"), f32::MAX);
    session
        .execute("initialize_modules", json!({"ids":["stock"]}))
        .unwrap();
    for parameter in ["mix_level", "level1", "level2", "level3", "level4"] {
        assert_eq!(value(&session, "stock", parameter), 1.0);
    }
    assert_eq!(value(&session, "stock", "channel_exp"), 0.0);
    assert_eq!(value(&session, "stock", "mix_exp"), 0.0);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.snapshot()["patch"], before["patch"]);
    assert_eq!(value(&session, "legacy", "level1"), 0.75);
    let before = session.snapshot();
    for (name, invalid) in [("channel_exp", 0.5), ("mix_exp", 2.0), ("level2", f64::MAX)] {
        let mut params = json!({});
        params[name] = json!(invalid);
        let preset = json!({"format":"openpatch-module-preset","version":1,"kind":"stock_vca_mix","params":params});
        assert!(session
            .execute("apply_module_preset", json!({"id":"stock","preset":preset}))
            .is_err());
        assert_eq!(session.snapshot(), before);
    }
}
