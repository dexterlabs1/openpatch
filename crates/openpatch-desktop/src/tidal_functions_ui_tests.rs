use super::*;

fn text_shapes(shape: &egui::Shape, texts: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for shape in children {
                text_shapes(shape, texts);
            }
        }
        egui::Shape::Text(text) => texts.push((
            text.galley.text().into(),
            text.galley.rect.translate(text.pos.to_vec2()),
        )),
        _ => {}
    }
}
fn tidal_patch(params: Value) -> Patch {
    serde_json::from_value(
        json!({"version":2,"nodes":[{"id":"tidal","kind":"tidal_functions","params":params}]}),
    )
    .unwrap()
}

#[test]
fn tidal_default_shared_preview_keeps_controls_print_and_ports_inside_the_panel() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "tidal_functions")
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
        for shape in &output.shapes {
            text_shapes(&shape.shape, &mut texts);
        }
        for expected in [
            "TIDAL FUNCTIONS",
            "RAMP",
            "OUTPUTS",
            "RANGE",
            "CLOCK ×",
            "LOOP",
            "AMPLITUDE",
            "CONTROL",
            "1/1",
            "FREQUENCY",
            "SLOPE",
            "SHAPE",
            "SMOOTHNESS",
            "SHIFT",
            "SHIFT SCANS FOUR OVERLAPPING LEVELS",
            "V/OCT",
            "SMOOTH",
            "TRIG",
            "CLK",
            "1",
            "2",
            "3",
            "4",
        ] {
            assert!(
                texts.iter().any(|(text, _)| text == expected),
                "{zoom}: missing {expected}"
            );
        }
        for (text, printed) in &texts {
            assert!(
                bounds.expand(0.5).contains_rect(*printed),
                "{zoom}: {text} outside panel: {printed:?}"
            );
        }
        let mut targets = Vec::new();
        for name in ["frequency", "slope", "shape", "smoothness", "shift"] {
            let (center, radius) = tidal_control(name);
            let hardware = generic_parameter_hardware("tidal_functions", name, radius, false);
            let GenericParameterHardware::Rotary { target_half, .. } = hardware else {
                panic!("{name} must be a rotary")
            };
            let target = Rect::from_center_size(
                bounds.min + center.to_vec2() * zoom,
                Vec2::splat(target_half * 2.0 * zoom),
            );
            let value = Rect::from_center_size(
                bounds.min + tidal_value_center(name).to_vec2() * zoom,
                vec2(88.0, 19.0) * zoom,
            );
            assert!(bounds.contains_rect(target) && bounds.contains_rect(value));
            assert!(
                !target.intersects(value),
                "{name} exact-entry target overlaps rotary"
            );
            targets.extend([target, value]);
        }
        for name in ["ramp_mode", "output_mode", "range", "clock_ratio"] {
            let (local, _, _) = tidal_selector(name).unwrap();
            targets.push(Rect::from_min_size(
                bounds.min + local.min.to_vec2() * zoom,
                local.size() * zoom,
            ));
        }
        let mut rims = Vec::new();
        for input in [true, false] {
            for index in 0..if input { 7 } else { 4 } {
                let center = bounds.min
                    + port_position(
                        "tidal_functions",
                        input,
                        index,
                        if input { 7 } else { 4 },
                        420.0,
                    )
                    .to_vec2()
                        * zoom;
                let radius =
                    if input { 10.3 } else { 13.0 } * port_material_scale("tidal_functions") * zoom;
                let rim = Rect::from_center_size(center, Vec2::splat(2.0 * radius));
                assert!(
                    bounds.contains_rect(rim),
                    "{zoom}: port {input}/{index} clips"
                );
                for target in &targets {
                    assert!(!target.intersects(rim), "{zoom}: port overlaps control");
                }
                for (text, printed) in &texts {
                    assert!(
                        !printed.intersects(rim),
                        "{zoom}: {text} overlaps port {input}/{index}"
                    );
                }
                rims.push(rim);
            }
        }
        for (index, target) in targets.iter().enumerate() {
            for other in &targets[index + 1..] {
                assert!(!target.intersects(*other), "{zoom}: controls overlap");
            }
        }
        for (index, rim) in rims.iter().enumerate() {
            for other in &rims[index + 1..] {
                assert!(!rim.intersects(*other), "{zoom}: port rims overlap");
            }
        }
    }
}

