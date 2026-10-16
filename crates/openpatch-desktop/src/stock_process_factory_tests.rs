use super::*;

#[test]
fn stock_process_factory_connects_restrained_pitch_motion_and_real_scope_evidence() {
    let patch: Patch =
        serde_json::from_str(include_str!("../../../examples/stock-process-voice.json")).unwrap();
    assert_eq!((patch.nodes.len(), patch.cables.len()), (8, 8));
    assert_eq!(patch.nodes.iter().find(|n| n.id == "process").unwrap().kind, "stock_process");
    assert_eq!(patch.nodes.iter().find(|n| n.id == "pitch-range").unwrap().params["gain"], 0.12);
    for (from, output, to, input) in [
        ("pitch-motion", "sine", "pitch-range", "signal"),
        ("pitch-range", "out", "process", "signal"),
        ("gate-clock", "out", "process", "gate"),
        ("process", "glide", "voice", "pitch"),
        ("pitch-range", "out", "scope", "x"),
        ("process", "glide", "scope", "y"),
        ("voice", "sine", "out", "left"),
        ("voice", "sine", "out", "right"),
    ] {
        assert!(patch.cables.iter().any(|c|
            c.from == from && c.output == output && c.to == to && c.input == input));
    }
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let before = app.capture_document();
    app.documents.mark_saved(&before);
    app.replace_from_menu(patch.clone(), "Stock Process voice");
    assert_eq!(app.title, "Stock Process voice");
    assert_eq!(app.patch.reference_view(), patch.reference_view());
    let scope = app.prepare_stock_process_fixture();
    assert!(app.audio.is_none());
    assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
    assert!(scope.x[0].iter().zip(&scope.y[0])
        .any(|(input, glide)| (input.max - glide.max).abs() > 0.005));
    assert!(app.status.contains("offline Engine"));
    assert!(app.status.contains("manual lamp off"));
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
            assert!(x + f64::from(crate::rack::module_width(node)) <= ox
                || ox + f64::from(crate::rack::module_width(other)) <= x
                || y + 396.0 <= oy || oy + 396.0 <= y,
                "{} overlaps {}", node.id, other.id);
        }
    }
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let revision = app.revision;
    for room in [1.0, 0.0] {
        app.rack.set_room_brightness(room);
        let output = ctx.run(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
            focused: true,
            ..Default::default()
        }, |ctx| app.rack_workspace(ctx, None));
        preset_text(&output, "STOCK PROCESS");
        for label in ["SLEW TIME", "MANUAL GATE", "S/H 1", "S/H 2", "GLIDE"] {
            preset_text(&output, label);
        }
        assert_eq!(app.revision, revision);
    }
    app.browser_open = false;
    app.replace_from_menu(patch, "Stock Process voice");
    assert!(app.confirm_discard.is_some(), "factory remains an unsaved document");
    assert_eq!(app.revision, revision);
}
