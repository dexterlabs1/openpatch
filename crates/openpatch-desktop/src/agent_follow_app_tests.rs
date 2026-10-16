use super::*;

fn controls(
    app: &mut OpenPatch,
    ctx: &egui::Context,
    focused: bool,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 600.0),
            )),
            focused,
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.agent_motion_controls(ui));
            app.service_agent_follow(ctx);
        },
    )
}

fn click(app: &mut OpenPatch, ctx: &egui::Context, label: &str, focused: bool) {
    controls(app, ctx, focused, vec![]);
    let output = controls(app, ctx, focused, vec![]);
    let point = preset_text(&output, label);
    controls(app, ctx, focused, vec![egui::Event::PointerMoved(point)]);
    controls(app, ctx, focused, vec![pointer_event(point, true)]);
    controls(app, ctx, focused, vec![pointer_event(point, false)]);
}

#[test]
fn follow_checkbox_is_opt_in_session_state_and_never_patch_work() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let ctx = egui::Context::default();
    let graph = app.shared.lock().unwrap().snapshot();
    let layout = app.rack.layout();
    let document = app.capture_document();
    let dirty = app.dirty();
    assert!(!app.rack.agent_follow_enabled());
    click(&mut app, &ctx, "Follow agent edits", true);
    assert!(app.rack.agent_follow_enabled());
    assert_eq!(app.shared.lock().unwrap().snapshot(), graph);
    assert_eq!(app.rack.layout(), layout);
    assert!(app.capture_document().equivalent(&document));
    assert_eq!(app.dirty(), dirty);
    click(&mut app, &ctx, "Follow agent edits", true);
    assert!(!app.rack.agent_follow_enabled());
    click(&mut app, &ctx, "Reduced", true);
    click(&mut app, &ctx, "Follow agent edits", true);
    assert!(app.rack.agent_follow_enabled());
    assert!(app.capture_document().equivalent(&document));
    assert_eq!(app.shared.lock().unwrap().snapshot(), graph);
    app.replace_rack(Rack::default());
    assert!(
        app.rack.agent_follow_enabled(),
        "A new document keeps the window opt-in"
    );
    let fresh_pan = app.rack.agent_follow_view_pan();
    for millis in [0, 260] {
        app.rack
            .set_construction_clock_override(Some(Instant::now() + Duration::from_millis(millis)));
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                focused: true,
                ..Default::default()
            },
            |ctx| app.rack_workspace(ctx, None),
        );
        assert_eq!(
            app.rack.agent_follow_view_pan(),
            fresh_pan,
            "Replacement must not replay earlier edits"
        );
    }
    assert!(!host(dir.path()).rack.agent_follow_enabled());
}

#[test]
fn follow_checkbox_respects_motion_modals_focus_and_library_drag() {
    for blocked in ["off", "settings", "file", "blur", "drag"] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let ctx = egui::Context::default();
        app.settings_open = blocked == "settings";
        app.file_mode = (blocked == "file").then_some(FileMode::Open);
        if blocked == "off" {
            app.construction
                .set_motion_mode(crate::construction_motion::MotionMode::Off);
            app.rack
                .set_construction_motion_mode(crate::construction_motion::MotionMode::Off);
        }
        if blocked == "drag" {
            egui::DragAndDrop::set_payload(
                &ctx,
                LibraryDrag {
                    kind: "constant".into(),
                },
            );
        }
        click(&mut app, &ctx, "Follow agent edits", blocked != "blur");
        assert!(!app.rack.agent_follow_enabled(), "{blocked} enabled follow");
    }
}

#[test]
fn controls_observe_manual_follow_disable_without_reenabling_on_later_frames() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let ctx = egui::Context::default();
    click(&mut app, &ctx, "Follow agent edits", true);
    assert!(app.rack.agent_follow_enabled());
    app.rack.set_zoom(0.85); // Real manual navigation uses Rack's disabling path.
    assert!(!app.rack.agent_follow_enabled());
    for _ in 0..3 {
        controls(&mut app, &ctx, true, vec![]);
        app.sync();
        assert!(!app.rack.agent_follow_enabled());
    }
    click(&mut app, &ctx, "Follow agent edits", true);
    assert!(app.rack.agent_follow_enabled());
}

#[test]
fn idle_view_menu_exposes_agent_motion_and_follow_checkbox() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let ctx = egui::Context::default();
    assert!(app.shared.lock().unwrap().construction_snapshot()["plan_id"].is_null());
    let mut time = 0.0;
    let mut frame = |app: &mut OpenPatch, events| {
        time += 0.1;
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440.0, 960.0),
                )),
                time: Some(time),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| app.toolbar(ctx),
        )
    };
    frame(&mut app, vec![]);
    for label in ["View", "Motion"] {
        let output = frame(&mut app, vec![]);
        let point = preset_text(&output, label);
        frame(&mut app, vec![egui::Event::PointerMoved(point)]);
        frame(&mut app, vec![pointer_event(point, true)]);
        frame(&mut app, vec![pointer_event(point, false)]);
    }
    let output = frame(&mut app, vec![]);
    let point = preset_text(&output, "Follow agent edits");
    frame(&mut app, vec![egui::Event::PointerMoved(point)]);
    frame(&mut app, vec![pointer_event(point, true)]);
    frame(&mut app, vec![pointer_event(point, false)]);
    assert!(app.rack.agent_follow_enabled());
}

