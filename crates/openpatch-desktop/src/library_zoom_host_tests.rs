use super::*;

fn settings(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path.join(".host-settings.json")).unwrap()).unwrap()
}
fn frame(
    ctx: &egui::Context,
    app: &mut OpenPatch,
    time: f64,
    events: Vec<egui::Event>,
    drain: bool,
) -> egui::FullOutput {
    let modifiers = events
        .iter()
        .rev()
        .find_map(|event| match event {
            egui::Event::Key { modifiers, .. } => Some(*modifiers),
            _ => None,
        })
        .unwrap_or_default();
    ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100.0, 800.0),
            )),
            focused: true,
            time: Some(time),
            modifiers,
            events,
            ..Default::default()
        },
        |ctx| {
            let action = if drain {
                app.browser(ctx)
            } else {
                // Exercise genuine UI changes with publication deferred until the
                // next App pass, so both independent change flags can be pending.
                app.library.show(
                    ctx,
                    &mut app.browser_open,
                    &app.catalog,
                    app.patch.version,
                    true,
                )
            };
            app.global_shortcuts(ctx);
            app.rack_workspace(ctx, action);
        },
    )
}
fn text(output: &egui::FullOutput, label: &str, last: bool) -> egui::Pos2 {
    fn collect(shape: &egui::Shape, label: &str, points: &mut Vec<egui::Pos2>) {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == label => {
                points.push(t.pos + t.galley.size() * 0.5)
            }
            egui::Shape::Vec(children) => {
                for child in children {
                    collect(child, label, points);
                }
            }
            _ => {}
        }
    }
    let mut points = Vec::new();
    for shape in &output.shapes {
        collect(&shape.shape, label, &mut points);
    }
    if last { points.last() } else { points.first() }
        .copied()
        .unwrap_or_else(|| panic!("missing actual Library label {label}"))
}
fn click(
    ctx: &egui::Context,
    app: &mut OpenPatch,
    at: egui::Pos2,
    time: f64,
    drain: bool,
) -> egui::FullOutput {
    frame(
        ctx,
        app,
        time,
        vec![egui::Event::PointerMoved(at), pointer_event(at, true)],
        drain,
    );
    frame(ctx, app, time + 0.05, vec![pointer_event(at, false)], drain)
}
fn key(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    }
}

#[test]
fn library_zoom_restores_clamped_finite_settings_and_roundtrips_without_document_edits() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let snapshot = app.shared.lock().unwrap().snapshot();
    let document = app.capture_document();
    let layout_revision = app.rack.layout_revision();
    let dirty = app.dirty();
    assert_eq!(app.library.preview_zoom(), 0.5);
    for (preferences, expected) in [
        (Value::Null, 0.5),
        (json!({}), 0.5),
        (json!({"library_zoom":"large"}), 0.5),
        (json!({"library_zoom":true}), 0.5),
        (json!({"library_zoom":null}), 0.5),
        (json!({"library_zoom":-1e200}), 0.25),
        (json!({"library_zoom":1e200}), 2.0),
        (json!({"library_zoom":0.75}), 0.75),
    ] {
        app.apply_host_preferences(&preferences);
        assert_eq!(app.library.preview_zoom(), expected);
        assert!(
            !app.library.take_preview_zoom_changed(),
            "restore manufactured a user edit"
        );
    }
    app.output_device = Some("chosen output".into());
    app.midi_device = Some("chosen input".into());
    app.library.set_preview_zoom(1.25);
    app.save_host_settings();
    let saved = settings(dir.path());
    assert_eq!(saved["library_zoom"], 1.25);
    assert_eq!(saved["output_device"], "chosen output");
    assert_eq!(saved["midi_device"], "chosen input");
    let mut reopened = host(dir.path());
    reopened.apply_host_preferences(&saved);
    assert_eq!(reopened.library.preview_zoom(), 1.25);
    assert!(!reopened.library.take_preview_zoom_changed());
    assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
    assert!(app.capture_document().equivalent(&document));
    assert_eq!(app.rack.layout_revision(), layout_revision);
    assert_eq!(app.dirty(), dirty);
    assert!(app.audio.is_none());
}

