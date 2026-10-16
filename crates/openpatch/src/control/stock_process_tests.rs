use super::*;

fn session() -> Session {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .execute(
            "replace_patch",
            json!({"patch":{
                "version":2,"sample_rate":48000,
                "nodes":[
                    {"id":"stock","kind":"stock_process","params":{"slew":-10.0}},
                    {"id":"legacy","kind":"process"}
                ],
                "output":"stock","output_port":"sh1"
            }}),
        )
        .unwrap();
    session
}

fn slew(session: &Session) -> f32 {
    session
        .patch()
        .nodes
        .iter()
        .find(|node| node.id == "stock")
        .unwrap()
        .params["slew"]
}

fn stock_catalog() -> Value {
    catalog()["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "stock_process")
        .unwrap()
        .clone()
}

#[derive(Default)]
struct Live {
    actions: Mutex<Vec<(String, ModuleAction)>>,
}
impl LiveModuleEvents for Live {
    fn trigger(&self, id: &str, action: ModuleAction) -> Result<Value, String> {
        self.actions.lock().unwrap().push((id.into(), action));
        Ok(json!({"status":"queued"}))
    }
    fn snapshot(&self) -> Value {
        json!({"modules":{}})
    }
}

#[test]
fn published_process_kind_has_source_ports_and_does_not_replace_legacy_process() {
    let session = session();
    let stock = stock_catalog();
    assert_eq!(stock["model"], "functional");
    assert_eq!(stock["inputs"], json!(["slew_cv", "signal", "gate"]));
    assert_eq!(
        stock["outputs"],
        json!(["sh1", "sh2", "track_hold", "hold_track", "slew", "glide"])
    );
    assert_eq!(stock["light_names"], json!(["gate"]));
    assert!(catalog()["modules"]
        .as_array()
        .unwrap()
        .iter()
        .any(|module| module["kind"] == "process"));
    assert_eq!(session.patch().nodes[1].kind, "process");
    assert_eq!(slew(&session), -10.0);
    let tool_catalog = tools();
    let trigger = tool_catalog
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "trigger_module")
        .unwrap();
    assert!(trigger["description"]
        .as_str()
        .unwrap()
        .contains("stock_process gate_press/gate_release"));
}

#[test]
fn interactive_slew_uses_authored_f64_bounds_while_raw_patch_can_keep_finite_extension() {
    let mut session = session();
    let stock = stock_catalog();
    let min = stock["params"]["slew"]["min"].as_f64().unwrap();
    let max = stock["params"]["slew"]["max"].as_f64().unwrap();
    assert_eq!(slew(&session), -10.0);
    for invalid in [min - 1e-9, max + 1e-9, f64::MAX] {
        let before = session.snapshot();
        assert!(session
            .execute(
                "set_parameter",
                json!({
                    "id":"stock","parameter":"slew","value":invalid,
                    "expected_revision":session.revision()
                })
            )
            .is_err());
        assert_eq!(session.snapshot(), before);
    }
    for valid in [min, 0.0, max] {
        session
            .execute(
                "set_parameter",
                json!({
                    "id":"stock","parameter":"slew","value":valid
                }),
            )
            .unwrap();
        assert_eq!(slew(&session), valid as f32);
    }
    session.execute("undo", json!({})).unwrap();
    assert_eq!(slew(&session), 0.0);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(slew(&session), max as f32);
}

