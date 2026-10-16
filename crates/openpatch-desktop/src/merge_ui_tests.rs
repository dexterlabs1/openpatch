use super::*;

fn merge_patch(value: Option<f32>) -> Patch {
    let mut node = json!({"id":"merge","kind":"poly_merge"});
    if let Some(value) = value {
        node["params"] = json!({"channels":value});
    }
    serde_json::from_value(json!({"version":2,"nodes":[node]})).unwrap()
}

fn texts(shape: &egui::Shape, out: &mut Vec<(String, Rect, Color32)>) {
    match shape {
        egui::Shape::Text(t) => out.push((
            t.galley.text().to_owned(), Rect::from_min_size(t.pos, t.galley.size()),
            t.override_text_color.unwrap_or(t.fallback_color),
        )),
        egui::Shape::Vec(shapes) => for shape in shapes { texts(shape, out); },
        _ => {}
    }
}

fn drawn(output: &egui::FullOutput) -> Vec<(String, Rect, Color32)> {
    let mut result = vec![];
    for shape in &output.shapes { texts(&shape.shape, &mut result); }
    result
}

fn text_at(output: &egui::FullOutput, label: &str) -> Pos2 {
    drawn(output).iter().rev().find(|(text, _, _)| text == label)
        .unwrap_or_else(|| panic!("Missing actual Merge label {label}")).1.center()
}

struct Fixture {
    ctx: egui::Context,
    rack: Rack,
    session: openpatch::control::Session,
    _workspace: tempfile::TempDir,
    time: f64,
    output: egui::FullOutput,
}

impl Fixture {
    fn new(value: Option<f32>, zoom: f32) -> Self {
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(merge_patch(value), None).unwrap();
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut fixture = Self {
            ctx, rack: Rack { zoom, ..Default::default() }, session,
            _workspace: workspace, time: 0.0, output: Default::default(),
        };
        fixture.draw(vec![]);
        fixture.draw(vec![]);
        fixture
    }
    fn draw(&mut self, events: Vec<egui::Event>) -> Option<RackAction> {
        let mut action = None;
        self.output = self.ctx.run(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1100.0))),
            time: Some(self.time), focused: true, events, ..Default::default()
        }, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                action = self.rack.show(ui, self.session.patch(), &openpatch_core::catalog(), 48000, self.session.revision());
            });
        });
        self.time += 0.1;
        action
    }
    fn click(&mut self, at: Pos2) -> Option<RackAction> {
        assert!(self.draw(vec![egui::Event::PointerMoved(at), button(at, true)]).is_none());
        self.draw(vec![egui::Event::PointerMoved(at), button(at, false)])
    }
    fn open(&mut self) {
        let value = self.session.patch().nodes[0].params.get("channels").copied().unwrap_or(0.0);
        let at = text_at(&self.output, &merge_channels_label(value));
        assert!(self.click(at).is_none());
        self.draw(vec![]);
        self.draw(vec![]);
        assert!(egui::Popup::is_any_open(&self.ctx));
    }
    fn choose(&mut self, label: &str) -> Option<RackAction> {
        self.open();
        let at = text_at(&self.output, label);
        self.click(at)
    }
}

#[test]
fn merge_pointer_dropdown_selects_auto_forced_zero_and_sixteen_as_single_guarded_edits() {
    for (label, expected) in [("Auto", 0.0), ("0", -1.0), ("16", 16.0)] {
        let mut f = Fixture::new(Some(8.0), 1.0);
        let before = f.session.snapshot();
        let revision = f.session.revision();
        let action = f.choose(label).expect("A changed row must emit one edit");
        let RackAction::Command("set_parameter", args) = action else { panic!("Expected a channel choice edit") };
        assert_eq!(args, json!({"id":"merge","parameter":"channels","value":expected,"expected_revision":revision}));
        f.session.execute("set_parameter", args).unwrap();
        assert_eq!(f.session.patch().nodes[0].params["channels"], expected);
        let after = f.session.snapshot();
        assert_eq!(after["revision"].as_u64(), Some(revision + 1));
        assert_eq!(after["history"]["undo_steps"].as_u64(), Some(before["history"]["undo_steps"].as_u64().unwrap() + 1));
        assert!(f.draw(vec![]).is_none(), "Settling the selection must not emit another edit");
        assert!(!egui::Popup::is_any_open(&f.ctx));
    }
}