#[test]
fn actual_favorite_and_zoom_edits_publish_once_and_both_pending_flags_are_drained() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    app.browse();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, 0.0, vec![], false);
    let output = frame(&ctx, &mut app, 0.1, vec![], false);
    let snapshot = app.shared.lock().unwrap().snapshot();
    let document = app.capture_document();
    let dirty = app.dirty();
    // Leave a real favorite edit pending, then a real ComboBox selection. The
    // App must save one combined snapshot and consume both flags this pass.
    click(&ctx, &mut app, text(&output, "☆", false), 1.0, false);
    assert_eq!(app.library.favorites().len(), 1);
    let output = frame(&ctx, &mut app, 1.2, vec![], false);
    click(&ctx, &mut app, text(&output, "50%", false), 2.0, false);
    let popup = frame(&ctx, &mut app, 2.1, vec![], false);
    click(&ctx, &mut app, text(&popup, "100%", true), 3.0, false);
    assert_eq!(app.library.preview_zoom(), 1.0);
    assert!(
        !dir.path().join(".host-settings.json").exists(),
        "deferred Library UI unexpectedly saved settings"
    );
    frame(&ctx, &mut app, 3.2, vec![], true);
    let mut saved = settings(dir.path());
    assert_eq!(saved["library_zoom"], 1.0);
    assert_eq!(saved["module_favorites"], json!(app.library.favorites()));
    saved["idle-write-sentinel"] = json!(true);
    std::fs::write(
        dir.path().join(".host-settings.json"),
        serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
    frame(&ctx, &mut app, 3.3, vec![], true);
    assert_eq!(
        settings(dir.path())["idle-write-sentinel"],
        true,
        "a second flag survived the first App publication"
    );
    assert!(!app.library.take_favorites_changed());
    assert!(!app.library.take_preview_zoom_changed());
    assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
    assert!(app.capture_document().equivalent(&document));
    assert_eq!(app.dirty(), dirty);
}

#[test]
fn programmatic_zoom_and_browser_reopen_do_not_write_host_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let ctx = egui::Context::default();
    app.library.set_preview_zoom(2.0);
    app.browse();
    frame(&ctx, &mut app, 0.0, vec![], true);
    app.browser_open = false;
    frame(&ctx, &mut app, 0.1, vec![], true);
    app.browse();
    frame(&ctx, &mut app, 0.2, vec![], true);
    assert_eq!(app.library.preview_zoom(), 2.0);
    assert!(!dir.path().join(".host-settings.json").exists());
}

#[test]
fn library_zoom_survives_new_template_example_open_import_revert_recovery_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let expected = Document::new(openpatch_core::demo_patch(), Rack::default().layout()).unwrap();
    {
        let store = Store::new(dir.path()).unwrap();
        store.save_recovery(&expected).unwrap();
        store.save_new("saved.openpatch", &expected).unwrap();
        store
            .save_template(&crate::startup::factory_document(), None)
            .unwrap();
    }
    std::fs::write(
        dir.path().join("legacy.json"),
        serde_json::to_vec(&expected.patch()).unwrap(),
    )
    .unwrap();
    let mut app = host(dir.path());
    app.library.set_preview_zoom(1.0);
    let recovery = app.recovery_names[0].clone();
    app.load_document(&recovery, true);
    assert_eq!(app.library.preview_zoom(), 1.0);
    app.new_from_template(false);
    assert_eq!(app.library.preview_zoom(), 1.0);
    app.replace_now(Patch::new_v2(), "Untitled");
    assert_eq!(app.library.preview_zoom(), 1.0);
    app.replace_now(openpatch_core::demo_patch(), "Factory example");
    assert_eq!(app.library.preview_zoom(), 1.0);
    app.load_document("saved.openpatch", false);
    assert_eq!(app.library.preview_zoom(), 1.0);
    edit(&mut app, 330.0);
    app.load_document("saved.openpatch", false);
    assert_eq!(app.library.preview_zoom(), 1.0);
    assert!(!app.dirty());
    app.load_document("legacy.json", false);
    assert_eq!(app.library.preview_zoom(), 1.0);
    assert!(app.active_file.is_none());
    edit(&mut app, 440.0);
    app.history(false);
    assert_eq!(app.library.preview_zoom(), 1.0);
    app.history(true);
    assert_eq!(app.library.preview_zoom(), 1.0);
    app.rack.reset_view();
    assert_eq!(app.library.preview_zoom(), 1.0);
    assert!(!app.library.take_preview_zoom_changed());
}

