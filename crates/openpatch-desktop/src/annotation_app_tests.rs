use super::*;
use crate::rack::BlankResizeProposal;

const NOTES: &str = "Signal flow\n\t日本語 🎛️ café\nLine three";

fn annotation_host(path: &Path) -> OpenPatch {
    let mut app = host(path);
    assert!(app.command(
        "replace_patch",
        json!({"patch":{"version":2,"nodes":[
            {"id":"notes","kind":"notes","content":{"type":"notes","text":NOTES}},
            {"id":"blank","kind":"blank","content":{"type":"blank","width_hp":10}},
            {"id":"obstacle","kind":"constant"}
        ]}})
    ));
    app.rack
        .restore_layout(&json!({"version":1,"zoom":1.0,"pan":[24,12],
        "positions":{"notes":[0,420],"blank":[300,0],"obstacle":[900,0]}}))
        .unwrap();
    app.capture_document();
    app
}

fn width(app: &OpenPatch) -> u16 {
    app.patch
        .nodes
        .iter()
        .find(|node| node.id == "blank")
        .unwrap()
        .effective_blank_width_hp()
        .unwrap()
}

fn proposal(app: &OpenPatch, width_hp: u16, left: bool) -> BlankResizeProposal {
    let before_hp = width(app);
    let delta = if left {
        f32::from(before_hp) * 15.0 - f32::from(width_hp) * 15.0
    } else {
        0.0
    };
    BlankResizeProposal {
        id: "blank".into(),
        before_hp,
        width_hp,
        movement: movement(app, &["blank".into()], egui::vec2(delta, 0.0)),
    }
}

fn set_width(app: &OpenPatch, hp: u16) -> Value {
    execute_shared(
        &app.shared,
        "set_module_content",
        json!({"id":"blank",
        "content":{"type":"blank","width_hp":hp}}),
    )
    .unwrap()
}

#[test]
fn both_resize_edges_commit_width_and_position_in_one_history_step_with_project_round_trip() {
    for left in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = annotation_host(dir.path());
        app.save_document("before.openpatch", true);
        let before = app.capture_document();
        let history = app.shared.lock().unwrap().snapshot()["history"]["undo_steps"]
            .as_u64()
            .unwrap();
        let resize = proposal(&app, 20, left);
        assert!(app.resize_blank(resize));
        let after = app.capture_document();
        assert_eq!(width(&app), 20);
        assert_eq!(
            after.layout["positions"]["blank"],
            json!([if left { 150.0 } else { 300.0 }, 0.0])
        );
        assert_eq!(
            after.layout["positions"]["notes"],
            before.layout["positions"]["notes"]
        );
        assert_eq!(
            app.shared.lock().unwrap().snapshot()["history"]["undo_steps"],
            history + 1
        );
        assert!(app.dirty());
        app.history(false);
        assert!(app.capture_document().equivalent(&before));
        assert!(!app.dirty());
        app.history(true);
        assert!(app.capture_document().equivalent(&after));
        app.save_document("resized.openpatch", true);
        let loaded = app
            .projects
            .as_ref()
            .unwrap()
            .load("resized.openpatch")
            .unwrap();
        assert!(loaded.document.equivalent(&after));
        assert!(!app.dirty());
    }
}

#[test]
fn unseen_mcp_width_revisions_restore_saved_widths_and_known_geometry_through_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = annotation_host(dir.path());
    let before = app.capture_document();
    set_width(&app, 14);
    set_width(&app, 24);
    // No sync, rack render, or document observation happened between these commits.
    assert_eq!(width(&app), 10);
    app.sync();
    assert_eq!(width(&app), 24);
    assert_eq!(app.rack.layout(), before.layout);
    app.history(false);
    assert_eq!(width(&app), 14);
    assert_eq!(app.rack.layout(), before.layout);
    app.history(false);
    assert!(app.capture_document().equivalent(&before));
    for hp in [14, 24] {
        app.history(true);
        assert_eq!(width(&app), hp);
        assert_eq!(app.rack.layout(), before.layout);
    }
    let resize = proposal(&app, 32, true);
    assert!(app.resize_blank(resize));
    let resized = app.capture_document();
    assert_eq!(resized.layout["positions"]["blank"], json!([180.0, 0.0]));
    app.history(false);
    assert_eq!(width(&app), 24);
    assert_eq!(app.rack.layout(), before.layout);
    app.history(false);
    assert_eq!(width(&app), 14);
    assert_eq!(app.rack.layout(), before.layout);
    app.history(true);
    app.history(true);
    assert!(app.capture_document().equivalent(&resized));
}

