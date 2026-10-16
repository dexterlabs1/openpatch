use super::*;

fn factory_patch() -> Patch {
    let mut patch: Patch =
        serde_json::from_str(include_str!("../../../examples/stock-mid-side-voice.json")).unwrap();
    patch.assign_missing_cable_ids().unwrap();
    patch
}

#[test]
fn stock_mid_side_factory_has_actual_encoded_scope_and_audible_decoded_stereo() {
    let patch = factory_patch();
    assert_eq!((patch.nodes.len(), patch.cables.len()), (6, 7));
    for (from, output, to, input) in [
        ("left", "sine", "width", "left"),
        ("right", "sine", "width", "right"),
        ("motion", "sine", "width", "decode_cv"),
        ("width", "mid", "scope", "x"),
        ("width", "side", "scope", "y"),
        ("width", "left", "out", "left"),
        ("width", "right", "out", "right"),
    ] {
        assert!(patch.cables.iter().any(|cable| cable.from == from
            && cable.output == output
            && cable.to == to
            && cable.input == input));
    }
    assert!(!patch
        .cables
        .iter()
        .any(|cable| cable.to == "width" && matches!(cable.input.as_str(), "mid" | "side")));
    let mut engine = patch.compile().unwrap();
    engine.set_port_lights_enabled(true);
    let taps = ["mid", "side", "left", "right"]
        .map(|port| Some(engine.resolve_tap("width", port, 0).unwrap()));
    let mut audio = vec![[0.0; 2]; 24032];
    let mut observed = vec![[openpatch_core::TapSample::default(); 4]; audio.len()];
    engine.process_audio_block_with_taps(&mut audio, &taps, &mut observed);
    assert!(audio.iter().flatten().all(|sample| sample.is_finite()));
    for channel in 0..2 {
        assert!(audio.iter().any(|sample| sample[channel].abs() > 0.001));
    }
    assert!(audio
        .iter()
        .any(|sample| (sample[0] - sample[1]).abs() > 0.001));
    for port in 0..4 {
        assert!(observed[32..]
            .iter()
            .all(|frame| frame[port].active && frame[port].value.is_finite()));
        assert!(observed.iter().any(|frame| frame[port].value.abs() > 0.001));
    }
    let scope = engine
        .module_scope(engine.resolve_module_scope("scope").unwrap())
        .unwrap();
    assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
    for lane in [&scope.x[0], &scope.y[0]] {
        assert!(lane
            .iter()
            .any(|point| point.min.is_finite() && point.max.abs() > 0.001));
    }
    let lights = crate::audio::offline_frame(&patch, &engine, 1).unwrap();
    for port in ["mid", "side", "left", "right"] {
        assert!(lights.ports.contains_key(&crate::audio::PortLightKey::new(
            "width",
            openpatch_core::PortDirection::Output,
            port
        )));
    }
    if let Ok(handle) = engine.resolve_module_lights("width") {
        assert!(engine
            .module_lights(handle)
            .is_none_or(|state| state.count == 0));
    }
}

#[test]
fn stock_mid_side_factory_layout_smoke_browser_and_discard_guard_share_real_patch() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = host(directory.path());
    let clean = app.capture_document();
    app.documents.mark_saved(&clean);
    app.replace_from_menu(factory_patch(), "Mid/Side stereo voice");
    assert!(
        app.confirm_discard.is_none(),
        "A saved document may open the factory immediately"
    );
    assert_eq!(app.title, "Mid/Side stereo voice");
    let scope = app.prepare_stock_mid_side_fixture();
    assert!(app.audio.is_none());
    assert_eq!(app.patch.reference_view(), factory_patch().reference_view());
    assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
    assert!(app.status.contains("offline"));
    let mut expected = Rack::default();
    expected
        .restore_layout(&OpenPatch::stock_mid_side_voice_layout())
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
            "STOCK MID/SIDE",
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
        (1.2, vec![egui::Event::Text("stock_mid_side".into())]),
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
    preset_text(&output.unwrap(), "STOCK MID/SIDE");
    assert!(app.browser_open);
    app.browser_open = false;
    app.replace_from_menu(factory_patch(), "Mid/Side stereo voice");
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
    assert_eq!(app.title, "Mid/Side stereo voice");
    assert_eq!(app.rack.layout(), expected.layout());
    assert_eq!(app.patch.reference_view(), factory_patch().reference_view());
}