#[test]
fn actual_zoom_selector_owns_keys_and_a_rack_click_restores_rack_shortcuts() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    assert!(app.command(
        "replace_patch",
        json!({"patch":{"version":2,"nodes":[{"id":"a","kind":"constant"}]}})
    ));
    app.rack
        .restore_layout(&json!({"version":1,"zoom":1.0,"pan":[24,12],"positions":{"a":[0,0]}}))
        .unwrap();
    app.browse();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, 0.0, vec![], true);
    let output = frame(&ctx, &mut app, 0.1, vec![], true);
    click(&ctx, &mut app, text(&output, "50%", false), 1.0, true);
    let popup = frame(&ctx, &mut app, 1.1, vec![], true);
    click(&ctx, &mut app, text(&popup, "100%", true), 1.2, true);
    frame(&ctx, &mut app, 1.4, vec![], true);
    assert_eq!(app.library.preview_zoom(), 1.0);
    assert!(ctx.memory(|memory| memory.focused()).is_some());
    assert!(app.library.keyboard_active());
    let snapshot = app.shared.lock().unwrap().snapshot();
    let layout = app.rack.layout();
    let status = app.status.clone();
    for (index, (button, modifiers)) in [
        (egui::Key::Enter, egui::Modifiers::NONE),
        (egui::Key::Space, egui::Modifiers::NONE),
        (egui::Key::ArrowUp, egui::Modifiers::NONE),
        (egui::Key::ArrowDown, egui::Modifiers::NONE),
        (egui::Key::ArrowLeft, egui::Modifiers::NONE),
        (egui::Key::ArrowRight, egui::Modifiers::NONE),
        (egui::Key::Equals, egui::Modifiers::COMMAND),
        (egui::Key::Minus, egui::Modifiers::COMMAND),
    ]
    .into_iter()
    .enumerate()
    {
        frame(
            &ctx,
            &mut app,
            2.0 + index as f64,
            vec![key(button, true, modifiers)],
            true,
        );
        frame(
            &ctx,
            &mut app,
            2.1 + index as f64,
            vec![key(button, false, modifiers)],
            true,
        );
        assert_eq!(
            app.shared.lock().unwrap().snapshot(),
            snapshot,
            "{button:?} leaked patch action"
        );
        assert_eq!(
            app.rack.layout(),
            layout,
            "{button:?} leaked rack navigation"
        );
        assert_eq!(app.status, status);
        assert!(app.browser_open);
    }
    click(&ctx, &mut app, egui::pos2(84.0, 44.0), 11.0, true);
    assert!(!app.library.keyboard_active());
    frame(
        &ctx,
        &mut app,
        12.0,
        vec![key(egui::Key::Equals, true, egui::Modifiers::COMMAND)],
        true,
    );
    frame(
        &ctx,
        &mut app,
        12.1,
        vec![key(egui::Key::Equals, false, egui::Modifiers::COMMAND)],
        true,
    );
    assert!(app.rack.zoom() > 1.0);
    frame(
        &ctx,
        &mut app,
        13.0,
        vec![key(egui::Key::D, true, egui::Modifiers::COMMAND)],
        true,
    );
    assert_eq!(
        app.patch.nodes.len(),
        2,
        "open Library blocked rack duplication after actual rack click"
    );
}

#[test]
fn appended_zoom_and_small_label_scenes_use_actual_panels_and_request_five_new_receipts() {
    use eframe::App;
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    app.smoke_test = Some((dir.path().join("unused.png"), 1970, 81));
    let ctx = egui::Context::default();
    app.smoke(&ctx);
    assert_eq!(app.library.preview_zoom(), 1.0);
    assert!(app.browser_open);
    // Exercise the production raw-input hook rather than replacing search or
    // feeding an independent thumbnail fixture.
    let mut input = egui::RawInput::default();
    app.raw_input_hook(&ctx, &mut input);
    assert!(input
        .events
        .iter()
        .any(|event| matches!(event,egui::Event::Text(text) if text=="analog_vco")));
    frame(&ctx, &mut app, 0.0, input.events, true);
    let narrow = frame(&ctx, &mut app, 0.1, vec![], true);
    text(&narrow, "VCO", false);
    app.smoke_test.as_mut().unwrap().1 = 1986;
    app.smoke(&ctx);
    assert_eq!(app.library.preview_zoom(), 1.0);
    let mut input = egui::RawInput::default();
    app.raw_input_hook(&ctx, &mut input);
    assert!(input
        .events
        .iter()
        .any(|event| matches!(event,egui::Event::Text(text) if text=="seq3")));
    frame(&ctx, &mut app, 0.2, input.events, true);
    let wide = frame(&ctx, &mut app, 0.3, vec![], true);
    text(&wide, "SEQ 3", false);
    app.smoke_test.as_mut().unwrap().1 = 2002;
    app.smoke(&ctx);
    assert_eq!(app.library.preview_zoom(), 0.25);
    app.smoke_test.as_mut().unwrap().1 = 2018;
    app.smoke(&ctx);
    assert_eq!(app.library.preview_zoom(), 2.0);
    let mut input = egui::RawInput::default();
    app.raw_input_hook(&ctx, &mut input);
    assert!(input.events.iter().any(|event| matches!(event,
        egui::Event::Text(text) if text == "seq3")));
    frame(&ctx, &mut app, 0.4, input.events, true);
    let maximum = frame(&ctx, &mut app, 0.5, vec![], true);
    text(&maximum, "SEQ 3", false);
    assert!(app.title.contains("200%"));
    app.smoke_test.as_mut().unwrap().1 = 2034;
    app.smoke(&ctx);
    assert_eq!(app.rack.zoom(), 0.5);
    assert_eq!(app.rack.room_brightness(), 1.0);
    assert!(!app.browser_open);
    assert_eq!(
        app.patch
            .nodes
            .iter()
            .map(|node| node.kind.as_str())
            .collect::<Vec<_>>(),
        ["multi_vco", "multi_lfo", "mix6", "audio_stereo"]
    );
    let small_labels = frame(&ctx, &mut app, 0.6, vec![], true);
    for label in ["Linear FM", "Soft sync", "Average", "Invert"] {
        text(&small_labels, label, false);
    }
    for logical in [1978, 1994, 2010, 2026, 2042] {
        app.smoke_test.as_mut().unwrap().1 = logical;
        let output = ctx.run(egui::RawInput::default(), |ctx| app.smoke(ctx));
        assert!(output.viewport_output.values().any(|viewport| viewport
            .commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::Screenshot(_)))));
    }
    assert_eq!(
        app.smoke_test.as_ref().unwrap().2,
        81,
        "a request was mistaken for an actual screenshot receipt"
    );
    assert!(app.audio.is_none());
}

