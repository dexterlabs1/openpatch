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
fn single(kind: &str, params: Value) -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"m","kind":kind,"params":params}]}))
        .unwrap()
}
fn lamp_circles(shapes: &[egui::epaint::ClippedShape], radius: f32) -> Vec<(Pos2, Color32)> {
    fn walk(shape: &egui::Shape, radius: f32, found: &mut Vec<(Pos2, Color32)>) {
        match shape {
            egui::Shape::Vec(children) => {
                for child in children {
                    walk(child, radius, found);
                }
            }
            egui::Shape::Circle(c) if (c.radius - radius).abs() < 0.01 => {
                found.push((c.center, c.fill))
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    for shape in shapes {
        walk(&shape.shape, radius, &mut found);
    }
    found
}

/// Geometry shared by both panels: print, controls and sockets stay inside
/// the face and never collide, at browser and rack scales.
fn check_shared_preview(
    kind: &str,
    expected: &[&str],
    controls: &dyn Fn(Rect, f32) -> Vec<Rect>,
) {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == kind)
        .unwrap();
    let inputs = module["inputs"].as_array().unwrap().len();
    let outputs = module["outputs"].as_array().unwrap().len();
    let width = panel_width(kind);
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
        for wanted in expected {
            assert!(
                texts.iter().any(|(text, _)| text == wanted),
                "{kind} {zoom}: missing {wanted}"
            );
        }
        for (text, printed) in &texts {
            assert!(
                bounds.expand(0.5).contains_rect(*printed),
                "{kind} {zoom}: {text} outside panel: {printed:?}"
            );
        }
        let targets = controls(bounds, zoom);
        for target in &targets {
            assert!(
                bounds.contains_rect(*target),
                "{kind} {zoom}: control outside panel"
            );
        }
        for (index, target) in targets.iter().enumerate() {
            for other in &targets[index + 1..] {
                assert!(!target.intersects(*other), "{kind} {zoom}: controls overlap");
            }
        }
        let mut rims = Vec::new();
        for (input, count) in [(true, inputs), (false, outputs)] {
            for index in 0..count {
                let center =
                    bounds.min + port_position(kind, input, index, count, width).to_vec2() * zoom;
                let radius = if input { 10.3 } else { 13.0 } * port_material_scale(kind) * zoom;
                let rim = Rect::from_center_size(center, Vec2::splat(2.0 * radius));
                assert!(
                    bounds.contains_rect(rim),
                    "{kind} {zoom}: port {input}/{index} clips"
                );
                for target in &targets {
                    assert!(
                        !target.intersects(rim),
                        "{kind} {zoom}: port {input}/{index} overlaps a control"
                    );
                }
                for (text, printed) in &texts {
                    assert!(
                        !printed.intersects(rim),
                        "{kind} {zoom}: {text} overlaps port {input}/{index}"
                    );
                }
                rims.push(rim);
            }
        }
        for (index, rim) in rims.iter().enumerate() {
            for other in &rims[index + 1..] {
                assert!(!rim.intersects(*other), "{kind} {zoom}: port rims overlap");
            }
        }
        for (index, (a, ra)) in texts.iter().enumerate() {
            for (b, rb) in &texts[index + 1..] {
                if !a.is_empty() && !b.is_empty() {
                    assert!(
                        !ra.shrink(0.5).intersects(rb.shrink(0.5)),
                        "{kind} {zoom}: {a} overlaps {b}"
                    );
                }
            }
        }
    }
}

#[test]
fn vca2_shared_preview_keeps_print_controls_and_ports_inside_the_panel() {
    check_shared_preview(
        "stock_vca2",
        &[
            "VCA-2", "LEVEL 1", "LEVEL 2", "100%", "1", "2", "EXP", "LIN", "IN", "OUT",
        ],
        &|bounds, zoom| {
            let mut targets = Vec::new();
            for name in ["level1", "level2"] {
                let (center, radius) = stock_vca2_control(name);
                let GenericParameterHardware::Rotary { target_half, .. } =
                    generic_parameter_hardware("stock_vca2", name, radius, false)
                else {
                    panic!("{name} must be a rotary")
                };
                let target = Rect::from_center_size(
                    bounds.min + center.to_vec2() * zoom,
                    Vec2::splat(target_half * 2.0 * zoom),
                );
                let value = Rect::from_center_size(
                    bounds.min + stock_vca2_value_center(name).to_vec2() * zoom,
                    vec2(stock_vca2_value_width(name), 19.0) * zoom,
                );
                assert!(!target.intersects(value), "{name} entry overlaps rotary");
                targets.extend([target, value]);
            }
            targets
        },
    );
}

#[test]
fn unity_shared_preview_keeps_print_toggles_lamps_and_ports_inside_the_panel() {
    check_shared_preview(
        "stock_unity",
        &[
            "STOCK UNITY",
            "IN 1",
            "IN 6",
            "MIX",
            "INV",
            "AVERAGE",
            "MERGE 1 + 2",
            "-24",
            "-6",
            "0",
        ],
        &|bounds, zoom| {
            let mut targets: Vec<Rect> = ["average1", "average2", "merge"]
                .iter()
                .map(|name| {
                    let (local, _) = stock_unity_toggle(name).unwrap();
                    Rect::from_min_size(
                        bounds.min + local.min.to_vec2() * zoom,
                        local.size() * zoom,
                    )
                })
                .collect();
            for section in 0..2 {
                for lamp in 0..5 {
                    targets.push(Rect::from_center_size(
                        bounds.min + stock_unity_lamp(section, lamp).to_vec2() * zoom,
                        Vec2::splat(7.0 * zoom),
                    ));
                }
            }
            targets
        },
    );
}

#[test]
fn vca2_readouts_follow_each_level() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut rack = Rack::default();
    let patch = single("stock_vca2", json!({"level1":0.25,"level2":0.0}));
    let (_, output) = frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 5);
    let mut texts = Vec::new();
    for shape in &output.shapes {
        text_shapes(&shape.shape, &mut texts);
    }
    for wanted in ["25%", "0%"] {
        assert!(
            texts.iter().any(|(text, _)| text == wanted),
            "missing {wanted}"
        );
    }
}

