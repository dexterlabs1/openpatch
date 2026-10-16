use super::*;
fn factory_patch() -> Patch {
    let mut patch: Patch =
        serde_json::from_str(include_str!("../../../examples/stock-mix-voice.json")).unwrap();
    patch.assign_missing_cable_ids().unwrap();
    patch
}
#[test]
fn stock_mix_factory_uses_real_modulation_averaging_and_centered_audio() {
    let patch = factory_patch();
    assert_eq!((patch.nodes.len(), patch.cables.len()), (7, 8));
    for (from, output, to, input) in [
        ("bass", "sine", "mix", "in1"),
        ("fifth", "sine", "mix", "in2"),
        ("upper", "sine", "mix", "in3"),
        ("motion", "sine", "upper", "fm"),
        ("mix", "out", "scope", "x"),
        ("bass", "sine", "scope", "y"),
        ("mix", "out", "out", "left"),
        ("mix", "out", "out", "right"),
    ] {
        assert!(patch
            .cables
            .iter()
            .any(|c| c.from == from && c.output == output && c.to == to && c.input == input));
    }
    let mut engine = patch.compile().unwrap();
    engine.set_port_lights_enabled(true);
    let taps = [
        Some(engine.resolve_tap("mix", "out", 0).unwrap()),
        Some(engine.resolve_tap("motion", "sine", 0).unwrap()),
    ];
    let mut audio = vec![[0.0; 2]; 24032];
    let mut observed = vec![[openpatch_core::TapSample::default(); 2]; audio.len()];
    engine.process_audio_block_with_taps(&mut audio, &taps, &mut observed);
    assert!(audio.iter().flatten().all(|v| v.is_finite()));
    assert!(audio.iter().any(|v| v[0].abs() > 0.001));
    assert!(
        audio.iter().all(|v| v[0] == v[1]),
        "One mix output explicitly feeds both audio channels"
    );
    for port in 0..2 {
        assert!(observed[32..]
            .iter()
            .all(|v| v[port].active && v[port].value.is_finite()));
        assert!(observed.iter().any(|v| v[port].value.abs() > 0.001));
    }
    let scope = engine
        .module_scope(engine.resolve_module_scope("scope").unwrap())
        .unwrap();
    assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
    for lane in [&scope.x[0], &scope.y[0]] {
        assert!(lane
            .iter()
            .any(|v| v.min.is_finite() && v.max.abs() > 0.001));
    }
    assert!(
        engine.resolve_module_lights("mix").is_err(),
        "Stock Mix must not fabricate module lamps"
    );
    let frame = crate::audio::offline_frame(&patch, &engine, 1).unwrap();
    assert!(frame.ports.contains_key(&crate::audio::PortLightKey::new(
        "mix",
        openpatch_core::PortDirection::Output,
        "out"
    )));
    let mut fixed = patch.clone();
    fixed
        .nodes
        .iter_mut()
        .find(|n| n.id == "upper")
        .unwrap()
        .params
        .insert("fm_amount".into(), 0.0);
    let mut fixed_engine = fixed.compile().unwrap();
    let mut fixed_audio = vec![[0.0; 2]; audio.len()];
    fixed_engine.process_audio_block(&mut fixed_audio);
    assert!(
        audio
            .iter()
            .zip(&fixed_audio)
            .any(|(a, b)| (a[0] - b[0]).abs() > 0.0001),
        "The actual LFO path must affect the audible mix"
    );
}

#[test]
fn stock_mix_factory_layout_smoke_browser_and_discard_guard_share_real_patch() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = host(directory.path());
    let clean = app.capture_document();
    app.documents.mark_saved(&clean);
    app.replace_from_menu(factory_patch(), "Stock Mix layered voice");
    assert!(
        app.confirm_discard.is_none(),
        "A saved document may open the factory immediately"
    );
    assert_eq!(app.title, "Stock Mix layered voice");
    let scope = app.prepare_stock_mix_fixture();
    assert!(app.audio.is_none());
    assert_eq!(app.patch.reference_view(), factory_patch().reference_view());
    assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
    assert!(app.status.contains("offline"));
    let mut expected = Rack::default();
    expected
        .restore_layout(&OpenPatch::stock_mix_voice_layout())
        .unwrap();
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
            "STOCK MIX",
            "STOCK VCO",
            "STOCK LFO",
            "SCOPE",
            "STEREO AUDIO",
        ] {
            preset_text(&output, label);
        }
        assert_eq!(app.revision, revision);
    }
    app.rack.set_room_brightness(1.0);
    app.library = ModuleLibrary::default();
    app.library.set_preview_zoom(0.7);
    app.browse();
    app.library.request_focus();
    let mut output = None;
    for (time, events) in [
        (1.0, vec![]),
        (1.2, vec![egui::Event::Text("stock_mix".into())]),
        (1.4, vec![]),
    ] {
        output = Some(ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 900.0),
                )),
                focused: true,
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| {
                let _ = app.browser(ctx);
            },
        ));
    }
    preset_text(&output.unwrap(), "STOCK MIX");
    assert!(app.browser_open);
    app.browser_open = false;
    app.replace_from_menu(factory_patch(), "Stock Mix layered voice");
    assert_eq!(
        app.revision, revision,
        "Unsaved fixture must retain discard guard"
    );
    let action = app
        .confirm_discard
        .take()
        .expect("Explicit discard choice required");
    app.perform_action(action);
    assert!(app.revision > revision);
    assert_eq!(app.title, "Stock Mix layered voice");
    assert_eq!(app.rack.layout(), expected.layout());
    assert_eq!(app.patch.reference_view(), factory_patch().reference_view());
}
