use super::*;

fn ui_frame(
    ctx: &egui::Context,
    app: &mut OpenPatch,
    size: Vec2,
    time: f64,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            time: Some(time),
            focused: true,
            events,
            ..Default::default()
        },
        |ctx| {
            app.toolbar(ctx);
            app.footer(ctx);
            egui::CentralPanel::default().show(ctx, |_ui| {});
            app.audio_performance(ctx);
        },
    )
}

fn click_panel(
    ctx: &egui::Context,
    app: &mut OpenPatch,
    size: Vec2,
    position: egui::Pos2,
    time: f64,
) {
    for (offset, pressed) in [(0.0, true), (0.05, false)] {
        ui_frame(
            ctx,
            app,
            size,
            time + offset,
            vec![
                egui::Event::PointerMoved(position),
                pointer_event(position, pressed),
            ],
        );
    }
}

#[test]
fn audio_history_uses_only_new_finite_observations_and_clears_on_stream_or_interval_change() {
    let mut history = AudioTimingHistory::default();
    let mut snapshot = AudioTimingSnapshot {
        available: true,
        coherent: true,
        sample_rate: 48000,
        utilization: Some(0.3),
        ..Default::default()
    };
    for sequence in 0..400 {
        snapshot.callback_sequence = sequence;
        history.observe(Some(&snapshot), sequence as f64 * 0.11);
    }
    assert_eq!(history.points.len(), AudioTimingHistory::LIMIT);
    let count = history.points.len();
    history.observe(Some(&snapshot), 100.0);
    assert_eq!(
        history.points.len(),
        count,
        "a repaint must not invent a callback"
    );
    snapshot.callback_sequence += 1;
    snapshot.utilization = Some(f64::NAN);
    history.observe(Some(&snapshot), 101.0);
    assert_eq!(history.points.len(), count);
    snapshot.utilization = Some(f64::INFINITY);
    history.observe(Some(&snapshot), 102.0);
    assert!(history.points.iter().all(|(_, value)| value.is_finite()));
    snapshot.utilization = Some(0.6);
    snapshot.epoch += 1;
    history.observe(Some(&snapshot), 103.0);
    assert_eq!(history.points.len(), 1);
    snapshot.callback_sequence = 1;
    history.observe(Some(&snapshot), 104.0);
    assert_eq!(
        history.points.len(),
        1,
        "a restarted stream cannot inherit another stream's plot"
    );
    history.observe(None, 105.0);
    assert!(history.points.is_empty());
}

#[test]
fn performance_dropup_anchors_to_footer_and_toggles_or_dismisses_without_a_measurement() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let ctx = egui::Context::default();
    let size = egui::vec2(940.0, 680.0);
    for time in [0.0, 0.2] {
        ui_frame(&ctx, &mut app, size, time, vec![]);
    }
    let anchor = ctx
        .data(|data| {
            data.get_temp::<(egui::Rect, egui::LayerId)>(egui::Id::new("audio-performance-anchor"))
        })
        .unwrap()
        .0;
    click_panel(&ctx, &mut app, size, anchor.center(), 1.0);
    let output = ui_frame(&ctx, &mut app, size, 1.2, vec![]);
    preset_text(&output, "DSP load history");
    preset_text(&output, "Waiting for measured callbacks");
    assert!(app.audio_performance_open);
    let popup = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("audio-performance-dropdown-rect")))
        .unwrap();
    assert!(popup.bottom() <= anchor.top());
    assert!((popup.right() - anchor.right()).abs() < 3.0);
    assert!(app.audio_timing_history.points.is_empty());
    click_panel(&ctx, &mut app, size, anchor.center(), 2.0);
    assert!(!app.audio_performance_open);
    click_panel(&ctx, &mut app, size, anchor.center(), 3.0);
    ui_frame(
        &ctx,
        &mut app,
        size,
        3.2,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(!app.audio_performance_open);
    click_panel(&ctx, &mut app, size, anchor.center(), 4.0);
    click_panel(&ctx, &mut app, size, egui::pos2(60.0, 220.0), 5.0);
    assert!(!app.audio_performance_open);
}

