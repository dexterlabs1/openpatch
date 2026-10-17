use super::*;
use openpatch::project_document::hosted::{HostedIdentity, HostedModule};

fn live_document() -> ProjectDocument {
    let native: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"source","kind":"constant","params":{"value":0.5}},
                 {"id":"amp","kind":"stock_vca","params":{"level":0.125}}],
        "cables":[],"output":"amp"}))
    .unwrap();
    let mut document = ProjectDocument::from_patch(native, ProjectLayout::default()).unwrap();
    document.graph.nodes.push(DocumentNode::Hosted {
        id: "binary".into(),
        source: None,
        module: HostedModule::offset(
            HostedIdentity {
                plugin: "Bogaudio".into(),
                model: "Bogaudio-Offset".into(),
                version: "2.6.47".into(),
                rack_version: "2.6.6".into(),
                plugin_sha256: "264e55e1389274c3873439372c5c51f48f63168bd6c98fb2c6ffec170224b6f6"
                    .into(),
                runtime_sha256: "55dd40f7dff09a6ed7b93374b150f33931034d3754c128da7b209ea37b6aaf1c"
                    .into(),
            },
            0.,
            0.25,
            true,
            false,
        )
        .unwrap(),
    });
    for route in [
        json!({"id":"input","from":"source","output":"out","to":"binary","input":"in"}),
        json!({"id":"output","from":"binary","output":"out","to":"amp","input":"signal"}),
    ] {
        document
            .graph
            .cables
            .push(serde_json::from_value(json!({"route":route})).unwrap());
    }
    for (index, id) in ["source", "amp", "binary"].iter().enumerate() {
        document
            .layout
            .positions
            .insert((*id).into(), [index as f32 * 180., 0.]);
    }
    document
}

fn live_app(projects: &Path, children: &Path) -> (OpenPatch, impl FnMut(usize)) {
    let mut app = host(projects);
    {
        let mut session = app.shared.lock().unwrap();
        session
            .replace_project_document(live_document(), None)
            .unwrap();
        session.request_delayed_hosted_execution(None).unwrap();
    }
    app.sync();
    let (audio, mut render) = Audio::hosted_save_test_audio(&app.patch, children);
    app.audio = Some(audio);
    app.attach_live_events();
    app.audio.as_ref().unwrap().set_playing(true);
    for _ in 0..8 {
        app.sync();
        render(128);
    }
    app.sync();
    assert!(app.shared.lock().unwrap().requires_hosted_project_capture());
    (app, render)
}

fn finish(app: &mut OpenPatch, render: &mut impl FnMut(usize)) {
    for _ in 0..150 {
        render(128);
        app.sync();
        app.service_hosted_save();
        if !app.hosted_save_pending() {
            return;
        }
    }
    panic!("save did not finish: {}", app.status);
}