#[test]
fn modal_disables_an_already_open_zoom_popup_without_edits_or_host_publication() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    app.browse();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, 0.0, vec![], true);
    let output = frame(&ctx, &mut app, 0.1, vec![], true);
    let selector = text(&output, "50%", false);
    click(&ctx, &mut app, selector, 1.0, true);
    let popup = frame(&ctx, &mut app, 1.1, vec![], true);
    assert!(egui::Popup::is_any_open(&ctx));
    let hundred = text(&popup, "100%", true);
    let snapshot = app.shared.lock().unwrap().snapshot();
    let document = app.capture_document();
    let dirty = app.dirty();
    let favorites = app.library.favorites();
    assert!(!dir.path().join(".host-settings.json").exists());

    // Change the real App eligibility predicate after the popup is already
    // shown. Its old option position must not remain an active control in a
    // separate popup layer while the owning Library is disabled.
    app.settings_open = true;
    assert!(!app.library_enabled());
    frame(&ctx, &mut app, 1.2, vec![], true);
    click(&ctx, &mut app, hundred, 2.0, true);
    frame(
        &ctx,
        &mut app,
        3.0,
        vec![key(egui::Key::Enter, true, egui::Modifiers::NONE)],
        true,
    );
    frame(
        &ctx,
        &mut app,
        3.1,
        vec![key(egui::Key::Enter, false, egui::Modifiers::NONE)],
        true,
    );
    assert_eq!(app.library.preview_zoom(), 0.5);
    assert_eq!(app.library.favorites(), favorites);
    assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
    assert!(app.capture_document().equivalent(&document));
    assert_eq!(app.dirty(), dirty);
    assert!(app.browser_open);
    assert!(
        !dir.path().join(".host-settings.json").exists(),
        "a disabled popup published host preferences"
    );
    assert!(!app.library.take_preview_zoom_changed());
    assert!(!app.library.take_favorites_changed());

    app.settings_open = false;
    let output = frame(&ctx, &mut app, 4.0, vec![], true);
    // If a popup remains visible, close it through its actual button before
    // reopening. Both close-on-disable and inert-open policies are acceptable.
    if egui::Popup::is_any_open(&ctx) {
        click(&ctx, &mut app, text(&output, "50%", false), 4.1, true);
    }
    let output = frame(&ctx, &mut app, 4.2, vec![], true);
    click(&ctx, &mut app, text(&output, "50%", false), 4.3, true);
    let popup = frame(&ctx, &mut app, 4.4, vec![], true);
    click(&ctx, &mut app, text(&popup, "100%", true), 4.5, true);
    assert_eq!(app.library.preview_zoom(), 1.0);
    assert_eq!(settings(dir.path())["library_zoom"], 1.0);
    assert_eq!(app.library.favorites(), favorites);
    assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
    assert!(app.capture_document().equivalent(&document));
    assert_eq!(app.dirty(), dirty);
    assert!(app.browser_open);
    assert!(app.audio.is_none());
}
