use super::*;

fn frame(
    ctx: &egui::Context,
    app: &mut OpenPatch,
    time: f64,
    events: Vec<egui::Event>,
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
            time: Some(time),
            focused: true,
            modifiers,
            events,
            ..Default::default()
        },
        |ctx| {
            app.global_shortcuts(ctx);
            app.rack_workspace(ctx, None);
        },
    )
}

fn tab(backwards: bool) -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Tab,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: if backwards {
            egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::NONE
        },
    }
}

fn text(output: &egui::FullOutput) -> String {
    fn collect(shape: &egui::Shape, texts: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => texts.push(text.galley.text().to_owned()),
            egui::Shape::Vec(children) => {
                for child in children {
                    collect(child, texts);
                }
            }
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for shape in &output.shapes {
        collect(&shape.shape, &mut texts);
    }
    texts.join("\n")
}

fn open_frequency(ctx: &egui::Context, app: &mut OpenPatch) {
    let mut layout = app.rack.layout();
    layout["zoom"] = json!(0.85);
    layout["pan"] = json!([12.0, 16.0]);
    layout["positions"] = json!({"tone":[0.0,0.0],"level":[400.0,0.0]});
    app.rack.restore_layout(&layout).unwrap();
    app.observe_document();
    app.rack
        .request_precision_control_for_smoke("tone", "frequency");
    // Opening uses the real rendered descriptor; the following frame gathers
    // the whole module's traversal and settles the anchored window geometry.
    frame(ctx, app, 0.0, vec![]);
    let output = frame(ctx, app, 0.1, vec![]);
    assert!(text(&output).contains("tone / frequency"));
    assert!(text(&output).contains("Apply & Next"));
    assert!(!app.browser_open);
}

#[test]
fn accepted_precision_tab_is_one_history_action_with_exact_graph_layout_selection_undo_redo() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = parameter_entry_host(dir.path());
    // A preceding grouped knob gesture must not absorb precision navigation.
    assert!(app.command(
        "set_parameter",
        json!({"id":"tone","parameter":"frequency","value":330.0,
            "history_group":"prior-knob-drag"})
    ));
    let ctx = egui::Context::default();
    open_frequency(&ctx, &mut app);
    frame(&ctx, &mut app, 0.2, vec![egui::Event::Text("440*2".into())]);
    let before = app.shared.lock().unwrap().snapshot();
    let before_patch = serde_json::to_value(&app.patch).unwrap();
    let before_state = app.state_id;
    let layout = app.rack.layout();
    let selection = app.rack.selected_ids();

    frame(&ctx, &mut app, 0.3, vec![tab(false)]);
    let after = app.shared.lock().unwrap().snapshot();
    let after_patch = serde_json::to_value(&app.patch).unwrap();
    let after_state = app.state_id;
    assert_eq!(app.status, "Value applied");
    assert_eq!(app.patch.nodes[0].params["frequency"], 880.0);
    assert_eq!(
        after["revision"].as_u64(),
        Some(before["revision"].as_u64().unwrap() + 1)
    );
    assert_eq!(
        after["history"]["undo_steps"].as_u64(),
        Some(before["history"]["undo_steps"].as_u64().unwrap() + 1)
    );
    assert_ne!(before_state, after_state);
    assert_eq!(app.rack.layout(), layout);
    assert_eq!(app.rack.selected_ids(), selection);
    let output = frame(&ctx, &mut app, 0.4, vec![]);
    assert!(
        text(&output).contains("tone / waveform"),
        "{}",
        text(&output)
    );

    app.history(false);
    assert_eq!(serde_json::to_value(&app.patch).unwrap(), before_patch);
    assert_eq!(app.state_id, before_state);
    assert_eq!(
        app.shared.lock().unwrap().snapshot()["history"]["redo_steps"],
        1
    );
    assert_eq!(app.rack.layout(), layout);
    assert_eq!(app.rack.selected_ids(), selection);
    app.history(true);
    assert_eq!(serde_json::to_value(&app.patch).unwrap(), after_patch);
    assert_eq!(app.state_id, after_state);
    assert_eq!(
        app.shared.lock().unwrap().snapshot()["history"]["redo_steps"],
        0
    );
    assert_eq!(app.rack.layout(), layout);
    assert_eq!(app.rack.selected_ids(), selection);
}

