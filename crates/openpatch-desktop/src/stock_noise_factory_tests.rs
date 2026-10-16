use super::*;

#[test]
fn stock_noise_factory_uses_real_excitation_and_shared_read_only_panels() {
    let patch: Patch =
        serde_json::from_str(include_str!("../../../examples/stock-noise-resonator.json")).unwrap();
    assert_eq!((patch.nodes.len(), patch.cables.len()), (8, 8));
    let noise = patch.nodes.iter().find(|node| node.id == "noise").unwrap();
    assert_eq!(noise.kind, "stock_noise");
    assert!(noise.params.is_empty());
    for (from, output, to, input) in [
        ("noise", "pink", "amplifier", "signal"),
        ("amplifier", "out", "body", "excite"),
        ("clock", "out", "envelope", "gate"),
        ("envelope", "out", "amplifier", "cv"),
        ("noise", "white", "scope", "x"),
        ("body", "out", "scope", "y"),
        ("body", "out", "out", "left"),
        ("body", "out", "out", "right"),
    ] {
        assert!(patch
            .cables
            .iter()
            .any(|c| c.from == from && c.output == output && c.to == to && c.input == input));
    }
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let before = app.capture_document();
    app.documents.mark_saved(&before);
    app.replace_from_menu(patch.clone(), "Noise into matter");
    assert_eq!(app.title, "Noise into matter");
    assert_eq!(app.patch.reference_view(), patch.reference_view());
    let scope = app.prepare_stock_noise_fixture();
    assert!(app.audio.is_none());
    assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
    assert!(app.status.contains("offline Engine"));
    let layout = app.rack.layout();
    let zoom = layout["zoom"].as_f64().unwrap();
    for node in &app.patch.nodes {
        let p = &layout["positions"][&node.id];
        let (x, y) = (p[0].as_f64().unwrap(), p[1].as_f64().unwrap());
        assert!(24.0 + (x + f64::from(crate::rack::module_width(node))) * zoom < 1280.0);
        assert!(12.0 + (y + f64::from(crate::rack::PANEL_HEIGHT)) * zoom < 800.0);
        for other in app.patch.nodes.iter().filter(|other| other.id != node.id) {
            let op = &layout["positions"][&other.id];
            let (ox, oy) = (op[0].as_f64().unwrap(), op[1].as_f64().unwrap());
            assert!(
                x + f64::from(crate::rack::module_width(node)) <= ox
                    || ox + f64::from(crate::rack::module_width(other)) <= x
                    || y + 396.0 <= oy
                    || oy + 396.0 <= y,
                "{} overlaps {}",
                node.id,
                other.id
            );
        }
    }
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let revision = app.revision;
    for room in [1.0, 0.0] {
        app.rack.set_room_brightness(room);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 900.0),
                )),
                focused: true,
                ..Default::default()
            },
            |ctx| app.rack_workspace(ctx, None),
        );
        preset_text(&output, "STOCK NOISE");
        for label in ["WHITE", "PINK", "RED", "VIOLET", "BLUE", "GRAY", "BLACK"] {
            preset_text(&output, label);
        }
        assert_eq!(app.revision, revision);
    }
    app.browser_open = false;
    app.replace_from_menu(patch.clone(), "Noise into matter");
    assert!(
        app.confirm_discard.is_some(),
        "unsaved patch retains discard guard"
    );
    assert_eq!(app.revision, revision);
}

#[test]
fn mcp_rows_keep_unseen_redo_intent_and_user_moved_history_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    assert!(app.command("replace_patch", json!({"patch":Patch::new_v2()})));
    execute_shared(
        &app.shared,
        "add_module",
        json!({"id":"row-voice","kind":"stock_noise","row":2}),
    )
    .unwrap();
    execute_shared(&app.shared, "undo", json!({})).unwrap();
    app.sync();
    assert!(!app.patch.nodes.iter().any(|n| n.id == "row-voice"));
    execute_shared(&app.shared, "redo", json!({})).unwrap();
    app.sync();
    assert_eq!(app.rack.layout()["positions"]["row-voice"][1], json!(840.0));
    let mut moved = app.rack.layout();
    moved["positions"]["row-voice"] = json!([450.0, 1260.0]);
    app.rack.restore_layout(&moved).unwrap();
    execute_shared(&app.shared, "undo", json!({})).unwrap();
    app.sync();
    execute_shared(&app.shared, "redo", json!({})).unwrap();
    app.sync();
    assert_eq!(
        app.rack.layout()["positions"]["row-voice"],
        json!([450.0, 1260.0])
    );
    // Reusing an ID is a fresh incarnation, and its row must not overwrite
    // the outgoing state's saved geometry before that state can be undone.
    execute_shared(
        &app.shared,
        "apply_edits",
        json!({"edits":[
            {"tool":"remove_module","arguments":{"id":"row-voice"}},
            {"tool":"add_module","arguments":{"id":"row-voice","kind":"stock_noise","row":1}}
        ]}),
    )
    .unwrap();
    app.sync();
    assert_eq!(app.rack.layout()["positions"]["row-voice"][1], json!(420.0));
    execute_shared(&app.shared, "undo", json!({})).unwrap();
    app.sync();
    assert_eq!(
        app.rack.layout()["positions"]["row-voice"],
        json!([450.0, 1260.0])
    );
}
