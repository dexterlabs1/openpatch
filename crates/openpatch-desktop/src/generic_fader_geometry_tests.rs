// Actual shared module paint and egui response geometry. Rectangular controls,
// their internal choice labels and their separate numeric entry targets matter.
use super::*;

#[derive(Clone, Copy, Debug)]
enum Position {
    Minimum,
    Default,
    Maximum,
}
#[derive(Debug)]
enum Presentation {
    Fader,
    Choices {
        labels: Vec<String>,
        selected: usize,
    },
}
#[derive(Debug)]
struct ControlGeometry {
    name: String,
    presentation: Presentation,
    painted_area: Rect,
    hit: Rect,
    value: Rect,
    senses_drag: bool,
}
#[derive(Debug)]
struct PaintedText {
    text: String,
    bounds: Rect,
    color: Color32,
}
struct PaintedSurface {
    bounds: Rect,
    scale: f32,
    thumbs: Vec<Rect>,
    choices: Vec<Rect>,
    readouts: Vec<Rect>,
    fills: Vec<(Rect, Color32)>,
    texts: Vec<PaintedText>,
    controls: Vec<ControlGeometry>,
}

fn collect(shape: &egui::Shape, surface: &mut PaintedSurface) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, surface);
            }
        }
        egui::Shape::Rect(rect) => {
            surface.fills.push((rect.rect, rect.fill));
            // Thumb fill excludes its transparent stroke and displaced shadow.
            if rect.fill == Color32::from_rgb(235, 231, 216) {
                surface.thumbs.push(rect.rect);
            }
            if rect.fill == Color32::from_rgb(237, 233, 222) {
                // Choice backgrounds share the numeric well material. Their
                // actual painted heights distinguish them, rather than treating
                // every rectangle of that color as a numeric input.
                let height = rect.rect.height() / surface.scale;
                if (height - 17.0).abs() < 0.001 {
                    surface.readouts.push(rect.rect);
                } else if (height - 24.0).abs() < 0.001 {
                    surface.choices.push(rect.rect);
                } else {
                    panic!("unclassified shared entry material at height {height}");
                }
            }
        }
        egui::Shape::Text(text) if !text.galley.text().is_empty() => {
            surface.texts.push(PaintedText {
                text: text.galley.text().to_owned(),
                bounds: text.galley.rect.translate(text.pos.to_vec2()),
                color: text.override_text_color.unwrap_or(text.fallback_color),
            });
        }
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
    // Use preview.show's fitting law; fractional fitting must not create a
    // false difference between the actual rack and browser paint.
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
        scale: fitted_scale,
        thumbs: vec![],
        choices: vec![],
        readouts: vec![],
        fills: vec![],
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
                            let node = &patch.nodes[0];
                            for (name, spec) in ordered_parameters(node, Some(module)) {
                                // Both presentations intentionally use the same
                                // canonical response identity; inspect the actual
                                // response and explicit choice metadata together.
                                let id = child.id().with(("knob", &node.id, name));
                                if let Some(control) = ctx.read_response(id) {
                                    let value_id = child.id().with(("value", &node.id, name));
                                    let value = ctx.read_response(value_id).unwrap_or_else(|| {
                                        panic!("{}: missing numeric target for {name}", node.kind)
                                    });
                                    let presentation = ParameterChoices::from_spec(spec).map_or(
                                        Presentation::Fader,
                                        |choices| Presentation::Choices {
                                            labels: choices
                                                .labels
                                                .iter()
                                                .map(|s| s.as_str().unwrap().to_owned())
                                                .collect(),
                                            selected: choices.index(node.params[name]),
                                        },
                                    );
                                    surface.controls.push(ControlGeometry {
                                        name: name.to_owned(),
                                        presentation,
                                        painted_area: control.rect,
                                        hit: control.interact_rect,
                                        value: value.interact_rect,
                                        senses_drag: control.sense.senses_drag(),
                                    });
                                }
                            }
                        }
                    });
            },
        );
        surface.thumbs.clear();
        surface.choices.clear();
        surface.readouts.clear();
        surface.fills.clear();
        surface.texts.clear();
        for shape in output.shapes {
            collect(&shape.shape, &mut surface);
        }
    }
    surface
}

