use super::*;

fn palette_host(path: &Path) -> OpenPatch {
    let mut app = host(path);
    assert!(app.command(
        "replace_patch",
        json!({"patch":{"version":2,"nodes":[
            {"id":"source","kind":"constant","params":{"value":1.0}},
            {"id":"sink","kind":"gain"}, {"id":"other","kind":"gain"}
        ]}})
    ));
    app.rack.select_only("source".into());
    app.observe_document();
    app
}

fn arguments(app: &OpenPatch, target: &str, color: [u8; 3]) -> Value {
    json!({"from":"source","output":"out","to":target,"input":"signal",
        "color":color,"expected_revision":app.revision})
}

fn cycle(ctx: &egui::Context, app: &mut OpenPatch, forward: bool) {
    let key = if forward {
        egui::Key::CloseBracket
    } else {
        egui::Key::OpenBracket
    };
    let _ = ctx.run(
        egui::RawInput {
            focused: true,
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ctx| app.global_shortcuts(ctx),
    );
}

#[test]
fn palette_host_success_advances_once_persists_cursor_and_undo_redo_keep_rgb() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = palette_host(directory.path());
    app.cable_palette.select_color(2);
    let captured = app.cable_palette.selection();
    let before = app.shared.lock().unwrap().snapshot();
    let args = arguments(&app, "sink", captured.color);
    app.connect_from_palette(args, captured);
    assert_eq!(app.patch.cables.len(), 1);
    assert_eq!(app.patch.cables[0].color, Some(captured.color));
    assert_eq!(app.cable_palette.next_index(), 3);
    let after = app.shared.lock().unwrap().snapshot();
    assert_eq!(
        after["revision"].as_u64().unwrap(),
        before["revision"].as_u64().unwrap() + 1
    );
    assert_eq!(
        after["history"]["undo_steps"].as_u64().unwrap(),
        before["history"]["undo_steps"].as_u64().unwrap() + 1
    );
    let preferences: Value = serde_json::from_slice(
        &std::fs::read(directory.path().join(".host-settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(preferences["cable_palette"], app.cable_palette.settings());
    let choice = app.cable_palette.selection();
    app.history(false);
    assert_eq!(serde_json::to_value(&app.patch).unwrap(), before["patch"]);
    assert_eq!(app.cable_palette.selection(), choice);
    app.history(true);
    assert_eq!(serde_json::to_value(&app.patch).unwrap(), after["patch"]);
    assert_eq!(app.cable_palette.selection(), choice);
}

#[test]
fn palette_host_stale_and_invalid_connects_preserve_new_choice_and_history() {
    for stale in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = palette_host(directory.path());
        let captured = app.cable_palette.selection();
        let mut args = arguments(&app, "sink", captured.color);
        if stale {
            execute_shared(
                &app.shared,
                "set_parameter",
                json!({"id":"source","parameter":"value","value":2.0}),
            )
            .unwrap();
        } else {
            args["to"] = json!("missing-module");
        }
        let before = app.shared.lock().unwrap().snapshot();
        let palette = app.cable_palette.clone();
        app.connect_from_palette(args, captured);
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
        assert_eq!(app.cable_palette, palette);
        assert!(app.patch.cables.is_empty());
        assert!(!directory.path().join(".host-settings.json").exists());
        assert_ne!(app.status, "Cable connected");
    }
}

#[test]
fn palette_host_accepted_old_ticket_cannot_advance_new_selection_even_with_same_rgb() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = palette_host(directory.path());
    let captured = app.cable_palette.selection();
    let args = arguments(&app, "sink", captured.color);
    app.cable_palette.cycle(true);
    app.cable_palette.cycle(false);
    let fresh = app.cable_palette.selection();
    assert_eq!(captured.color, fresh.color);
    assert_ne!(captured.generation, fresh.generation);
    app.connect_from_palette(args, captured);
    assert_eq!(app.patch.cables[0].color, Some(captured.color));
    assert_eq!(app.cable_palette.selection(), fresh);
}

#[test]
fn palette_host_shortcuts_and_preferences_do_not_change_document_history_layout_or_selection() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = palette_host(directory.path());
    app.save_document("base.openpatch", true);
    assert!(!app.dirty());
    let before = app.shared.lock().unwrap().snapshot();
    let layout = app.rack.layout();
    let selected = app.rack.selected_ids();
    let state = app.state_id;
    let document = app.capture_document();
    let ctx = egui::Context::default();
    cycle(&ctx, &mut app, true);
    assert_eq!(app.cable_palette.next_index(), 1);
    cycle(&ctx, &mut app, false);
    assert_eq!(app.cable_palette.next_index(), 0);
    app.cable_palette.select_color(4);
    app.cable_palette.set_auto_advance(false);
    app.save_host_settings();
    let preferences: Value = serde_json::from_slice(
        &std::fs::read(directory.path().join(".host-settings.json")).unwrap(),
    )
    .unwrap();
    let mut restored = palette_host(directory.path());
    restored.apply_host_preferences(&preferences);
    assert_eq!(
        restored.cable_palette.settings(),
        app.cable_palette.settings()
    );
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    assert_eq!(app.rack.layout(), layout);
    assert_eq!(app.rack.selected_ids(), selected);
    assert_eq!(app.state_id, state);
    assert!(app.capture_document().equivalent(&document));
    assert!(!app.dirty());
}

