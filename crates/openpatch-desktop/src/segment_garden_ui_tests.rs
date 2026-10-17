use super::*;

fn garden_patch(params: Value, gates: &[usize]) -> Patch {
    let cables: Vec<_> = gates
        .iter()
        .map(|i| json!({"from":"dc","output":"out","to":"garden","input":format!("gate{i}")}))
        .collect();
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"garden","kind":"segment_garden","params":params}],"cables":cables})).unwrap()
}
fn garden_text(shape: &egui::Shape, texts: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                garden_text(child, texts);
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
fn segment_garden_grouping_uses_physical_gate_cables_and_leaves_the_prefix_independent() {
    let patch = garden_patch(json!({}), &[3, 5]);
    let groups = segment_garden_groups(&patch.nodes[0], &patch);
    assert_eq!(
        groups
            .iter()
            .map(|g| (g.first, g.end, g.triggered))
            .collect::<Vec<_>>(),
        vec![(0, 1, false), (1, 2, false), (2, 4, true), (4, 6, true)]
    );
    let unpatched = garden_patch(json!({}), &[]);
    assert_eq!(
        segment_garden_groups(&unpatched.nodes[0], &unpatched).len(),
        6
    );
    // No voltage/telemetry is present. A physically patched 0 V still starts a group.
    let one = garden_patch(json!({}), &[1]);
    assert_eq!(segment_garden_groups(&one.nodes[0], &one)[0].end, 6);
}
#[test]
fn segment_garden_roles_follow_saved_types_loops_groups_and_sequencer_detection() {
    let p = garden_patch(json!({"type2":1,"type3":2,"loop3":1}), &[]);
    let groups = segment_garden_groups(&p.nodes[0], &p);
    assert_eq!(
        segment_garden_roles(&p.nodes[0], groups[0], 0),
        ("RATE", "SHAPE")
    );
    assert_eq!(
        segment_garden_roles(&p.nodes[0], groups[1], 1),
        ("LEVEL", "GLIDE")
    );
    assert_eq!(
        segment_garden_roles(&p.nodes[0], groups[2], 2),
        ("LEVEL", "DELAY")
    );
    for (params, roles) in [
        (json!({"loop1":0}), ("TIME", "SHAPE")),
        (json!({"loop1":1}), ("DIVISION", "SHAPE")),
        (json!({"type1":2,"loop1":0}), ("LEVEL", "TIME")),
        (json!({"type1":2,"loop1":1}), ("LEVEL", "CHANCE")),
    ] {
        let p = garden_patch(params, &[1, 2]);
        assert_eq!(
            segment_garden_roles(&p.nodes[0], segment_garden_groups(&p.nodes[0], &p)[0], 0),
            roles
        );
    }
    let p = garden_patch(json!({}), &[1]);
    let group = segment_garden_groups(&p.nodes[0], &p)[0];
    assert_eq!(
        segment_garden_roles(&p.nodes[0], group, 0),
        ("TIME", "SHAPE")
    );
    assert_eq!(
        segment_garden_roles(&p.nodes[0], group, 1),
        ("TIME", "LEVEL")
    );
    assert_eq!(
        segment_garden_roles(&p.nodes[0], group, 5),
        ("TIME", "SHAPE")
    );
    for leader in [0, 2] {
        let p = garden_patch(
            json!({"type1":leader,"loop1":0,"type2":1,"type3":1,"type4":1,"type5":1,"type6":1}),
            &[1],
        );
        let group = segment_garden_groups(&p.nodes[0], &p)[0];
        assert!(group.sequencer);
        assert_eq!(
            segment_garden_roles(&p.nodes[0], group, 0),
            ("RESET", "DIRECTION")
        );
        assert_eq!(
            segment_garden_roles(&p.nodes[0], group, 1),
            ("LEVEL", "GLIDE")
        );
    }
    let p = garden_patch(json!({"type2":1,"type3":1}), &[1, 4]);
    assert!(
        !segment_garden_groups(&p.nodes[0], &p)[0].sequencer,
        "leader LOOP prevents sequence mode"
    );
}
#[test]
fn segment_garden_mixed_group_hold_and_first_ramp_labels_follow_source_roles() {
    for (params, gates, stage, expected) in [
        // Both ends of a section loop: the Hold retains its timed transition.
        (
            json!({"loop1":1,"type3":2,"loop3":1}),
            vec![1],
            2,
            ("LEVEL", "TIME"),
        ),
        // One Hold loop point disables its secondary timing control.
        (
            json!({"loop1":0,"type3":2,"loop3":1}),
            vec![1],
            2,
            ("LEVEL", "HELD"),
        ),
        // Only a triggered singleton Hold uses gate-generator probability.
        (
            json!({"type1":2,"loop1":1}),
            vec![1, 2],
            0,
            ("LEVEL", "CHANCE"),
        ),
        // A preceding Step does not make the first Ramp an internal level stage.
        (json!({"type1":1}), vec![1], 1, ("TIME", "SHAPE")),
        (json!({"type1":1}), vec![1], 2, ("TIME", "LEVEL")),
    ] {
        let patch = garden_patch(params, &gates);
        let node = &patch.nodes[0];
        let group = segment_garden_groups(node, &patch)
            .into_iter()
            .find(|g| stage >= g.first && stage < g.end)
            .unwrap();
        assert_eq!(segment_garden_roles(node, group, stage), expected);
    }
}
#[test]
fn segment_garden_shared_preview_keeps_readable_controls_ports_and_fasteners_clear() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "segment_garden")
        .unwrap();
    assert_eq!(panel_width("segment_garden"), 720.0);
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
        let mut texts = Vec::new();
        for shape in &output.unwrap().shapes {
            garden_text(&shape.shape, &mut texts);
        }
        for expected in [
            "SEGMENT GARDEN",
            "CONFIGURED 1 · FREE",
            "CONFIGURED 6 · FREE",
            "RAMP",
            "STEP",
            "HOLD",
            "RATE",
            "TIME",
            "SHAPE",
            "GATE",
            "CV",
            "OUT",
            "ACTIVE",
            "SEED",
            "EDGE",
            "50.0%",
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
        for stage in 1..=6 {
            for prefix in ["level", "shape"] {
                let name = format!("{prefix}{stage}");
                let (center, radius) = segment_garden_control(&name);
                let GenericParameterHardware::Rotary { target_half, .. } =
                    generic_parameter_hardware("segment_garden", &name, radius, false)
                else {
                    panic!("must be rotary")
                };
                targets.push(Rect::from_center_size(
                    bounds.min + center.to_vec2() * zoom,
                    Vec2::splat(2.0 * target_half * zoom),
                ));
                targets.push(Rect::from_center_size(
                    bounds.min + segment_garden_value_center(&name).to_vec2() * zoom,
                    vec2(82.0, 19.0) * zoom,
                ));
            }
            targets.push(Rect::from_min_size(
                bounds.min + vec2((stage - 1) as f32 * 120.0 + 8.0, 78.0) * zoom,
                vec2(104.0, 20.0) * zoom,
            ));
            targets.push(Rect::from_min_size(
                bounds.min + vec2((stage - 1) as f32 * 120.0 + 34.0, 103.0) * zoom,
                vec2(52.0, 12.0) * zoom,
            ));
        }
        for (i, target) in targets.iter().enumerate() {
            assert!(bounds.contains_rect(*target));
            for other in &targets[i + 1..] {
                assert!(
                    !target.intersects(*other),
                    "{zoom}: interactive controls overlap"
                );
            }
        }
        let mut rims = Vec::new();
        for input in [true, false] {
            for i in 0..12 {
                let center = bounds.min + segment_garden_port(input, i).to_vec2() * zoom;
                let radius =
                    if input { 10.3 } else { 13.0 } * port_material_scale("segment_garden") * zoom;
                let rim = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
                assert!(bounds.contains_rect(rim));
                for target in &targets {
                    assert!(!target.intersects(rim), "{zoom}: port overlaps control");
                }
                for (text, printed) in &texts {
                    assert!(
                        !printed.intersects(rim),
                        "{zoom}: {text} overlaps {input}/{i}"
                    );
                }
                for x in [20.0, 400.0] {
                    for y in [21.0, 699.0] {
                        let screw =
                            bounds.min + vec2(x * 720.0 / 420.0, y * PANEL_HEIGHT / 720.0) * zoom;
                        assert!(
                            (center - screw).length() > radius + 4.9 * zoom,
                            "{zoom}: jack overlaps seated fastener"
                        );
                    }
                }
                rims.push(rim);
            }
        }
        for (i, rim) in rims.iter().enumerate() {
            for other in &rims[i + 1..] {
                assert!(!rim.intersects(*other), "{zoom}: jack rims overlap");
            }
        }
    }
}
#[test]
fn segment_garden_primary_and_secondary_rotaries_share_native_gesture_authority() {
    for name in ["level1", "shape6"] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = garden_patch(json!({}), &[]);
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let from = pos2(24.0, 12.0) + segment_garden_control(name).0.to_vec2();
        assert!(frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(from), button(from, true)],
            true,
            true,
            17
        )
        .0
        .is_none());
        let mut groups = Vec::new();
        for (time, distance) in [(1.1, 20.0), (1.2, 35.0)] {
            let Some(RackAction::Command(command, args)) = frame_revision_state(
                &ctx,
                &mut rack,
                &patch,
                time,
                vec![egui::Event::PointerMoved(from - vec2(0.0, distance))],
                true,
                true,
                17,
            )
            .0
            else {
                panic!("rotary must edit")
            };
            assert_eq!(command, "set_parameter");
            assert_eq!(args["parameter"], name);
            assert_eq!(args["expected_revision"], 17);
            assert!(args["value"].as_f64().unwrap() > 0.5);
            groups.push(args["history_group"].as_str().unwrap().to_owned());
        }
        assert_eq!(groups[0], groups[1]);
    }
}
#[test]
fn segment_garden_type_and_loop_selectors_emit_ordinary_revision_checked_edits() {
    for (point, name, value) in [
        (pos2(60.0, 88.0), "type1", 1),
        (pos2(60.0, 109.0), "loop1", 0),
    ] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = garden_patch(json!({}), &[]);
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let point = point + vec2(24.0, 12.0);
        frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.0,
            vec![egui::Event::PointerMoved(point), button(point, true)],
            true,
            true,
            23,
        );
        let Some(RackAction::Command(command, args)) = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            1.1,
            vec![button(point, false)],
            true,
            true,
            23,
        )
        .0
        else {
            panic!("selector must edit")
        };
        assert_eq!(command, "set_parameter");
        assert_eq!(args["parameter"], name);
        assert_eq!(args["value"], value);
        assert_eq!(args["expected_revision"], 23);
    }
}
#[test]
fn segment_garden_lamps_require_real_complete_telemetry_and_preserve_bipolar_output_components() {
    let ctx = egui::Context::default();
    let patch = garden_patch(json!({}), &[]);
    let node = &patch.nodes[0];
    let mut rack = Rack::default();
    let circles = |rack: &Rack| {
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                rack.paint_output_signal_light(ui.painter(), node, 2, pos2(100.0, 100.0))
            });
        });
        output
            .shapes
            .into_iter()
            .filter_map(|s| match s.shape {
                egui::Shape::Circle(c)
                    if (c.radius - crate::lighting::lamp_radius(2.2)).abs() < 0.01 =>
                {
                    Some((c.center, c.fill))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let dark = circles(&rack);
    assert_eq!(dark.len(), 2);
    assert_eq!(segment_garden_lamp(None, 6), 0.0);
    let mut lights = ModuleLightState {
        count: 13,
        ..Default::default()
    };
    lights.brightness[2] = [0.0, 0.75];
    lights.brightness[8] = [0.0, 1.0];
    lights.brightness[12] = [1.0, 0.0];
    assert_eq!(segment_garden_lamp(Some(&lights), 8), 1.0);
    assert_eq!(segment_garden_lamp(Some(&lights), 12), 1.0);
    lights.count = 6;
    assert_eq!(segment_garden_lamp(Some(&lights), 8), 0.0);
    lights.count = 13;
    rack.module_lights.insert(node.id.clone(), lights);
    let active = circles(&rack);
    assert_eq!(active[0], dark[0]);
    assert_eq!(active[1].0, dark[1].0);
    assert_ne!(active[1].1, dark[1].1);
}
#[test]
fn segment_garden_mcp_motion_anchors_the_real_rotaries_and_configuration_controls() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let mut before = garden_patch(json!({}), &[]);
    let mut after = before.clone();
    let changes: Vec<_> = [("level2", 0.25, 0.75), ("shape5", 0.25, 0.75), ("type3", 0.0, 2.0), ("loop4", 0.0, 1.0), ("seed", 1.0, 42.0)].into_iter().map(|(name, old, new)| {
        before.nodes[0].params.insert(name.into(), old);
        after.nodes[0].params.insert(name.into(), new);
        json!({"id":"garden","kind":"segment_garden","parameter":name,"old_effective":old,"new_effective":new})
    }).collect();
    frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
    let now = std::time::Instant::now();
    let receipt = json!({"plan_id":26,"completed_steps":1,"last_revision":10,"current_revision":10,"last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":26,"step_index":0,"before_revision":9,"revision":10,"label":"Shape segments","new_modules":[],"new_cables":[],"parameter_changes":changes}});
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
    assert!(
        frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10)
            .0
            .is_none()
    );
    for name in ["level2", "shape5", "type3", "loop4", "seed"] {
        let anchor = &rack.construction_anchors[&("garden".into(), name.into())];
        assert_ne!(anchor.old_text, anchor.new_text);
        if name.starts_with("level") || name.starts_with("shape") {
            assert!(matches!(
                anchor.marker,
                Some(crate::construction_motion::Marker::Rotary { .. })
            ));
        }
    }
}