fn rect_near(a: Rect, b: Rect) -> bool {
    a.min.distance(b.min) < 0.001 && a.max.distance(b.max) < 0.001
}
fn assert_rects_near(a: &[Rect], b: &[Rect], context: &str) {
    assert_eq!(a.len(), b.len(), "{context}: primitive count");
    for (a, b) in a.iter().zip(b) {
        assert!(
            rect_near(*a, *b),
            "{context}: rack {a:?} versus browser {b:?}"
        );
    }
}
fn own_choice_label(control: &ControlGeometry, text: &PaintedText) -> bool {
    match &control.presentation {
        Presentation::Choices { labels, .. } => {
            labels.contains(&text.text) && control.painted_area.contains_rect(text.bounds)
        }
        Presentation::Fader => false,
    }
}

#[test]
fn generic_fader_rectangles_and_targets_keep_dense_labels_and_numeric_entry_separate() {
    let catalog = openpatch_core::catalog();
    // Exact continuous/choice counts prevent missing paint, accidental fallback
    // to a thumb, or missing actual response registration from passing.
    for (kind, faders, choices) in [
        ("seq3", 24, &[][..]),
        ("stereo_delay", 10, &[][..]),
        ("analog_vco", 2, &["waveform"][..]),
        ("ladder_vcf", 3, &[][..]),
        ("vca_mix", 5, &[][..]),
        ("crossfade", 1, &["law"][..]),
        ("audio_stereo", 1, &["dc_block"][..]),
        ("score_sequencer", 2, &["running"][..]),
        ("drum_voice", 5, &["mode"][..]),
    ] {
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        for scale in [0.42, 0.85, 1.0, 2.0] {
            for position in [Position::Minimum, Position::Default, Position::Maximum] {
                let rack = paint(module, position, scale, false);
                let browser = paint(module, position, scale, true);
                let context = format!("{kind} scale={scale} position={position:?}");
                let count = faders + choices.len();
                assert_eq!(
                    rack.controls.len(),
                    count,
                    "{context}: actual parameter targets"
                );
                let actual_choices: Vec<_> = rack
                    .controls
                    .iter()
                    .filter(|control| matches!(control.presentation, Presentation::Choices { .. }))
                    .map(|control| control.name.as_str())
                    .collect();
                assert_eq!(
                    actual_choices, choices,
                    "{context}: categorical target identity"
                );
                for (name, surface) in [("rack", &rack), ("browser", &browser)] {
                    assert_eq!(
                        surface.thumbs.len(),
                        faders,
                        "{context}: {name} continuous thumbs"
                    );
                    assert_eq!(
                        surface.choices.len(),
                        choices.len(),
                        "{context}: {name} categorical backgrounds"
                    );
                    assert!(
                        surface.readouts.len() >= count,
                        "{context}: {name} numeric wells missing"
                    );
                }
                assert_rects_near(&rack.thumbs, &browser.thumbs, &context);
                assert_rects_near(&rack.choices, &browser.choices, &context);
                assert_rects_near(&rack.readouts, &browser.readouts, &context);
                assert_eq!(
                    rack.texts.len(),
                    browser.texts.len(),
                    "{context}: text count"
                );
                for (a, b) in rack.texts.iter().zip(&browser.texts) {
                    assert_eq!(
                        (&a.text, a.color),
                        (&b.text, b.color),
                        "{context}: rack/browser text or selection color"
                    );
                    assert!(
                        rect_near(a.bounds, b.bounds),
                        "{context}: rack/browser text bounds for {}",
                        a.text
                    );
                }
                for (surface_name, surface) in [("rack", &rack), ("browser", &browser)] {
                    for (index, thumb) in surface.thumbs.iter().enumerate() {
                        assert!(
                            surface.bounds.contains_rect(*thumb),
                            "{context} {surface_name}: thumb {index} escapes panel"
                        );
                        for well in surface.readouts.iter().chain(&surface.choices) {
                            assert!(!thumb.intersects(*well), "{context} {surface_name}: thumb {index} {thumb:?} overlaps entry/choice paint {well:?}");
                        }
                        for text in &surface.texts {
                            assert!(!thumb.intersects(text.bounds), "{context} {surface_name}: thumb {index} overlaps label/readout {:?}",text);
                        }
                        for other in &surface.thumbs[index + 1..] {
                            assert!(
                                !thumb.intersects(*other),
                                "{context} {surface_name}: adjacent thumbs overlap"
                            );
                        }
                    }
                    for (index, choice) in surface.choices.iter().enumerate() {
                        assert!(
                            surface.bounds.contains_rect(*choice),
                            "{context} {surface_name}: choice escapes panel"
                        );
                        let control = rack
                            .controls
                            .iter()
                            .find(|control| {
                                matches!(control.presentation, Presentation::Choices { .. })
                                    && rect_near(control.painted_area, *choice)
                            })
                            .expect("choice paint must match an actual categorical response");
                        for well in &surface.readouts {
                            assert!(
                                !choice.intersects(*well),
                                "{context} {surface_name}: {} choice paint overlaps numeric well",
                                control.name
                            );
                        }
                        for other in &surface.choices[index + 1..] {
                            assert!(
                                !choice.intersects(*other),
                                "{context} {surface_name}: neighboring choice paints overlap"
                            );
                        }
                        for text in &surface.texts {
                            if !own_choice_label(control, text) {
                                assert!(!choice.intersects(text.bounds),"{context} {surface_name}: {} choice paint overlaps external label/readout {:?}",control.name,text);
                            }
                        }
                        let Presentation::Choices { labels, selected } = &control.presentation
                        else {
                            unreachable!()
                        };
                        for (index, label) in labels.iter().enumerate() {
                            let matches: Vec<_> = surface
                                .texts
                                .iter()
                                .filter(|text| {
                                    text.text == *label && choice.contains_rect(text.bounds)
                                })
                                .collect();
                            assert_eq!(matches.len(),1,"{context} {surface_name}: {} choice label {label:?} missing, clipped or duplicated",control.name);
                            assert_eq!(
                                matches[0].color,
                                if index == *selected {
                                    Color32::from_rgb(249, 246, 235)
                                } else {
                                    INK
                                },
                                "{context} {surface_name}: selected label color"
                            );
                        }
                        let selected_fills: Vec<_> = surface
                            .fills
                            .iter()
                            .filter(|(rect, color)| {
                                *color == crate::panel_skin::family_accent(kind)
                                    && choice.contains_rect(*rect)
                            })
                            .collect();
                        assert_eq!(
                            selected_fills.len(),
                            1,
                            "{context} {surface_name}: one actual selected-segment fill required"
                        );
                    }
                }
                for (index, control) in rack.controls.iter().enumerate() {
                    assert!(
                        rack.bounds.contains_rect(control.hit),
                        "{context}: {} control target escapes panel",
                        control.name
                    );
                    match &control.presentation {
                        Presentation::Fader => {
                            assert!(
                                control.senses_drag,
                                "{context}: {} continuous target lost drag sense",
                                control.name
                            );
                            assert_eq!(
                                rack.thumbs
                                    .iter()
                                    .filter(|thumb| thumb.intersects(control.hit))
                                    .count(),
                                1,
                                "{context}: {} target must have one actual thumb",
                                control.name
                            );
                        }
                        Presentation::Choices { .. } => {
                            assert!(
                                !control.senses_drag,
                                "{context}: {} categorical target must not drag",
                                control.name
                            );
                            assert!(
                                rect_near(control.hit, control.painted_area),
                                "{context}: {} categorical hit area differs from paint",
                                control.name
                            );
                            assert_eq!(
                                rack.choices
                                    .iter()
                                    .filter(|rect| rect_near(**rect, control.painted_area))
                                    .count(),
                                1,
                                "{context}: {} missing categorical paint",
                                control.name
                            );
                            assert!(
                                !rack
                                    .thumbs
                                    .iter()
                                    .any(|thumb| thumb.intersects(control.hit)),
                                "{context}: {} categorical control retains an unexpected thumb",
                                control.name
                            );
                        }
                    }
                    for other in &rack.controls {
                        assert!(
                            !control.hit.intersects(other.value),
                            "{context}: {} control target {:?} overlaps {} numeric target {:?}",
                            control.name,
                            control.hit,
                            other.name,
                            other.value
                        );
                    }
                    for well in &rack.readouts {
                        assert!(
                            !control.hit.intersects(*well),
                            "{context}: {} control target overlaps numeric paint",
                            control.name
                        );
                    }
                    for text in &rack.texts {
                        if !own_choice_label(control, text) {
                            assert!(
                                !control.hit.intersects(text.bounds),
                                "{context}: {} target overlaps external label/readout {:?}",
                                control.name,
                                text
                            );
                        }
                    }
                    for other in &rack.controls[index + 1..] {
                        assert!(
                            !control.hit.intersects(other.hit),
                            "{context}: {} and {} parameter targets overlap",
                            control.name,
                            other.name
                        );
                        assert!(
                            !control.value.intersects(other.value),
                            "{context}: {} and {} numeric targets overlap",
                            control.name,
                            other.name
                        );
                    }
                }
            }
        }
    }
}