#[test]
fn exact_resize_receipt_keeps_its_geometry_when_a_newer_mcp_width_commit_holds_the_mutex() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = annotation_host(dir.path());
    let before = app.capture_document();
    let base_state = app.state_id;
    let resize = proposal(&app, 20, true);
    let receipt = set_width(&app, 20);
    let committed_state = receipt["state_id"].as_u64().unwrap();
    let shared = app.shared.clone();
    let mut live = shared.lock().unwrap();
    live.execute(
        "set_module_content",
        json!({"id":"blank","content":{"type":"blank","width_hp":30}}),
    )
    .unwrap();
    let later_state = live.state_id();
    app.install_move_receipt(
        &receipt,
        &resize.movement,
        base_state,
        before.layout.clone(),
    )
    .unwrap();
    let resized = app.capture_document();
    assert_eq!(width(&app), 20);
    assert_eq!(resized.layout["positions"]["blank"], json!([150.0, 0.0]));
    assert_eq!(app.history_layouts[&committed_state], resized.layout);
    assert_eq!(app.history_layouts[&base_state], before.layout);
    assert!(!app.history_layouts.contains_key(&later_state));
    app.sync();
    assert_eq!(app.state_id, committed_state);
    drop(live);
    app.sync();
    assert_eq!(width(&app), 30);
    assert_eq!(app.state_id, later_state);
    assert_eq!(app.rack.layout(), resized.layout);
    app.history(false);
    assert!(app.capture_document().equivalent(&resized));
    app.history(false);
    assert!(app.capture_document().equivalent(&before));
    app.history(true);
    assert!(app.capture_document().equivalent(&resized));
    app.history(true);
    assert_eq!(width(&app), 30);
    assert_eq!(app.rack.layout(), resized.layout);
}

#[test]
fn no_op_collision_invalid_and_stale_resize_cannot_advance_history_or_move_other_modules() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = annotation_host(dir.path());
    let before = app.capture_document();
    let snapshot = app.shared.lock().unwrap().snapshot();
    let no_op = proposal(&app, 10, false);
    assert!(app.resize_blank(no_op));
    assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
    // Right expansion ends past the obstacle at x=900; this must never move it.
    let collision = proposal(&app, 48, false);
    assert!(!app.resize_blank(collision));
    let mut invalid = proposal(&app, 20, true);
    invalid.movement.after.get_mut("blank").unwrap().y += 1.0;
    assert!(!app.resize_blank(invalid));
    let mut invalid = proposal(&app, 20, true);
    invalid.movement.after.get_mut("blank").unwrap().x = f32::NAN;
    assert!(!app.resize_blank(invalid));
    assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
    assert!(app.capture_document().equivalent(&before));
    let stale = proposal(&app, 20, true);
    set_width(&app, 12);
    let newer = app.shared.lock().unwrap().snapshot();
    assert!(!app.resize_blank(stale));
    assert_eq!(app.shared.lock().unwrap().snapshot(), newer);
    assert_eq!(width(&app), 12);
    assert_eq!(app.rack.layout(), before.layout);
    // A caller holding the current revision cannot disguise the old base width.
    let mut stale_width = proposal(&app, 20, true);
    stale_width.before_hp = 10;
    assert!(!app.resize_blank(stale_width));
    assert_eq!(app.shared.lock().unwrap().snapshot(), newer);
    assert_eq!(app.rack.layout(), before.layout);
}

