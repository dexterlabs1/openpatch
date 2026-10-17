use super::*;

fn labels(shape: &egui::Shape, result: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                labels(shape, result);
            }
        }
        egui::Shape::Text(t) => result.push((
            t.galley.text().into(),
            t.galley.rect.translate(t.pos.to_vec2()),
        )),
        _ => {}
    }
}

#[test]
fn score_authored_notes_rests_and_disabled_probability_never_invent_a_playhead() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[{
        "id":"score","kind":"score_sequencer","score":{
            "length":8,"steps_per_beat":2,"tracks":[{"steps":[
                {"step":0,"notes":[0,4,7]},
                {"step":1,"notes":[]},
                {"step":2,"notes":[12],"probability":0}
            ]}],"lanes":[{"points":[{"bar":0,"value":2}]}]
        }
    }]}))
    .unwrap();
    patch.compile().unwrap();
    let mut rack = Rack::default();
    let (_, first) = frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 9);
    let (_, second) = frame_revision_state(&ctx, &mut rack, &patch, 1.0, vec![], true, true, 9);
    for output in [first, second] {
        let mut texts = vec![];
        for shape in &output.shapes {
            labels(&shape.shape, &mut texts);
        }
        for expected in [
            "AUTHORED · 8 STEPS · 2 / BEAT",
            "AUTOMATION · 1 / 16 LANES",
            "INITIAL RUN",
        ] {
            assert!(
                texts.iter().any(|(s, _)| s == expected),
                "Missing {expected}"
            );
        }
        let cells: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(r)
                    if r.fill == Color32::from_rgb(229, 183, 103)
                        && (r.rect.height() - 8.0).abs() < 0.01 =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect();
        assert_eq!(cells.len(), 1, "Only the authored nonempty enabled step is filled; rests and zero probability remain unfilled");
    }
    assert!(rack.sequence_states.is_empty());
    assert!(rack.module_lights.is_empty());
}

#[test]
fn score_preview_has_identical_hardware_and_print_clear_of_every_socket() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "score_sequencer")
        .unwrap();
    for zoom in [0.25, 0.42, 1.0] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut preview = ModulePreview::new(module);
        let bounds = Rect::from_min_size(pos2(24.0, 12.0), preview.size() * zoom);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| preview.show(ui, bounds, &catalog));
            },
        );
        let mut texts = vec![];
        for shape in output.shapes {
            labels(&shape.shape, &mut texts);
        }
        for (label, rect) in texts {
            assert!(
                bounds.expand(0.5).contains_rect(rect),
                "{zoom}: {label} outside panel"
            );
            for input in [true, false] {
                let ports = module[if input { "inputs" } else { "outputs" }]
                    .as_array()
                    .unwrap();
                for index in 0..ports.len() {
                    let at = bounds.min
                        + port_position("score_sequencer", input, index, ports.len(), 600.0)
                            .to_vec2()
                            * zoom;
                    let rim =
                        if input { 10.3 } else { 13.0 } * port_material_scale("score_sequencer");
                    assert!(
                        rect.distance_to_pos(at) >= (rim - 0.1) * zoom,
                        "{zoom}: {label} touches {}",
                        ports[index]
                    );
                }
            }
        }
    }
}