#[test]
fn rejected_precision_navigation_keeps_authoritative_history_and_visible_draft() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = parameter_entry_host(dir.path());
    let ctx = egui::Context::default();
    open_frequency(&ctx, &mut app);
    frame(&ctx, &mut app, 0.2, vec![egui::Event::Text("440*2".into())]);
    let opened_revision = app.revision;
    let layout = app.rack.layout();
    let selection = app.rack.selected_ids();
    // Model an MCP commit after the UI emitted its guarded navigation action,
    // before the host acquires the session lock.
    execute_shared(
        &app.shared,
        "set_parameter",
        json!({
            "id":"level","parameter":"value","value":2.0
        }),
    )
    .unwrap();
    let newer = app.shared.lock().unwrap().snapshot();
    app.edit_parameter_and_navigate(
        json!({
            "id":"tone","parameter":"frequency","value":880.0,
            "expected_revision":opened_revision
        }),
        true,
    );
    assert!(app.status.contains("revision conflict"), "{}", app.status);
    assert_eq!(app.shared.lock().unwrap().snapshot(), newer);
    assert_eq!(app.patch.nodes[0].params["frequency"], 220.0);
    assert_eq!(app.patch.nodes[1].params["value"], 2.0);
    assert_eq!(app.rack.layout(), layout);
    assert_eq!(app.rack.selected_ids(), selection);

    for (time, events) in [(0.3, vec![]), (0.4, vec![tab(false)])] {
        let output = frame(&ctx, &mut app, time, events);
        let visible = text(&output);
        assert!(visible.contains("tone / frequency"), "{visible}");
        assert!(
            visible.contains("440*2"),
            "the rejected draft remains visible: {visible}"
        );
        assert!(visible.contains("revision conflict"), "{visible}");
        assert!(!visible.contains("tone / waveform"));
        assert_eq!(app.shared.lock().unwrap().snapshot(), newer);
        assert_eq!(app.rack.layout(), layout);
        assert_eq!(app.rack.selected_ids(), selection);
    }
}

#[test]
fn precision_navigation_uses_session_accepted_revision_when_gui_lags_and_continues_guarded_entry() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = parameter_entry_host(dir.path());
    assert!(app.command(
        "set_parameter",
        json!({
            "id":"tone","parameter":"waveform","value":0.0
        })
    ));
    let ctx = egui::Context::default();
    open_frequency(&ctx, &mut app);
    let gui_revision = app.revision;
    for value in [2.0, 3.0] {
        execute_shared(
            &app.shared,
            "set_parameter",
            json!({
                "id":"level","parameter":"value","value":value
            }),
        )
        .unwrap();
    }
    let fresh_revision = app.shared.lock().unwrap().revision();
    assert_eq!(
        app.revision, gui_revision,
        "the GUI has not observed either commit"
    );
    assert!(fresh_revision >= gui_revision + 2);
    // Supply a freshly captured caller guard while GUI presentation still
    // lags. The host must capture the accepted session revision, not GUI + 1.
    app.edit_parameter_and_navigate(
        json!({
            "id":"tone","parameter":"frequency","value":880.0,
            "expected_revision":fresh_revision
        }),
        true,
    );
    let accepted = app.shared.lock().unwrap().snapshot();
    assert_eq!(app.revision, fresh_revision + 1);
    assert_eq!(app.patch.nodes[1].params["value"], 3.0);
    let output = frame(&ctx, &mut app, 0.2, vec![]);
    assert!(
        text(&output).contains("tone / waveform"),
        "{}",
        text(&output)
    );
    assert!(!text(&output).contains("Patch changed"));

    // Continue through a real reverse Tab after the lagging GUI catches up.
    // Explicit parameter applies are transactions even at the same value.
    frame(&ctx, &mut app, 0.3, vec![tab(true)]);
    let reverse = app.shared.lock().unwrap().snapshot();
    assert_eq!(reverse["patch"], accepted["patch"]);
    assert_eq!(
        reverse["revision"].as_u64(),
        Some(accepted["revision"].as_u64().unwrap() + 1)
    );
    assert_eq!(
        reverse["history"]["undo_steps"].as_u64(),
        Some(accepted["history"]["undo_steps"].as_u64().unwrap() + 1)
    );
    let output = frame(&ctx, &mut app, 0.4, vec![]);
    let visible = text(&output);
    assert!(visible.contains("tone / frequency"), "{visible}");
    assert!(visible.contains("Result: 880 Hz"), "{visible}");
    assert!(!visible.contains("Waiting for"));
    assert!(!visible.contains("Patch changed"));
}