#[test]
fn gate_events_are_live_only_and_releases_work_while_bypassed() {
    let mut session = session();
    let live = Arc::new(Live::default());
    let before = session.snapshot();
    assert!(session
        .execute(
            "trigger_module",
            json!({
                "id":"stock","action":"gate_press"
            })
        )
        .is_err());
    assert_eq!(session.snapshot(), before);
    session.set_live_module_events(Some(live.clone())).unwrap();
    let before = session.snapshot();
    let history = (session.undo.len(), session.redo.len());
    for action in ["gate_press", "gate_release"] {
        let receipt = session
            .execute(
                "trigger_module",
                json!({
                    "id":"stock","action":action,"expected_revision":session.revision()
                }),
            )
            .unwrap();
        assert_eq!(receipt["receipt"]["status"], "queued");
    }
    session
        .execute(
            "trigger_module",
            json!({
                "id":"legacy","action":"gate_press"
            }),
        )
        .unwrap();
    session
        .execute(
            "trigger_module",
            json!({
                "id":"legacy","action":"gate_release"
            }),
        )
        .unwrap();
    assert_eq!(session.snapshot(), before);
    assert_eq!((session.undo.len(), session.redo.len()), history);
    assert_eq!(
        *live.actions.lock().unwrap(),
        vec![
            ("stock".into(), ModuleAction::GatePress),
            ("stock".into(), ModuleAction::GateRelease),
            ("legacy".into(), ModuleAction::GatePress),
            ("legacy".into(), ModuleAction::GateRelease),
        ]
    );
    let captured = session
        .capture_project_patch(Some(session.revision()))
        .unwrap();
    assert_eq!(
        captured.patch.nodes[0].params,
        session.patch().nodes[0].params
    );
    assert!(!captured.patch.nodes[0].params.contains_key("gate"));
    let preset = session
        .execute("capture_module_preset", json!({"id":"stock"}))
        .unwrap();
    assert_eq!(preset["preset"]["params"], json!({"slew":-10.0}));

    session
        .execute("set_bypass", json!({"id":"stock","bypassed":true}))
        .unwrap();
    let bypassed = session.snapshot();
    assert!(session
        .execute(
            "trigger_module",
            json!({
                "id":"stock","action":"gate_press"
            })
        )
        .is_err());
    session
        .execute(
            "trigger_module",
            json!({
                "id":"stock","action":"gate_release"
            }),
        )
        .unwrap();
    assert_eq!(session.snapshot(), bypassed);
    assert_eq!(
        live.actions.lock().unwrap().last(),
        Some(&(String::from("stock"), ModuleAction::GateRelease))
    );
}

#[test]
fn raw_preset_roundtrip_initialize_and_history_preserve_distinct_policies() {
    let mut session = session();
    let project = session
        .capture_project_patch(Some(session.revision()))
        .unwrap();
    assert_eq!(project.patch.nodes[0].params["slew"], -10.0);
    let preset = session
        .execute("capture_module_preset", json!({"id":"stock"}))
        .unwrap()["preset"]
        .clone();
    assert_eq!(preset["params"], json!({"slew":-10.0}));
    session
        .execute(
            "set_parameter",
            json!({
                "id":"stock","parameter":"slew","value":0.0
            }),
        )
        .unwrap();
    session
        .execute(
            "apply_module_preset",
            json!({
                "id":"stock","preset":preset
            }),
        )
        .unwrap();
    assert_eq!(slew(&session), -10.0);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(slew(&session), 0.0);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(slew(&session), -10.0);

    session
        .execute("initialize_modules", json!({"ids":["stock"]}))
        .unwrap();
    assert_eq!(slew(&session), 1e-3_f32.log2());
    assert_eq!(session.patch().nodes[1].kind, "process");
    session.execute("undo", json!({})).unwrap();
    assert_eq!(slew(&session), -10.0);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(slew(&session), 1e-3_f32.log2());

    session
        .execute("replace_patch", json!({"patch":project.patch}))
        .unwrap();
    assert_eq!(slew(&session), -10.0);
}

#[test]
fn invalid_batch_and_unknown_gate_preset_are_atomic() {
    let mut session = session();
    let max = stock_catalog()["params"]["slew"]["max"].as_f64().unwrap();
    let request = json!({"expected_revision":session.revision(),"edits":[
        {"tool":"set_parameter","arguments":{"id":"stock","parameter":"slew","value":0.0}},
        {"tool":"set_parameter","arguments":{"id":"stock","parameter":"slew","value":max+1e-9}}
    ]});
    let before = session.snapshot();
    for tool in ["preview_edits", "apply_edits"] {
        assert!(session.execute(tool, request.clone()).is_err());
        assert_eq!(session.snapshot(), before);
    }
    let invalid = json!({"format":"openpatch-module-preset","version":1,
        "kind":"stock_process","params":{"gate":1.0}});
    assert!(session
        .execute(
            "apply_module_preset",
            json!({
                "id":"stock","preset":invalid
            })
        )
        .is_err());
    assert_eq!(session.snapshot(), before);
}
