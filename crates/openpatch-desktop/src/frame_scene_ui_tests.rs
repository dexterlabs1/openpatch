use super::*;

fn scene_patch(keys: Vec<openpatch_core::FrameSceneKeyframe>) -> Patch {
    let mut patch: Patch = serde_json::from_value(
        json!({"version":2,"nodes":[{"id":"scene","kind":"frame_scene","params":{"frame":0.4}}]}),
    )
    .unwrap();
    if !keys.is_empty() {
        patch.nodes[0].content = Some(openpatch_core::ModuleContent::FrameScene(
            openpatch_core::FrameSceneData {
                next_id: 20,
                keyframes: keys,
            },
        ));
    }
    patch
}
fn scene_key(id: u16, timestamp: u16, values: [u16; 4]) -> openpatch_core::FrameSceneKeyframe {
    openpatch_core::FrameSceneKeyframe {
        id,
        timestamp,
        values,
    }
}
fn scene_text(shape: &egui::Shape, texts: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for shape in children {
                scene_text(shape, texts)
            }
        }
        egui::Shape::Text(text) => texts.push((
            text.galley.text().into(),
            text.galley.rect.translate(text.pos.to_vec2()),
        )),
        _ => {}
    }
}
#[test]
fn frame_scene_shared_faceplate_keeps_five_large_controls_timeline_and_ports_separate() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "frame_scene")
        .unwrap();
    for zoom in [0.25, 0.42, 0.85, 1.0, 1.4] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut preview = ModulePreview::new(module);
        let bounds = Rect::from_min_size(pos2(24.0, 12.0), preview.size() * zoom);
        let render = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 800.0))),
            ..Default::default()
        };
        let _ = ctx.run(render(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| preview.show(ui, bounds, &catalog));
        });
        let output = ctx.run(render(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| preview.show(ui, bounds, &catalog));
        });
        let mut texts = Vec::new();
        for shape in output.shapes {
            scene_text(&shape.shape, &mut texts);
        }
        for expected in [
            "FRAME SCENE",
            "FRAME",
            "LEVEL 1",
            "LEVEL 2",
            "LEVEL 3",
            "LEVEL 4",
            "MODULATION",
            "ALL NORMAL",
            "AUTHORED  0/64",
            "MANUAL LEVELS",
            "CONFIGURED FRAME 0.0%",
            "FRAME CV",
            "MIX",
            "STEP",
        ] {
            assert!(
                texts.iter().any(|(text, _)| text == expected),
                "{zoom}: missing {expected}"
            );
        }
        for (text, printed) in &texts {
            assert!(
                bounds.expand(0.5).contains_rect(*printed),
                "{zoom}: {text} clips {printed:?}"
            );
        }
        let mut targets = Vec::new();
        for index in 0..6 {
            let (center, radius) = frame_scene_control(index);
            if index < 5 {
                assert!(radius >= 29.0);
            }
            let target = Rect::from_center_size(
                bounds.min + center.to_vec2() * zoom,
                Vec2::splat((radius + 3.0) * 2.0 * zoom),
            );
            assert!(bounds.contains_rect(target));
            targets.push(target);
        }
        let timeline = frame_scene_timeline();
        targets.push(Rect::from_min_max(
            bounds.min + timeline.min.to_vec2() * zoom,
            bounds.min + timeline.max.to_vec2() * zoom,
        ));
        for (index, target) in targets.iter().enumerate() {
            for other in &targets[index + 1..] {
                assert!(!target.intersects(*other));
            }
        }
        for input in [true, false] {
            for index in 0..6 {
                let center = bounds.min + frame_scene_port(input, index).to_vec2() * zoom;
                let radius =
                    if input { 10.3 } else { 13.0 } * port_material_scale("frame_scene") * zoom;
                let rim = Rect::from_center_size(center, Vec2::splat(2.0 * radius));
                assert!(bounds.contains_rect(rim));
                for x in [20.0, 400.0] {
                    for y in [21.0, 699.0] {
                        let screw =
                            bounds.min + vec2(x * 540.0 / 420.0, y * PANEL_HEIGHT / 720.0) * zoom;
                        assert!(
                            center.distance(screw) > radius + (4.7 + 1.5) * zoom,
                            "{zoom}: port {input}/{index} crowds a mounting fastener"
                        );
                    }
                }
                for target in &targets {
                    assert!(!rim.intersects(*target));
                }
                for (text, printed) in &texts {
                    assert!(
                        !rim.intersects(*printed),
                        "{zoom}: {text} overlaps port {input}/{index}"
                    );
                }
            }
        }
    }
}
#[test]
fn frame_scene_selected_level_drag_edits_stable_key_values_with_revision_and_one_history_group() {
    let ctx = egui::Context::default();
    let patch = scene_patch(vec![
        scene_key(7, 1000, [10000, 20000, 30000, 40000]),
        scene_key(9, 30000, [50000; 4]),
    ]);
    let mut rack = Rack::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    rack.frame_scene_ui.selected.insert("scene".into(), 7);
    let from = pos2(24.0, 12.0) + frame_scene_control(2).0.to_vec2();
    assert!(frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(from), button(from, true)],
        true,
        true,
        31
    )
    .0
    .is_none());
    let mut groups = Vec::new();
    for (time, distance) in [(1.1, 15.0), (1.2, 25.0)] {
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            time,
            vec![egui::Event::PointerMoved(from - vec2(0.0, distance))],
            true,
            true,
            31,
        )
        .0;
        let Some(RackAction::Command("edit_frame_scene", args)) = action else {
            panic!("selected level must edit authored data")
        };
        assert_eq!(args["expected_revision"], 31);
        assert_eq!(args["edit"]["keyframe_id"], 7);
        let values = args["edit"]["values"].as_array().unwrap();
        assert_eq!(values[0], 10000);
        assert!(values[1].as_u64().unwrap() > 20000);
        assert_eq!(values[2], 30000);
        assert_eq!(values[3], 40000);
        groups.push(args["history_group"].as_str().unwrap().to_owned());
    }
    assert_eq!(groups[0], groups[1]);
}
#[test]
fn frame_scene_without_keys_edits_manual_level_parameters() {
    let ctx = egui::Context::default();
    let patch = scene_patch(vec![]);
    let mut rack = Rack::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let from = pos2(24.0, 12.0) + frame_scene_control(4).0.to_vec2();
    frame(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(from), button(from, true)],
    );
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![egui::Event::PointerMoved(from - vec2(0.0, 15.0))],
        true,
        true,
        8,
    )
    .0;
    assert!(
        matches!(action,Some(RackAction::Command("set_parameter",args))if args["parameter"]=="level4"&&args["expected_revision"]==8&&args["value"].as_f64().unwrap()>0.25)
    );
}
#[test]
fn frame_scene_selection_is_stable_through_reordering_missing_content_and_undo() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = scene_patch(vec![
        scene_key(7, 1000, [1000; 4]),
        scene_key(9, 30000, [2000; 4]),
    ]);
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let pointer = pos2(24.0 + frame_scene_key_x(30000), 12.0 + 200.0);
    assert!(frame(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(pointer), button(pointer, true)]
    )
    .is_none());
    frame(&ctx, &mut rack, &patch, 1.1, vec![button(pointer, false)]);
    assert_eq!(rack.frame_scene_ui.selected["scene"], 9);
    let moved = scene_patch(vec![
        scene_key(9, 50, [2000; 4]),
        scene_key(7, 1000, [1000; 4]),
    ]);
    frame(&ctx, &mut rack, &moved, 2.0, vec![]);
    assert_eq!(rack.frame_scene_ui.selected["scene"], 9);
    frame(&ctx, &mut rack, &scene_patch(vec![]), 3.0, vec![]);
    assert_eq!(rack.frame_scene_ui.selected["scene"], 9);
    frame(&ctx, &mut rack, &patch, 4.0, vec![]);
    assert_eq!(rack.frame_scene_ui.selected["scene"], 9);
    assert_eq!(
        patch.nodes[0].params["frame"], 0.4,
        "selection never moves the DSP frame"
    );
}
#[test]
fn frame_scene_timeline_timestamp_drag_addresses_id_and_reuses_history_across_reordering() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut patch = scene_patch(vec![
        scene_key(7, 1000, [1000; 4]),
        scene_key(9, 30000, [2000; 4]),
    ]);
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let from = pos2(24.0 + frame_scene_key_x(1000), 12.0 + 200.0);
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(from), button(from, true)],
        true,
        true,
        12,
    );
    let mut groups = Vec::new();
    for (time, distance, revision) in [(1.1, 180.0, 12), (1.2, 200.0, 13)] {
        let action = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            time,
            vec![egui::Event::PointerMoved(from + vec2(distance, 0.0))],
            true,
            true,
            revision,
        )
        .0;
        let Some(RackAction::Command("edit_frame_scene", args)) = action else {
            panic!("tick drag must move the authored timestamp")
        };
        assert_eq!(args["edit"]["keyframe_id"], 7);
        assert_eq!(args["expected_revision"], revision);
        assert!(args["edit"]["timestamp"].as_u64().unwrap() > 30000);
        groups.push(args["history_group"].as_str().unwrap().to_owned());
        let edit: openpatch_core::FrameSceneEdit =
            serde_json::from_value(args["edit"].clone()).unwrap();
        let Some(openpatch_core::ModuleContent::FrameScene(data)) = patch.nodes[0].content.as_mut()
        else {
            unreachable!()
        };
        data.apply(edit).unwrap();
        assert_eq!(data.keyframes[1].id, 7, "the moving ID survives sorting");
    }
    assert_eq!(groups[0], groups[1]);
}