#[test]
fn focused_knob_history_shortcuts_undo_release_and_keyboard_edits_and_redo() {
    for control in [false, true] {
        for same_frame_release in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = host(dir.path());
            assert!(app.command(
                "replace_patch",
                json!({"patch":{"version":2,"nodes":[
                    {"id":"tone","kind":"stock_vca","params":{"level":0.5}}
                ]}})
            ));
            let mut layout = app.rack.layout();
            layout["zoom"] = json!(1.0);
            layout["pan"] = json!([24.0, 12.0]);
            layout["positions"] = json!({"tone":[0.0,0.0]});
            app.rack.restore_layout(&layout).unwrap();
            app.rack.select_only("tone".into());
            app.observe_document();
            let ctx = egui::Context::default();
            frame(&ctx, &mut app, 0.0, vec![]);
            let at = egui::pos2(114.0, 118.0);
            let end = at - egui::vec2(0.0, 30.0);
            let value = |app: &OpenPatch| app.patch.nodes[0].params["level"];
            let pointer = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(
                &ctx,
                &mut app,
                1.0,
                vec![egui::Event::PointerMoved(at), pointer(at, true)],
            );
            frame(&ctx, &mut app, 1.1, vec![egui::Event::PointerMoved(end)]);
            let edited = value(&app);
            assert!(edited > 0.5, "real knob drag must edit");
            assert!(app.rack.parameter_keyboard_active(&ctx));
            let modifier = egui::Modifiers {
                command: !control,
                ctrl: control,
                ..Default::default()
            };
            let key = |key, modifiers| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            };
            let mut undo = vec![key(egui::Key::Z, modifier)];
            if same_frame_release {
                undo.insert(0, pointer(end, false));
            } else {
                frame(&ctx, &mut app, 1.2, vec![pointer(end, false)]);
            }
            frame(&ctx, &mut app, 1.3, undo);
            assert!(
                (value(&app) - 0.5).abs() < 1e-6,
                "release must not replay the old cached value"
            );
            let redo = egui::Modifiers {
                shift: true,
                ..modifier
            };
            frame(&ctx, &mut app, 1.4, vec![key(egui::Key::Z, redo)]);
            assert!((value(&app) - edited).abs() < 1e-6);
            assert!(app.rack.parameter_keyboard_active(&ctx));
            frame(
                &ctx,
                &mut app,
                1.5,
                vec![key(egui::Key::ArrowUp, egui::Modifiers::NONE)],
            );
            assert!(
                value(&app) > edited,
                "arrow editing stays available after redo"
            );
            frame(&ctx, &mut app, 1.6, vec![key(egui::Key::Z, modifier)]);
            assert!(
                (value(&app) - edited).abs() < 1e-6,
                "immediate undo of arrow edit"
            );
        }
    }
}

#[test]
fn real_text_editor_retains_patch_history_when_handling_undo() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    assert!(app.command(
        "set_parameter",
        json!({"id":"tone","parameter":"frequency","value":330.0})
    ));
    let before = app.shared.lock().unwrap().snapshot();
    let ctx = egui::Context::default();
    let mut text = String::from("Patch name");
    for time in [0.0, 0.1] {
        let _ = ctx.run(
            egui::RawInput {
                time: Some(time),
                focused: true,
                modifiers: egui::Modifiers::COMMAND,
                events: vec![egui::Event::Key {
                    key: egui::Key::Z,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.text_edit_singleline(&mut text).request_focus();
                });
                app.global_shortcuts(ctx);
            },
        );
    }
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
}
