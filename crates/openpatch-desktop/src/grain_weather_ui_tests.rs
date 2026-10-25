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
fn grain_patch(params: Value) -> Patch {
    serde_json::from_value(
        json!({"version":2,"nodes":[{"id":"grain","kind":"grain_weather","params":params}]}),
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
fn grain_weather_shared_preview_keeps_controls_print_and_ports_inside_the_panel() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "grain_weather")
        .unwrap();
    let width = HP * 26.0;
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
            "GRAIN WEATHER",
            "MODE",
            "QUALITY",
            "FREEZE",
            "GRANULAR",
            "16-BIT STEREO",
            "RECORD",
            "POSITION",
            "SIZE",
            "PITCH",
            "DENSITY",
            "TEXTURE",
            "BLEND",
            "SPREAD",
            "FEEDBACK",
            "REVERB",
            "Random 36%",
            "0 st",
            "50%",
            "UP TO 32 GRAINS FROM A 1.0 S STEREO BUFFER · TRIG ADDS A GRAIN",
            "IN L",
            "IN R",
            "TRIG",
            "V/OCT",
            "OUT L",
            "OUT R",
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
        for name in [
            "position", "size", "pitch", "density", "texture", "blend", "spread", "feedback",
            "reverb",
        ] {
            let (center, radius) = grain_weather_control(name);
            let hardware = generic_parameter_hardware("grain_weather", name, radius, false);
            let GenericParameterHardware::Rotary { target_half, .. } = hardware else {
                panic!("{name} must be a rotary")
            };
            let target = Rect::from_center_size(
                bounds.min + center.to_vec2() * zoom,
                Vec2::splat(target_half * 2.0 * zoom),
            );
            let value = Rect::from_center_size(
                bounds.min + grain_weather_value_center(name).to_vec2() * zoom,
                vec2(grain_weather_value_width(name), 19.0) * zoom,
            );
            assert!(bounds.contains_rect(target) && bounds.contains_rect(value));
            assert!(
                !target.intersects(value),
                "{name} exact-entry target overlaps rotary"
            );
            targets.extend([target, value]);
        }
        for name in ["mode", "quality", "freeze"] {
            let (local, _, _) = grain_weather_selector(name).unwrap();
            targets.push(Rect::from_min_size(
                bounds.min + local.min.to_vec2() * zoom,
                local.size() * zoom,
            ));
        }
        let mut rims = Vec::new();
        for input in [true, false] {
            let count = if input { 10 } else { 2 };
            for index in 0..count {
                let center = bounds.min
                    + port_position("grain_weather", input, index, count, width).to_vec2() * zoom;
                let radius =
                    if input { 10.3 } else { 13.0 } * port_material_scale("grain_weather") * zoom;
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
fn grain_weather_captions_readouts_and_guide_follow_mode_and_quality() {
    for (params, expected) in [
        (
            json!({"mode":0,"density":0.3,"pitch":-7.25}),
            &["DENSITY", "TEXTURE", "Even 36%", "−7.2 st"][..],
        ),
        (
            json!({"mode":0,"density":0.5,"quality":3}),
            &["No grains", "8-BIT MONO", "UP TO 57 GRAINS FROM A 7.4 S MONO BUFFER · TRIG ADDS A GRAIN"][..],
        ),
        (
            json!({"mode":1,"quality":1,"pitch":12}),
            &["WINDOW", "DIFFUSION", "FILTER", "STRETCH", "+12.0 st", "PITCH SHIFT AND TIME STRETCH OVER A 1.9 S MONO BUFFER · TRIG RESTARTS"][..],
        ),
        (
            json!({"mode":2,"quality":2,"freeze":1}),
            &["DELAY", "LOOPING DELAY", "FROZEN", "LOOPING DELAY OVER A 4.1 S STEREO BUFFER · TRIG TAPS THE TIME"][..],
        ),
        (
            json!({"mode":3,"texture":0.25}),
            &["WARP", "REFRESH", "QUANTIZE", "SPECTRAL", "25%", "PHASE VOCODER · FREEZE HOLDS THE SPECTRUM · TRIG HELD GLITCHES"][..],
        ),
    ] {
        let texts = rack_texts(&grain_patch(params.clone()));
        for wanted in expected {
            assert!(
                texts.iter().any(|(text, _)| text == wanted),
                "{params}: missing {wanted}"
            );
        }
    }
}

#[test]
fn grain_weather_rotary_drag_edits_the_native_parameter_with_one_undo_gesture() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let patch = grain_patch(json!({}));
    let mut rack = Rack::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let from = pos2(24.0, 12.0) + grain_weather_control("texture").0.to_vec2();
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
        assert_eq!(args["id"], "grain");
        assert_eq!(args["parameter"], "texture");
        assert!(args["value"].as_f64().unwrap() > 0.5);
        groups.push(args["history_group"].as_str().unwrap().to_owned());
    }
    assert_eq!(groups[0], groups[1]);
    frame(&ctx, &mut rack, &patch, 1.3, vec![button(from - vec2(0.0, 35.0), false)]);
}

#[test]
fn grain_weather_lamps_follow_measured_state_and_stay_dark_without_telemetry() {
    let ctx = egui::Context::default();
    let patch = grain_patch(json!({}));
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
        count: 3,
        ..Default::default()
    };
    lights.brightness[1] = [0.6, 0.0];
    rack.module_lights.insert(node.id.clone(), lights);
    let active = circles(&rack);
    assert_ne!(active[0].1, dark[0].1, "negative component follows measured voltage");
    assert_eq!(active[1], dark[1], "positive component stays dark");
    assert_eq!(grain_weather_freeze_lamp(None), 0.0);
    assert_eq!(grain_weather_freeze_lamp(Some(&lights)), 0.0);
    lights.brightness[2] = [0.0, 1.0];
    assert_eq!(grain_weather_freeze_lamp(Some(&lights)), 1.0);
}
