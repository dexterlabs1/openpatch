// Actual shared module paint and egui response geometry. This intentionally
// includes rectangular controls missed by the older text/circle-only checks.
use super::*;

#[derive(Clone, Copy, Debug)]
enum Position {
    Minimum,
    Default,
    Maximum,
}
#[derive(Debug)]
struct ControlGeometry {
    name: String,
    fader: Rect,
    value: Rect,
}
struct PaintedSurface {
    bounds: Rect,
    thumbs: Vec<Rect>,
    readouts: Vec<Rect>,
    texts: Vec<(String, Rect)>,
    controls: Vec<ControlGeometry>,
}

fn collect(shape: &egui::Shape, surface: &mut PaintedSurface) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, surface);
            }
        }
        // Select the filled thumb and readout well, excluding their separate
        // transparent strokes and the thumb's displaced physical shadow.
        egui::Shape::Rect(rect) if rect.fill == Color32::from_rgb(235, 231, 216) => {
            surface.thumbs.push(rect.rect);
        }
        egui::Shape::Rect(rect) if rect.fill == Color32::from_rgb(237, 233, 222) => {
            surface.readouts.push(rect.rect);
        }
        egui::Shape::Text(text) if !text.galley.text().is_empty() => surface.texts.push((
            text.galley.text().to_owned(),
            text.galley.rect.translate(text.pos.to_vec2()),
        )),
        _ => {}
    }
}

fn paint(module: &Value, position: Position, scale: f32, browser: bool) -> PaintedSurface {
    let catalog = openpatch_core::catalog();
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut preview = ModulePreview::new(module);
    let id = "geometry-target".to_owned();
    preview.patch.nodes[0].id = id.clone();
    preview.renderer.positions.clear();
    preview.renderer.positions.insert(id.clone(), Pos2::ZERO);
    for (name, spec) in module["params"].as_object().unwrap() {
        if spec["hidden"] == true {
            continue;
        }
        let field = match position {
            Position::Minimum => "min",
            Position::Default => "default",
            Position::Maximum => "max",
        };
        preview.patch.nodes[0]
            .params
            .insert(name.clone(), spec[field].as_f64().unwrap() as f32);
    }
    let patch = preview.patch.clone();
    let size = vec2(module_width(&patch.nodes[0]), PANEL_HEIGHT);
    let bounds = Rect::from_min_size(pos2(24.0, 12.0), size * scale);
    // Use preview.show's own fitting law so fractional coordinates cannot
    // introduce a false rack/preview difference.
    let fitted_scale = (bounds.width() / size.x).min(bounds.height() / size.y);
    let fitted = Rect::from_center_size(bounds.center(), size * fitted_scale);
    let mut rack = Rack {
        pan: Vec2::ZERO,
        zoom: fitted_scale,
        ..Rack::default()
    };
    rack.positions.insert(id, Pos2::ZERO);
    let mut surface = PaintedSurface {
        bounds: fitted,
        thumbs: vec![],
        readouts: vec![],
        texts: vec![],
        controls: vec![],
    };
    for _ in 0..2 {
        surface.controls.clear();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(2400.0, 1200.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        if browser {
                            preview.show(ui, bounds, &catalog);
                        } else {
                            let mut child = ui.new_child(
                                egui::UiBuilder::new()
                                    .id_salt("geometry-rack-surface")
                                    .max_rect(fitted),
                            );
                            child.set_clip_rect(ui.clip_rect().intersect(bounds));
                            let rendered = rack.render_modules(
                                &mut child,
                                &patch,
                                &catalog,
                                fitted,
                                48_000,
                                0,
                                RenderMode::Interactive {
                                    drag_interactive: false,
                                    allow_manual_press: false,
                                },
                            );
                            assert!(rendered.action.is_none());
                            // Read the hit targets registered by the actual
                            // renderer, not reconstructed radius/offset formulas.
                            let node = &patch.nodes[0];
                            for (name, _) in ordered_parameters(node, Some(module)) {
                                let fader_id = child.id().with(("knob", &node.id, name));
                                if let Some(fader) = ctx.read_response(fader_id) {
                                    let value_id = child.id().with(("value", &node.id, name));
                                    let value = ctx.read_response(value_id).unwrap_or_else(|| {
                                        panic!("{}: missing numeric target for {name}", node.kind)
                                    });
                                    surface.controls.push(ControlGeometry {
                                        name: name.to_owned(),
                                        fader: fader.interact_rect,
                                        value: value.interact_rect,
                                    });
                                }
                            }
                        }
                    });
            },
        );
        surface.thumbs.clear();
        surface.readouts.clear();
        surface.texts.clear();
        for shape in output.shapes {
            collect(&shape.shape, &mut surface);
        }
    }
    surface
}