#[test]
fn frame_scene_timeline_cancels_when_external_content_replaces_the_same_id_during_a_drag() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = scene_patch(vec![
        scene_key(7, 1000, [1000; 4]),
        scene_key(9, 30000, [2000; 4]),
    ]);
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let from = pos2(24.0 + frame_scene_key_x(1000), 12.0 + 200.0);
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(from), button(from, true)],
        true,
        true,
        12,
    );
    assert!(matches!(
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![egui::Event::PointerMoved(from + vec2(30.0, 0.0))],
            true,
            true,
            12
        )
        .0,
        Some(RackAction::Command("edit_frame_scene", _))
    ));
    let replaced = scene_patch(vec![
        scene_key(7, 2000, [50000; 4]),
        scene_key(9, 30000, [2000; 4]),
    ]);
    for (time, distance) in [(1.2, 40.0), (1.3, 50.0)] {
        assert!(
            frame_revision_state(
                &ctx,
                &mut rack,
                &replaced,
                time,
                vec![egui::Event::PointerMoved(from + vec2(distance, 0.0))],
                true,
                true,
                13
            )
            .0
            .is_none(),
            "cached drag must not overwrite an external same-ID replacement"
        );
    }
    assert!(rack.frame_scene_ui.drag.is_none());
}

#[test]
fn frame_scene_selected_level_cancels_when_external_content_replaces_the_same_id_during_a_drag() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = scene_patch(vec![scene_key(7, 1000, [10000; 4])]);
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let from = pos2(24.0, 12.0) + frame_scene_control(2).0.to_vec2();
    frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(from), button(from, true)],
        true,
        true,
        12,
    );
    assert!(matches!(
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![egui::Event::PointerMoved(from - vec2(0.0, 20.0))],
            true,
            true,
            12
        )
        .0,
        Some(RackAction::Command("edit_frame_scene", _))
    ));
    let replaced = scene_patch(vec![scene_key(7, 1000, [50000; 4])]);
    for (time, distance) in [(1.2, 30.0), (1.3, 40.0)] {
        assert!(
            frame_revision_state(
                &ctx,
                &mut rack,
                &replaced,
                time,
                vec![egui::Event::PointerMoved(from - vec2(0.0, distance))],
                true,
                true,
                13
            )
            .0
            .is_none(),
            "stored-level motion must not overwrite an external same-ID replacement"
        );
    }
}

