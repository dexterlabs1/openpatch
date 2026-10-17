use super::*;

#[test]
fn bridge_host_polling_does_not_start_transport_or_publish_layout_history_or_graph() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    assert!(app.command("replace_patch", json!({"patch":{"version":2,"nodes":[{"id":"bridge","kind":"rackmcp_bridge"},{"id":"bridge-two","kind":"rackmcp_bridge"}]}})));
    let revision = app.revision;
    let state_id = app.state_id;
    let generation = app.shared.lock().unwrap().layout_generation();
    let before = execute_shared(
        &app.shared,
        "inspect_bridge",
        json!({"expected_revision":revision}),
    )
    .unwrap();
    let history = app.shared.lock().unwrap().snapshot()["history"].clone();
    let document = app.capture_document();
    let ctx = egui::Context::default();
    app.bridge_live_ui(&ctx);
    app.bridge_live_ui(&ctx);
    assert!(app.command("inspect_bridge", json!({})));
    let after = execute_shared(
        &app.shared,
        "inspect_bridge",
        json!({"expected_revision":revision}),
    )
    .unwrap();
    assert_eq!(
        before["transport"], after["transport"],
        "Observing duplicate host panels cannot start or restart a bridge"
    );
    let session = app.shared.lock().unwrap();
    assert_eq!(session.revision(), revision);
    assert_eq!(session.state_id(), state_id);
    assert_eq!(session.layout_generation(), generation);
    assert_eq!(session.snapshot()["history"], history);
    drop(session);
    assert!(app.capture_document().equivalent(&document));
}

#[test]
fn bridge_native_action_rejects_an_obsolete_panel_revision_before_lifecycle_or_save() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let revision = app.revision;
    let before = execute_shared(
        &app.shared,
        "inspect_bridge",
        json!({"expected_revision":revision}),
    )
    .unwrap();
    for name in [
        "_native_bridge_start",
        "_native_bridge_stop",
        "_native_bridge_restart",
        "_native_bridge_save",
    ] {
        assert!(!app.bridge_command(name, json!({"expected_revision":revision + 1})));
    }
    let after = execute_shared(
        &app.shared,
        "inspect_bridge",
        json!({"expected_revision":revision}),
    )
    .unwrap();
    assert_eq!(before["transport"], after["transport"]);
    assert_eq!(before["save"], after["save"]);
    assert_eq!(app.revision, revision);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn bridge_native_lifecycle_uses_one_service_and_survives_panel_removal_without_history_edits() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    assert!(app.command("replace_patch", json!({"patch":{"version":2,"nodes":[{"id":"bridge","kind":"rackmcp_bridge"},{"id":"copy","kind":"rackmcp_bridge"}]}})));
    let before = app.shared.lock().unwrap().snapshot();
    assert!(app.bridge_command("_native_bridge_start", json!({})));
    let inspect = || execute_shared(&app.shared, "inspect_bridge", json!({})).unwrap();
    let first = inspect();
    assert_eq!(first["transport"]["state"], "running");
    assert_eq!(first["last_operation"]["name"], "bridge.start");
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    let first_id = first["transport"]["attachment_id"].clone();
    assert!(app.bridge_command("_native_bridge_restart", json!({})));
    let restarted = execute_shared(&app.shared, "inspect_bridge", json!({})).unwrap();
    assert_ne!(restarted["transport"]["attachment_id"], first_id);
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    assert!(app.command("remove_module", json!({"id":"bridge"})));
    assert!(app.command("remove_module", json!({"id":"copy"})));
    let after = app.shared.lock().unwrap().snapshot();
    assert_eq!(execute_shared(&app.shared, "inspect_bridge", json!({})).unwrap()["transport"], restarted["transport"]);
    assert!(app.bridge_command("_native_bridge_stop", json!({})));
    let stopped = execute_shared(&app.shared, "inspect_bridge", json!({})).unwrap();
    assert_eq!(stopped["transport"]["state"], "stopped");
    assert!(stopped["transport"]["attachment_id"].is_null());
    assert_eq!(stopped["last_operation"]["name"], "bridge.stop");
    assert_eq!(app.shared.lock().unwrap().snapshot(), after);
}
