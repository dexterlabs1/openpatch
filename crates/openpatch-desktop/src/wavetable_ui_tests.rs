use super::*;

fn wavetable_patch(kind: &str) -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"wt","kind":kind}]})).unwrap()
}

#[test]
fn wavetable_all_controls_have_revision_guarded_numeric_entry() {
    for kind in ["wavetable_vco", "wavetable_lfo"] {
        for index in 0..6 {
            let patch = wavetable_patch(kind);
            let module = openpatch_core::catalog()["modules"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["kind"] == kind)
                .unwrap()
                .clone();
            let parameters = ordered_parameters(&patch.nodes[0], Some(&module));
            let name = parameters[index].0;
            let ctx = egui::Context::default();
            let mut rack = Rack::default();
            frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
            let position = wavetable_parameter_position(index) + vec2(24.0, 12.0 + 27.0);
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.0,
                vec![egui::Event::PointerMoved(position), button(position, true)],
                true,
                true,
                29,
            );
            frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.1,
                vec![button(position, false)],
                true,
                true,
                29,
            );
            let edit = rack
                .editing
                .as_ref()
                .expect("value well opens direct entry");
            assert_eq!(edit.parameter, name);
            assert_eq!(edit.revision, 29);
            frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![], true, true, 29);
            let edit = rack.editing.as_mut().unwrap();
            let (text, expected) = match name {
                "tune" => ("12 st", 12.0),
                "rate" => ("0.25 Hz", -2.0),
                "position" => ("75%", 0.75),
                "fm_amount" => ("-50%", -0.5),
                "position_cv_amount" => ("25%", 0.25),
                _ => ("1", 1.0),
            };
            edit.text = text.into();
            assert!(
                (parameter_edit_raw_value(edit, parsed_parameter_result(edit).unwrap()) - expected)
                    .abs()
                    < 1e-6
            );
            let result = frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                1.3,
                vec![expression_enter()],
                true,
                true,
                29,
            )
            .0;
            let Some(RackAction::Command("set_parameter", arguments)) = result else {
                panic!("{kind} {name}")
            };
            assert_eq!(arguments["parameter"], name);
            assert_eq!(arguments["expected_revision"], 29);
            assert!((arguments["value"].as_f64().unwrap() as f32 - expected).abs() < 1e-6);
        }
    }
}

#[test]
fn wavetable_waveform_uses_raw_assets_geometry_and_actual_cv_position() {
    let mut patch = wavetable_patch("wavetable_lfo");
    let asset = Arc::new(
        openpatch_core::WaveAsset::new(&[0.0, 1.0, 0.0, -1.0, 1.0, 0.0, -1.0, 0.0, 99.0]).unwrap(),
    );
    let id = asset.id().to_owned();
    patch.assets.insert(id.clone(), asset);
    patch.nodes[0].content = Some(openpatch_core::ModuleContent::Wavetable {
        asset_id: id,
        wave_points: 4,
        name: "Original".into(),
    });
    patch.nodes[0].params.insert("position".into(), 0.0);
    let lights = ModuleLightState {
        waveform_position: Some(0.75),
        ..Default::default()
    };
    assert_eq!(
        wavetable_display_position(&patch.nodes[0], Some(&lights)),
        0.75
    );
    let samples = wavetable_display_samples(&patch.nodes[0], &patch, 0.75).unwrap();
    assert_eq!(samples[0], 0.75);
    assert_eq!(samples[40], 0.25);
    assert_eq!(samples[80], -0.75);
    assert_eq!(samples[120], -0.25);
    assert_eq!(samples[160], samples[0]);
    patch.assets.clear();
    assert!(wavetable_display_samples(&patch.nodes[0], &patch, 0.75).is_none());
}

#[test]
fn wavetable_basic_preview_matches_mathematical_factory_samples() {
    for kind in ["wavetable_vco", "wavetable_lfo"] {
        let patch = wavetable_patch(kind);
        let asset = openpatch_core::default_wavetable_asset();
        for wave in 0..4 {
            let samples =
                wavetable_display_samples(&patch.nodes[0], &patch, wave as f32 / 3.0).unwrap();
            for (preview_index, point) in [(0, 0), (40, 256), (80, 512), (120, 768), (160, 0)] {
                assert_eq!(
                    samples[preview_index],
                    asset.sample(wave * 1024 + point).unwrap()
                );
            }
        }
        assert_eq!(panel_width(kind), 24.0 * HP);
        let mut positions = BTreeSet::new();
        for input in 0..4 {
            let position = port_position(kind, true, input, 4, panel_width(kind));
            assert!(positions.insert((position.x as i32, position.y as i32)));
        }
        let output = port_position(kind, false, 0, 1, panel_width(kind));
        assert!(positions.insert((output.x as i32, output.y as i32)));
    }
}

