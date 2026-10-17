use super::*;
use openpatch::project_document::{ExecutionDisposition, PlaybackMode};

#[test]
fn hosted_document_native_app_history_save_and_inactive_panel_preserve_authority() {
    use openpatch::project_document::hosted::{HostedIdentity, HostedModule};
    let directory = tempfile::tempdir().unwrap();
    let patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"dc","kind":"constant","params":{"value":0.5}}],
        "cables":[],"output":"dc"}))
    .unwrap();
    let mut document = ProjectDocument::from_patch(patch, ProjectLayout::default()).unwrap();
    document.graph.nodes.push(DocumentNode::Hosted {
        id: "offset".into(),
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
            0.25,
            0.5,
            true,
            false,
        )
        .unwrap(),
    });
    document.graph.cables.push(
        serde_json::from_value(json!({"route":{
        "id":"host-in","from":"dc","output":"out","to":"offset","input":"in"}}))
        .unwrap(),
    );
    document.layout.positions.insert("dc".into(), [0., 0.]);
    document
        .layout
        .positions
        .insert("offset".into(), [180., 0.]);
    std::fs::write(
        directory.path().join("hosted.json"),
        document.encode().unwrap(),
    )
    .unwrap();
    let mut app = host(directory.path());
    app.load_document("hosted.json", false);
    assert_eq!(app.project.graph.nodes.len(), 2, "{}", app.status);
    assert_eq!(app.patch.nodes.len(), 1);
    assert!(app.audio.is_none());
    let original = serde_json::to_value(&app.project).unwrap();
    let positions = app.rack.layout()["positions"].clone();
    app.save_document("before.openpatch", true);
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let size = egui::vec2(1200., 800.);
    let mut capture = crate::headless_review::CpuCapture::default();
    for frame in 0..2 {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(frame as f64 * 0.1),
                ..Default::default()
            },
            |ctx| app.rack_workspace(ctx, None),
        );
        capture.observe(&output);
        if frame == 1 {
            let text: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                    _ => None,
                })
                .collect();
            assert!(text.contains(&"HOST NOT RUNNING"), "{text:?}");
            assert!(text.contains(&"OFFSET CV"), "{text:?}");
            assert!(text.contains(&"SCALE CV"), "{text:?}");
            assert!(text.contains(&"Execution not enabled"), "{text:?}");
            assert!(!text.contains(&"Click to inspect / repair"), "{text:?}");
            if let Some(path) = std::env::var_os("OPENPATCH_HOSTED_DOCUMENT_REVIEW") {
                let path = PathBuf::from(path);
                std::fs::create_dir_all(&path).unwrap();
                capture.save(&ctx, output, size, path.join("hosted-inactive.png"));
            }
        }
    }
    assert!(
        app.command(
            "set_hosted_parameter",
            json!({"id":"offset","parameter":"offset","value":0.125})
        ),
        "{}",
        app.status
    );
    app.observe_document();
    assert!(app.dirty());
    assert_eq!(app.rack.layout()["positions"], positions);
    let edited = serde_json::to_value(&app.project).unwrap();
    assert!(app.command("undo", json!({})), "{}", app.status);
    app.observe_document();
    assert!(!app.dirty());
    assert_eq!(serde_json::to_value(&app.project).unwrap(), original);
    assert!(app.command("redo", json!({})), "{}", app.status);
    app.save_document("after.openpatch", true);
    let mut reopened = host(directory.path());
    reopened.load_document("after.openpatch", false);
    assert_eq!(serde_json::to_value(&reopened.project).unwrap(), edited);
    assert_eq!(
        reopened.shared.lock().unwrap().execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    assert!(reopened.audio.is_none());
}