#[test]
fn actual_agent_camera_preserves_saved_project_until_manual_navigation() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    assert!(app.command(
        "replace_patch",
        json!({"patch":{"version":2,"nodes":[
            {"id":"near-voice","kind":"multi_vco","params":{"frequency":110}},
            {"id":"far-voice","kind":"multi_vco","params":{"frequency":220}}
        ]}})
    ));
    app.replace_rack(Rack::default());
    app.rack
        .restore_layout(&json!({"version":1,"zoom":1.0,"pan":[72,48],
        "positions":{"near-voice":[0,0],"far-voice":[1500,0]}}))
        .unwrap();
    let ctx = egui::Context::default();
    let origin = Instant::now();
    app.smoke_motion_origin = Some(origin);
    app.rack.set_construction_clock_override(Some(origin));
    app.rack.set_agent_follow(true);
    let mut time = 0.0;
    let mut rack_frame = |app: &mut OpenPatch, millis: u64| {
        app.rack
            .set_construction_clock_override(Some(origin + Duration::from_millis(millis)));
        time += 0.05;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                focused: true,
                time: Some(time),
                ..Default::default()
            },
            |ctx| app.rack_workspace(ctx, None),
        );
    };
    // Establish the actual shared renderer's viewport and offscreen anchor.
    rack_frame(&mut app, 0);
    let before_pan = app.rack.agent_follow_view_pan();
    let before_revision = app.revision;
    execute_shared(&app.shared, "start_construction", json!({
        "title":"Tune the far voice","expected_revision":before_revision,"interval_ms":100,
        "steps":[{"label":"Tune the offscreen oscillator","edits":[
            {"tool":"set_parameter","arguments":{"id":"far-voice","parameter":"frequency","value":440}}
        ]}]
    })).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let receipt = loop {
        let state = app.shared.lock().unwrap().construction_snapshot();
        if state["status"] == "completed" {
            break state["last_commit"].clone();
        }
        assert_eq!(state["status"], "running", "{state}");
        assert!(
            Instant::now() < deadline,
            "Agent camera fixture did not complete"
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(receipt["before_revision"], before_revision);
    while app.revision < receipt["revision"].as_u64().unwrap() {
        app.sync();
        assert!(
            Instant::now() < deadline,
            "Agent camera fixture was not observed"
        );
        std::thread::yield_now();
    }
    assert_eq!(
        app.patch
            .nodes
            .iter()
            .find(|node| node.id == "far-voice")
            .unwrap()
            .params["frequency"],
        440.0
    );
    // Save the committed graph before its camera presentation begins. This
    // isolates presentation from the legitimate dirty change made by the edit.
    let saved = app.capture_document();
    app.documents.mark_saved(&saved);
    assert!(!app.dirty());
    let graph = app.shared.lock().unwrap().snapshot();
    let layout_revision = app.rack.layout_revision();
    for millis in [0, 110, 260] {
        rack_frame(&mut app, millis);
        let observed = app.capture_document();
        assert!(
            observed.equivalent(&saved),
            "Camera changed saved content at {millis}ms"
        );
        assert!(app.documents.matches_saved(&observed));
        assert!(!app.dirty(), "Camera dirtied the project at {millis}ms");
        assert_eq!(app.rack.layout_revision(), layout_revision);
        assert_eq!(app.shared.lock().unwrap().snapshot(), graph);
    }
    let settled_pan = app.rack.agent_follow_view_pan();
    assert_ne!(settled_pan, before_pan);
    click(&mut app, &ctx, "Follow agent edits", true);
    assert!(!app.rack.agent_follow_enabled());
    assert_eq!(
        app.rack.agent_follow_view_pan(),
        settled_pan,
        "Disabling follow jumped the camera"
    );
    let disabled = app.capture_document();
    assert!(disabled.equivalent(&saved));
    assert!(app.documents.matches_saved(&disabled));
    assert!(!app.dirty());

    // Return keyboard ownership to the rack before using the host shortcut.
    if let Some(id) = ctx.memory(|memory| memory.focused()) {
        ctx.memory_mut(|memory| memory.surrender_focus(id));
    }
    // Use the real host shortcut path to take over the visible camera.
    let modifiers = egui::Modifiers::COMMAND;
    let _ = ctx.run(
        egui::RawInput {
            focused: true,
            modifiers,
            events: vec![egui::Event::Key {
                key: egui::Key::Plus,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        },
        |ctx| app.global_shortcuts(ctx),
    );
    let manual = app.capture_document();
    assert!(!manual.equivalent(&saved));
    assert!(!app.documents.matches_saved(&manual));
    assert!(app.dirty());
    let visible = app.rack.agent_follow_view_pan();
    let persisted_pan = &manual.layout["pan"];
    assert!((persisted_pan[0].as_f64().unwrap() as f32 - visible.x).abs() < 0.001);
    assert!((persisted_pan[1].as_f64().unwrap() as f32 - visible.y).abs() < 0.001);
    assert_eq!(app.shared.lock().unwrap().snapshot(), graph);
}