fn assert_rects_near(a: &[Rect], b: &[Rect], context: &str) {
    assert_eq!(a.len(), b.len(), "{context}: primitive count");
    for (a, b) in a.iter().zip(b) {
        assert!(
            a.min.distance(b.min) < 0.001 && a.max.distance(b.max) < 0.001,
            "{context}: rack {a:?} versus browser {b:?}"
        );
    }
}

#[test]
fn generic_fader_rectangles_and_targets_keep_dense_labels_and_numeric_entry_separate() {
    let catalog = openpatch_core::catalog();
    // These counts require the genuinely dense paths: 24 SEQ3 cells and the
    // 17-point delay cells, analog secondary controls, and VCA MIX's 11-point
    // readout gap. Missing paint or missing response registration cannot pass.
    for (kind, count) in [
        ("seq3", 24),
        ("stereo_delay", 10),
        ("analog_vco", 3),
        ("ladder_vcf", 3),
        ("vca_mix", 5),
    ] {
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|module| module["kind"] == kind)
            .unwrap();
        for scale in [0.42, 0.85, 1.0, 2.0] {
            for position in [Position::Minimum, Position::Default, Position::Maximum] {
                let rack = paint(module, position, scale, false);
                let browser = paint(module, position, scale, true);
                let context = format!("{kind} scale={scale} position={position:?}");
                assert_eq!(rack.thumbs.len(), count, "{context}: rack painted thumbs");
                assert_eq!(
                    browser.thumbs.len(),
                    count,
                    "{context}: browser painted thumbs"
                );
                assert_eq!(rack.controls.len(), count, "{context}: real fader targets");
                assert!(
                    rack.readouts.len() >= count,
                    "{context}: numeric wells missing"
                );
                assert_rects_near(&rack.thumbs, &browser.thumbs, &context);
                assert_rects_near(&rack.readouts, &browser.readouts, &context);
                for (surface_name, surface) in [("rack", &rack), ("browser", &browser)] {
                    for (index, thumb) in surface.thumbs.iter().enumerate() {
                        assert!(
                            surface.bounds.contains_rect(*thumb),
                            "{context} {surface_name}: thumb {index} escapes panel"
                        );
                        for readout in &surface.readouts {
                            assert!(!thumb.intersects(*readout), "{context} {surface_name}: rectangular thumb {index} {thumb:?} overlaps numeric well {readout:?}");
                        }
                        for (text, bounds) in &surface.texts {
                            assert!(!thumb.intersects(*bounds), "{context} {surface_name}: rectangular thumb {index} {thumb:?} overlaps label/readout {text:?} {bounds:?}");
                        }
                        for other in &surface.thumbs[index + 1..] {
                            assert!(
                                !thumb.intersects(*other),
                                "{context} {surface_name}: adjacent thumbs overlap"
                            );
                        }
                    }
                }
                for (index, control) in rack.controls.iter().enumerate() {
                    assert!(
                        rack.bounds.contains_rect(control.fader),
                        "{context}: {} fader target escapes panel",
                        control.name
                    );
                    assert!(
                        rack.thumbs
                            .iter()
                            .any(|thumb| thumb.intersects(control.fader)),
                        "{context}: {} target has no actual rectangular thumb",
                        control.name
                    );
                    for other in &rack.controls {
                        assert!(
                            !control.fader.intersects(other.value),
                            "{context}: {} fader target {:?} overlaps {} numeric entry target {:?}",
                            control.name,
                            control.fader,
                            other.name,
                            other.value
                        );
                    }
                    for readout in &rack.readouts {
                        assert!(
                            !control.fader.intersects(*readout),
                            "{context}: {} fader target overlaps painted numeric well",
                            control.name
                        );
                    }
                    for (text, bounds) in &rack.texts {
                        assert!(!control.fader.intersects(*bounds), "{context}: {} fader target {:?} overlaps label/readout {text:?} {bounds:?}", control.name, control.fader);
                    }
                    for other in &rack.controls[index + 1..] {
                        assert!(
                            !control.fader.intersects(other.fader),
                            "{context}: {} and {} fader targets overlap",
                            control.name,
                            other.name
                        );
                        assert!(
                            !control.value.intersects(other.value),
                            "{context}: {} and {} numeric entry targets overlap",
                            control.name,
                            other.name
                        );
                    }
                }
            }
        }
    }
}
