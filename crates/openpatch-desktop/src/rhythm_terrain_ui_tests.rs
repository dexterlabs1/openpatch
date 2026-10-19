use super::*;
fn terrain_patch(params: Value) -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"terrain","kind":"rhythm_terrain","params":params}],"cables":[]})).unwrap()
}
fn terrain_text(shape: &egui::Shape, texts: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => children.iter().for_each(|s| terrain_text(s, texts)),
        egui::Shape::Text(t) => texts.push((
            t.galley.text().into(),
            t.galley.rect.translate(t.pos.to_vec2()),
        )),
        _ => {}
    }
}
fn terrain_texts(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    let mut texts = Vec::new();
    for shape in &output.shapes {
        terrain_text(&shape.shape, &mut texts);
    }
    texts
}
#[test]
fn rhythm_terrain_shared_preview_has_seven_knobs_six_selectors_sixteen_ports_and_clear_print_at_five_scales(
) {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "rhythm_terrain")
        .unwrap();
    assert_eq!(panel_width("rhythm_terrain"), 720.0);
    assert_eq!(PANEL_HEIGHT, 396.0);
    assert_eq!(
        module["inputs"],
        json!(["clock", "reset", "x_cv", "y_cv", "chaos_cv", "fill1_cv", "fill2_cv", "fill3_cv"])
    );
    assert_eq!(
        module["outputs"],
        json!(["trig1", "trig2", "trig3", "aux1", "aux2", "aux3", "clock", "random"])
    );
    assert_eq!(module["params"]["seed"]["max"], 65535.0);
    for zoom in [0.25, 0.42, 0.85, 1.0, 1.4] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut preview = ModulePreview::new(module);
        let bounds = Rect::from_min_size(pos2(24.0, 12.0), preview.size() * zoom);
        let mut output = None;
        for _ in 0..2 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .show(ctx, |ui| preview.show(ui, bounds, &catalog));
                },
            ));
        }
        let texts = terrain_texts(&output.unwrap());
        for expected in [
            "RHYTHM TERRAIN",
            "MAP X",
            "MAP Y",
            "RANDOMNESS",
            "FILL 1",
            "FILL 2",
            "FILL 3",
            "TEMPO",
            "DRUM MAP",
            "24 PPQN",
            "TRIGGERS",
            "INDIVIDUAL",
            "TAP TEMPO",
            "SEED",
            "TAP",
            "RELEASE TAP",
            "CLOCK UNAVAILABLE",
            "NEXT —",
            "BD",
            "SD",
            "HH",
            "ACC 1",
            "ACC 2",
            "ACC 3",
            "NATIVE CLK",
            "NATIVE RND",
        ] {
            assert!(
                texts.iter().any(|(t, _)| t == expected),
                "{zoom}: missing {expected}"
            );
        }
        for (text, printed) in &texts {
            assert!(
                bounds.expand(0.5).contains_rect(*printed),
                "{zoom}: {text} clips {printed:?}"
            );
        }
        let targets = RHYTHM_TERRAIN_KNOBS
            .into_iter()
            .flat_map(|name| {
                let (center, radius) = rhythm_terrain_control(name);
                let GenericParameterHardware::Rotary { target_half, .. } =
                    generic_parameter_hardware("rhythm_terrain", name, radius, false)
                else {
                    panic!("rotary")
                };
                [
                    Rect::from_center_size(center, Vec2::splat(2.0 * target_half)),
                    Rect::from_center_size(rhythm_terrain_value_center(name), vec2(84.0, 19.0)),
                ]
            })
            .chain(
                [
                    "mode",
                    "resolution",
                    "gate_mode",
                    "alternate",
                    "swing",
                    "tap_tempo",
                ]
                .into_iter()
                .map(|n| rhythm_terrain_selector(n).unwrap().0),
            )
            .chain([
                rhythm_terrain_seed_rect(),
                rhythm_terrain_button_rect(false),
                rhythm_terrain_button_rect(true),
            ])
            .map(|r| Rect::from_min_size(bounds.min + r.min.to_vec2() * zoom, r.size() * zoom))
            .collect::<Vec<_>>();
        let screws = [20.0, 400.0]
            .into_iter()
            .flat_map(|x| {
                [21.0, 699.0]
                    .map(|y| bounds.min + vec2(x * 720.0 / 420.0, y * 396.0 / 720.0) * zoom)
            })
            .collect::<Vec<_>>();
        for (i, target) in targets.iter().enumerate() {
            assert!(bounds.contains_rect(*target));
            for other in &targets[i + 1..] {
                assert!(!target.intersects(*other), "{zoom}: targets overlap");
            }
            for screw in &screws {
                assert!(
                    !target.intersects(Rect::from_center_size(*screw, Vec2::splat(9.8 * zoom))),
                    "screw overlap"
                );
            }
        }
        let mut rims = Vec::new();
        for input in [true, false] {
            for i in 0..8 {
                let center = bounds.min + rhythm_terrain_port(input, i).to_vec2() * zoom;
                let radius =
                    if input { 10.3 } else { 13.0 } * port_material_scale("rhythm_terrain") * zoom;
                let rim = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
                assert!(bounds.contains_rect(rim));
                for target in &targets {
                    assert!(!target.intersects(rim), "control / port overlap");
                }
                for (text, printed) in &texts {
                    assert!(
                        !printed.intersects(rim),
                        "{zoom}: {text} overlaps {input}/{i}"
                    );
                }
                for screw in &screws {
                    assert!(center.distance(*screw) > radius + 4.9 * zoom);
                }
                rims.push(rim);
            }
        }
        for (i, rim) in rims.iter().enumerate() {
            for other in &rims[i + 1..] {
                assert!(!rim.intersects(*other));
            }
        }
    }
}
#[test]
fn rhythm_terrain_knob_drag_uses_one_revision_fenced_native_undo_gesture() {
    let ctx = egui::Context::default();
    let patch = terrain_patch(json!({}));
    let mut rack = Rack::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let at = pos2(24.0, 12.0) + rhythm_terrain_control("x").0.to_vec2();
    assert!(frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(at), button(at, true)],
        true,
        true,
        17
    )
    .0
    .is_none());
    let mut groups = Vec::new();
    for (time, distance) in [(1.1, 20.0), (1.2, 35.0)] {
        let Some(RackAction::Command("set_parameter", args)) = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            time,
            vec![egui::Event::PointerMoved(at - vec2(0.0, distance))],
            true,
            true,
            17,
        )
        .0
        else {
            panic!("native gesture")
        };
        assert_eq!(args["parameter"], "x");
        assert_eq!(args["expected_revision"], 17);
        assert!(args["value"].as_f64().unwrap() > 0.5);
        groups.push(args["history_group"].as_str().unwrap().to_owned());
    }
    assert_eq!(groups[0], groups[1]);
}
#[test]
fn rhythm_terrain_seed_precision_is_exact_bounded_and_retains_opening_revision() {
    let ctx = egui::Context::default();
    let patch = terrain_patch(json!({}));
    let mut rack = Rack::default();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 11);
    let at = rhythm_terrain_seed_rect().center() + vec2(24.0, 12.0);
    for (time, pressed) in [(1.0, true), (1.1, false)] {
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            time,
            vec![egui::Event::PointerMoved(at), button(at, pressed)],
            true,
            true,
            11,
        );
    }
    let edit = rack.editing.as_mut().expect("shared seed entry");
    assert_eq!(edit.parameter, "seed");
    assert_eq!(edit.revision, 11);
    assert_eq!(edit.max, 65535.0);
    assert_eq!(edit.raw_max, 65535.0);
    assert_eq!(edit.step, Some(1.0));
    edit.text = "65535".into();
    let enter = || egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let action =
        frame_revision_state(&ctx, &mut rack, &patch, 1.2, vec![enter()], true, true, 11).0;
    assert!(
        matches!(action,Some(RackAction::Command("set_parameter",args)) if args["value"]==65535.0 && args["expected_revision"]==11)
    );
    // Reopen, then let another native edit advance the document before submit.
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        2.0,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        true,
        true,
        11,
    );
    for (time, pressed) in [(3.0, true), (3.1, false)] {
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            time,
            vec![egui::Event::PointerMoved(at), button(at, pressed)],
            true,
            true,
            11,
        );
    }
    rack.editing.as_mut().expect("reopen").text = "12345".into();
    assert!(
        frame_revision_state(&ctx, &mut rack, &patch, 3.2, vec![enter()], true, true, 12)
            .0
            .is_none()
    );
}
#[test]
fn rhythm_terrain_modes_alternate_labels_and_configured_lengths_follow_saved_controls() {
    for (params, labels) in [
        (json!({}), ["BD", "SD", "HH", "ACC 1", "ACC 2", "ACC 3"]),
        (
            json!({"alternate":1}),
            ["BD", "SD", "HH", "COMMON ACC", "CLOCK", "BAR"],
        ),
        (
            json!({"mode":0}),
            ["1", "2", "3", "CYCLE 1", "CYCLE 2", "CYCLE 3"],
        ),
        (
            json!({"mode":0,"alternate":1}),
            ["1", "2", "3", "COMMON CYCLE", "CLOCK", "ALL CYCLE"],
        ),
    ] {
        let patch = terrain_patch(params);
        let node = &patch.nodes[0];
        for (port, label) in ["trig1", "trig2", "trig3", "aux1", "aux2", "aux3"]
            .into_iter()
            .zip(labels)
        {
            assert_eq!(rhythm_terrain_port_label(node, port, false), label);
        }
        assert_eq!(rhythm_terrain_length(node, "x"), 17);
        assert_eq!(rhythm_terrain_length(node, "chaos"), 1);
        if !rhythm_terrain_drums(node) {
            assert_eq!(rhythm_terrain_control_label(node, "chaos"), "LENGTH 3");
        }
    }
    let patch = terrain_patch(json!({"swing":1}));
    assert_eq!(
        rhythm_terrain_control_label(&patch.nodes[0], "chaos"),
        "SWING AMOUNT"
    );
}
#[test]
fn rhythm_terrain_actual_telemetry_uses_only_output_component_and_actual_clock_lock() {
    assert_eq!(rhythm_terrain_status(None), "CLOCK UNAVAILABLE");
    let mut state = ModuleLightState {
        count: 10,
        clock_frequency: Some(2.5),
        waveform_position: Some(7.0 / 32.0),
        ..Default::default()
    };
    assert_eq!(rhythm_terrain_status(Some(&state)), "EFFECTIVE 150.0 BPM");
    state.brightness[8] = [1.0, 0.0];
    assert_eq!(
        rhythm_terrain_status(Some(&state)),
        "EFFECTIVE 150.0 BPM · LOCK"
    );
    state.clock_frequency = None;
    assert_eq!(rhythm_terrain_status(Some(&state)), "EXTERNAL CLOCK · LOCK");
    for index in 0..8 {
        state.brightness[index] = [0.0, 1.0];
        assert_eq!(rhythm_terrain_lamp(Some(&state), index), 0.0);
        state.brightness[index] = [0.8, 0.0];
        assert_eq!(rhythm_terrain_lamp(Some(&state), index), 0.8);
    }
    state.count = 0;
    assert_eq!(rhythm_terrain_lamp(Some(&state), 8), 0.0);
    let ctx = egui::Context::default();
    let patch = terrain_patch(json!({"tempo":480}));
    let mut rack = Rack::default();
    state.count = 10;
    rack.module_lights.insert("terrain".into(), state);
    let (_, out) = frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 1);
    let texts = terrain_texts(&out);
    assert!(texts.iter().any(|(t, _)| t == "NEXT 07"));
    assert!(texts.iter().any(|(t, _)| t == "EXTERNAL CLOCK · LOCK"));
}
#[test]
fn rhythm_terrain_tap_uses_shared_momentary_cleanup_and_runtime_anchor() {
    let ctx = egui::Context::default();
    let patch = terrain_patch(json!({}));
    let mut rack = Rack {
        performance_available: true,
        ..Default::default()
    };
    rack.construction_animation
        .parameters
        .push(crate::construction_motion::ParameterCue {
            id: "terrain".into(),
            kind: "rhythm_terrain".into(),
            parameter: "action".into(),
            old: 0.0,
            new: 1.0,
            started: std::time::Instant::now(),
        });
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 9);
    let anchor = rack
        .construction_anchors
        .get(&("terrain".into(), "action".into()))
        .expect("runtime anchor");
    let at = rhythm_terrain_button_rect(false).center() + vec2(24.0, 12.0);
    assert!(anchor.rect.contains(at));
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(at), button(at, true)],
        true,
        true,
        9,
    )
    .0;
    assert!(
        matches!(action,Some(RackAction::Command("trigger_module",args)) if args["action"]=="tap_press" && args["expected_revision"]==9)
    );
    assert_eq!(rack.held_manual.as_ref().unwrap().release, "tap_release");
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(at, false)],
        false,
        true,
        9,
    )
    .0;
    assert!(
        matches!(action,Some(RackAction::Command("trigger_module",args)) if args["action"]=="tap_release")
    );
}