#[test]
fn vca2_rotary_drag_edits_its_own_channel_with_one_undo_gesture() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let patch = single("stock_vca2", json!({"level2":0.2}));
    let mut rack = Rack::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let from = pos2(24.0, 12.0) + stock_vca2_control("level2").0.to_vec2();
    let pressed = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(from), button(from, true)],
        true,
        true,
        9,
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
            9,
        )
        .0;
        let Some(RackAction::Command(command, args)) = edit else {
            panic!("rotary must emit a native parameter edit")
        };
        assert_eq!(command, "set_parameter");
        assert_eq!(args["parameter"], "level2");
        assert!(args["value"].as_f64().unwrap() > 0.2);
        groups.push(args["history_group"].as_str().unwrap().to_owned());
    }
    assert_eq!(groups[0], groups[1]);
    frame(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![button(from - vec2(0.0, 35.0), false)],
    );
}

#[test]
fn unity_toggles_flip_their_own_setting_against_the_current_revision() {
    for (name, params, expected) in [
        ("average1", json!({}), 1.0),
        ("average2", json!({"average2":1}), 0.0),
        ("merge", json!({}), 1.0),
    ] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let patch = single("stock_unity", params);
        let mut rack = Rack::default();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let (local, _) = stock_unity_toggle(name).unwrap();
        let at = pos2(24.0, 12.0) + local.center().to_vec2();
        let mut edit = None;
        for (time, events) in [
            (1.0, vec![egui::Event::PointerMoved(at), button(at, true)]),
            (1.1, vec![button(at, false)]),
        ] {
            if let Some(action) =
                frame_revision_state(&ctx, &mut rack, &patch, time, events, true, true, 23).0
            {
                edit = Some(action);
            }
        }
        let Some(RackAction::Command(command, args)) = edit else {
            panic!("{name} toggle must emit a parameter edit")
        };
        assert_eq!(command, "set_parameter");
        assert_eq!(args["parameter"], name);
        assert_eq!(args["value"], expected);
        assert_eq!(args["expected_revision"], 23);
    }
}

#[test]
fn unity_lamps_follow_measured_state_and_stay_dark_without_telemetry() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let patch = single("stock_unity", json!({}));
    let mut rack = Rack::default();
    let radius = crate::lighting::lamp_radius(2.6);
    let draw = |rack: &mut Rack| {
        let (_, output) = frame_revision_state(&ctx, rack, &patch, 0.0, vec![], true, true, 5);
        let mut lamps = lamp_circles(&output.shapes, radius);
        lamps.sort_by(|a, b| (a.0.y, a.0.x).partial_cmp(&(b.0.y, b.0.x)).unwrap());
        lamps
    };
    let dark = draw(&mut rack);
    assert_eq!(dark.len(), 10);
    // Section 1 at about -10 dB: clip and -6 dB dark, -12/-18/-24 dB lit.
    let mut lights = ModuleLightState {
        count: 10,
        ..Default::default()
    };
    for light in [2, 3, 4] {
        lights.brightness[light] = [1.0, 0.0];
    }
    rack.module_lights.insert("m".into(), lights);
    let lit = draw(&mut rack);
    // Lamps are drawn quietest first: -24, -18, -12, -6, clip.
    for lamp in 0..3 {
        assert_ne!(lit[lamp].1, dark[lamp].1, "lamp {lamp} follows its light");
    }
    for lamp in 3..10 {
        assert_eq!(lit[lamp].1, dark[lamp].1, "lamp {lamp} stays dark");
    }
}
