use super::*;

#[test]
fn bridge_native_save_obsolete_generation_preserves_active_metadata_and_recovery_then_loads_snapshot(
) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    app.save_document("working.openpatch", true);
    edit(&mut app, 330.0);
    make_autosave_due(&mut app);
    app.autosave();
    let recovery = recovery_files(dir.path());
    assert_eq!(recovery.len(), 1);
    let recovery_bytes = std::fs::read(&recovery[0]).unwrap();
    let captured = app.capture_document();
    let metadata = (
        app.active_file.clone(),
        app.title.clone(),
        app.filename.clone(),
    );
    let token = format!("{:?}", app.active_token);
    let generation = app
        .shared
        .lock()
        .unwrap()
        .project_save_baseline()
        .generation();
    let replacement = Document::new(
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"replacement","kind":"constant","params":{"value":7}}]})).unwrap(),
        json!({"version":1,"zoom":1,"pan":[24,12],"positions":{"replacement":[0,0]}}),
    ).unwrap();
    let shared = app.shared.clone();
    crate::project::before_next_publish(move || {
        execute_shared(
            &shared,
            "replace_document",
            json!({"document":replacement.project()}),
        )
        .unwrap();
    });
    app.after_save = Some(PendingAction::Close);
    app.save_document("captured.openpatch", true);
    assert_eq!(
        (
            app.active_file.clone(),
            app.title.clone(),
            app.filename.clone()
        ),
        metadata
    );
    assert_eq!(format!("{:?}", app.active_token), token);
    assert_eq!(std::fs::read(&recovery[0]).unwrap(), recovery_bytes);
    assert_eq!(recovery_files(dir.path()), recovery);
    assert!(
        app.after_save.is_none(),
        "Obsolete completion cancels the pending continuation"
    );
    assert!(!app.allow_close);
    assert!(app.dirty());
    assert_eq!(app.patch.nodes[0].id, "replacement");
    let bridge = execute_shared(&app.shared, "inspect_bridge", json!({})).unwrap();
    assert!(bridge["document_generation"].as_u64().unwrap() > generation);
    assert!(bridge["save"]["target"].is_null());
    assert_eq!(bridge["save"]["dirty"], true);
    let on_disk = app
        .projects
        .as_ref()
        .unwrap()
        .load("captured.openpatch")
        .unwrap()
        .document;
    assert!(
        on_disk.equivalent(&captured),
        "The older disk snapshot remains valid without becoming the replacement's saved baseline"
    );

    // Opening the completed snapshot is a separate, explicitly accepted native
    // operation. Its own generation can acquire the target and clean baseline.
    let replacement_generation = bridge["document_generation"].as_u64().unwrap();
    app.load_document("captured.openpatch", false);
    assert_eq!(app.active_file.as_deref(), Some("captured.openpatch"));
    assert!(!app.dirty());
    assert!(recovery_files(dir.path()).is_empty());
    let loaded = execute_shared(&app.shared, "inspect_bridge", json!({})).unwrap();
    assert!(loaded["document_generation"].as_u64().unwrap() > replacement_generation);
    assert_eq!(loaded["save"]["state"], "saved");
    assert_eq!(loaded["save"]["target"], "captured.openpatch");
}