#[test]
fn unicode_notes_custom_blank_width_presets_duplicates_projects_and_recovery_preserve_content_and_layout(
) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = annotation_host(dir.path());
    let resize = proposal(&app, 27, true);
    assert!(app.resize_blank(resize));
    let notes_preset = execute_shared(&app.shared, "capture_module_preset", json!({"id":"notes"}))
        .unwrap()["preset"]
        .clone();
    let blank_preset = execute_shared(&app.shared, "capture_module_preset", json!({"id":"blank"}))
        .unwrap()["preset"]
        .clone();
    assert!(app.duplicate_modules(
        &["notes".into(), "blank".into()],
        DuplicateCableMode::None,
        app.revision,
        true
    ));
    let duplicated = app.capture_document();
    for (id, copy) in [("notes", "notes-copy-1"), ("blank", "blank-copy-1")] {
        assert_eq!(
            app.patch.nodes.iter().find(|n| n.id == id).unwrap().content,
            app.patch
                .nodes
                .iter()
                .find(|n| n.id == copy)
                .unwrap()
                .content
        );
        assert_ne!(
            duplicated.layout["positions"][id],
            duplicated.layout["positions"][copy]
        );
    }
    app.history(false);
    assert!(!app.patch.nodes.iter().any(|n| n.id.ends_with("copy-1")));
    app.history(true);
    assert!(app.capture_document().equivalent(&duplicated));
    assert!(app.command("initialize_modules", json!({"ids":["notes","blank"]})));
    assert_eq!(width(&app), 10);
    assert!(app.command(
        "apply_edits",
        json!({"edits":[
            {"tool":"apply_module_preset","arguments":{"id":"notes","preset":notes_preset}},
            {"tool":"apply_module_preset","arguments":{"id":"blank","preset":blank_preset}}
        ]})
    ));
    assert_eq!(width(&app), 27);
    assert_eq!(
        app.patch
            .nodes
            .iter()
            .find(|n| n.id == "notes")
            .unwrap()
            .effective_notes_text(),
        Some(NOTES)
    );
    let document = app.capture_document();
    app.save_document("annotations.openpatch", true);
    assert!(app
        .projects
        .as_ref()
        .unwrap()
        .load("annotations.openpatch")
        .unwrap()
        .document
        .equivalent(&document));
    app.projects
        .as_ref()
        .unwrap()
        .save_recovery(&document)
        .unwrap();
    drop(app.projects.take());
    app.projects = Some(Store::new(dir.path()).unwrap());
    let recovery = app
        .projects
        .as_ref()
        .unwrap()
        .recoveries()
        .into_iter()
        .next()
        .unwrap();
    assert!(app
        .projects
        .as_ref()
        .unwrap()
        .load_recovery(&recovery)
        .unwrap()
        .document
        .equivalent(&document));
    assert!(app.command(
        "initialize_modules",
        json!({"ids":["notes","blank","notes-copy-1","blank-copy-1"]})
    ));
    app.load_document(&recovery, true);
    assert!(app.capture_document().equivalent(&document));
    assert!(app.dirty());
    app.load_document("annotations.openpatch", false);
    assert!(app.capture_document().equivalent(&document));
    assert!(!app.dirty());
}

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
                egui::vec2(1200.0, 1000.0),
            )),
            time: Some(time),
            focused: true,
            events,
            modifiers,
            ..Default::default()
        },
        |ctx| {
            app.global_shortcuts(ctx);
            app.rack_workspace(ctx, None);
        },
    )
}

fn text_position(output: &egui::FullOutput, label: &str) -> Option<egui::Pos2> {
    fn find(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            egui::Shape::Vec(children) => children.iter().find_map(|child| find(child, label)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, label))
}

#[test]
fn real_notes_editor_blocks_global_and_performance_shortcuts_and_escape_discards_the_draft() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = annotation_host(dir.path());
    app.keyboard_open = true;
    let ctx = egui::Context::default();
    let mut emitted = Vec::new();
    let _ = ctx.run(
        egui::RawInput {
            focused: true,
            events: vec![performance_key(egui::Key::Z, true, egui::Modifiers::NONE)],
            ..Default::default()
        },
        |ctx| {
            emitted = app.update_performance_keyboard(ctx, true);
        },
    );
    assert!(matches!(
        emitted.as_slice(),
        [openpatch_core::MidiEvent::NoteOn { .. }]
    ));
    let output = frame(&ctx, &mut app, 0.0, vec![]);
    let edit =
        text_position(&output, "EDIT NOTES").expect("real Notes panel renders its editor button");
    frame(
        &ctx,
        &mut app,
        0.1,
        vec![egui::Event::PointerMoved(edit), pointer_event(edit, true)],
    );
    frame(&ctx, &mut app, 0.2, vec![pointer_event(edit, false)]);
    assert!(app.rack.annotation_edit_active());
    let before = app.shared.lock().unwrap().snapshot();
    let layout = app.rack.layout();
    let palette = app.cable_palette.next_index();
    let _ = ctx.run(
        egui::RawInput {
            focused: true,
            events: vec![performance_key(egui::Key::Q, true, egui::Modifiers::NONE)],
            ..Default::default()
        },
        |ctx| {
            assert!(!app.performance_keyboard_enabled(ctx, true));
            emitted = app.update_performance_keyboard(ctx, true);
        },
    );
    assert!(matches!(
        emitted.as_slice(),
        [openpatch_core::MidiEvent::NoteOff { note: 60, .. }]
    ));
    for (index, key, modifiers) in [
        (0, egui::Key::Enter, egui::Modifiers::NONE),
        (1, egui::Key::CloseBracket, egui::Modifiers::NONE),
        (2, egui::Key::N, egui::Modifiers::COMMAND),
        (3, egui::Key::O, egui::Modifiers::COMMAND),
        (4, egui::Key::Z, egui::Modifiers::COMMAND),
        (5, egui::Key::D, egui::Modifiers::COMMAND),
    ] {
        frame(
            &ctx,
            &mut app,
            1.0 + index as f64,
            vec![performance_key(key, true, modifiers)],
        );
        assert!(app.rack.annotation_edit_active());
        assert!(!app.browser_open);
        assert!(app.file_mode.is_none());
        assert!(app.confirm_discard.is_none());
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    }
    frame(
        &ctx,
        &mut app,
        8.0,
        vec![egui::Event::Text("uncommitted 📝\nnew text".into())],
    );
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    frame(
        &ctx,
        &mut app,
        9.0,
        vec![performance_key(
            egui::Key::Escape,
            true,
            egui::Modifiers::NONE,
        )],
    );
    assert!(!app.rack.annotation_edit_active());
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    assert_eq!(app.rack.layout(), layout);
    assert_eq!(app.cable_palette.next_index(), palette);
}

