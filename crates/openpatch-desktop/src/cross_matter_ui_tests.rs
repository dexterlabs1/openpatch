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
fn cross_patch(params: Value) -> Patch {
    serde_json::from_value(
        json!({"version":2,"nodes":[{"id":"cross","kind":"cross_matter","params":params}]}),
    )
    .unwrap()
}
fn rack_texts(patch: &Patch) -> Vec<(String, Rect)> {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut rack = Rack::default();
    let (_, output) = frame_revision_state(&ctx, &mut rack, patch, 0.0, vec![], true, true, 5);
    let mut texts = Vec::new();
    for shape in &output.shapes {
        text_shapes(&shape.shape, &mut texts);
    }
    texts
}

#[test]
fn cross_matter_shared_preview_keeps_controls_print_and_ports_inside_the_panel() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "cross_matter")
        .unwrap();
    let width = HP * 24.0;
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
            "CROSS MATTER",
            "CARRIER",
            "MODE",
            "SINE",
            "CROSS MODULATION",
            "ALGORITHM",
            "TIMBRE",
            "PITCH",
            "LEVEL 2",
            "Analog ring",
            "50%",
            "BUILT-IN CARRIER × MODULATOR · AUX CARRIER",
            "MODULATOR",
            "LEVEL 1",
            "OUT",
            "AUX",
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
        for name in ["algorithm", "timbre", "level1", "level2"] {
            let (center, radius) = cross_matter_control(name);
            let hardware = generic_parameter_hardware("cross_matter", name, radius, false);
            let GenericParameterHardware::Rotary { target_half, .. } = hardware else {
                panic!("{name} must be a rotary")
            };
            let target = Rect::from_center_size(
                bounds.min + center.to_vec2() * zoom,
                Vec2::splat(target_half * 2.0 * zoom),
            );
            let value = Rect::from_center_size(
                bounds.min + cross_matter_value_center(name).to_vec2() * zoom,
                vec2(if name == "algorithm" { 150.0 } else { 76.0 }, 19.0) * zoom,
            );
            assert!(bounds.contains_rect(target) && bounds.contains_rect(value));
            assert!(
                !target.intersects(value),
                "{name} exact-entry target overlaps rotary"
            );
            targets.extend([target, value]);
        }
        for name in ["carrier", "mode"] {
            let (local, _, _) = cross_matter_selector(name).unwrap();
            targets.push(Rect::from_min_size(
                bounds.min + local.min.to_vec2() * zoom,
                local.size() * zoom,
            ));
        }
        let mut rims = Vec::new();
        for input in [true, false] {
            let count = if input { 6 } else { 2 };
            for index in 0..count {
                let center = bounds.min
                    + port_position("cross_matter", input, index, count, width).to_vec2() * zoom;
                let radius =
                    if input { 10.3 } else { 13.0 } * port_material_scale("cross_matter") * zoom;
                let rim = Rect::from_center_size(center, Vec2::splat(2.0 * radius));
                assert!(bounds.contains_rect(rim), "{zoom}: port {input}/{index} clips");
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
        // Printed captions, readouts and the guide never collide.
        for (index, (a, ra)) in texts.iter().enumerate() {
            for (b, rb) in &texts[index + 1..] {
                if a != b && !a.is_empty() && !b.is_empty() {
                    assert!(!ra.shrink(0.5).intersects(rb.shrink(0.5)), "{zoom}: {a} overlaps {b}");
                }
            }
        }
    }
}

#[test]
fn cross_matter_captions_and_readouts_follow_mode_and_carrier() {
    for (params, expected) in [
        (
            json!({"carrier":0,"algorithm":0.9}),
            &["LEVEL 1", "ALGORITHM", "Vocoder", "MODULATOR SHAPES THE CARRIER INPUT · AUX BOTH INPUTS"][..],
        ),
        (
            json!({"carrier":2,"level1":0.55}),
            &["PITCH", "440 Hz", "TRIANGLE"][..],
        ),
        (
            json!({"mode":1,"carrier":1,"algorithm":0.75}),
            &["SHIFT", "UP / DOWN", "FEEDBACK", "DRY / WET", "FREQUENCY SHIFTER", "+11.3 Hz"][..],
        ),
        (
            json!({"mode":1,"carrier":0,"algorithm":0.25}),
            &["PHASE", "90°", "MODULATOR SHIFTED BY THE CARRIER INPUT · AUX OPPOSITE SHIFT"][..],
        ),
    ] {
        let texts = rack_texts(&cross_patch(params.clone()));
        for wanted in expected {
            assert!(
                texts.iter().any(|(text, _)| text == wanted),
                "{params}: missing {wanted}"
            );
        }
    }
}

#[test]
fn cross_matter_algorithm_readout_names_each_source_region() {
    let names: Vec<String> = [0.0, 0.05, 0.27, 0.38, 0.52, 0.66, 0.68, 0.9]
        .into_iter()
        .map(cross_matter_algorithm_name)
        .collect();
    assert_eq!(names[0], "Crossfade");
    assert_eq!(names[7], "Vocoder");
    for name in &names {
        assert!(!name.contains("NaN") && name.len() <= 21, "{name}");
    }
    let mut distinct = names.clone();
    distinct.dedup();
    assert!(distinct.len() >= 6, "{names:?}");
}

#[test]
fn cross_matter_rotary_drag_edits_the_native_parameter_with_one_undo_gesture() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let patch = cross_patch(json!({}));
    let mut rack = Rack::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let from = pos2(24.0, 12.0) + cross_matter_control("timbre").0.to_vec2();
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
        assert_eq!(args["id"], "cross");
        assert_eq!(args["parameter"], "timbre");
        assert!(args["value"].as_f64().unwrap() > 0.5);
        groups.push(args["history_group"].as_str().unwrap().to_owned());
    }
    assert_eq!(groups[0], groups[1]);
    frame(&ctx, &mut rack, &patch, 1.3, vec![button(from - vec2(0.0, 35.0), false)]);
}

#[test]
fn cross_matter_output_lamps_follow_measured_volts_and_stay_dark_without_telemetry() {
    let ctx = egui::Context::default();
    let patch = cross_patch(json!({}));
    let node = &patch.nodes[0];
    let mut rack = Rack::default();
    let circles = |rack: &Rack| {
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                rack.paint_output_signal_light(ui.painter(), node, 1, pos2(100.0, 100.0))
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
        count: 2,
        ..Default::default()
    };
    lights.brightness[1] = [0.6, 0.0];
    rack.module_lights.insert(node.id.clone(), lights);
    let active = circles(&rack);
    assert_ne!(active[0].1, dark[0].1, "negative component follows measured voltage");
    assert_eq!(active[1], dark[1], "positive component stays dark");
}