#[test]
fn footer_room_slider_is_visible_in_narrow_and_wide_windows_and_persists_without_patch_edits() {
    for width in [940.0, 1440.0] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        let before = app.shared.lock().unwrap().snapshot();
        let ctx = egui::Context::default();
        let size = egui::vec2(width, 680.0);
        app.status = "Long project status ".repeat(30);
        for time in [0.0, 0.2] {
            ui_frame(&ctx, &mut app, size, time, vec![]);
        }
        let response = ctx
            .data(|data| {
                data.get_temp::<egui::Response>(egui::Id::new("footer-room-brightness-response"))
            })
            .unwrap();
        assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(response.rect));
        assert!(response.rect.top() > 630.0);
        click_panel(
            &ctx,
            &mut app,
            size,
            response.rect.left_center() + egui::vec2(20.0, 0.0),
            1.0,
        );
        assert!(app.rack.room_brightness() < 0.9);
        let settings: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join(".host-settings.json")).unwrap())
                .unwrap();
        assert_eq!(
            settings["room_brightness"],
            json!(app.rack.room_brightness())
        );
        assert_eq!(before, app.shared.lock().unwrap().snapshot());
    }
}

#[test]
fn successful_project_opens_and_saves_persist_bounded_recency_while_failures_leave_order_unchanged()
{
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    app.save_document("first.openpatch", true);
    app.save_document("second.openpatch", true);
    assert_eq!(app.recent_files, ["second.openpatch", "first.openpatch"]);
    app.load_document("first.openpatch", false);
    assert_eq!(app.recent_files, ["first.openpatch", "second.openpatch"]);
    app.load_document("missing.openpatch", false);
    app.save_document("second.openpatch", true);
    assert_eq!(app.recent_files, ["first.openpatch", "second.openpatch"]);
    std::fs::write(dir.path().join("source.vcv"), serde_json::to_vec(&json!({
        "version":"2.6.4", "modules":[{"id":42,"plugin":"MissingVendor","model":"OpaqueVoice","version":"1.2.3","pos":[1,0]}], "cables":[]
    })).unwrap()).unwrap();
    app.load_document("source.vcv", false);
    assert_eq!(app.recent_files[0], "source.vcv", "{}", app.status);
    assert!(
        app.active_file.is_none(),
        "Rack imports still require Save As"
    );
    let settings: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join(".host-settings.json")).unwrap())
            .unwrap();
    let mut reopened = host(dir.path());
    reopened.apply_host_preferences(&settings);
    assert_eq!(reopened.recent_files, app.recent_files);
    let names: Vec<_> = (0..20)
        .map(|n| json!(format!("patch{n}.openpatch")))
        .collect();
    reopened.apply_host_preferences(&json!({"recent_files":names}));
    assert_eq!(reopened.recent_files.len(), RECENT_FILE_LIMIT);
    assert_eq!(
        recent_files_from_settings(&json!([
            "../escape.openpatch",
            "a.openpatch",
            "a.openpatch",
            "bogus",
            "b.json"
        ])),
        ["a.openpatch", "b.json"]
    );
}

#[test]
fn organize_applies_one_undoable_layout_move_and_position_lock_refuses_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let mut layout = app.rack.layout();
    for (index, value) in layout["positions"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .enumerate()
    {
        *value = json!([index as f32 * 1700.0, index as f32 * 900.0]);
    }
    app.rack.restore_layout(&layout).unwrap();
    app.publish_current_layout().unwrap();
    app.sync();
    let positions = app.rack.layout()["positions"].clone();
    let graph = json!(app.project.graph);
    let count = app.shared.lock().unwrap().history_state_ids().count();
    app.organize_rows = Some(2);
    app.organize_rack();
    assert_ne!(app.rack.layout()["positions"], positions, "{}", app.status);
    assert_eq!(
        app.shared.lock().unwrap().history_state_ids().count(),
        count + 1
    );
    assert_eq!(json!(app.project.graph), graph);
    app.history(false);
    assert_eq!(app.rack.layout()["positions"], positions);
    app.rack.set_positions_locked(true);
    app.organize_rack();
    assert!(app.status.contains("Unlock"), "{}", app.status);
    assert_eq!(app.rack.layout()["positions"], positions);
}

