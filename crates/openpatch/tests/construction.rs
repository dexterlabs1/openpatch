//! Construction through the public MCP transport, without a GUI or audio device.
use openpatch::{
    control::{execute_shared, Application, SharedSession},
    mcp,
};
use serde_json::{json, Value};
use std::{
    io::Cursor,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "openpatch-construction-public-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        )))
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn mcp_start(shared: SharedSession, steps: Value) -> Value {
    let messages = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"start_construction","arguments":{
            "title":"Build a signal path","expected_revision":0,"interval_ms":100,"steps":steps
        }}}),
    ];
    let input = messages
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let mut output = Vec::new();
    mcp::run_shared(Cursor::new(input), &mut output, shared).unwrap();
    let messages: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1]["id"], 2);
    messages[1]["result"].clone()
}

#[test]
fn mcp_construction_survives_transport_eof_and_every_visible_step_is_undoable() {
    let workspace = Workspace::new();
    let session = Application::new(&workspace.0).unwrap().into_shared();
    let result = mcp_start(
        session.clone(),
        json!([
            {"label":"Add source","edits":[{"tool":"add_module","arguments":{"id":"source","kind":"constant","params":{"value":0.25}}}]},
            {"label":"Add level control","edits":[{"tool":"add_module","arguments":{"id":"level","kind":"gain","params":{"gain":0.5}}}]},
            {"label":"Patch the signal","edits":[{"tool":"connect","arguments":{"from":"source","to":"level","input":"signal","cable_id":"signal"}}]},
            {"label":"Select output","edits":[{"tool":"set_output","arguments":{"id":"level"}}]}
        ]),
    );
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["structuredContent"]["last_commit"], Value::Null);
    let id = result["structuredContent"]["plan_id"].as_u64().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state =
            execute_shared(&session, "inspect_construction", json!({"plan_id":id})).unwrap();
        if state["status"] == "completed" {
            assert_eq!(state["completed_steps"], 4);
            assert_eq!(state["last_revision"], 4);
            let mut receipt = state["last_commit"].clone();
            assert!(receipt
                .as_object_mut()
                .unwrap()
                .remove("age_ms")
                .unwrap()
                .as_u64()
                .is_some());
            assert_eq!(
                receipt,
                json!({
                    "plan_id":id,"step_index":3,"before_revision":3,"revision":4,
                    "label":"Select output","new_modules":[],"new_cables":[],"moved_cables":[],"parameter_changes":[],
                    "invalidate_existing_cables":false
                })
            );
            break;
        }
        assert_eq!(state["status"], "running", "{state}");
        assert!(
            Instant::now() < deadline,
            "construction did not finish: {state}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let final_patch = session.lock().unwrap().snapshot();
    assert_eq!(final_patch["revision"], 4);
    assert_eq!(final_patch["history"]["undo_steps"], 4);
    assert_eq!(final_patch["patch"]["cables"][0]["id"], "signal");
    assert_eq!(final_patch["patch"]["output"], "level");
    // Ordinary shared history reverses actual graph transitions, not an animation.
    execute_shared(&session, "undo", json!({"expected_revision":4})).unwrap();
    assert_ne!(
        session.lock().unwrap().patch().output.as_deref(),
        Some("level")
    );
    execute_shared(&session, "undo", json!({"expected_revision":5})).unwrap();
    assert!(session.lock().unwrap().patch().cables.is_empty());
    execute_shared(&session, "undo", json!({"expected_revision":6})).unwrap();
    assert_eq!(session.lock().unwrap().patch().nodes.len(), 1);
    execute_shared(&session, "undo", json!({"expected_revision":7})).unwrap();
    assert!(session.lock().unwrap().patch().nodes.is_empty());
    for revision in 8..12 {
        execute_shared(&session, "redo", json!({"expected_revision":revision})).unwrap();
    }
    assert_eq!(
        session.lock().unwrap().snapshot()["patch"],
        final_patch["patch"]
    );
}

#[test]
fn mcp_rejects_an_invalid_intermediate_graph_before_registering_or_editing() {
    let workspace = Workspace::new();
    let session = Application::new(&workspace.0).unwrap().into_shared();
    let before = session.lock().unwrap().snapshot();
    let result = mcp_start(
        session.clone(),
        json!([
            {"label":"Add source","edits":[{"tool":"add_module","arguments":{"id":"source","kind":"constant"}}]},
            {"label":"Connect too early","edits":[{"tool":"connect","arguments":{"from":"source","to":"level","input":"signal"}}]},
            {"label":"Add sink","edits":[{"tool":"add_module","arguments":{"id":"level","kind":"gain"}}]}
        ]),
    );
    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(session.lock().unwrap().snapshot(), before);
    let state = execute_shared(&session, "inspect_construction", json!({})).unwrap();
    assert_eq!(state["status"], "idle");
    assert_eq!(state["last_commit"], Value::Null);
}