#[test]
fn frame_scene_playhead_and_lamps_only_use_published_positive_telemetry() {
    let patch = scene_patch(vec![]);
    let node = &patch.nodes[0];
    assert_eq!(frame_scene_position(node, None), (0.4, false));
    for index in 0..8 {
        assert_eq!(frame_scene_lamp(None, index), 0.0);
    }
    let mut lights = ModuleLightState {
        count: 8,
        waveform_position: Some(0.9),
        ..Default::default()
    };
    for index in 0..8 {
        lights.brightness[index] = [1.0, index as f32 / 8.0];
        assert_eq!(frame_scene_lamp(Some(&lights), index), index as f32 / 8.0);
    }
    assert_eq!(frame_scene_position(node, Some(&lights)), (0.9, true));
    lights.waveform_position = None;
    assert_eq!(frame_scene_position(node, Some(&lights)), (0.4, false));
    lights.count = 4;
    assert_eq!(frame_scene_lamp(Some(&lights), 7), 0.0);
    // The negative component cannot illuminate these unipolar gain/RGB/step lamps.
    lights.brightness[0] = [1.0, 0.0];
    assert_eq!(frame_scene_lamp(Some(&lights), 0), 0.0);
}

#[test]
fn frame_scene_add_stores_configured_position_and_authored_levels_independent_of_live_cv() {
    let ctx = egui::Context::default();
    let patch = scene_patch(vec![]);
    let mut rack = Rack::default();
    rack.module_lights.insert(
        "scene".into(),
        ModuleLightState {
            count: 8,
            waveform_position: Some(0.9),
            ..Default::default()
        },
    );
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let target = pos2(24.0 + 313.0, 12.0 + 178.0);
    frame(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(target), button(target, true)],
    );
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.1,
        vec![button(target, false)],
        true,
        true,
        19,
    )
    .0;
    let Some(RackAction::Command("edit_frame_scene", args)) = action else {
        panic!("ADD must store an authored frame")
    };
    assert_eq!(args["expected_revision"], 19);
    assert_eq!(args["edit"]["operation"], "add");
    assert_eq!(args["edit"]["timestamp"], 26214);
    assert_eq!(args["edit"]["values"], json!([65535, 49151, 32768, 16384]));
    assert!(args.get("history_group").is_none());
}