#[test]
fn recent_menu_keeps_missing_files_visible_and_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    app.save_document("gone.openpatch", true);
    std::fs::remove_file(dir.path().join("gone.openpatch")).unwrap();
    let ctx = egui::Context::default();
    let draw = |app: &mut OpenPatch, time, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(940.0, 680.0),
                )),
                time: Some(time),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.recent_files_menu(ui));
            },
        )
    };
    draw(&mut app, 0.0, vec![]);
    let output = draw(&mut app, 0.2, vec![]);
    let at = preset_text(&output, "Open recent");
    for (time, pressed) in [(1.0, true), (1.1, false)] {
        draw(
            &mut app,
            time,
            vec![egui::Event::PointerMoved(at), pointer_event(at, pressed)],
        );
    }
    let output = draw(&mut app, 1.3, vec![]);
    preset_text(&output, "gone.openpatch · unavailable");
    assert_eq!(app.recent_files, ["gone.openpatch"]);
    let at = ctx
        .data(|data| {
            data.get_temp::<egui::Rect>(egui::Id::new(("recent-file-rect", "gone.openpatch")))
        })
        .unwrap()
        .center();
    for (time, pressed) in [(2.0, true), (2.1, false)] {
        draw(
            &mut app,
            time,
            vec![egui::Event::PointerMoved(at), pointer_event(at, pressed)],
        );
    }
    assert!(!app.status.starts_with("Open failed:"));
}

#[test]
fn organize_includes_imported_unresolved_panels_and_undo_preserves_the_source_document() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("source.vcv"), serde_json::to_vec(&json!({
        "version":"2.6.6", "modules":[
            {"id":1,"plugin":"Core","model":"Notes","version":"2.6.6","params":[],"pos":[60,9],"data":{"text":"Keep source"}},
            {"id":2,"plugin":"Missing","model":"Effect","version":"2.0","pos":[80,12],"data":{"opaque":[1,2,3]}}
        ], "cables":[]
    })).unwrap()).unwrap();
    let mut app = host(dir.path());
    app.load_document("source.vcv", false);
    let before = app.rack.layout()["positions"].clone();
    let graph = json!(app.project.graph);
    let sources = json!(app.project.sources);
    app.organize_rows = Some(1);
    app.organize_rack();
    assert!(app.status.starts_with("Organized"), "{}", app.status);
    assert_ne!(app.rack.layout()["positions"], before);
    assert_eq!(json!(app.project.graph), graph);
    assert_eq!(json!(app.project.sources), sources);
    app.history(false);
    assert_eq!(app.rack.layout()["positions"], before);
    assert_eq!(json!(app.project.graph), graph);
}

#[test]
fn cable_finish_host_preference_roundtrips_without_changing_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let before = app.shared.lock().unwrap().snapshot();
    let mut presentation = app.rack.cable_presentation();
    presentation.finish = crate::cable_presentation::CableFinish::Woven;
    app.set_cable_presentation(presentation);
    app.save_host_settings();
    let settings: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join(".host-settings.json")).unwrap())
            .unwrap();
    assert_eq!(settings["cable_finish"], "woven");
    let mut reopened = host(dir.path());
    reopened.apply_host_preferences(&settings);
    assert_eq!(
        reopened.rack.cable_presentation().finish,
        presentation.finish
    );
    assert_eq!(before, app.shared.lock().unwrap().snapshot());
}

