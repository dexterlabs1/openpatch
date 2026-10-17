use super::*;

fn patch(kind: &str) -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"matrix","kind":kind}]})).unwrap()
}

// Named physical positions are independent of either catalog's port ordering.
const INPUTS: [(&str, f32, f32); 6] = [
    ("left", 45.0, 245.0),
    ("right", 105.0, 245.0),
    ("mid", 195.0, 245.0),
    ("side", 255.0, 245.0),
    ("encode_cv", 75.0, 305.0),
    ("decode_cv", 225.0, 305.0),
];
const OUTPUTS: [(&str, f32, f32); 4] = [
    ("mid", 45.0, 374.0),
    ("side", 105.0, 374.0),
    ("left", 195.0, 374.0),
    ("right", 255.0, 374.0),
];

fn primitives(output: &egui::FullOutput) -> Vec<String> {
    fn visit(shape: &egui::Shape, result: &mut Vec<String>) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, result);
                }
            }
            egui::Shape::Text(text) => result.push(format!(
                "text:{:?}:{:?}:{}:{:?}:{:?}:{:?}",
                text.pos,
                text.galley.size(),
                text.galley.text(),
                text.galley.job.sections,
                text.fallback_color,
                text.override_text_color
            )),
            shape => result.push(format!("{shape:?}")),
        }
    }
    let mut result = vec![];
    for shape in &output.shapes {
        visit(&shape.shape, &mut result);
    }
    result
}

fn texts(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    fn visit(shape: &egui::Shape, result: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, result);
                }
            }
            egui::Shape::Text(text) if !text.galley.text().is_empty() => result.push((
                text.galley.text().into(),
                Rect::from_min_size(text.pos, text.galley.size()),
            )),
            _ => {}
        }
    }
    let mut result = vec![];
    for shape in &output.shapes {
        visit(&shape.shape, &mut result);
    }
    result
}

#[test]
fn stock_mid_side_metadata_preserves_distinct_kind_and_legacy_panel_contract() {
    let catalog = openpatch_core::catalog();
    for (kind, heading) in [
        ("stock_mid_side", "STOCK MID/SIDE"),
        ("mid_side", "MID / SIDE"),
    ] {
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|module| module["kind"] == kind)
            .unwrap();
        assert_eq!(title(kind), heading);
        assert_eq!(panel_width(kind), 300.0);
        assert_eq!(
            crate::panel_skin::family_accent(kind),
            Color32::from_rgb(79, 116, 141)
        );
        assert_eq!(
            ordered_parameters(&patch(kind).nodes[0], Some(module))
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>(),
            ["encode_width", "decode_width"]
        );
        assert_eq!(module["outputs"], json!(["mid", "side", "left", "right"]));
        let expected_inputs = if kind == "stock_mid_side" {
            json!(["encode_cv", "left", "right", "decode_cv", "mid", "side"])
        } else {
            json!(["left", "right", "mid", "side", "encode_cv", "decode_cv"])
        };
        assert_eq!(module["inputs"], expected_inputs);
    }
}