#[test]
fn frame_scene_drawn_lamp_stays_dark_for_negative_components_and_uses_actual_positive_gain() {
    let ctx = egui::Context::default();
    let patch = scene_patch(vec![]);
    let mut rack = Rack::default();
    let center = pos2(
        24.0 + frame_scene_port(false, 2).x + 25.0,
        12.0 + frame_scene_port(false, 2).y,
    );
    let lens = |output: egui::FullOutput| {
        output
            .shapes
            .into_iter()
            .find_map(|s| match s.shape {
                egui::Shape::Circle(c)
                    if c.center.distance(center) < 0.01
                        && (c.radius - crate::lighting::lamp_radius(2.2)).abs() < 0.01 =>
                {
                    Some(c.fill)
                }
                _ => None,
            })
            .expect("gain lens is drawn")
    };
    let dark = lens(frame_output(&ctx, &mut rack, &patch, 0.0, vec![]).1);
    let mut lights = ModuleLightState {
        count: 8,
        ..Default::default()
    };
    lights.brightness[2] = [1.0, 0.0];
    rack.module_lights.insert("scene".into(), lights);
    assert_eq!(
        lens(frame_output(&ctx, &mut rack, &patch, 1.0, vec![]).1),
        dark
    );
    lights.brightness[2] = [0.0, 0.75];
    rack.module_lights.insert("scene".into(), lights);
    assert_ne!(
        lens(frame_output(&ctx, &mut rack, &patch, 2.0, vec![]).1),
        dark
    );
}

#[test]
fn frame_scene_mcp_receipts_anchor_native_rotaries_without_replacing_authoritative_values() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut before = scene_patch(vec![]);
    let mut after = before.clone();
    let names = [
        "frame",
        "level1",
        "level2",
        "level3",
        "level4",
        "modulation",
    ];
    let changes: Vec<_> = names.iter().map(|name| {
        let (old, new) = if *name == "modulation" { (-0.5, 0.5) } else { (0.25, 0.75) };
        before.nodes[0].params.insert((*name).into(), old);
        after.nodes[0].params.insert((*name).into(), new);
        json!({"id":"scene","kind":"frame_scene","parameter":name,"old_effective":old,"new_effective":new})
    }).collect();
    frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
    let now = std::time::Instant::now();
    let receipt = json!({"plan_id":25,"completed_steps":1,"last_revision":10,"current_revision":10,"last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":25,"step_index":0,"before_revision":9,"revision":10,"label":"Shape scene","new_modules":[],"new_cables":[],"parameter_changes":changes}});
    rack.observe_construction_transition_at(
        &before,
        &after,
        9,
        10,
        &receipt,
        crate::construction_motion::MotionMode::Full,
        now,
    );
    rack.set_construction_clock_override(Some(now + std::time::Duration::from_millis(150)));
    let (action, output) =
        frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10);
    assert!(action.is_none());
    assert_eq!(rack.construction_anchors.len(), 6);
    for name in names {
        let anchor = &rack.construction_anchors[&("scene".into(), name.into())];
        assert!(
            matches!(anchor.marker, Some(crate::construction_motion::Marker::Rotary { old, new, .. }) if (old-0.25).abs()<1e-5 && (new-0.75).abs()<1e-5)
        );
        assert_eq!(
            anchor.new_text,
            if name == "modulation" { "+50%" } else { "+75%" }
        );
    }
    let mut texts = Vec::new();
    for shape in &output.shapes {
        scene_text(&shape.shape, &mut texts);
    }
    assert!(texts
        .iter()
        .any(|(text, _)| text == "CONFIGURED FRAME 75.0%"));
    assert_eq!(after.nodes[0].params["frame"], 0.75);
    // Level parameters no longer represent the knob values when content exists.
    let mut keyed = after.clone();
    keyed.nodes[0].content = Some(openpatch_core::ModuleContent::FrameScene(
        openpatch_core::FrameSceneData {
            next_id: 2,
            keyframes: vec![scene_key(1, 100, [10000; 4])],
        },
    ));
    frame_revision_state(&ctx, &mut rack, &keyed, 0.2, vec![], true, true, 10);
    assert_eq!(rack.construction_anchors.len(), 2);
    assert!(rack
        .construction_anchors
        .contains_key(&("scene".into(), "frame".into())));
    assert!(rack
        .construction_anchors
        .contains_key(&("scene".into(), "modulation".into())));
}
