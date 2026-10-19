use super::*;
#[test]
fn recall_random_shared_panel_has_readable_controls_and_all_ports_inside_the_face() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "recall_random")
        .unwrap();
    assert_eq!(panel_width("recall_random"), 810.0);
    for input in [true, false] {
        for index in 0..if input { 11 } else { 7 } {
            let p = recall_random_port(input, index);
            assert!(p.x > 20.0 && p.x < 790.0);
            assert!(p.y > 300.0 && p.y < PANEL_HEIGHT - 15.0);
        }
    }
    for zoom in [0.25, 0.5, 1.0, 1.4] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut preview = ModulePreview::new(module);
        let bounds = Rect::from_min_size(pos2(20.0, 12.0), preview.size() * zoom);
        let mut output = None;
        for _ in 0..2 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0))),
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
            collect_recall_text(&shape.shape, &mut texts);
        }
        for label in [
            "RECALL RANDOM",
            "RATE",
            "DEJA VU",
            "LOOP LENGTH",
            "X SPREAD",
            "Y STEPS",
            "SCALE 1",
            "NOTE WEIGHTS",
            "RHYTHM",
            "X / Y",
            "SCALES / RECORD",
        ] {
            assert!(
                texts.iter().any(|(s, _)| s == label),
                "missing {label} at {zoom}"
            );
        }
        for (label, rect) in texts {
            if [
                "RATE",
                "DEJA VU",
                "LOOP LENGTH",
                "X SPREAD",
                "Y STEPS",
                "SCALE 1",
                "NOTE WEIGHTS",
                "RHYTHM",
                "X / Y",
                "SCALES / RECORD",
            ]
            .contains(&label.as_str())
            {
                assert!(
                    bounds.expand(1.0).contains_rect(rect),
                    "clipped {label} at {zoom}: {rect:?}"
                );
            }
        }
    }
}
fn collect_recall_text(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_recall_text(shape, out)
            }
        }
        egui::Shape::Text(t) => out.push((
            t.galley.text().into(),
            t.galley.rect.translate(t.pos.to_vec2()),
        )),
        _ => {}
    }
}
