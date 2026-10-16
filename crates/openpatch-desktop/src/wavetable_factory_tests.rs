use super::*;

fn wavetable_factory_patch() -> Patch {
    let mut patch: Patch =
        serde_json::from_str(include_str!("../../../examples/wavetable-voice.json")).unwrap();
    patch.assign_missing_cable_ids().unwrap();
    patch
}

#[test]
fn wavetable_factory_has_real_position_modulation_and_audible_stereo_output() {
    let patch = wavetable_factory_patch();
    assert!(patch
        .cables
        .iter()
        .any(|cable| cable.from == "morph" && cable.to == "voice" && cable.input == "position_cv"));
    let mut engine = patch.compile().unwrap();
    let morph = engine.resolve_tap("morph", "out", 0).unwrap();
    let voice = engine.resolve_module_lights("voice").unwrap();
    let lfo = engine.resolve_module_lights("morph").unwrap();
    let mut audio = vec![[0.0; 2]; 4096];
    let mut taps = vec![[openpatch_core::TapSample::default(); 1]; audio.len()];
    let mut positions = Vec::new();
    let mut captured_scope = None;
    for _ in 0..4 {
        engine.process_audio_block_with_taps(&mut audio, &[Some(morph)], &mut taps);
        assert!(audio.iter().flatten().all(|value| value.is_finite()));
        assert!(taps.iter().all(|sample| sample[0].active));
        let position = engine
            .module_lights(voice)
            .unwrap()
            .waveform_position
            .unwrap();
        let expected = (0.1 + taps[taps.len() - 2][0].value * 0.8 / 10.0).clamp(0.0, 1.0);
        assert!((position - expected).abs() < 0.000001);
        if captured_scope.is_none() {
            assert!(audio.iter().flatten().any(|value| value.abs() > 0.001));
            for id in ["voice", "morph", "amplifier", "envelope"] {
                let lights = engine
                    .module_lights(engine.resolve_module_lights(id).unwrap())
                    .unwrap();
                assert!(lights.brightness.iter().flatten().any(|value| *value > 0.0));
            }
            captured_scope = Some(
                engine
                    .module_scope(engine.resolve_module_scope("scope").unwrap())
                    .unwrap()
                    .clone(),
            );
        }
        positions.push(position);
        assert_eq!(
            engine.module_lights(lfo).unwrap().waveform_position,
            Some(0.2)
        );
    }
    assert!(positions.iter().all(|position| *position > 0.1));
    assert!(
        (positions[3] - positions[0]).abs() > 0.02,
        "the actual LFO must move the displayed table position over time"
    );

    let mut unmodulated = patch.clone();
    unmodulated
        .cables
        .retain(|cable| !(cable.to == "voice" && cable.input == "position_cv"));
    let mut static_engine = unmodulated.compile().unwrap();
    static_engine.process_audio_block_with_taps(
        &mut audio,
        &[None::<openpatch_core::ResolvedTap>],
        &mut taps,
    );
    let fixed = static_engine
        .module_lights(static_engine.resolve_module_lights("voice").unwrap())
        .unwrap();
    assert_eq!(fixed.waveform_position, Some(0.1));
    assert!(audio.iter().flatten().any(|value| value.abs() > 0.001));
    let scope = captured_scope.unwrap();
    assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
    assert!(scope.x[0]
        .iter()
        .any(|point| point.min.is_finite() && point.max.abs() > 0.001));
    assert!(scope.y[0]
        .iter()
        .any(|point| point.min.is_finite() && point.max.abs() > 0.001));
}

#[test]
fn wavetable_factory_menu_layout_and_actual_snapshot_fit_both_room_captures() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = host(directory.path());
    app.replace_now(wavetable_factory_patch(), "Wavetable voice");
    let mut expected = Rack::default();
    expected
        .restore_layout(&OpenPatch::wavetable_voice_layout())
        .unwrap();
    assert_eq!(app.rack.layout(), expected.layout());
    let layout = app.rack.layout();
    for node in &app.patch.nodes {
        let position = &layout["positions"][&node.id];
        let x = position[0].as_f64().unwrap();
        let y = position[1].as_f64().unwrap();
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
    app.prepare_wavetable_fixture();
    assert!(app.audio.is_none());
    assert_eq!(
        app.patch.reference_view(),
        wavetable_factory_patch().reference_view()
    );
    let revision = app.revision;
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    for room in [1.0, 0.0] {
        app.rack.set_room_brightness(room);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1320.0, 880.0),
                )),
                focused: true,
                ..Default::default()
            },
            |ctx| app.rack_workspace(ctx, None),
        );
        for label in ["WAVETABLE VCO", "WAVETABLE LFO", "VCA", "ADSR EG", "SCOPE"] {
            preset_text(&output, label);
        }
        assert_eq!(app.revision, revision);
    }
}

#[test]
fn wavetable_library_search_shows_both_shared_faceplates() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = host(directory.path());
    app.library = ModuleLibrary::default();
    app.library.set_preview_zoom(0.5);
    app.browse();
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut output = None;
    for (time, events) in [
        (0.0, vec![]),
        (0.2, vec![egui::Event::Text("Wavetable".into())]),
        (0.4, vec![]),
    ] {
        output = Some(ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1320.0, 880.0),
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
    let output = output.unwrap();
    preset_text(&output, "3 modules");
    preset_text(&output, "WAVETABLE VCO");
    preset_text(&output, "WAVETABLE LFO");
    assert!(app.browser_open);
}