#[test]
fn palette_host_save_load_recovery_and_rack_replacement_keep_explicit_route_rgb() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = palette_host(directory.path());
    let captured = app.cable_palette.selection();
    app.connect_from_palette(arguments(&app, "sink", captured.color), captured);
    app.save_document("colored.openpatch", true);
    assert!(!app.dirty());
    app.cable_palette.select_preset(1);
    app.cable_palette.select_color(3);
    let preferences = app.cable_palette.settings();
    let document = app.capture_document();
    app.projects
        .as_ref()
        .unwrap()
        .save_recovery(&document)
        .unwrap();
    // Recovery candidates become visible when the writing session releases its lock.
    drop(app.projects.take());
    app.projects = Some(Store::new(directory.path()).unwrap());
    let recovery = app
        .projects
        .as_ref()
        .unwrap()
        .recoveries()
        .into_iter()
        .next()
        .unwrap();
    app.load_document(&recovery, true);
    assert_eq!(app.patch.cables[0].color, Some(captured.color));
    assert_eq!(app.cable_palette.settings(), preferences);
    app.load_document("colored.openpatch", false);
    assert_eq!(app.patch.cables[0].color, Some(captured.color));
    assert_eq!(app.cable_palette.settings(), preferences);
    assert!(!app.dirty());
    // An explicit command can use the very same RGB without consuming palette intent.
    let selected = app.cable_palette.selection();
    assert!(app.command("connect", arguments(&app, "other", selected.color)));
    assert_eq!(app.cable_palette.selection(), selected);
    assert_eq!(app.patch.cables[0].color, Some(captured.color));
}

#[test]
fn palette_host_real_pointer_release_dispatches_palette_commit_and_honors_auto_advance() {
    for advance in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = palette_host(directory.path());
        app.cable_palette.set_auto_advance(advance);
        app.cable_palette.select_color(2);
        let captured = app.cable_palette.selection();
        let ctx = egui::Context::default();
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 800.0));
        let render = |app: &mut OpenPatch, time, events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(canvas),
                    time: Some(time),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    app.global_shortcuts(ctx);
                    app.rack_workspace(ctx, None);
                },
            );
        };
        render(&mut app, 0.0, vec![]);
        let from = app
            .rack
            .port_center_for_smoke(canvas, &app.patch, &app.catalog, "source", "out", false)
            .unwrap();
        let to = app
            .rack
            .port_center_for_smoke(canvas, &app.patch, &app.catalog, "sink", "signal", true)
            .unwrap();
        let before = app.shared.lock().unwrap().snapshot();
        render(
            &mut app,
            1.0,
            vec![egui::Event::PointerMoved(from), pointer_event(from, true)],
        );
        render(&mut app, 1.1, vec![egui::Event::PointerMoved(to)]);
        // Brackets during a cable gesture must leave the captured intent intact.
        render(
            &mut app,
            1.15,
            vec![egui::Event::Key {
                key: egui::Key::CloseBracket,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(app.cable_palette.selection(), captured);
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
        render(&mut app, 1.2, vec![pointer_event(to, false)]);
        assert_eq!(app.patch.cables.len(), 1);
        assert_eq!(app.patch.cables[0].color, Some(captured.color));
        assert_eq!(app.cable_palette.next_index(), if advance { 3 } else { 2 });
        let after = app.shared.lock().unwrap().snapshot();
        assert_eq!(
            after["history"]["undo_steps"].as_u64().unwrap(),
            before["history"]["undo_steps"].as_u64().unwrap() + 1
        );
        assert_eq!(app.status, "Cable connected");
    }
}

#[test]
fn palette_host_brackets_respect_modal_text_and_library_keyboard_ownership() {
    for blocked in [
        "settings",
        "modal",
        "text",
        "library_search",
        "library_card",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = palette_host(directory.path());
        let ctx = egui::Context::default();
        match blocked {
            "settings" => app.settings_open = true,
            "modal" => app.file_mode = Some(FileMode::Save),
            "text" => {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.text_edit_singleline(&mut String::new()).request_focus();
                    });
                });
                assert!(ctx.wants_keyboard_input());
            }
            _ => {
                app.browse();
                library_frame(&mut app, &ctx, 0.0, vec![], true);
                if blocked == "library_card" {
                    library_frame(
                        &mut app,
                        &ctx,
                        0.1,
                        vec![egui::Event::Key {
                            key: egui::Key::Tab,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        }],
                        true,
                    );
                    assert_ne!(
                        ctx.memory(|memory| memory.focused()),
                        Some(egui::Id::new("module-library-search"))
                    );
                }
                assert!(app.library.keyboard_active());
            }
        }
        let palette = app.cable_palette.clone();
        let before = app.shared.lock().unwrap().snapshot();
        let layout = app.rack.layout();
        let selected = app.rack.selected_ids();
        cycle(&ctx, &mut app, true);
        assert_eq!(app.cable_palette, palette, "{blocked}");
        assert_eq!(app.shared.lock().unwrap().snapshot(), before, "{blocked}");
        assert_eq!(app.rack.layout(), layout, "{blocked}");
        assert_eq!(app.rack.selected_ids(), selected, "{blocked}");
        assert!(
            !directory.path().join(".host-settings.json").exists(),
            "{blocked}"
        );
    }
}