#[test]
fn real_notes_apply_race_rejects_stale_host_command_and_keeps_the_draft_with_apply_disabled() {
    fn rack_frame(
        ctx: &egui::Context,
        app: &mut OpenPatch,
        time: f64,
        events: Vec<egui::Event>,
    ) -> (Option<RackAction>, egui::FullOutput) {
        let modifiers = events
            .iter()
            .rev()
            .find_map(|event| match event {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let mut action = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 1000.0),
                )),
                time: Some(time),
                focused: true,
                modifiers,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        action = app
                            .rack
                            .show(ui, &app.patch, &app.catalog, 48_000, app.revision);
                    });
            },
        );
        (action, output)
    }

    let dir = tempfile::tempdir().unwrap();
    let mut app = annotation_host(dir.path());
    let ctx = egui::Context::default();
    let output = rack_frame(&ctx, &mut app, 0.0, vec![]).1;
    let edit = text_position(&output, "EDIT NOTES").unwrap();
    assert!(rack_frame(
        &ctx,
        &mut app,
        0.1,
        vec![egui::Event::PointerMoved(edit), pointer_event(edit, true)]
    )
    .0
    .is_none());
    assert!(
        rack_frame(&ctx, &mut app, 0.2, vec![pointer_event(edit, false)])
            .0
            .is_none()
    );
    rack_frame(&ctx, &mut app, 0.3, vec![]);
    assert!(app.rack.annotation_edit_active());
    let draft = "Retain this unsaved draft\n日本語 📝";
    rack_frame(
        &ctx,
        &mut app,
        0.4,
        vec![
            performance_key(egui::Key::A, true, egui::Modifiers::COMMAND),
            egui::Event::Paste(draft.into()),
        ],
    );
    let output = rack_frame(&ctx, &mut app, 0.5, vec![]).1;
    assert!(
        text_position(&output, draft).is_some(),
        "the real multiline editor owns the draft"
    );
    let apply = text_position(&output, "Apply").unwrap();
    assert!(rack_frame(
        &ctx,
        &mut app,
        0.6,
        vec![egui::Event::PointerMoved(apply), pointer_event(apply, true)]
    )
    .0
    .is_none());
    let action = rack_frame(&ctx, &mut app, 0.7, vec![pointer_event(apply, false)]).0;
    let Some(RackAction::ApplyAnnotation(arguments)) = action else {
        panic!("the real Apply button must produce a guarded content request");
    };
    assert_eq!(arguments["expected_revision"], app.revision);
    assert_eq!(arguments["content"]["text"], draft);
    let external = "Authoritative concurrent MCP text\n🎛️";
    execute_shared(
        &app.shared,
        "set_module_content",
        json!({"id":"notes",
        "content":{"type":"notes","text":external},"expected_revision":app.revision}),
    )
    .unwrap();
    let committed = app.shared.lock().unwrap().snapshot();
    // Match the production RackAction dispatcher while inserting the concurrent
    // commit between request production and execution, without rebasing its guard.
    let accepted = app.command("set_module_content", arguments);
    assert!(!accepted);
    app.rack.finish_annotation_edit(accepted);
    assert!(app.rack.annotation_edit_active());
    assert_eq!(app.shared.lock().unwrap().snapshot(), committed);
    assert_eq!(
        app.patch
            .nodes
            .iter()
            .find(|n| n.id == "notes")
            .unwrap()
            .effective_notes_text(),
        Some(external)
    );
    let output = rack_frame(&ctx, &mut app, 0.8, vec![]).1;
    assert!(
        text_position(&output, draft).is_some(),
        "a rejected request must retain the exact draft"
    );
    assert!(text_position(
        &output,
        "The patch changed. Your draft is retained; reload before applying."
    )
    .is_some());
    assert!(text_position(&output, "Reload current content").is_some());
    let disabled_apply = text_position(&output, "Apply").unwrap();
    assert!(rack_frame(
        &ctx,
        &mut app,
        0.9,
        vec![
            egui::Event::PointerMoved(disabled_apply),
            pointer_event(disabled_apply, true)
        ]
    )
    .0
    .is_none());
    assert!(
        rack_frame(
            &ctx,
            &mut app,
            1.0,
            vec![pointer_event(disabled_apply, false)]
        )
        .0
        .is_none(),
        "stale Apply is disabled"
    );
    assert!(app.rack.annotation_edit_active());
    assert_eq!(app.shared.lock().unwrap().snapshot(), committed);
}
