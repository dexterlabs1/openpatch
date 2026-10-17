use super::*;

fn split_patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"split","kind":"poly_split"}]})).unwrap()
}
fn split_texts(shape: &egui::Shape, out: &mut Vec<(String, Rect, Color32)>) {
    match shape {
        egui::Shape::Vec(shapes) => for shape in shapes { split_texts(shape, out); },
        egui::Shape::Text(text) => out.push((text.galley.text().into(), Rect::from_min_size(text.pos, text.galley.size()), text.override_text_color.unwrap_or(text.fallback_color))),
        _ => {}
    }
}
fn split_drawn(output: &egui::FullOutput) -> Vec<(String, Rect, Color32)> {
    let mut out = vec![];
    for shape in &output.shapes { split_texts(&shape.shape, &mut out); }
    out
}
fn split_readout(output: &egui::FullOutput) -> (String, Rect, Color32) {
    split_drawn(output).into_iter().find(|(_, rect, _)| rect.center().distance(pos2(261.5, 95.0)) < 2.0).expect("Measured input-channel readout")
}

#[test]
fn split_shows_only_valid_measured_counts_and_clears_unavailable_state() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut rack = Rack::default();
    let patch = split_patch();
    for (channels, expected) in [(None, "—"), (Some(0), "0"), (Some(1), "1"), (Some(16), "16"), (Some(17), "—"), (Some(255), "—")] {
        rack.set_module_states(BTreeMap::from([("split".into(), ModuleLightState { input_channels: channels, count: 0, ..Default::default() })]), BTreeMap::new());
        let (_, output) = frame_output(&ctx, &mut rack, &patch, 1.0, vec![]);
        assert_eq!(split_readout(&output).0, expected);
    }
    // An unrelated live module and sixteen available outputs cannot supply this count.
    rack.set_module_states(BTreeMap::from([("other".into(), ModuleLightState { input_channels: Some(16), ..Default::default() })]), BTreeMap::new());
    let (_, output) = frame_output(&ctx, &mut rack, &patch, 2.0, vec![]);
    assert_eq!(split_readout(&output).0, "—");
    rack.set_module_states(BTreeMap::new(), BTreeMap::new());
    let (_, output) = frame_output(&ctx, &mut rack, &patch, 3.0, vec![]);
    assert_eq!(split_readout(&output).0, "—");
}

#[test]
fn split_valid_zero_and_active_counts_remain_emissive_in_darkroom() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut rack = Rack::default();
    let patch = split_patch();
    for channels in [0, 1, 16] {
        rack.set_module_states(BTreeMap::from([("split".into(), ModuleLightState { input_channels: Some(channels), count: 0, ..Default::default() })]), BTreeMap::new());
        for room in [1.0, 0.0] {
            rack.set_room_brightness(room);
            let (_, output) = frame_output(&ctx, &mut rack, &patch, 1.0, vec![]);
            let readout = split_readout(&output);
            assert_eq!(readout.0, channels.to_string());
            assert_eq!(readout.2, Color32::from_rgb(155, 220, 197));
        }
    }
    rack.set_module_states(BTreeMap::new(), BTreeMap::new());
    let (_, output) = frame_output(&ctx, &mut rack, &patch, 2.0, vec![]);
    let unavailable = split_readout(&output);
    assert_eq!(unavailable.0, "—");
    assert_ne!(unavailable.2, Color32::from_rgb(155, 220, 197));
}

#[test]
fn split_all_sixteen_named_outputs_accept_real_pointer_cable_starts() {
    for index in 1..=16 {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let patch = split_patch();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        frame(&ctx, &mut rack, &patch, 0.1, vec![]);
        let port = format!("out{index}");
        let at = fixture_jack(&rack, &patch, "split", &port, false);
        assert!(frame(&ctx, &mut rack, &patch, 1.0, vec![egui::Event::PointerMoved(at), button(at, true)]).is_none());
        assert!(frame(&ctx, &mut rack, &patch, 1.1, vec![egui::Event::PointerMoved(at), button(at, false)]).is_none());
        assert!(matches!(rack.pending.as_ref().map(|pending| &pending.origin), Some(CableEnd::Output { node, port: actual }) if node == "split" && actual == &port), "{port} must remain accessible as an independent cable target");
    }
}

fn split_primitive(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Vec(shapes) => for shape in shapes { split_primitive(shape, out); },
        egui::Shape::Text(text) => out.push(format!("text:{:?}:{:?}:{}:{:?}:{:?}:{:?}", text.pos, text.galley.size(), text.galley.text(), text.galley.job.sections, text.fallback_color, text.override_text_color)),
        egui::Shape::Rect(rect) => out.push(format!("rect:{rect:?}")),
        egui::Shape::Circle(circle) => out.push(format!("circle:{circle:?}")),
        egui::Shape::LineSegment { points, stroke } => out.push(format!("line:{points:?}:{stroke:?}")),
        egui::Shape::Mesh(mesh) => out.push(format!("mesh:{:?}:{:?}", mesh.vertices, mesh.indices)),
        other => out.push(format!("shape:{other:?}")),
    }
}
fn split_screen(shape: &egui::Shape, out: &mut Vec<Rect>) {
    match shape {
        egui::Shape::Vec(shapes) => for shape in shapes { split_screen(shape, out); },
        egui::Shape::Rect(rect) if rect.fill == Color32::from_rgb(15, 24, 18) => out.push(rect.rect),
        _ => {}
    }
}