#[test]
fn palette_host_editor_background_and_button_focus_block_global_rack_and_performance_keys() {
    fn text_position(output: &egui::FullOutput, needle: &str) -> egui::Pos2 {
        fn find(shape: &egui::Shape, needle: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == needle => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(children) => children.iter().find_map(|shape| find(shape, needle)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape, needle))
            .unwrap_or_else(|| panic!("missing editor text {needle}"))
    }
    for button_focus in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut app = palette_host(directory.path());
        app.keyboard_open = true;
        app.cable_palette_ui.open();
        let ctx = egui::Context::default();
        let render = |app: &mut OpenPatch, time, events: Vec<egui::Event>| {
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
                    time: Some(time),
                    focused: true,
                    events,
                    modifiers,
                    ..Default::default()
                },
                |ctx| {
                    // The palette resolves current-frame focus before host shortcut dispatch.
                    app.cable_palette_ui.show(ctx, &mut app.cable_palette, true);
                    app.global_shortcuts(ctx);
                    app.rack_workspace(ctx, None);
                },
            )
        };
        render(&mut app, 0.0, vec![]);
        let output = render(&mut app, 0.1, vec![]);
        let point = text_position(
            &output,
            if button_focus {
                "Add color"
            } else {
                "Make every connection yours."
            },
        );
        for (time, pressed) in [(0.2, true), (0.3, false)] {
            render(
                &mut app,
                time,
                vec![
                    egui::Event::PointerMoved(point),
                    pointer_event(point, pressed),
                ],
            );
        }
        assert!(app.cable_palette_ui.keyboard_active());
        assert!(!app.performance_keyboard_enabled(&ctx, true));
        assert!(!app.performance_keyboard_enabled(&ctx, false));
        let before = app.shared.lock().unwrap().snapshot();
        let layout = app.rack.layout();
        let selected = app.rack.selected_ids();
        let palette = app.cable_palette.clone();
        for (index, (key, modifiers)) in [
            (egui::Key::Space, egui::Modifiers::NONE),
            (egui::Key::Enter, egui::Modifiers::NONE),
            (egui::Key::Delete, egui::Modifiers::NONE),
            (egui::Key::I, egui::Modifiers::COMMAND),
        ]
        .into_iter()
        .enumerate()
        {
            render(
                &mut app,
                0.4 + index as f64 * 0.1,
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
            );
            assert!(!app.browser_open, "{button_focus}: {key:?}");
            assert_eq!(
                app.shared.lock().unwrap().snapshot(),
                before,
                "{button_focus}: {key:?}"
            );
            assert_eq!(app.rack.layout(), layout);
            assert_eq!(app.rack.selected_ids(), selected);
            assert_eq!(app.cable_palette, palette);
            assert!(!app.performance_keyboard_enabled(&ctx, true));
        }
        let outside = egui::pos2(1050.0, 760.0);
        for (time, pressed) in [(1.0, true), (1.1, false)] {
            render(
                &mut app,
                time,
                vec![
                    egui::Event::PointerMoved(outside),
                    pointer_event(outside, pressed),
                ],
            );
        }
        assert!(!app.cable_palette_ui.keyboard_active());
        assert!(app.performance_keyboard_enabled(&ctx, true));
        render(
            &mut app,
            1.2,
            vec![egui::Event::Key {
                key: egui::Key::CloseBracket,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(app.cable_palette.next_index(), palette.next_index() + 1);
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    }
}