#[test]
fn tidal_relationship_print_follows_saved_modes_without_inventing_a_waveform() {
    for (params, expected) in [
        (
            json!({"output_mode":0}),
            "1 SHAPED · 2 RAW · 3 END ATTACK · 4 END RELEASE",
        ),
        (
            json!({"output_mode":2,"ramp_mode":0}),
            "SHIFT SPREADS FOUR RELATED SLOPES",
        ),
        (
            json!({"output_mode":2,"ramp_mode":2}),
            "SHIFT SPREADS FOUR RELATED SLOPES",
        ),
        (
            json!({"output_mode":2,"ramp_mode":1}),
            "SHIFT SPACES FOUR RELATED PHASES",
        ),
        (
            json!({"output_mode":3}),
            "SHIFT SELECTS FOUR RELATED FREQUENCIES",
        ),
    ] {
        let patch = tidal_patch(params);
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut rack = Rack::default();
        for time in [0.0, 30.0] {
            let (_, output) =
                frame_revision_state(&ctx, &mut rack, &patch, time, vec![], true, true, 5);
            let mut texts = Vec::new();
            for shape in &output.shapes {
                text_shapes(&shape.shape, &mut texts);
            }
            assert!(texts.iter().any(|(text, _)| text == expected));
            assert!(rack.module_lights.is_empty());
            assert!(rack.scope_states.is_empty());
        }
    }
}

#[test]
fn tidal_rotary_drag_edits_the_native_parameter_with_one_undo_gesture() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let patch = tidal_patch(json!({}));
    let mut rack = Rack::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    // Rack's first module begins at its standard 24,12 origin.
    let from = pos2(24.0, 12.0) + tidal_control("slope").0.to_vec2();
    let pressed = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(from), button(from, true)],
        true,
        true,
        17,
    )
    .0;
    assert!(pressed.is_none(), "press does not jump the control");
    let mut groups = Vec::new();
    for (time, distance) in [(1.1, 20.0), (1.2, 35.0)] {
        let edit = frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            time,
            vec![egui::Event::PointerMoved(from - vec2(0.0, distance))],
            true,
            true,
            17,
        )
        .0;
        let Some(RackAction::Command(command, args)) = edit else {
            panic!("rotary must emit a native parameter edit")
        };
        assert_eq!(command, "set_parameter");
        assert_eq!(args["id"], "tidal");
        assert_eq!(args["parameter"], "slope");
        assert_eq!(args["expected_revision"], 17);
        assert!(args["value"].as_f64().unwrap() > 0.5);
        groups.push(args["history_group"].as_str().unwrap().to_owned());
    }
    assert_eq!(
        groups[0], groups[1],
        "successive drag updates share an undo gesture"
    );
    frame(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![button(from - vec2(0.0, 35.0), false)],
    );
}

#[test]
fn tidal_output_lamps_use_measured_bipolar_components_and_stay_dark_without_telemetry() {
    let ctx = egui::Context::default();
    let patch = tidal_patch(json!({}));
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
    let mut lights = ModuleLightState {
        count: 4,
        ..Default::default()
    };
    lights.brightness[2] = [0.0, 0.75];
    rack.module_lights.insert(node.id.clone(), lights);
    let active = circles(&rack);
    assert_eq!(active[0], dark[0], "negative component stays dark");
    assert_eq!(active[1].0, dark[1].0);
    assert_ne!(
        active[1].1, dark[1].1,
        "positive component follows measured voltage"
    );
}