#[test]
fn bridge_native_save_concurrent_authored_edit_keeps_captured_baseline_and_newer_content_dirty() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    edit(&mut app, 330.0);
    let revision = app.revision;
    let generation = app
        .shared
        .lock()
        .unwrap()
        .project_save_baseline()
        .generation();
    let shared = app.shared.clone();
    crate::project::before_next_publish(move || {
        execute_shared(
            &shared,
            "set_parameter",
            json!({"id":"tone","parameter":"frequency","value":440}),
        )
        .unwrap();
    });
    app.save_document("authored.openpatch", true);
    assert_eq!(app.active_file.as_deref(), Some("authored.openpatch"));
    assert_eq!(app.patch.nodes[0].params["frequency"], 440.0);
    assert!(app.dirty());
    let on_disk = app
        .projects
        .as_ref()
        .unwrap()
        .load("authored.openpatch")
        .unwrap()
        .document;
    assert_eq!(on_disk.patch().nodes[0].params["frequency"], 330.0);
    let bridge = execute_shared(&app.shared, "inspect_bridge", json!({})).unwrap();
    assert_eq!(bridge["document_generation"], generation);
    assert_eq!(bridge["save"]["saved_revision"], revision);
    assert_eq!(bridge["save"]["state"], "modified");
    assert_eq!(bridge["save"]["target"], "authored.openpatch");
    assert!(app.command("undo", json!({})));
    app.observe_document();
    assert!(
        !app.dirty(),
        "Undo back to the written authored content matches its captured baseline"
    );
}

struct BridgeSavePublishedParameters(std::sync::Mutex<Value>);
impl openpatch::control::LiveModuleEvents for BridgeSavePublishedParameters {
    fn supports_live_parameters(&self) -> bool {
        true
    }
    fn snapshot(&self) -> Value {
        self.0.lock().unwrap().clone()
    }
    fn trigger(&self, _: &str, _: openpatch_core::ModuleAction) -> Result<Value, String> {
        Err("native save must not trigger a runtime action".into())
    }
}

#[test]
fn bridge_native_save_samples_published_effective_values_and_later_publication_stays_clean() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    assert!(app.command(
        "replace_patch",
        json!({"patch":{"version":2,"nodes":[
            {"id":"seq","kind":"seq3","params":{"running":1}},
            {"id":"vca","kind":"stock_vca","params":{"level":1}}
        ]}})
    ));
    let telemetry = std::sync::Arc::new(BridgeSavePublishedParameters(std::sync::Mutex::new(
        json!({"coherent":true,"modules":{
            "seq":{"kind":"seq3","available":true,"revision":app.revision,"frame":128,"state":{"running":false}},
            "vca":{"kind":"stock_vca","available":true,"revision":app.revision,"frame":128,"parameters":{"level":0.25}}
        }}),
    )));
    app.shared
        .lock()
        .unwrap()
        .set_live_module_events(Some(telemetry.clone()))
        .unwrap();
    let published = telemetry.clone();
    crate::project::before_next_publish(move || {
        let mut next = published.0.lock().unwrap();
        next["modules"]["vca"]["parameters"]["level"] = json!(0.75);
        next["modules"]["vca"]["frame"] = json!(256);
        next["modules"]["seq"]["frame"] = json!(256);
    });
    app.save_document("effective.openpatch", true);
    let on_disk = app
        .projects
        .as_ref()
        .unwrap()
        .load("effective.openpatch")
        .unwrap()
        .document;
    assert_eq!(
        on_disk.patch().nodes[1].params["level"],
        0.25,
        "Disk receives the single captured publication"
    );
    assert_eq!(
        on_disk.patch().nodes[0].params["running"],
        0.0,
        "Durable transport state is captured with the effective value"
    );
    assert_eq!(
        app.shared.lock().unwrap().patch().nodes[1].params["level"],
        1.0,
        "Saving does not rewrite the authored level"
    );
    assert!(
        !app.dirty(),
        "Later effective values alone are outside the authored/durable dirty basis"
    );
    let bridge = execute_shared(&app.shared, "inspect_bridge", json!({})).unwrap();
    assert_eq!(bridge["save"]["state"], "saved");
    assert_eq!(bridge["save"]["saved_frame"], 128);
    assert_eq!(
        bridge["save"]["basis"],
        "authored_content_and_durable_state"
    );
    telemetry.0.lock().unwrap()["modules"]["seq"]["state"]["running"] = json!(true);
    app.observe_document();
    assert!(
        app.dirty(),
        "A subsequent durable transport change still requires Save"
    );
}