#[test]
fn imported_review_repair_history_and_native_save_preserve_source() {
    let directory = tempfile::tempdir().unwrap();
    let source =
        crate::project::test_source_document(json!({"custom":{"opaque":[1,2,3]}}), &[0, 255, 9]);
    std::fs::write(
        directory.path().join("source.json"),
        source.encode().unwrap(),
    )
    .unwrap();
    let mut app = host(directory.path());
    app.load_document("source.json", false);
    assert!(
        app.active_file.is_none(),
        "portable source imports must save as a new project"
    );
    assert_eq!(
        serde_json::to_value(&app.project.sources).unwrap(),
        serde_json::to_value(&source.project().sources).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&app.project.source_blobs).unwrap(),
        serde_json::to_value(&source.project().source_blobs).unwrap()
    );
    assert_eq!(app.project.graph.cables.len(), 1);
    assert!(
        app.shared.lock().unwrap().execution_disposition()
            == ExecutionDisposition::SuspendedUnresolved
    );
    assert!(!app.shared.lock().unwrap().layout_pending());
    app.import_review = None;
    app.import_review_open = false;
    let ctx = egui::Context::default();
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200., 800.),
            )),
            ..Default::default()
        },
        |ctx| app.rack_workspace(ctx, None),
    );
    assert!(!output.shapes.is_empty());
    let pointer = egui::pos2(80., 100.);
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200., 800.),
            )),
            events: vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ctx| app.rack_workspace(ctx, None),
    );
    assert_eq!(
        app.import_review.as_deref(),
        Some("unknown"),
        "clicking a source-only panel opens its dependency review"
    );
    assert!(app.import_review_open);
    assert_eq!(app.rack.take_dependency_focus(), None);
    assert!(
        app.command(
            "substitute_unresolved_module",
            json!({"id":"unknown","kind":"gain","inputs":{},"outputs":{"vcv_out_0":"out"}})
        ),
        "{}",
        app.status
    );
    assert!(app
        .project
        .graph
        .nodes
        .iter()
        .all(|node| matches!(node, DocumentNode::Native { .. })));
    assert_eq!(
        serde_json::to_value(&app.project.sources).unwrap(),
        serde_json::to_value(&source.project().sources).unwrap()
    );
    assert_eq!(app.project.graph.cables[0].route.output, "out");
    assert!(app.command("undo", json!({})));
    assert!(matches!(
        &app.project.graph.nodes[1],
        DocumentNode::Unresolved { .. }
    ));
    assert_eq!(app.project.graph.cables[0].route.output, "vcv_out_0");
    assert!(app.command("set_document_playback", json!({"mode":"partial"})));
    app.save_document("preserved.openpatch", true);
    assert_eq!(
        app.active_file.as_deref(),
        Some("preserved.openpatch"),
        "{}",
        app.status
    );
    let saved = app
        .projects
        .as_ref()
        .unwrap()
        .load("preserved.openpatch")
        .unwrap()
        .document;
    assert_eq!(
        serde_json::to_value(&saved.project().sources).unwrap(),
        serde_json::to_value(&source.project().sources).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&saved.project().source_blobs).unwrap(),
        serde_json::to_value(&source.project().source_blobs).unwrap()
    );
    let layout = app.rack.layout();
    app.load_document("preserved.openpatch", false);
    assert_eq!(app.rack.layout(), layout);
    assert_eq!(
        app.shared.lock().unwrap().playback_mode(),
        PlaybackMode::Exact
    );
    assert!(app.command("remove_unresolved_module", json!({"id":"unknown"})));
    assert_eq!(app.project.graph.nodes.len(), 1);
    assert!(app.command("undo", json!({})));
    assert_eq!(app.project.graph.nodes.len(), 2);
    assert_eq!(
        serde_json::to_value(&app.project.source_blobs).unwrap(),
        serde_json::to_value(&source.project().source_blobs).unwrap()
    );
}