#[test]
fn wavetable_every_jack_is_a_named_connection_target() {
    for kind in ["wavetable_vco", "wavetable_lfo"] {
        let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"wt","kind":kind},{"id":"source","kind":"constant"},
            {"id":"sink","kind":"audio_stereo"}]}))
        .unwrap();
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let source = fixture_jack(&rack, &patch, "source", "out", false);
        for (index, input) in [
            "fm",
            if kind == "wavetable_vco" {
                "sync"
            } else {
                "reset"
            },
            "position_cv",
            if kind == "wavetable_vco" {
                "pitch"
            } else {
                "clock"
            },
        ]
        .into_iter()
        .enumerate()
        {
            let target = fixture_jack(&rack, &patch, "wt", input, true);
            let action = drag(&ctx, &mut rack, &patch, 1.0 + index as f64, source, target);
            assert!(
                matches!(action,Some(RackAction::Command("connect",ref args)) if args["to"]=="wt" && args["input"]==input)
            );
        }
        let output = fixture_jack(&rack, &patch, "wt", "out", false);
        let sink = fixture_jack(&rack, &patch, "sink", "left", true);
        let action = drag(&ctx, &mut rack, &patch, 6.0, output, sink);
        assert!(
            matches!(action,Some(RackAction::Command("connect",ref args)) if args["from"]=="wt" && args["output"]=="out")
        );
    }
}

fn open_wavetable_rate_editor(raw: f32, clocked: bool) -> ParameterEdit {
    let mut patch = wavetable_patch("wavetable_lfo");
    patch.nodes[0].params.insert("rate".into(), raw);
    if clocked {
        patch
            .nodes
            .push(serde_json::from_value(json!({"id":"clock","kind":"constant"})).unwrap());
        patch.cables.push(
            serde_json::from_value(json!({"from":"clock","to":"wt","input":"clock"})).unwrap(),
        );
    }
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 29);
    let position = wavetable_parameter_position(0) + vec2(24.0, 12.0 + 27.0);
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(position), button(position, true)],
        true,
        true,
        29,
    );
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(position, false)],
        true,
        true,
        29,
    );
    rack.editing.take().expect("rate numeric editor")
}

#[test]
fn wavetable_rate_entry_displays_hz_or_clock_multiplier_with_exact_bounds() {
    for clocked in [false, true] {
        let scale = if clocked { 0.5 } else { 1.0 };
        for raw in [-8.0_f32, 1.0, 10.0] {
            let mut edit = open_wavetable_rate_editor(raw, clocked);
            assert_eq!(edit.display_unit, if clocked { "x" } else { "Hz" });
            assert_eq!(edit.min, (-8.0_f32).exp2() * scale);
            assert_eq!(edit.max, 10.0_f32.exp2() * scale);
            assert_eq!(edit.text.parse::<f32>().unwrap(), raw.exp2() * scale);
            assert_eq!(
                parameter_edit_raw_value(&edit, parsed_parameter_result(&edit).unwrap()),
                raw
            );
            edit.text = format!("{} {}", raw.exp2() * scale, edit.display_unit);
            assert_eq!(
                parameter_edit_raw_value(&edit, parsed_parameter_result(&edit).unwrap()),
                raw
            );
            let readout = wavetable_rate_value(raw, clocked);
            assert!(!readout.contains("log"));
        }
        let mut edit = open_wavetable_rate_editor(1.0, clocked);
        edit.text = if clocked { "2x" } else { "4 Hz" }.into();
        assert_eq!(
            parameter_edit_raw_value(&edit, parsed_parameter_result(&edit).unwrap()),
            2.0
        );
        for text in ["0", "-1", "nan", "inf", "1/0"] {
            edit.text = text.into();
            assert!(parsed_parameter_result(&edit).is_err(), "{clocked}: {text}");
        }
        edit.text = (edit.min * 0.5).to_string();
        assert!(parsed_parameter_result(&edit).is_err());
        edit.text = (edit.max * 2.0).to_string();
        assert!(parsed_parameter_result(&edit).is_err());
    }
    let mut edit = open_wavetable_rate_editor(-8.0, false);
    edit.text = wavetable_rate_value(-8.0, false);
    assert_eq!(
        parameter_edit_raw_value(&edit, parsed_parameter_result(&edit).unwrap()),
        -8.0
    );
    assert_eq!(wavetable_parameter_label("rate"), "FREE RATE");
}
