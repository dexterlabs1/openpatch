use super::*;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"lfo","kind":"stock_lfo"}]})).unwrap()
}
fn texts(shape: &egui::Shape, out: &mut Vec<(String, Rect, Color32)>) {
    match shape {
        egui::Shape::Vec(shapes) => for shape in shapes { texts(shape, out); },
        egui::Shape::Text(t) => out.push((t.galley.text().into(), Rect::from_min_size(t.pos, t.galley.size()), t.override_text_color.unwrap_or(t.fallback_color))),
        _ => {}
    }
}
fn drawn(output: &egui::FullOutput) -> Vec<(String, Rect, Color32)> {
    let mut out = vec![]; for shape in &output.shapes { texts(&shape.shape, &mut out); } out
}
fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}
struct Fixture {
    ctx: egui::Context, rack: Rack, session: openpatch::control::Session,
    _workspace: tempfile::TempDir, time: f64, output: egui::FullOutput,
    knobs: BTreeMap<String, Rect>, values: BTreeMap<String, Rect>,
}
impl Fixture {
    fn new(clock: Option<f32>) -> Self {
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch(), None).unwrap();
        let ctx = egui::Context::default(); crate::panel_skin::install_fonts(&ctx);
        let mut rack = Rack::default();
        if clock.is_some() { rack.module_lights.insert("lfo".into(), ModuleLightState { count: 5, clock_frequency: clock, ..Default::default() }); }
        let mut f = Self { ctx, rack, session, _workspace: workspace, time: 0.0, output: Default::default(), knobs: BTreeMap::new(), values: BTreeMap::new() };
        f.draw(vec![]); f.draw(vec![]); f
    }
    fn draw(&mut self, events: Vec<egui::Event>) -> Option<RackAction> {
        let modifiers = events.iter().rev().find_map(|event| match event { egui::Event::Key { modifiers, .. } => Some(*modifiers), _ => None }).unwrap_or_default();
        let mut action = None;
        self.output = self.ctx.run(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1100.0))), time: Some(self.time), focused: true, modifiers, events, ..Default::default() }, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                action = self.rack.show(ui, self.session.patch(), &openpatch_core::catalog(), 48000, self.session.revision());
                for parameter in ["rate", "pulse_width", "fm_amount", "pwm_amount"] {
                    if let Some(response) = ctx.read_response(ui.id().with(("knob", "lfo", parameter))) { self.knobs.insert(parameter.into(), response.rect); }
                    if let Some(response) = ctx.read_response(ui.id().with(("value", "lfo", parameter))) { self.values.insert(parameter.into(), response.rect); }
                }
            });
        });
        self.time += 0.1; action
    }
    fn click(&mut self, at: Pos2) -> Vec<RackAction> {
        let mut out = vec![];
        for pressed in [true, false] { if let Some(action) = self.draw(vec![egui::Event::PointerMoved(at), button(at, pressed)]) { out.push(action); } }
        out
    }
    fn execute(&mut self, action: RackAction) -> Value {
        let RackAction::Command("set_parameter", args) = action else { panic!("Expected parameter edit") };
        self.session.execute("set_parameter", args.clone()).unwrap(); args
    }
    fn open_rate(&mut self) {
        let at = self.values["rate"].center(); assert!(self.click(at).is_empty());
        self.draw(vec![]); self.draw(vec![]);
        assert_eq!(self.rack.editing.as_ref().unwrap().parameter, "rate");
    }
}
#[test]
fn stock_lfo_all_six_controls_use_real_pointer_targets_and_guarded_edits() {
    for parameter in ["rate", "pulse_width", "fm_amount", "pwm_amount"] {
        let mut f = Fixture::new(Some(2.0));
        let at = f.knobs[parameter].center();
        assert!(f.draw(vec![egui::Event::PointerMoved(at), button(at, true)]).is_none());
        let action = f.draw(vec![egui::Event::PointerMoved(at - vec2(0.0, 9.0))]).expect("Fader drag edit");
        let revision = f.session.revision();
        let args = f.execute(action);
        assert_eq!(args["parameter"], parameter); assert_eq!(args["expected_revision"], revision);
        f.draw(vec![button(at - vec2(0.0, 9.0), false)]);
    }
    for (label, parameter, expected) in [("0–10 V", "offset", 0.0), ("Invert", "invert", 1.0)] {
        let mut f = Fixture::new(Some(2.0));
        let at = drawn(&f.output).into_iter().find(|(text, _, _)| text == label).unwrap().1.center();
        let mut actions = f.click(at); assert_eq!(actions.len(), 1);
        let args = f.execute(actions.remove(0));
        assert_eq!(args["parameter"], parameter); assert_eq!(args["value"], expected);
    }
}
#[test]
fn stock_lfo_rate_units_follow_actual_clock_and_exact_entry_inverts_log2_correctly() {
    for (clock, typed, expected, scale) in [(Some(2.0), "4 Hz", 2.0, 1.0), (Some(4.0), "2x", 2.0, 0.5), (Some(1.0), "0.25x", -1.0, 0.5), (None, "0.5 Hz", -1.0, 1.0)] {
        let mut f = Fixture::new(clock);
        let labels = drawn(&f.output);
        if clock == Some(2.0) { assert!(labels.iter().any(|(text, _, _)| text == "2.000 Hz")); }
        else if clock.is_some() { assert!(labels.iter().any(|(text, _, _)| text == "1.0000x")); }
        else { assert!(labels.iter().any(|(text, _, _)| text == "CLOCK —")); assert!(labels.iter().any(|(text, _, _)| text == "2.000 Hz / 1.0000x")); }
        f.open_rate(); assert_eq!(f.rack.editing.as_ref().unwrap().display_scale, scale);
        f.draw(vec![egui::Event::Text(typed.into())]);
        let action = f.draw(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]).expect("Typed rate edit");
        let args = f.execute(action); assert!((args["value"].as_f64().unwrap() - expected).abs() < 1e-6);
    }
    for clock in [None, Some(0.0), Some(-1.0), Some(f32::NAN), Some(f32::INFINITY)] {
        let state = ModuleLightState { clock_frequency: clock, ..Default::default() };
        assert_eq!(stock_lfo_clock(Some(&state)), None);
    }
}
#[test]
fn stock_lfo_open_ratio_draft_keeps_its_units_when_clock_measurement_changes() {
    for clock_after in [Some(2.0), None] {
        for newer_revision in [false, true] {
            let mut f = Fixture::new(Some(4.0));
            f.open_rate();
            let opening_revision = f.session.revision();
            f.rack.module_lights.get_mut("lfo").unwrap().clock_frequency = clock_after;
            f.draw(vec![]);
            let draft = f.rack.editing.as_ref().unwrap();
            assert_eq!(draft.display_scale, 0.5);
            assert_eq!(draft.display_unit, "x");
            f.draw(vec![egui::Event::Text("2x".into())]);
            let draft = f.rack.editing.as_ref().unwrap();
            assert_eq!(parsed_parameter_result(draft).unwrap(), 2.0, "The open ratio draft still means log2(2 / 0.5)");
            if newer_revision {
                let guarded = parameter_edit_action(draft, 2.0);
                f.session.execute("set_parameter", json!({"id":"lfo","parameter":"pulse_width","value":0.7})).unwrap();
                let before = f.session.snapshot();
                let RackAction::Command(tool, args) = guarded else { unreachable!() };
                assert_eq!(args["expected_revision"], opening_revision);
                assert!(f.session.execute(tool, args).is_err());
                f.draw(vec![]);
                assert!(f.rack.editing.as_ref().unwrap().stale);
                assert!(f.draw(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]).is_none());
                assert_eq!(f.session.snapshot(), before);
            } else {
                let action = f.draw(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]).expect("Ratio draft should commit with its opening units");
                let args = f.execute(action);
                assert_eq!(args["expected_revision"], opening_revision);
                assert_eq!(args["value"], 2.0);
                assert_eq!(f.session.patch().nodes[0].params["rate"], 2.0);
            }
        }
    }
}
#[test]
fn stock_lfo_rate_editor_rejects_invalid_values_and_newer_session_revisions() {
    for text in ["0 Hz", "-1 Hz", "1025 Hz", "NaN", "1/0"] {
        let mut f = Fixture::new(Some(2.0)); f.open_rate();
        f.draw(vec![egui::Event::Text(text.into())]);
        assert!(f.draw(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]).is_none());
        assert!(f.rack.editing.is_some());
    }
    let mut f = Fixture::new(Some(4.0)); f.open_rate();
    let action = parameter_edit_action(f.rack.editing.as_ref().unwrap(), 2.0);
    f.session.execute("set_parameter", json!({"id":"lfo","parameter":"pulse_width","value":0.7})).unwrap();
    let before = f.session.snapshot();
    let RackAction::Command(tool, args) = action else { unreachable!() };
    assert!(f.session.execute(tool, args).is_err());
    f.draw(vec![]); assert!(f.rack.editing.as_ref().unwrap().stale);
    assert!(f.draw(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]).is_none());
    assert_eq!(f.session.snapshot(), before);
}
#[test]
fn stock_lfo_fine_keyboard_adjustment_is_smaller_and_all_eight_jacks_accept_pointer_starts() {
    let mut coarse = Fixture::new(Some(2.0)); let at = coarse.knobs["rate"].center(); coarse.click(at);
    let coarse_action = coarse.draw(vec![key(egui::Key::ArrowUp, egui::Modifiers::NONE)]).unwrap();
    let coarse_args = coarse.execute(coarse_action);
    let mut fine = Fixture::new(Some(2.0)); let at = fine.knobs["rate"].center(); fine.click(at);
    let fine_action = fine.draw(vec![key(egui::Key::ArrowUp, egui::Modifiers { ctrl: true, shift: true, ..Default::default() })]).unwrap();
    let fine_args = fine.execute(fine_action);
    assert!((fine_args["value"].as_f64().unwrap() - 1.0).abs() < (coarse_args["value"].as_f64().unwrap() - 1.0).abs());
    for (input, ports) in [(true, ["fm", "pwm", "reset", "clock"]), (false, ["sine", "triangle", "saw", "square"])] {
        for port in ports {
            let mut f = Fixture::new(None); let at = fixture_jack(&f.rack, f.session.patch(), "lfo", port, input);
            assert!(f.click(at).is_empty());
            let expected = if input { CableEnd::Input { node: "lfo".into(), port: port.into() } } else { CableEnd::Output { node: "lfo".into(), port: port.into() } };
            assert_eq!(f.rack.pending.as_ref().map(|pending| &pending.origin), Some(&expected));
        }
    }
}
fn primitive(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Vec(shapes) => for shape in shapes { primitive(shape, out); },
        egui::Shape::Text(t) => out.push(format!("text:{:?}:{:?}:{}:{:?}:{:?}:{:?}", t.pos, t.galley.size(), t.galley.text(), t.galley.job.sections, t.fallback_color, t.override_text_color)),
        egui::Shape::Rect(r) => out.push(format!("rect:{r:?}")), egui::Shape::Circle(c) => out.push(format!("circle:{c:?}")),
        egui::Shape::LineSegment { points, stroke } => out.push(format!("line:{points:?}:{stroke:?}")),
        egui::Shape::Mesh(m) => out.push(format!("mesh:{:?}:{:?}", m.vertices, m.indices)), other => out.push(format!("shape:{other:?}")),
    }
}
#[test]
fn stock_lfo_shared_rack_browser_geometry_keeps_controls_labels_and_jacks_separate() {
    assert_eq!(title("stock_lfo"), "STOCK LFO");
    assert_eq!(title("multi_lfo"), "LFO");
    let catalog = openpatch_core::catalog(); let module = catalog["modules"].as_array().unwrap().iter().find(|m| m["kind"] == "stock_lfo").unwrap();
    for scale in [0.42, 0.7, 1.0, 1.75] {
        let mut surfaces = vec![];
        for browser in [false, true] {
            let ctx = egui::Context::default(); crate::panel_skin::install_fonts(&ctx);
            let mut preview = ModulePreview::new(module); let size = vec2(panel_width("stock_lfo"), PANEL_HEIGHT);
            let bounds = Rect::from_min_size(pos2(24.0, 12.0), size * scale); let zoom = (bounds.width() / size.x).min(bounds.height() / size.y);
            let fitted = Rect::from_center_size(bounds.center(), size * zoom);
            let mut rack = Rack { zoom, pan: Vec2::ZERO, ..Default::default() }; rack.positions.insert("lfo".into(), Pos2::ZERO);
            let patch = patch(); let mut output = egui::FullOutput::default();
            for _ in 0..2 {
                output = ctx.run(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1100.0))), ..Default::default() }, |ctx| {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                        if browser { preview.show(ui, bounds, &catalog); } else {
                            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(fitted)); child.set_clip_rect(ui.clip_rect().intersect(bounds));
                            let rendered = rack.render_modules(&mut child, &patch, &catalog, fitted, 48000, 0, RenderMode::Interactive { drag_interactive: false, allow_manual_press: false });
                            assert!(rendered.action.is_none()); assert_eq!(rendered.inputs.len(), 4); assert_eq!(rendered.outputs.len(), 4);
                        }
                    });
                });
            }
            let labels = drawn(&output);
            for required in ["STOCK LFO", "PHASE", "RATE", "WIDTH", "FM DEPTH", "PWM DEPTH", "CLOCK —", "0–10 V", "Invert", "FM", "PWM", "RESET", "CLK", "SIN", "TRI", "SAW", "SQR"] {
                assert!(labels.iter().any(|(text, _, _)| text == required), "{scale} browser={browser}: missing {required}");
            }
            for (i, (label, rect, _)) in labels.iter().enumerate() {
                assert!(bounds.expand(0.5).contains_rect(*rect), "{scale}: {label} outside panel");
                for (other, other_rect, _) in &labels[i + 1..] { assert!(!rect.shrink(0.2 * scale).intersects(other_rect.shrink(0.2 * scale)), "{scale}: {label} overlaps {other}"); }
            }
            let mut primitives = vec![]; for shape in &output.shapes { primitive(&shape.shape, &mut primitives); } surfaces.push(primitives);
            assert!(preview.renderer.editing.is_none());
        }
        assert_eq!(surfaces[0], surfaces[1], "Stock LFO rack/browser mismatch at {scale}");
    }
}
#[test]
fn stock_lfo_runtime_phase_and_mode_lamps_emit_in_darkness_without_invented_preview_values() {
    let mut f = Fixture::new(Some(4.0));
    let state = f.rack.module_lights.get_mut("lfo").unwrap(); state.brightness[2][0] = 1.0; state.brightness[3][0] = 1.0; state.brightness[4][0] = 1.0;
    let mut signatures = vec![];
    for room in [1.0, 0.0] {
        f.rack.set_room_brightness(room); f.draw(vec![]);
        let mut emitting = vec![];
        for shape in &f.output.shapes { if let egui::Shape::Circle(circle) = &shape.shape { if circle.fill.b() > 200 || circle.fill.g() > 150 || circle.fill.r() > 200 { emitting.push((circle.center, circle.radius, circle.fill)); } } }
        // Only compare known measured lamp centers, excluding reflective sockets.
        emitting.retain(|(center, _, _)| [pos2(204.0, 120.0), pos2(181.0, 305.0), pos2(361.0, 305.0)].iter().any(|p| center.distance(*p) < 0.1));
        assert!(!emitting.is_empty()); signatures.push(emitting);
        assert!(drawn(&f.output).iter().any(|(text, _, color)| text == "CLOCK 4.000 Hz" && *color == Color32::from_rgb(155, 220, 197)));
    }
    assert_eq!(signatures[0], signatures[1]);
    f.rack.set_module_states(BTreeMap::new(), BTreeMap::new()); f.draw(vec![]);
    assert!(drawn(&f.output).iter().any(|(text, _, _)| text == "CLOCK —"));
}
