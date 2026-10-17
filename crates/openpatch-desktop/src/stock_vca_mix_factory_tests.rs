use super::*;

fn factory_patch() -> Patch {
    serde_json::from_str(include_str!("../../../examples/stock-vca-mix-voice.json"))
        .expect("bundled Stock VCA Mix factory voice")
}

#[test]
fn stock_vca_mix_factory_loads_the_reviewed_voice_and_actual_meter_snapshots() {
    let patch = factory_patch();
    assert_eq!((patch.nodes.len(), patch.cables.len()), (9, 10));
    for id in ["bass", "low", "fifth", "air", "mix", "motion", "scope", "out", "guide"] {
        assert!(patch.nodes.iter().any(|node| node.id == id), "{id}");
    }
    for (from, output, to, input) in [
        ("bass", "triangle", "mix", "in1"),
        ("low", "sine", "mix", "in2"),
        ("fifth", "saw", "mix", "in3"),
        ("air", "sine", "mix", "in4"),
        ("motion", "sine", "mix", "mix_cv"),
        ("motion", "triangle", "mix", "cv4"),
        ("mix", "out1", "scope", "x"),
        ("mix", "mix", "scope", "y"),
        ("mix", "mix", "out", "left"),
        ("mix", "mix", "out", "right"),
    ] {
        assert!(patch.cables.iter().any(|cable| {
            cable.from == from
                && cable.output == output
                && cable.to == to
                && cable.input == input
        }));
    }
    assert_eq!(patch.output.as_deref(), Some("out"));

    let directory = tempfile::tempdir().unwrap();
    let mut app = host(directory.path());
    let saved = app.capture_document();
    app.documents.mark_saved(&saved);
    let revision = app.revision;
    app.replace_from_menu(patch.clone(), "Stock VCA Mix voice");
    assert!(app.confirm_discard.is_none());
    assert_eq!(app.title, "Stock VCA Mix voice");
    assert!(app.revision > revision);
    assert_eq!(app.patch.reference_view(), patch.reference_view());

    let scope = app.prepare_stock_vca_mix_fixture();
    assert!(app.audio.is_none(), "factory fixture must leave audio stopped");
    assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
    assert!(app.status.contains("offline Engine"));
    let expected_layout = OpenPatch::stock_vca_mix_voice_layout();
    let mut expected = Rack::default();
    expected.restore_layout(&expected_layout).unwrap();
    assert_eq!(app.rack.layout(), expected.layout());
    let layout = app.rack.layout();
    let zoom = layout["zoom"].as_f64().unwrap();
    for node in &app.patch.nodes {
        let position = &layout["positions"][&node.id];
        let x = position[0].as_f64().unwrap();
        let y = position[1].as_f64().unwrap();
        assert!(24.0 + (x + f64::from(crate::rack::module_width(node))) * zoom < 1280.0);
        assert!(12.0 + (y + f64::from(crate::rack::PANEL_HEIGHT)) * zoom < 800.0);
        for other in app.patch.nodes.iter().filter(|other| other.id != node.id) {
            let other_position = &layout["positions"][&other.id];
            let other_x = other_position[0].as_f64().unwrap();
            let other_y = other_position[1].as_f64().unwrap();
            assert!(
                x + f64::from(crate::rack::module_width(node)) <= other_x
                    || other_x + f64::from(crate::rack::module_width(other)) <= x
                    || y + f64::from(crate::rack::PANEL_HEIGHT) <= other_y
                    || other_y + f64::from(crate::rack::PANEL_HEIGHT) <= y,
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
        for label in [
            "STOCK VCA MIX",
            "CHANNEL CV  LIN",
            "MIX CV  LIN",
            "VU LIVE",
            "CH 1",
            "IN × L² × c",
            "MASTER",
            "Σ",
            "× MIX LEVEL",
            "× h",
            "h=max(V/10,0)",
        ] {
            preset_text(&output, label);
        }
        assert_eq!(app.revision, revision, "rendering must remain read-only");
    }

    app.rack.set_room_brightness(1.0);
    app.library = ModuleLibrary::default();
    app.library.set_preview_zoom(0.7);
    app.browse();
    assert!(app.library.inspect("stock_vca_mix", &app.catalog));
    app.library.request_focus();
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 900.0),
            )),
            focused: true,
            events: vec![egui::Event::Text("stock_vca_mix".into())],
            ..Default::default()
        },
        |ctx| {
            let _ = app.browser(ctx);
        },
    );
    preset_text(&output, "STOCK VCA MIX");
    assert!(app.browser_open);
    assert_eq!(app.revision, revision, "browser preview must not mutate the voice");

    app.browser_open = false;
    app.replace_from_menu(factory_patch(), "Stock VCA Mix voice");
    assert_eq!(app.revision, revision, "unsaved edits retain the discard guard");
    let action = app.confirm_discard.take().expect("discard confirmation");
    app.perform_action(action);
    assert_eq!(app.patch.reference_view(), patch.reference_view());
    assert_eq!(app.rack.layout(), expected.layout());
}