#[test]
fn visible_layout_is_shared_with_mcp_save_without_extra_undo_or_graph_revision() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = parameter_entry_host(directory.path());
    app.publish_current_layout().unwrap();
    let (revision, state_id, can_undo) = {
        let live = app.shared.lock().unwrap();
        (live.revision(), live.state_id(), live.can_undo())
    };
    let mut layout = app.rack.layout();
    layout["pan"] = json!([112, -34]);
    layout["zoom"] = json!(0.75);
    app.rack.restore_layout(&layout).unwrap();
    app.sync();
    {
        let live = app.shared.lock().unwrap();
        assert_eq!(live.revision(), revision);
        assert_eq!(live.state_id(), state_id);
        assert_eq!(live.can_undo(), can_undo);
        assert_eq!(
            serde_json::to_value(&live.project_document().layout).unwrap(),
            app.rack.layout()
        );
    }
    execute_shared(
        &app.shared,
        "save_document",
        json!({"filename":"from-mcp.json","expected_revision":revision}),
    )
    .unwrap();
    let saved =
        ProjectDocument::decode(&std::fs::read(directory.path().join("from-mcp.json")).unwrap())
            .unwrap();
    assert_eq!(saved.layout.pan, [112., -34.]);
    assert_eq!(saved.layout.zoom, 0.75);
    assert_eq!(saved.layout.positions["tone"], [0., 0.]);
    let previous = app.project.layout.clone();
    execute_shared(
        &app.shared,
        "set_parameter",
        json!({"id":"level","parameter":"value","value":2.}),
    )
    .unwrap();
    assert!(app.publish_current_layout().is_err());
    assert_eq!(
        app.shared.lock().unwrap().project_document().layout,
        previous
    );
}

#[test]
#[ignore = "reads user-supplied VCV fixture; optionally writes CPU-rendered review images"]
fn actual_vcv_native_open_save_reopen_and_capture() {
    let path = std::env::var("OPENPATCH_VCV_IMPORT_FIXTURE").expect("set source .vcv path");
    let bytes = std::fs::read(&path).unwrap();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("source.vcv"), &bytes).unwrap();
    let mut app = host(directory.path());
    app.load_document("source.vcv", false);
    assert_eq!(app.project.graph.nodes.len(), 55, "{}", app.status);
    assert_eq!(app.patch.nodes.len(), 37);
    assert_eq!(app.project.graph.cables.len(), 83);
    assert!(!app.shared.lock().unwrap().layout_pending());
    let expected = app.project.clone();
    app.save_document("imported.openpatch", true);
    assert_eq!(
        app.active_file.as_deref(),
        Some("imported.openpatch"),
        "{}",
        app.status
    );
    app.load_document("imported.openpatch", false);
    assert_eq!(app.project.reference_view(), expected.reference_view());
    assert_eq!(
        app.shared.lock().unwrap().playback_mode(),
        PlaybackMode::Exact
    );
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    if let Some(destination) = std::env::var_os("OPENPATCH_IMPORT_PREVIEW") {
        let destination = PathBuf::from(destination);
        std::fs::create_dir_all(&destination).unwrap();
        let size = egui::vec2(1560., 940.);
        // Review a useful cluster of the original patch, using actual panels,
        // retained routes and the dependency inspector. Never open a device.
        app.rack
            .restore_layout(&{
                let mut layout = app.rack.layout();
                layout["zoom"] = json!(0.65);
                layout["pan"] = json!([24, 24]);
                layout
            })
            .unwrap();
        for amount in [1., 0.08] {
            app.rack.set_room_brightness(amount);
            let ctx = egui::Context::default();
            let mut capture = crate::headless_review::CpuCapture::default();
            let mut last = None;
            for time in [0., 0.1] {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        time: Some(time),
                        ..Default::default()
                    },
                    |ctx| app.rack_workspace(ctx, None),
                );
                capture.observe(&output);
                last = Some(output);
            }
            capture.save(
                &ctx,
                last.unwrap(),
                size,
                destination.join(format!("import-{amount:.2}.png")),
            );
        }
    }
}

#[test]
fn mcp_authored_replacement_uses_its_layout_even_when_module_ids_are_reused() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = parameter_entry_host(directory.path());
    let mut document = app.project.clone();
    document.layout.pan = [150., -83.];
    document.layout.zoom = 0.8;
    document
        .layout
        .positions
        .insert("tone".into(), [750., 420.]);
    document
        .layout
        .positions
        .insert("level".into(), [45., 840.]);
    let expected = document.layout.clone();
    execute_shared(
        &app.shared,
        "replace_document",
        json!({"document":document}),
    )
    .unwrap();
    assert!(app.shared.lock().unwrap().layout_pending());
    app.sync();
    assert_eq!(
        serde_json::from_value::<ProjectLayout>(app.rack.layout()).unwrap(),
        expected
    );
    assert_eq!(
        app.shared.lock().unwrap().project_document().layout,
        expected
    );
    assert!(!app.shared.lock().unwrap().layout_pending());
}

