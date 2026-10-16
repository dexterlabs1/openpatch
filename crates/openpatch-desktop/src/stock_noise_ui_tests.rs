use super::*;

fn stock_noise_patch() -> Patch {
    serde_json::from_value(json!({
        "version": 2,
        "nodes": [{"id":"noise", "kind":"stock_noise"}]
    }))
    .unwrap()
}

fn stock_noise_texts(shape: &egui::Shape, result: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                stock_noise_texts(child, result);
            }
        }
        egui::Shape::Text(text) if !text.galley.text().is_empty() => result.push((
            text.galley.text().to_owned(),
            Rect::from_min_size(text.pos, text.galley.size()),
        )),
        _ => {}
    }
}

fn stock_noise_circles(shape: &egui::Shape, result: &mut Vec<(Pos2, f32)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                stock_noise_circles(child, result);
            }
        }
        egui::Shape::Circle(circle) => result.push((circle.center, circle.radius)),
        _ => {}
    }
}

#[test]
fn stock_noise_contract_and_native_output_rows_are_exact() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "stock_noise")
        .unwrap();
    assert_eq!(panel_width("stock_noise"), 300.0);
    assert_eq!(PANEL_HEIGHT, 396.0);
    assert_eq!(title("stock_noise"), "STOCK NOISE");
    assert_eq!(module["inputs"].as_array().unwrap().len(), 0);
    assert_eq!(module["params"].as_object().unwrap().len(), 0);
    assert_eq!(
        ordered_parameters(&stock_noise_patch().nodes[0], Some(module)).len(),
        0
    );
    let outputs: Vec<_> = module["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|port| port.as_str().unwrap())
        .collect();
    assert_eq!(
        outputs,
        vec!["white", "pink", "red", "violet", "blue", "gray", "black"]
    );
    for (index, output) in outputs.iter().enumerate() {
        assert_eq!(
            port_position("stock_noise", false, index, outputs.len(), 300.0),
            pos2(270.0, 92.0 + 46.0 * index as f32),
            "native output {output}"
        );
        assert_eq!(
            stock_noise_port_label(output),
            output.to_ascii_uppercase().as_str()
        );
        assert_eq!(
            STOCK_NOISE_PROCESSES[index],
            match *output {
                "white" => "NORMAL DRAW",
                "pink" => "VOSS 8 ROWS",
                "red" => "WHITE > FIXED LP",
                "violet" => "WHITE > FIRST DIFFERENCE",
                "blue" => "PINK > FIRST DIFFERENCE",
                "gray" => "WHITE > FFT1024 EQUAL-LOUD",
                "black" => "UNIFORM U > 10U-5",
                _ => unreachable!(),
            }
        );
    }
}

#[test]
fn stock_noise_print_and_jack_rings_clear_at_rack_and_browser_scales() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "stock_noise")
        .unwrap();
    for zoom in [0.25, 0.42, 0.85, 1.0] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut preview = ModulePreview::new(module);
        let origin = pos2(20.0, 20.0);
        let panel_size = vec2(panel_width("stock_noise"), PANEL_HEIGHT) * zoom;
        let bounds = Rect::from_min_size(origin, panel_size);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    preview.show(ui, bounds, &catalog);
                });
            },
        );
        let mut texts = Vec::new();
        let mut circles = Vec::new();
        for shape in &output.shapes {
            stock_noise_texts(&shape.shape, &mut texts);
            stock_noise_circles(&shape.shape, &mut circles);
        }
        let text_for = |label: &str| {
            texts
                .iter()
                .find(|(text, _)| text == label)
                .map(|(_, rect)| *rect)
                .unwrap_or_else(|| panic!("missing {label:?} at zoom {zoom}"))
        };
        for index in 0..7 {
            let y = origin.y + stock_noise_row_y(index) * zoom;
            let output_label = text_for(STOCK_NOISE_OUTPUTS[index].to_uppercase().as_str());
            let process_label = text_for(STOCK_NOISE_PROCESSES[index]);
            let center = origin + vec2(270.0, stock_noise_row_y(index)) * zoom;
            let ring_radius = 13.0 * STOCK_NOISE_PORT_SCALE * zoom;
            let ring = Rect::from_center_size(center, vec2(2.0 * ring_radius, 2.0 * ring_radius));
            assert!((output_label.center().y - y).abs() < 0.1);
            assert!((process_label.center().y - y).abs() < 0.1);
            assert!(
                process_label.right() < output_label.left(),
                "{zoom}: {process_label:?} {output_label:?}"
            );
            assert!(
                !output_label.intersects(ring),
                "{zoom}: {output_label:?} {ring:?}"
            );
            assert!(
                circles.iter().any(|(at, radius)| {
                    at.distance(center) < 0.1 && (*radius - ring_radius).abs() < 0.01
                }),
                "missing enlarged output rim at {center:?}, zoom {zoom}"
            );
            assert!(
                Rect::from_min_size(origin, panel_size).contains_rect(process_label),
                "process text escapes panel at zoom {zoom}: {process_label:?}"
            );
            assert!(
                Rect::from_min_size(origin, panel_size).contains_rect(ring),
                "socket ring escapes panel at zoom {zoom}: {ring:?}"
            );
        }
        assert!(texts
            .iter()
            .all(|(text, _)| !text.eq_ignore_ascii_case("openpatch")));
    }
}

#[test]
fn stock_noise_enlarged_jack_target_reaches_past_the_visible_rim() {
    let patch = stock_noise_patch();
    let mut rack = Rack {
        zoom: 1.0,
        ..Default::default()
    };
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let _ = frame_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true);
    let center = fixture_jack(&rack, &patch, "noise", "white", false);
    let press = center + vec2(18.4, 0.0); // outside rim, inside enlarged target.
    frame_state(
        &ctx,
        &mut rack,
        &patch,
        0.1,
        vec![egui::Event::PointerMoved(press), button(press, true)],
        true,
        true,
    );
    let drag = press + vec2(0.0, 7.0);
    frame_state(
        &ctx,
        &mut rack,
        &patch,
        0.2,
        vec![egui::Event::PointerMoved(drag)],
        true,
        true,
    );
    assert!(
        rack.pending.is_some(),
        "pointer should start a cable within the enlarged jack target"
    );
    assert!(rack.dragging_cable);
}