#[test]
fn stock_mid_side_shared_renderer_preserves_named_jacks_labels_and_preview_geometry() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "stock_mid_side")
        .unwrap();
    for scale in [0.25, 0.42, 0.7, 1.0, 1.75] {
        let mut surfaces = vec![];
        for browser in [false, true] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let size = vec2(300.0, PANEL_HEIGHT);
            let bounds = Rect::from_min_size(pos2(24.0, 12.0), size * scale);
            let zoom = (bounds.width() / size.x).min(bounds.height() / size.y);
            let fitted = Rect::from_center_size(bounds.center(), size * zoom);
            let mut preview = ModulePreview::new(module);
            assert_eq!(preview.size(), size);
            let mut rack = Rack {
                zoom,
                pan: Vec2::ZERO,
                ..Default::default()
            };
            rack.positions.insert("matrix".into(), Pos2::ZERO);
            let patch = patch("stock_mid_side");
            let mut output = egui::FullOutput::default();
            for _ in 0..2 {
                output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 900.0))),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE)
                            .show(ctx, |ui| {
                                if browser {
                                    preview.show(ui, bounds, &catalog);
                                } else {
                                    let mut child =
                                        ui.new_child(egui::UiBuilder::new().max_rect(fitted));
                                    child.set_clip_rect(ui.clip_rect().intersect(bounds));
                                    let rendered = rack.render_modules(
                                        &mut child,
                                        &patch,
                                        &catalog,
                                        fitted,
                                        48000,
                                        0,
                                        RenderMode::Interactive {
                                            drag_interactive: false,
                                            allow_manual_press: false,
                                        },
                                    );
                                    assert!(rendered.action.is_none());
                                    assert_eq!(
                                        (rendered.inputs.len(), rendered.outputs.len()),
                                        (6, 4)
                                    );
                                    assert_eq!(rendered.jack_targets.len(), 10);
                                    for (input, ports) in
                                        [(true, INPUTS.as_slice()), (false, OUTPUTS.as_slice())]
                                    {
                                        for &(port, x, y) in ports {
                                            let expected = fitted.min + vec2(x, y) * zoom;
                                            let positions = if input {
                                                &rendered.inputs
                                            } else {
                                                &rendered.outputs
                                            };
                                            let actual = positions[&("matrix".into(), port.into())];
                                            assert!(
                                                actual.distance(expected) < 0.001,
                                                "{port} input={input}: {actual:?} != {expected:?}"
                                            );
                                            let endpoint = if input {
                                                CableEnd::Input {
                                                    node: "matrix".into(),
                                                    port: port.into(),
                                                }
                                            } else {
                                                CableEnd::Output {
                                                    node: "matrix".into(),
                                                    port: port.into(),
                                                }
                                            };
                                            assert!(rendered
                                                .jack_targets
                                                .iter()
                                                .any(|(end, at)| *end == endpoint
                                                    && at.distance(expected) < 0.001));
                                        }
                                    }
                                }
                            });
                    },
                );
            }
            let text = texts(&output);
            for required in [
                "STOCK MID/SIDE",
                "ENCODER",
                "DECODER",
                "M = (L + R) / 2",
                "R = M − S × D",
            ] {
                assert!(
                    text.iter().any(|(label, _)| label == required),
                    "{scale} browser={browser}: missing {required}"
                );
            }
            for label in ["WIDTH", "WIDTH CV", "LEFT", "RIGHT", "MID", "SIDE"] {
                assert_eq!(
                    text.iter().filter(|(actual, _)| actual == label).count(),
                    2,
                    "{scale} browser={browser}: {label}"
                );
            }
            for (label, rect) in &text {
                assert!(
                    bounds.expand(0.5).contains_rect(*rect),
                    "{scale} browser={browser}: {label} clipped at {rect:?}"
                );
            }
            for section in ["ENCODER", "DECODER"] {
                let heading = text.iter().find(|(label, _)| label == section).unwrap().1;
                let width = text
                    .iter()
                    .filter(|(label, _)| label == "WIDTH")
                    .map(|(_, rect)| *rect)
                    .min_by(|a, b| {
                        (a.center().x - heading.center().x)
                            .abs()
                            .total_cmp(&(b.center().x - heading.center().x).abs())
                    })
                    .unwrap();
                assert!(
                    heading.bottom() + zoom <= width.top(),
                    "{scale} browser={browser}: {section} crowds WIDTH: {heading:?} {width:?}"
                );
                assert!(heading.top() >= fitted.top() + 65.0 * zoom);
            }
            assert!(preview.renderer.editing.is_none());
            assert!(preview.renderer.pending.is_none());
            surfaces.push(primitives(&output));
        }
        assert_eq!(surfaces[0], surfaces[1], "Rack/browser mismatch at {scale}");
    }
}

#[test]
fn stock_mid_side_real_readout_clicks_select_encoder_then_decoder() {
    for (index, parameter) in ["encode_width", "decode_width"].into_iter().enumerate() {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut rack = Rack::default();
        let patch = patch("stock_mid_side");
        let mut knobs = [Rect::NOTHING; 2];
        let mut values = [Rect::NOTHING; 2];
        for frame in 0..4 {
            let events = if frame >= 2 {
                let at = values[index].center();
                vec![egui::Event::PointerMoved(at), button(at, frame == 2)]
            } else {
                vec![]
            };
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                    time: Some(frame as f64 * 0.1),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            assert!(rack
                                .show(ui, &patch, &openpatch_core::catalog(), 48000, 17)
                                .is_none());
                            for (i, name) in
                                ["encode_width", "decode_width"].into_iter().enumerate()
                            {
                                knobs[i] = ctx
                                    .read_response(ui.id().with(("knob", "matrix", name)))
                                    .unwrap()
                                    .rect;
                                values[i] = ctx
                                    .read_response(ui.id().with(("value", "matrix", name)))
                                    .unwrap()
                                    .rect;
                            }
                        });
                },
            );
        }
        assert!(knobs[0].right() < knobs[1].left());
        assert_eq!(knobs[0].center().y, knobs[1].center().y);
        assert!(values[0].right() < values[1].left());
        let edit = rack
            .editing
            .as_ref()
            .expect("Actual readout opens precision entry");
        assert_eq!(edit.parameter, parameter);
        assert_eq!(edit.revision, 17);
    }
}