#[test]
fn repeat_raw_import_refits_source_coordinates_but_saved_authored_layout_does_not() {
    let directory = tempfile::tempdir().unwrap();
    let bytes = serde_json::to_vec(&json!({"version":"2.6.6","modules":[
        {"id":1,"plugin":"Fundamental","model":"VCMixer","version":"2.6.4","pos":[0,0]},
        {"id":2,"plugin":"Unknown","model":"Effect","version":"1","pos":[1,0]}
    ],"cables":[]}))
    .unwrap();
    std::fs::write(directory.path().join("source.vcv"), bytes).unwrap();
    let mut app = host(directory.path());
    execute_shared(&app.shared, "import_vcv", json!({"filename":"source.vcv"})).unwrap();
    app.sync();
    let fitted = app.project.layout.clone();
    assert!(fitted.positions["vcv_m_2"][0] >= crate::rack::module_width(&app.patch.nodes[0]) + 15.);
    let mut authored = app.project.clone();
    authored.layout.pan = [29., -30.];
    authored.layout.zoom = 0.75;
    authored
        .layout
        .positions
        .insert("vcv_m_1".into(), [150., 420.]);
    authored
        .layout
        .positions
        .insert("vcv_m_2".into(), [30., 840.]);
    let expected = authored.layout.clone();
    execute_shared(
        &app.shared,
        "replace_document",
        json!({"document":authored}),
    )
    .unwrap();
    app.sync();
    assert_eq!(
        app.project.layout, expected,
        "loading the same source package must preserve authored geometry"
    );
    execute_shared(&app.shared, "import_vcv", json!({"filename":"source.vcv"})).unwrap();
    app.sync();
    assert_eq!(
        app.project.layout, fitted,
        "a new raw import must start from source coordinates, even for the same package/IDs"
    );
}

#[test]
fn undo_into_unseen_console_state_restores_its_settled_geometry() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = host(directory.path());
    assert!(app.command(
        "replace_patch",
        json!({"patch":{"version":2,"nodes":[
            {"id":"console","kind":"studio_mixer","params":{"channels":2}},
            {"id":"neighbor","kind":"constant"}
        ]}})
    ));
    let mut layout = app.rack.layout();
    layout["positions"] = json!({"console":[0,0],"neighbor":[372,0]});
    app.rack.restore_layout(&layout).unwrap();
    app.publish_current_layout().unwrap();
    // B is a complete, unobserved state; C then grows the console.
    execute_shared(
        &app.shared,
        "set_parameter",
        json!({"id":"neighbor","parameter":"value","value":2}),
    )
    .unwrap();
    execute_shared(
        &app.shared,
        "set_parameter",
        json!({"id":"console","parameter":"channels","value":4}),
    )
    .unwrap();
    app.sync();
    assert_eq!(app.project.layout.positions["neighbor"], [516., 0.]);
    execute_shared(&app.shared, "undo", json!({})).unwrap();
    app.sync();
    assert_eq!(app.project.layout.positions["neighbor"], [372., 0.]);
    assert_eq!(app.patch.nodes[0].params["channels"], 2.);
    execute_shared(
        &app.shared,
        "set_parameter",
        json!({"id":"neighbor","parameter":"value","value":3}),
    )
    .unwrap();
    app.sync();
    execute_shared(&app.shared, "undo", json!({})).unwrap();
    app.sync();
    assert_eq!(
        app.shared.lock().unwrap().layout_origin(),
        None,
        "an unchanged authored layout still needs acknowledgement"
    );
    execute_shared(
        &app.shared,
        "set_parameter",
        json!({"id":"console","parameter":"channels","value":4}),
    )
    .unwrap();
    app.sync();
    assert_eq!(app.project.layout.positions["neighbor"], [516., 0.]);
}

