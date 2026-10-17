use super::*;

fn guide_texts(shape: &egui::Shape, texts: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                guide_texts(shape, texts);
            }
        }
        egui::Shape::Text(text) => texts.push((
            text.galley.text().to_owned(),
            Rect::from_min_size(text.pos, text.galley.size()),
        )),
        _ => {}
    }
}

#[test]
fn atlas_quantizer_guides_distinguish_empty_banks_and_actual_pitch_laws() {
    for (kind, width, law, empty, route) in [
        (
            "quantizer",
            240.0,
            "NEAREST NOTE · TIES ↑",
            "EMPTY → PASS ADJUSTED",
            "PITCH + OFFSET + CV",
        ),
        (
            "stock_quantizer",
            300.0,
            "NEAREST INTEGER NOTE · TIES ↓",
            "EMPTY → CHROMATIC",
            "PITCH + OFFSET → 24 BINS / V",
        ),
    ] {
        for mask in [0_u16, 1, 4095] {
            for zoom in [0.5, 1.0] {
                let ctx = egui::Context::default();
                crate::panel_skin::install_fonts(&ctx);
                let node: Node = serde_json::from_value(json!({
                    "id":"q", "kind":kind, "params":{"scale_mask":mask}
                }))
                .unwrap();
                let panel = Rect::from_min_size(pos2(10.0, 10.0), vec2(width, 396.0) * zoom);
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 500.0))),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            paint_atlas_quantizer_print(ui.painter(), panel, kind, zoom);
                            paint_atlas_quantizer_configuration(
                                ui.painter(),
                                &node.kind,
                                mask,
                                panel,
                                zoom,
                                &crate::lighting::RoomLighting::default(),
                            );
                        });
                    },
                );
                let mut texts = vec![];
                for shape in output.shapes {
                    guide_texts(&shape.shape, &mut texts);
                }
                for expected in [law, route, "CONFIGURED PITCH PATH"] {
                    assert!(texts.iter().any(|(text, _)| text == expected));
                }
                assert_eq!(texts.iter().any(|(text, _)| text == empty), mask == 0);
                let bank = if mask == 0 {
                    "EMPTY BANK".to_owned()
                } else {
                    format!(
                        "{} {} ENABLED",
                        mask.count_ones(),
                        if mask.count_ones() == 1 {
                            "NOTE"
                        } else {
                            "NOTES"
                        }
                    )
                };
                assert!(texts.iter().any(|(text, _)| text == &bank));
                let catalog = openpatch_core::catalog();
                let spec = &catalog["modules"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|m| m["kind"] == kind)
                    .unwrap()["params"]["offset"];
                let factor = if kind == "stock_quantizer" { 12.0 } else { 1.0 };
                let range = format!(
                    "−{:.0} TO +{:.0} {}",
                    -spec["min"].as_f64().unwrap() * factor,
                    spec["max"].as_f64().unwrap() * factor,
                    if kind == "stock_quantizer" { "st" } else { "V" }
                );
                assert!(
                    texts.iter().any(|(text, _)| text == &range),
                    "{kind}: printed range must match actual catalog units"
                );
                assert!(!texts.iter().any(|(text, _)| text.contains("PLAYED")));
                for (index, (label, bounds)) in texts.iter().enumerate() {
                    assert!(
                        panel.contains_rect(*bounds),
                        "{kind} {zoom}: {label} outside panel"
                    );
                    for (other, other_bounds) in &texts[index + 1..] {
                        assert!(
                            !bounds
                                .shrink(0.2 * zoom)
                                .intersects(other_bounds.shrink(0.2 * zoom)),
                            "{kind} {zoom}: {label} overlaps {other}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn atlas_quantizer_offset_values_and_keys_keep_their_meaning() {
    assert_eq!(stock_quantizer_offset_value(-1.0), "-12.00 st");
    assert_eq!(stock_quantizer_offset_value(0.125), "+1.50 st");
    assert_eq!(stock_quantizer_offset_value(1.0), "+12.00 st");
    // Key paint must use black-key precedence in the overlapping upper region,
    // and the same enabled-note bits in the exposed lower white-key region.
    for note in 0..12 {
        let key = stock_quantizer_key_rect(note);
        let point = if matches!(note, 1 | 3 | 6 | 8 | 10) {
            key.center()
        } else {
            key.center_bottom() - vec2(0.0, 10.0)
        };
        assert_eq!(stock_quantizer_hit(point), Some(note));
    }
    assert_eq!(stock_quantizer_hit(pos2(10.0, 150.0)), None);
    assert_eq!(stock_quantizer_hit(pos2(150.0, 210.0)), None);
}
