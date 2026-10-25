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
fn strings_patch(params: Value) -> Patch {
    serde_json::from_value(
        json!({"version":2,"nodes":[{"id":"strings","kind":"string_field","params":params}]}),
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
fn string_field_shared_preview_keeps_controls_print_and_ports_inside_the_panel() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "string_field")
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
            "STRING FIELD",
            "POLYPHONY",
            "MODEL",
            "1 VOICE",
            "MODAL",
            "FREQUENCY",
            "STRUCTURE",
            "BRIGHTNESS",
            "DAMPING",
            "POSITION",
            "AMOUNT",
            "F#2",
            "50%",
            "0%",
            "MODAL BODY OF UP TO 60 MODES · 1 VOICE",
            "IN",
            "STRUM",
            "V/OCT",
            "FREQ",
            "STRUCT",
            "BRIGHT",
            "DAMP",
            "POS",
            "ODD",
            "EVEN",
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
            "frequency",
            "structure",
            "brightness",
            "damping",
            "position",
            "frequency_amount",
            "structure_amount",
            "brightness_amount",
            "damping_amount",
            "position_amount",
        ] {
            let (center, radius) = string_field_control(name);
            let hardware = generic_parameter_hardware("string_field", name, radius, false);
            let GenericParameterHardware::Rotary { target_half, .. } = hardware else {
                panic!("{name} must be a rotary")
            };
            let target = Rect::from_center_size(
                bounds.min + center.to_vec2() * zoom,
                Vec2::splat(target_half * 2.0 * zoom),
            );
            let value = Rect::from_center_size(
                bounds.min + string_field_value_center(name).to_vec2() * zoom,
                vec2(string_field_value_width(name), 19.0) * zoom,
            );
            assert!(bounds.contains_rect(target) && bounds.contains_rect(value));
            assert!(
                !target.intersects(value),
                "{name} exact-entry target overlaps rotary"
            );
            targets.extend([target, value]);
        }
        for name in ["polyphony", "model"] {
            let (local, _, _) = string_field_selector(name).unwrap();
            targets.push(Rect::from_min_size(
                bounds.min + local.min.to_vec2() * zoom,
                local.size() * zoom,
            ));
        }
        let mut rims = Vec::new();
        for input in [true, false] {
            let count = if input { 8 } else { 2 };
            for index in 0..count {
                let center = bounds.min
                    + port_position("string_field", input, index, count, width).to_vec2() * zoom;
                let radius =
                    if input { 10.3 } else { 13.0 } * port_material_scale("string_field") * zoom;
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
fn string_field_captions_readouts_and_guide_follow_model_and_polyphony() {
    for (params, expected) in [
        (
            json!({"model":0,"polyphony":4,"frequency":45.3,"structure_amount":-0.5}),
            &["STRUCTURE", "A3 +30¢", "−50%", "4 VOICES", "MODAL BODY OF UP TO 12 MODES · 4 VOICES · ODD AND EVEN VOICES SPLIT"][..],
        ),
        (
            json!({"model":1,"polyphony":2}),
            &["INTERVALS", "SYMPATHETIC STRINGS", "4 SYMPATHETIC STRINGS · 2 VOICES · ODD AND EVEN VOICES SPLIT"][..],
        ),
        (
            json!({"model":2,"polyphony":3,"damping":0.25}),
            &["STIFFNESS", "INHARMONIC STRING", "25%", "INHARMONIC STRING · 3 VOICES · ODD AND EVEN VOICES SPLIT"][..],
        ),
        (
            json!({"model":3,"position":0.2}),
            &["RATIO", "INDEX", "DECAY", "FEEDBACK", "FM VOICE", "−60%", "TWO-OPERATOR FM · 1 VOICE"][..],
        ),
        (
            json!({"model":4,"structure":0.3}),
            &["CHORD", "CHORD STRINGS", "Minor", "8 STRINGS TUNED TO A CHORD · 1 VOICE"][..],
        ),
        (
            json!({"model":4,"structure":0.5,"polyphony":4}),
            &["Fifth", "2 STRINGS TUNED TO A CHORD · 4 VOICES · ODD AND EVEN VOICES SPLIT"][..],
        ),
        (
            json!({"model":5,"frequency":0}),
            &["STIFFNESS", "STRING AND REVERB", "C0", "STRING WITH STEREO REVERB · 1 VOICE"][..],
        ),
    ] {
        let texts = rack_texts(&strings_patch(params.clone()));
        for wanted in expected {
            assert!(
                texts.iter().any(|(text, _)| text == wanted),
                "{params}: missing {wanted}"
            );
        }
    }
}

#[test]
fn string_field_rotary_drag_edits_the_native_parameter_with_one_undo_gesture() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let patch = strings_patch(json!({}));
    let mut rack = Rack::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let from = pos2(24.0, 12.0) + string_field_control("damping").0.to_vec2();
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
        assert_eq!(args["id"], "strings");
        assert_eq!(args["parameter"], "damping");
        assert!(args["value"].as_f64().unwrap() > 0.5);
        groups.push(args["history_group"].as_str().unwrap().to_owned());
    }
    assert_eq!(groups[0], groups[1]);
    frame(&ctx, &mut rack, &patch, 1.3, vec![button(from - vec2(0.0, 35.0), false)]);
}

#[test]
fn string_field_lamps_follow_measured_state_and_stay_dark_without_telemetry() {
    let ctx = egui::Context::default();
    let patch = strings_patch(json!({}));
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
    assert_eq!(string_field_strum_lamp(None), 0.0);
    assert_eq!(string_field_strum_lamp(Some(&lights)), 0.0);
    lights.brightness[2] = [0.0, 1.0];
    assert_eq!(string_field_strum_lamp(Some(&lights)), 1.0);
}