#[test]
#[ignore = "requires explicit real ARM companion/SDK/plugin paths; no device opened"]
fn actual_application_native_save_as_and_save_capture_hosted_state_and_guard_disk_conflicts() {
    let projects = tempfile::tempdir().unwrap();
    let children = tempfile::tempdir().unwrap();
    let (mut app, mut render) = live_app(projects.path(), children.path());
    render(37);
    app.save_document("hosted.openpatch", true);
    assert!(app.hosted_save_pending(), "{}", app.status);
    assert!(!projects.path().join("hosted.openpatch").exists());
    let baseline = app
        .shared
        .lock()
        .unwrap()
        .project_save_baseline()
        .status_from_content(None);
    app.save_document("duplicate.openpatch", true);
    app.request_action(PendingAction::NewTemplate);
    assert!(app.hosted_save_pending());
    assert_eq!(
        app.shared
            .lock()
            .unwrap()
            .project_save_baseline()
            .status_from_content(None),
        baseline
    );
    finish(&mut app, &mut render);
    assert_eq!(
        app.active_file.as_deref(),
        Some("hosted.openpatch"),
        "{}",
        app.status
    );
    assert!(!projects.path().join("duplicate.openpatch").exists());
    let saved = app
        .projects
        .as_ref()
        .unwrap()
        .load("hosted.openpatch")
        .unwrap();
    assert_eq!(
        json!(saved.document.project().graph),
        json!(app.project.graph)
    );
    for _ in 0..8 {
        render(128);
        app.sync();
    }
    app.observe_document();
    assert!(!app.dirty(), "{}", app.status);

    assert!(app.command(
        "set_hosted_parameter",
        json!({"id":"binary","parameter":"offset","value":0.5})
    ));
    for _ in 0..4 {
        render(128);
        app.sync();
    }
    app.save_current();
    assert!(app.hosted_save_pending(), "{}", app.status);
    finish(&mut app, &mut render);
    let saved = app
        .projects
        .as_ref()
        .unwrap()
        .load("hosted.openpatch")
        .unwrap();
    assert_eq!(
        json!(saved.document.project().graph),
        json!(app.project.graph)
    );

    for _ in 0..8 {
        render(128);
        app.sync();
    }
    assert!(app.command(
        "set_hosted_parameter",
        json!({"id":"binary","parameter":"offset","value":0.75})
    ));
    for _ in 0..4 {
        render(128);
        app.sync();
    }
    let before = app
        .shared
        .lock()
        .unwrap()
        .project_save_baseline()
        .status_from_content(None);
    let external = b"external writer must survive";
    std::fs::write(projects.path().join("hosted.openpatch"), external).unwrap();
    app.save_current();
    finish(&mut app, &mut render);
    assert!(app.status.starts_with("Save failed:"), "{}", app.status);
    assert_eq!(
        std::fs::read(projects.path().join("hosted.openpatch")).unwrap(),
        external
    );
    assert_eq!(
        app.shared
            .lock()
            .unwrap()
            .project_save_baseline()
            .status_from_content(None),
        before
    );
    assert!(app.audio.is_some());
    drop(render);
    drop(app);
    assert_eq!(std::fs::read_dir(children.path()).unwrap().count(), 0);
}

#[test]
#[ignore = "requires explicit real ARM companion/SDK/plugin paths; no device opened"]
fn actual_application_native_save_rejects_newer_edits_without_dropping_audio_or_cleaning_project() {
    let projects = tempfile::tempdir().unwrap();
    let children = tempfile::tempdir().unwrap();
    let (mut app, mut render) = live_app(projects.path(), children.path());
    render(37);
    app.after_save = Some(PendingAction::Close);
    app.save_document("stale.openpatch", true);
    assert!(app.hosted_save_pending(), "{}", app.status);
    let before = app
        .shared
        .lock()
        .unwrap()
        .project_save_baseline()
        .status_from_content(None);
    app.shared
        .lock()
        .unwrap()
        .execute(
            "set_parameter",
            json!({"id":"source","parameter":"value","value":0.625}),
        )
        .unwrap();
    app.sync();
    assert!(
        app.audio.is_some(),
        "newer graph waits without tearing down audio"
    );
    finish(&mut app, &mut render);
    assert!(app.status.starts_with("Save failed:"), "{}", app.status);
    assert!(!projects.path().join("stale.openpatch").exists());
    assert_eq!(
        app.shared
            .lock()
            .unwrap()
            .project_save_baseline()
            .status_from_content(None),
        before
    );
    assert!(matches!(app.confirm_discard, Some(PendingAction::Close)));
    assert!(!app.allow_close);
    assert!(app.audio.is_some());
    for _ in 0..8 {
        render(128);
        app.sync();
    }
    assert_eq!(app.audio_revision, Some(app.revision));
    assert_eq!(
        app.patch
            .nodes
            .iter()
            .find(|node| node.id == "source")
            .unwrap()
            .params["value"],
        0.625
    );
    drop(render);
    drop(app);
    assert_eq!(std::fs::read_dir(children.path()).unwrap().count(), 0);
}