#[test]
fn initial_host_publication_fits_an_early_mcp_raw_import_before_clearing_its_origin() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = host(directory.path());
    let bytes = serde_json::to_vec(&json!({"version":"2.6.6","modules":[
        {"id":1,"plugin":"Fundamental","model":"VCMixer","version":"2.6.4","pos":[0,0]},
        {"id":2,"plugin":"Unknown","model":"Effect","version":"1","pos":[1,0]}
    ],"cables":[]}))
    .unwrap();
    {
        let mut live = app.shared.lock().unwrap();
        live.import_vcv_bytes(&bytes, None).unwrap();
        // The constructor takes this same coherent snapshot before creating its rack.
        app.project = live.project_document().clone();
        app.patch = live.patch().clone();
        app.revision = live.revision();
        app.state_id = live.state_id();
        app.layout_generation = live.layout_generation();
    }
    app.rack = Rack::default();
    app.rack
        .restore_layout(&serde_json::to_value(&app.project.layout).unwrap())
        .unwrap();
    app.rack
        .reconcile_document(&app.patch, &app.project, &app.catalog);
    assert_eq!(app.project.layout.positions["vcv_m_2"], [15., 0.]);
    app.publish_current_layout().unwrap();
    assert!(
        app.project.layout.positions["vcv_m_2"][0]
            >= crate::rack::module_width(&app.patch.nodes[0]) + 15.
    );
    assert_eq!(app.shared.lock().unwrap().layout_origin(), None);
    assert!(!app.shared.lock().unwrap().layout_pending());
}