#[test]
fn merge_noop_auto_and_stale_dropdown_preserve_the_session_command_boundary() {
    let mut f = Fixture::new(None, 1.0);
    let before = f.session.snapshot();
    assert!(f.choose("Auto").is_none(), "An omitted channels parameter is already Auto");
    assert_eq!(f.session.snapshot(), before);

    let mut f = Fixture::new(Some(8.0), 1.0);
    let opening_revision = f.session.revision();
    f.open();
    f.session.execute("set_parameter", json!({"id":"merge","parameter":"channels","value":4})).unwrap();
    f.draw(vec![]);
    let at = text_at(&f.output, "16");
    let Some(RackAction::Command("set_parameter", args)) = f.click(at) else { panic!("Expected guarded stale choice") };
    assert_eq!(args["expected_revision"], opening_revision);
    let before = f.session.snapshot();
    assert!(f.session.execute("set_parameter", args).is_err(), "A dropdown opened on an older revision cannot overwrite a newer edit");
    assert_eq!(f.session.snapshot(), before);
}

#[test]
fn merge_measured_zero_is_distinct_from_unavailable_and_does_not_guess_from_parameters() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut rack = Rack::default();
    let patch = merge_patch(Some(16.0));
    for (output_channels, expected) in [(None, "—"), (Some(0), "0"), (Some(16), "16"), (Some(17), "—")] {
        rack.module_lights.insert("merge".into(), ModuleLightState {
            output_channels, automatic_channels: Some(7), count: 0, ..Default::default()
        });
        let (_, output) = frame_output(&ctx, &mut rack, &patch, 1.0, vec![]);
        let readout = drawn(&output).into_iter().find(|(_, rect, _)| (rect.center() - pos2(261.5, 368.0)).length() < 3.0)
            .expect("Measured output count screen text");
        assert_eq!(readout.0, expected, "Saved forced channels must never substitute for measured output");
    }
    rack.module_lights.clear();
    let (_, output) = frame_output(&ctx, &mut rack, &patch, 2.0, vec![]);
    assert!(drawn(&output).iter().any(|(text, _, _)| text == "—"));
}

#[test]
fn merge_measured_output_display_emits_in_darkroom_including_valid_zero() {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut rack = Rack::default();
    let patch = merge_patch(None);
    for channels in [0, 9, 16] {
        rack.module_lights.insert("merge".into(), ModuleLightState {
            output_channels: Some(channels), automatic_channels: Some(9), count: 0, ..Default::default()
        });
        let mut colors = vec![];
        for room in [1.0, 0.0] {
            rack.set_room_brightness(room);
            let (_, output) = frame_output(&ctx, &mut rack, &patch, 1.0, vec![]);
            colors.push(drawn(&output).into_iter().find(|(text, _, _)| text == &channels.to_string()).unwrap().2);
        }
        assert_eq!(colors[0], Color32::from_rgb(155, 220, 197));
        assert_eq!(colors[0], colors[1], "A valid measured channel count remains emissive");
    }
    rack.module_lights.clear();
    let (_, dark) = frame_output(&ctx, &mut rack, &patch, 2.0, vec![]);
    let unavailable = drawn(&dark).into_iter().find(|(text, _, _)| text == "—").unwrap();
    assert_ne!(unavailable.2, Color32::from_rgb(155, 220, 197));
}

fn primitive(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Vec(shapes) => for shape in shapes { primitive(shape, out); },
        egui::Shape::Text(t) => out.push(format!("text:{:?}:{:?}:{}:{:?}:{:?}:{:?}", t.pos, t.galley.size(), t.galley.text(), t.galley.job.sections, t.fallback_color, t.override_text_color)),
        egui::Shape::Rect(r) => out.push(format!("rect:{r:?}")),
        egui::Shape::Circle(c) => out.push(format!("circle:{c:?}")),
        egui::Shape::LineSegment { points, stroke } => out.push(format!("line:{points:?}:{stroke:?}")),
        egui::Shape::Mesh(m) => out.push(format!("mesh:{:?}:{:?}", m.vertices, m.indices)),
        other => out.push(format!("shape:{other:?}")),
    }
}

