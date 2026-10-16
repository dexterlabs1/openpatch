use super::*;

#[test]
fn studio_mixer_factory_has_two_real_stereo_fx_returns_and_shared_offline_panels() {
    let patch: Patch =
        serde_json::from_str(include_str!("../../../examples/studio-mixer-fx.json")).unwrap();
    assert_eq!((patch.nodes.len(), patch.cables.len()), (12, 18));
    for (from, output, to, input) in [
        ("console", "send_a_left", "delay", "left"),
        ("console", "send_a_right", "delay", "right"),
        ("console", "send_b_left", "reverb", "left"),
        ("console", "send_b_right", "reverb", "right"),
        ("return-a", "out", "console", "return_a"),
        ("return-b", "out", "console", "return_b"),
    ] {
        assert!(patch
            .cables
            .iter()
            .any(|c| c.from == from && c.output == output && c.to == to && c.input == input));
    }
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let saved = app.capture_document();
    app.documents.mark_saved(&saved);
    app.replace_from_menu(patch.clone(), "Studio Mixer FX");
    assert_eq!(app.title, "Studio Mixer FX");
    assert_eq!(app.patch.reference_view(), patch.reference_view());
    let scope = app.prepare_studio_mixer_fixture();
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
            let p = &layout["positions"][&other.id];
            let (ox, oy) = (p[0].as_f64().unwrap(), p[1].as_f64().unwrap());
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
        preset_text(&output, "STUDIO MIXER");
        assert_eq!(app.revision, revision);
    }
}

#[test]
fn committed_console_growth_moves_only_the_collision_chain_and_undo_restores_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"console","kind":"studio_mixer","params":{"channels":2}},
        {"id":"near","kind":"constant"},{"id":"chain","kind":"constant"},
        {"id":"far","kind":"constant"},{"id":"other-row","kind":"constant"}
    ]}))
    .unwrap();
    assert!(app.command("replace_patch", json!({"patch":patch})));
    app.rack
        .restore_layout(&json!({"version":1,"zoom":0.7,"pan":[24,12],"positions":{
            "console":[0,0],"near":[372,0],"chain":[552,0],"far":[2000,0],"other-row":[372,420]
        }}))
        .unwrap();
    let before = app.rack.layout();
    // External MCP has no GUI action wrapper: sync must handle the resize.
    execute_shared(
        &app.shared,
        "set_parameter",
        json!({"id":"console","parameter":"channels","value":4}),
    )
    .unwrap();
    app.sync();
    let after = app.rack.layout();
    assert_eq!(after["positions"]["near"], json!([516.0, 0.0]));
    assert_eq!(after["positions"]["chain"], json!([636.0, 0.0]));
    for id in ["console", "far", "other-row"] {
        assert_eq!(after["positions"][id], before["positions"][id]);
    }
    app.history(false);
    assert_eq!(app.rack.layout(), before);
    app.history(true);
    assert_eq!(
        app.rack.layout(),
        after,
        "redo restores the settled layout without pushing twice"
    );
}