#[test]
fn split_rack_browser_share_honest_unavailable_readout_and_preserve_all_socket_geometry() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"].as_array().unwrap().iter().find(|module| module["kind"] == "poly_split").unwrap();
    for scale in [0.42, 0.7, 0.85, 1.0, 1.75] {
        let mut surfaces = vec![];
        for browser in [false, true] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut preview = ModulePreview::new(module);
            let size = vec2(panel_width("poly_split"), PANEL_HEIGHT);
            let bounds = Rect::from_min_size(pos2(24.0, 12.0), size * scale);
            let zoom = (bounds.width() / size.x).min(bounds.height() / size.y);
            let fitted = Rect::from_center_size(bounds.center(), size * zoom);
            let mut rack = Rack { zoom, pan: Vec2::ZERO, ..Default::default() };
            rack.positions.insert("split".into(), Pos2::ZERO);
            let patch = split_patch();
            let mut output = egui::FullOutput::default();
            for _ in 0..2 {
                output = ctx.run(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1100.0))), ..Default::default() }, |ctx| {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                        if browser { preview.show(ui, bounds, &catalog); }
                        else {
                            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(fitted));
                            child.set_clip_rect(ui.clip_rect().intersect(bounds));
                            let rendered = rack.render_modules(&mut child, &patch, &catalog, fitted, 48000, 0, RenderMode::Interactive { drag_interactive: false, allow_manual_press: false });
                            assert!(rendered.action.is_none());
                            assert_eq!(rendered.inputs.len(), 1);
                            assert_eq!(rendered.outputs.len(), 16);
                            let signal = fitted.min + pos2(150.0, 126.0).to_vec2() * zoom;
                            assert!(rendered.inputs[&("split".into(), "signal".into())].distance(signal) < 0.001);
                            for index in 0..16 {
                                let expected = fitted.min + pos2(60.0 * ((index % 4) + 1) as f32, 187.0 + (index / 4) as f32 * 58.0).to_vec2() * zoom;
                                assert!(rendered.outputs[&("split".into(), format!("out{}", index + 1))].distance(expected) < 0.001);
                            }
                        }
                    });
                });
            }
            let labels = split_drawn(&output);
            for required in ["SPLIT", "INPUT CHANNELS", "—", "IN"] {
                assert!(labels.iter().any(|(text, _, _)| text == required), "{scale} browser={browser}: missing {required}");
            }
            for index in 1..=16 { assert!(labels.iter().any(|(text, _, _)| text == &format!("OUT{index}"))); }
            assert!(!labels.iter().any(|(text, _, _)| text == "16 named mono outputs" || text == "16"));
            let mut screens = vec![];
            for shape in &output.shapes { split_screen(&shape.shape, &mut screens); }
            assert_eq!(screens.len(), 1);
            let screen = screens[0];
            assert!(screen.top() >= fitted.min.y + 69.0 * zoom, "Measured display must clear title/header hit region");
            let count = labels.iter().find(|(text, _, _)| text == "—").unwrap().1;
            assert!(screen.expand(0.5).contains_rect(count), "{scale}: full count line box escaped display");
            for (i, (label, rect, _)) in labels.iter().enumerate() {
                assert!(bounds.expand(0.5).contains_rect(*rect), "{scale}: {label} escaped panel bounds");
                for (other, other_rect, _) in &labels[i + 1..] {
                    assert!(!rect.shrink(0.2 * scale).intersects(other_rect.shrink(0.2 * scale)), "{scale}: {label} overlaps {other}");
                }
                if ["INPUT CHANNELS", "—"].contains(&label.as_str()) {
                    for input in [true, false] {
                        let count = if input { 1 } else { 16 };
                        for index in 0..count {
                            let center = fitted.min + port_position("poly_split", input, index, count, size.x).to_vec2() * zoom;
                            let jack = Rect::from_center_size(center, vec2(29.0, 29.0) * zoom);
                            assert!(!rect.intersects(jack) && !screen.intersects(jack), "{scale}: measured count overlaps a socket");
                        }
                    }
                }
            }
            let mut primitives = vec![];
            for shape in &output.shapes { split_primitive(&shape.shape, &mut primitives); }
            surfaces.push(primitives);
            assert!(preview.renderer.editing.is_none());
        }
        assert_eq!(surfaces[0], surfaces[1], "Split rack/browser primitives diverged at {scale}");
    }
}