#[test]
fn merge_rack_and_browser_share_geometry_with_separate_jacks_and_readable_channel_rows() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"].as_array().unwrap().iter().find(|m| m["kind"] == "poly_merge").unwrap();
    for scale in [0.42, 0.7, 0.85, 1.0, 1.75] {
        let mut surfaces = vec![];
        for browser in [false, true] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut preview = ModulePreview::new(module);
            let size = vec2(panel_width("poly_merge"), PANEL_HEIGHT);
            let bounds = Rect::from_min_size(pos2(24.0, 12.0), size * scale);
            let fitted_scale = (bounds.width() / size.x).min(bounds.height() / size.y);
            let fitted = Rect::from_center_size(bounds.center(), size * fitted_scale);
            let mut rack = Rack { zoom: fitted_scale, pan: Vec2::ZERO, ..Default::default() };
            rack.positions.insert("merge".into(), Pos2::ZERO);
            let patch = merge_patch(None);
            let mut output = egui::FullOutput::default();
            for _ in 0..2 {
                output = ctx.run(egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1100.0))),
                    ..Default::default()
                }, |ctx| {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                        if browser { preview.show(ui, bounds, &catalog); }
                        else {
                            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(fitted));
                            child.set_clip_rect(ui.clip_rect().intersect(bounds));
                            let rendered = rack.render_modules(&mut child, &patch, &catalog, fitted, 48000, 0, RenderMode::Interactive { drag_interactive: false, allow_manual_press: false });
                            assert!(rendered.action.is_none());
                            assert_eq!(rendered.inputs.len(), 16);
                            assert_eq!(rendered.outputs.len(), 1);
                        }
                    });
                });
            }
            let labels = drawn(&output);
            let mut screens = vec![];
            fn measured_screens(shape: &egui::Shape, screens: &mut Vec<Rect>) {
                match shape {
                    egui::Shape::Vec(shapes) => for shape in shapes { measured_screens(shape, screens); },
                    egui::Shape::Rect(rect) if rect.fill == Color32::from_rgb(20, 31, 31) => screens.push(rect.rect),
                    _ => {}
                }
            }
            for shape in &output.shapes { measured_screens(&shape.shape, &mut screens); }
            assert_eq!(screens.len(), 1, "One actual measured-channel display");
            let screen = screens[0];
            assert!(screen.top() > fitted.min.y + 342.0 * fitted_scale, "The display frame must clear the unchanged Channels control");
            let count = labels.iter().find(|(text, _, _)| text == "—").unwrap().1;
            assert!(screen.expand(0.5).contains_rect(count), "{scale}: measured text {count:?} escaped display {screen:?}");
            for input in [true, false] {
                let count = if input { 16 } else { 1 };
                for index in 0..count {
                    let jack = fitted.min + port_position("poly_merge", input, index, count, size.x).to_vec2() * fitted_scale;
                    assert!(!screen.intersects(Rect::from_center_size(jack, vec2(27.0, 27.0) * fitted_scale)), "{scale}: measured display frame overlaps a jack");
                }
            }
            for required in ["MERGE", "OUTPUT CHANNELS", "CHANNELS", "Auto", "—", "OUT"] {
                assert!(labels.iter().any(|(text, _, _)| text == required), "{scale} browser={browser}: missing {required}");
            }
            assert!(!labels.iter().any(|(text, _, _)| text.contains("OPENPATCH") || text.contains("OpenPatch")));
            for (i, (label, rect, _)) in labels.iter().enumerate() {
                assert!(bounds.expand(0.5).contains_rect(*rect), "{scale} {label}: label escaped module bounds");
                for (other, other_rect, _) in &labels[i + 1..] {
                    assert!(!rect.shrink(0.2 * scale).intersects(other_rect.shrink(0.2 * scale)), "{scale}: {label} overlaps {other}");
                }
                if ["OUTPUT CHANNELS", "CHANNELS", "Auto", "—"].contains(&label.as_str()) {
                    for index in 0..16 {
                        let jack = fitted.min + port_position("poly_merge", true, index, 16, size.x).to_vec2() * fitted_scale;
                        assert!(!rect.intersects(Rect::from_center_size(jack, vec2(27.0, 27.0) * fitted_scale)), "{scale}: {label} overlaps input {}", index + 1);
                    }
                    let out = fitted.min + port_position("poly_merge", false, 0, 1, size.x).to_vec2() * fitted_scale;
                    assert!(!rect.intersects(Rect::from_center_size(out, vec2(27.0, 27.0) * fitted_scale)), "{scale}: {label} overlaps output");
                }
            }
            let mut shapes = vec![];
            for shape in &output.shapes { primitive(&shape.shape, &mut shapes); }
            surfaces.push(shapes);
            assert!(preview.renderer.editing.is_none());
        }
        assert_eq!(surfaces[0], surfaces[1], "Merge rack/browser primitives diverged at {scale}");
    }
}

#[test]
fn merge_auto_menu_can_show_actual_automatic_count_without_changing_the_selected_mode() {
    let mut f = Fixture::new(None, 1.0);
    f.rack.module_lights.insert("merge".into(), ModuleLightState {
        output_channels: Some(0), automatic_channels: Some(13), count: 0, ..Default::default()
    });
    f.draw(vec![]);
    f.open();
    assert!(drawn(&f.output).iter().any(|(text, _, _)| text == "Auto · 13 channels"));
    assert_eq!(f.session.patch().nodes[0].params.get("channels"), None);
}