#[test]
#[ignore = "explicit CPU rendering review; no native window or audio device"]
fn render_audio_mcp_and_footer_review() {
    let destination = std::env::var_os("OPENPATCH_UI_PANEL_REVIEW")
        .map(PathBuf::from)
        .expect("set OPENPATCH_UI_PANEL_REVIEW output directory");
    std::fs::create_dir_all(&destination).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let ctx = egui::Context::default();
    let mut capture = crate::headless_review::CpuCapture::default();
    let draw = |app: &mut OpenPatch, size: Vec2, time, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(time),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                app.toolbar(ctx);
                app.footer(ctx);
                app.rack_workspace(ctx, None);
                app.mcp_session
                    .show(ctx, &app.shared, Path::new(&app.workspace));
                app.audio_performance(ctx);
            },
        )
    };
    let size = egui::vec2(1440.0, 960.0);
    app.smoke_test = Some((destination.join("unused.png"), 0, 0));
    let mut snapshot = AudioTimingSnapshot {
        available: true,
        coherent: true,
        sample_rate: 48000,
        frames: 256,
        channels: 2,
        epoch: 1,
        peak_utilization: Some(1.08),
        max_duration_ns: 5_760_000,
        overruns: 3,
        total_frames: 81_920,
        total_duration_ns: 650_000_000,
        callbacks: 320,
        ..Default::default()
    };
    // Declared illustration data for visual inspection only. Production history
    // receives only coherent publications from Audio::diagnostics.
    for sequence in 1..=160 {
        let phase = sequence as f64 * 0.13;
        let load = if sequence % 43 == 0 {
            1.08
        } else {
            0.22 + phase.sin().abs() * 0.43
        };
        snapshot.callback_sequence = sequence;
        snapshot.utilization = Some(load);
        snapshot.duration_ns = (load * 256.0 / 48_000.0 * 1_000_000_000.0) as u64;
        app.audio_timing_history
            .observe(Some(&snapshot), sequence as f64 * 0.11);
    }
    app.smoke_diagnostics = Some(snapshot);
    app.audio_performance_open = true;
    app.status = "CPU review · declared simulated timing fixture · no audio device".into();
    for time in [18.0, 18.1] {
        let output = draw(&mut app, size, time, vec![]);
        capture.observe(&output);
    }
    let output = draw(&mut app, size, 18.2, vec![]);
    capture.observe(&output);
    capture.save(
        &ctx,
        output,
        size,
        destination.join("audio-performance-dropup.png"),
    );

    app.audio_performance_open = false;
    app.smoke_diagnostics = None;
    app.smoke_test = None;
    app.mcp_session = crate::mcp_session_ui::Panel::new(false);
    app.status = "CPU review · MCP bridge stopped · no audio device".into();
    egui::Popup::close_all(&ctx);
    let output = draw(&mut app, size, 19.0, vec![]);
    capture.observe(&output);
    let position = preset_text(&output, "MCP off");
    for (time, pressed) in [(19.1, true), (19.2, false)] {
        let output = draw(
            &mut app,
            size,
            time,
            vec![
                egui::Event::PointerMoved(position),
                pointer_event(position, pressed),
            ],
        );
        capture.observe(&output);
    }
    let output = draw(&mut app, size, 19.4, vec![]);
    capture.observe(&output);
    assert!(app.mcp_session.is_open());
    capture.save(
        &ctx,
        output,
        size,
        destination.join("mcp-session-dropup.png"),
    );

    app.mcp_session = crate::mcp_session_ui::Panel::new(false);
    egui::Popup::close_all(&ctx);
    let size = egui::vec2(940.0, 680.0);
    app.rack.set_room_brightness(0.55);
    app.status = "Room brightness stays available in the bottom toolbar".into();
    for time in [20.0, 20.1] {
        let output = draw(&mut app, size, time, vec![]);
        capture.observe(&output);
    }
    let output = draw(&mut app, size, 20.2, vec![]);
    capture.observe(&output);
    capture.save(
        &ctx,
        output,
        size,
        destination.join("footer-room-narrow.png"),
    );
}