#[test]
fn imported_automation_review_suspends_exact_and_explicitly_enables_partial_with_retained_lane() {
    let directory = tempfile::tempdir().unwrap();
    let source = json!({"version":"2.6.6","modules":[
        {"id":101,"plugin":"RackMCP","model":"Sequencer","version":"2.0.0","pos":[0,0],
         "data":{"patterns":[{"name":"A","length":4,"tracks":[{"steps":[{"step":0,"notes":[0,7],"accent":true}]}]}],
           "lanes":[{"moduleId":"42","paramId":0,"points":[{"bar":0,"value":0.25}]},
                    {"moduleId":"42","paramId":1,"points":[{"bar":0,"value":1}]}]}},
        {"id":42,"plugin":"Fundamental","model":"VCA-1","version":"2.6.4","pos":[50,0]}
    ],"cables":[]});
    let imported = openpatch::vcv_import::import(&serde_json::to_vec(&source).unwrap()).unwrap();
    let mut app = host(directory.path());
    app.shared
        .lock()
        .unwrap()
        .replace_project_document(imported.document, None)
        .unwrap();
    app.sync();
    app.observe_document();
    app.import_review_open = true;
    assert_eq!(
        app.shared.lock().unwrap().execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let size = egui::vec2(1560., 900.);
    let draw = |app: &mut OpenPatch, time, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| app.rack_workspace(ctx, None),
        )
    };
    let mut capture = crate::headless_review::CpuCapture::default();
    let first = draw(&mut app, 0., vec![]);
    capture.observe(&first);
    let output = draw(&mut app, 0.1, vec![]);
    capture.observe(&output);
    let text: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
            _ => None,
        })
        .collect();
    assert!(text
        .iter()
        .any(|text| text.contains("0 unresolved modules · 1 omitted automation lane")));
    assert!(text
        .iter()
        .any(|text| text.contains("source 42 / control 1: no qualified control binding")));
    let pointer = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "Enable partial" => {
                Some(egui::Rect::from_min_size(text.pos, text.galley.size()).center())
            }
            _ => None,
        })
        .expect("explicit Partial playback action is visible");
    let destination = std::env::var_os("OPENPATCH_AUTOMATION_IMPORT_REVIEW").map(PathBuf::from);
    if let Some(destination) = &destination {
        std::fs::create_dir_all(destination).unwrap();
        capture.save(&ctx, output, size, destination.join("exact-suspended.png"));
    }
    let output = draw(
        &mut app,
        0.2,
        vec![
            egui::Event::PointerMoved(pointer),
            egui::Event::PointerButton {
                pos: pointer,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos: pointer,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    capture.observe(&output);
    assert_eq!(
        app.shared.lock().unwrap().playback_mode(),
        PlaybackMode::Partial,
        "{}",
        app.status
    );
    let state = app.shared.lock().unwrap().document_execution_status();
    assert_eq!(state["omitted_automation"].as_array().unwrap().len(), 1);
    assert_eq!(state["requested_disposition"], "runnable_partial");
    if let Some(destination) = &destination {
        let output = draw(&mut app, 0.3, vec![]);
        capture.observe(&output);
        capture.save(&ctx, output, size, destination.join("partial-explicit.png"));
    }
}

#[test]
fn core_notes_import_edit_undo_save_reopen_preserves_source_and_dependency_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let text = "Envelope guide\n\t日本語 🎛️ café\n  keep whitespace  ";
    let source = json!({"version":"2.6.6","modules":[
        {"id":1,"plugin":"Core","model":"Notes","version":"2.6.6","params":[],"pos":[0,0],"data":{"text":text}},
        {"id":2,"plugin":"Missing","model":"Effect","version":"2.0","pos":[16,0],"data":{"opaque":[1,2,3]}}
    ],"cables":[]});
    let bytes = serde_json::to_vec_pretty(&source).unwrap();
    let path = directory.path().join("notes.vcv");
    std::fs::write(&path, &bytes).unwrap();
    let mut app = host(directory.path());
    app.load_document("notes.vcv", false);
    assert_eq!(app.project.graph.nodes.len(), 2, "{}", app.status);
    assert_eq!(app.patch.nodes.len(), 1);
    assert_eq!(app.patch.nodes[0].effective_notes_text(), Some(text));
    assert!(matches!(
        &app.project.graph.nodes[1],
        DocumentNode::Unresolved { .. }
    ));
    assert_eq!(
        app.shared.lock().unwrap().execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    assert!(app.audio.is_none());
    assert_eq!(
        execute_shared(&app.shared, "inspect_bridge", json!({})).unwrap()["transport"]["state"],
        "stopped"
    );
    let payload = |app: &OpenPatch| {
        serde_json::to_value((&app.project.sources, &app.project.source_blobs)).unwrap()
    };
    let original = payload(&app);
    app.save_document("original-notes.openpatch", true);
    assert!(!app.dirty(), "{}", app.status);
    let changed = "Updated annotation\nα β γ\n  preserved  ";
    assert!(
        app.command(
            "set_module_content",
            json!({"id":"vcv_m_1","content":{"type":"notes","text":changed}})
        ),
        "{}",
        app.status
    );
    assert_eq!(app.patch.nodes[0].effective_notes_text(), Some(changed));
    app.observe_document(); // End-of-frame saved-state observation.
    assert!(app.dirty());
    assert!(app.command("undo", json!({})));
    assert_eq!(app.patch.nodes[0].effective_notes_text(), Some(text));
    app.observe_document();
    assert!(!app.dirty());
    assert!(app.command("redo", json!({})));
    assert_eq!(app.patch.nodes[0].effective_notes_text(), Some(changed));
    app.observe_document();
    assert!(app.dirty());
    assert_eq!(payload(&app), original);
    app.save_document("edited-notes.openpatch", true);
    assert_eq!(
        app.active_file.as_deref(),
        Some("edited-notes.openpatch"),
        "{}",
        app.status
    );
    assert!(!app.dirty());
    let mut reopened = host(directory.path());
    reopened.load_document("edited-notes.openpatch", false);
    assert_eq!(
        reopened.patch.nodes[0].effective_notes_text(),
        Some(changed)
    );
    assert_eq!(payload(&reopened), original);
    assert_eq!(reopened.project.graph.nodes.len(), 2);
    assert_eq!(
        reopened.shared.lock().unwrap().execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    assert!(!reopened.dirty(), "{}", reopened.status);
    assert!(reopened.audio.is_none());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
