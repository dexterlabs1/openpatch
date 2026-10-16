use super::*;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"seq","kind":"stock_seq3"}]})).unwrap()
}
fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}
fn labels(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    fn visit(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Vec(children) => {
                for child in children {
                    visit(child, out);
                }
            }
            egui::Shape::Text(t) if !t.galley.text().is_empty() => out.push((
                t.galley.text().into(),
                Rect::from_min_size(t.pos, t.galley.size()),
            )),
            _ => {}
        }
    }
    let mut out = vec![];
    for shape in &output.shapes {
        visit(&shape.shape, &mut out);
    }
    out
}
struct Fixture {
    ctx: egui::Context,
    rack: Rack,
    session: openpatch::control::Session,
    _workspace: tempfile::TempDir,
    time: f64,
    output: egui::FullOutput,
    knobs: BTreeMap<String, Rect>,
    values: BTreeMap<String, Rect>,
    held: BTreeMap<usize, Rect>,
}
impl Fixture {
    fn new() -> Self {
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch(), None).unwrap();
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut f = Self {
            ctx,
            rack: Rack::default(),
            session,
            _workspace: workspace,
            time: 0.0,
            output: Default::default(),
            knobs: BTreeMap::new(),
            values: BTreeMap::new(),
            held: BTreeMap::new(),
        };
        f.rack.set_sequence_states(BTreeMap::new(), true);
        f.draw(vec![]);
        f.draw(vec![]);
        f
    }
    fn draw(&mut self, events: Vec<egui::Event>) -> Option<RackAction> {
        let mut action = None;
        let modifiers = events
            .iter()
            .rev()
            .find_map(|event| {
                if let egui::Event::Key { modifiers, .. } = event {
                    Some(*modifiers)
                } else {
                    None
                }
            })
            .unwrap_or_default();
        self.output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1800.0, 1100.0))),
                focused: true,
                time: Some(self.time),
                modifiers,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        action = self.rack.show(
                            ui,
                            self.session.patch(),
                            &openpatch_core::catalog(),
                            48000,
                            self.session.revision(),
                        );
                        let module = openpatch_core::catalog()["modules"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|m| m["kind"] == "stock_seq3")
                            .unwrap()
                            .clone();
                        for name in module["params"]
                            .as_object()
                            .unwrap()
                            .keys()
                            .filter(|name| stock_seq3_scalar(name))
                        {
                            if let Some(r) =
                                ctx.read_response(ui.id().with(("knob", "seq", name.as_str())))
                            {
                                self.knobs.insert(name.clone(), r.rect);
                            }
                            if let Some(r) =
                                ctx.read_response(ui.id().with(("value", "seq", name.as_str())))
                            {
                                self.values.insert(name.clone(), r.rect);
                            }
                        }
                        for index in 0..11 {
                            if let Some(r) =
                                ctx.read_response(ui.id().with(("seq", "stock-seq3-held", index)))
                            {
                                self.held.insert(index, r.rect);
                            }
                        }
                    });
            },
        );
        self.time += 0.1;
        action
    }
    fn click(&mut self, at: Pos2) -> Vec<RackAction> {
        let mut out = vec![];
        for pressed in [true, false] {
            if let Some(a) = self.draw(vec![egui::Event::PointerMoved(at), button(at, pressed)]) {
                out.push(a);
            }
        }
        out
    }
    fn open_tempo(&mut self) {
        let at = self.values["tempo"].center();
        assert!(self.click(at).is_empty());
        self.draw(vec![]);
        self.draw(vec![]);
        assert_eq!(self.rack.editing.as_ref().unwrap().display_unit, "BPM");
    }
}
#[test]
fn stock_seq3_all_twenty_eight_faders_have_real_pointer_edits_and_revision_guards() {
    let first = Fixture::new();
    assert_eq!(first.knobs.len(), 28);
    assert_eq!(first.values.len(), 28);
    for parameter in first.knobs.keys() {
        let mut f = Fixture::new();
        let at = f.knobs[parameter].center();
        assert!(f
            .draw(vec![egui::Event::PointerMoved(at), button(at, true)])
            .is_none());
        let direction = if parameter == "steps" || parameter.ends_with("_amount") {
            1.0
        } else {
            -1.0
        };
        let action = f
            .draw(vec![egui::Event::PointerMoved(
                at + vec2(0.0, 9.0 * direction),
            )])
            .expect("Real fader drag");
        let RackAction::Command("set_parameter", args) = action else {
            panic!("Expected parameter edit")
        };
        assert_eq!(args["parameter"], parameter.as_str());
        assert_eq!(args["expected_revision"], f.session.revision());
        f.session.execute("set_parameter", args).unwrap();
    }
}
#[test]
fn stock_seq3_all_eleven_held_buttons_press_and_release_on_drag_out() {
    for index in 0..11 {
        let mut f = Fixture::new();
        assert_eq!(f.held.len(), 11);
        let at = f.held[&index].center();
        let action = f
            .draw(vec![egui::Event::PointerMoved(at), button(at, true)])
            .unwrap();
        let press = if index < 3 {
            ["clock_press", "run_press", "reset_press"][index]
        } else {
            STOCK_SEQ3_GATE_PRESS[index - 3]
        };
        let release = if index < 3 {
            ["clock_release", "run_release", "reset_release"][index]
        } else {
            STOCK_SEQ3_GATE_RELEASE[index - 3]
        };
        assert!(
            matches!(action,RackAction::Command("trigger_module",args) if args["action"]==press && args["expected_revision"]==f.session.revision())
        );
        let action = f
            .draw(vec![egui::Event::PointerMoved(pos2(1400.0, 900.0))])
            .expect("Drag-out releases");
        assert!(
            matches!(action,RackAction::Command("trigger_module",args) if args["action"]==release)
        );
        assert!(f.rack.held_manual.is_none());
    }
}
#[test]
fn stock_seq3_held_buttons_cancel_on_focus_modal_engine_loss_bypass_and_removal() {
    for case in 0..5 {
        let mut f = Fixture::new();
        let at = f.held[&10].center();
        f.draw(vec![egui::Event::PointerMoved(at), button(at, true)])
            .unwrap();
        match case {
            0 => {
                f.draw(vec![egui::Event::WindowFocused(false)]);
            }
            1 => {
                f.rack.set_shortcuts_enabled(false);
                f.draw(vec![]);
            }
            2 => {
                f.rack.performance_available = false;
                f.draw(vec![]);
            }
            3 => {
                f.session
                    .execute("set_bypass", json!({"id":"seq","bypassed":true}))
                    .unwrap();
                f.draw(vec![]);
            }
            _ => {
                f.session
                    .execute("remove_module", json!({"id":"seq"}))
                    .unwrap();
                f.draw(vec![]);
            }
        }
        assert!(f.rack.held_manual.is_none(), "case {case}");
    }
}
#[test]
fn stock_seq3_bpm_exact_entry_inverts_source_log_and_rejects_stale_or_invalid_drafts() {
    for (bpm, raw) in [(15.0, -2.0), (60.0, 0.0), (120.0, 1.0), (960.0, 4.0)] {
        let mut f = Fixture::new();
        f.open_tempo();
        let edit = f.rack.editing.as_ref().unwrap();
        assert_eq!((edit.min, edit.max), (15.0, 960.0));
        f.draw(vec![egui::Event::Text(format!("{bpm} BPM"))]);
        let action = f.draw(vec![key(egui::Key::Enter)]).unwrap();
        let RackAction::Command("set_parameter", args) = action else {
            panic!()
        };
        assert!((args["value"].as_f64().unwrap() - raw).abs() < 1e-6);
        f.session.execute("set_parameter", args).unwrap();
    }
    for invalid in ["0 BPM", "14 BPM", "961 BPM", "NaN", "1/0"] {
        let mut f = Fixture::new();
        f.open_tempo();
        f.draw(vec![egui::Event::Text(invalid.into())]);
        assert!(f.draw(vec![key(egui::Key::Enter)]).is_none());
        assert!(f.rack.editing.is_some());
    }
    let mut f = Fixture::new();
    f.open_tempo();
    let stale = parameter_edit_action(f.rack.editing.as_ref().unwrap(), 0.0);
    f.session
        .execute(
            "set_parameter",
            json!({"id":"seq","parameter":"row1_1","value":2}),
        )
        .unwrap();
    let before = f.session.snapshot();
    let RackAction::Command(tool, args) = stale else {
        panic!()
    };
    assert!(f.session.execute(tool, args).is_err());
    f.draw(vec![]);
    assert!(f.rack.editing.as_ref().unwrap().stale);
    assert_eq!(f.session.snapshot(), before);
}
#[test]
fn stock_seq3_gate_mode_dropdown_and_atomic_rotation_use_real_pointer_targets() {
    let mut f = Fixture::new();
    let at = labels(&f.output)
        .iter()
        .find(|(s, _)| s == "Trigger pulses")
        .unwrap()
        .1
        .center();
    assert!(f.click(at).is_empty());
    f.draw(vec![]);
    let at = labels(&f.output)
        .iter()
        .find(|(s, _)| s == "Gate passthrough")
        .unwrap()
        .1
        .center();
    let mut actions = f.click(at);
    assert_eq!(actions.len(), 1);
    let RackAction::Command("set_parameter", args) = actions.remove(0) else {
        panic!()
    };
    assert_eq!(args["parameter"], "passthrough");
    assert_eq!(args["value"], 1.0);
    assert_eq!(args["expected_revision"], f.session.revision());
    f.session.execute("set_parameter", args).unwrap();
    f.draw(vec![]);
    f.draw(vec![]);
    for (label, direction) in [("Rotate ←", "left"), ("Rotate →", "right")] {
        let at = labels(&f.output)
            .iter()
            .find(|(s, _)| s == label)
            .unwrap()
            .1
            .center();
        let mut actions = f.click(at);
        assert_eq!(actions.len(), 1);
        assert!(
            matches!(actions.remove(0),RackAction::Command("rotate_sequence",args) if args["direction"]==direction&&args["expected_revision"]==f.session.revision())
        );
    }
}
#[test]
fn stock_seq3_all_twenty_one_named_jacks_are_accessible() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "stock_seq3")
        .unwrap();
    for (input, key) in [(true, "inputs"), (false, "outputs")] {
        for port in module[key].as_array().unwrap() {
            let port = port.as_str().unwrap();
            let mut f = Fixture::new();
            let at = fixture_jack(&f.rack, f.session.patch(), "seq", port, input);
            assert!(f.click(at).is_empty());
            let expected = if input {
                CableEnd::Input {
                    node: "seq".into(),
                    port: port.into(),
                }
            } else {
                CableEnd::Output {
                    node: "seq".into(),
                    port: port.into(),
                }
            };
            assert_eq!(f.rack.pending.as_ref().map(|p| &p.origin), Some(&expected));
        }
    }
}
#[test]
fn stock_seq3_shared_preview_and_rack_keep_all_text_faders_lamps_and_sockets_separate() {
    assert_eq!(title("stock_seq3"), "STOCK SEQ3");
    assert_eq!(title("seq3"), "SEQUENCE 3");
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "stock_seq3")
        .unwrap();
    for scale in [0.25, 0.42, 0.7, 1.0, 1.75] {
        for browser in [false, true] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let size = vec2(panel_width("stock_seq3"), PANEL_HEIGHT);
            let bounds = Rect::from_min_size(pos2(24.0, 12.0), size * scale);
            let zoom = (bounds.width() / size.x).min(bounds.height() / size.y);
            let fitted = Rect::from_center_size(bounds.center(), size * zoom);
            let mut preview = ModulePreview::new(module);
            let mut rack = Rack {
                zoom,
                pan: Vec2::ZERO,
                ..Default::default()
            };
            rack.positions.insert("seq".into(), Pos2::ZERO);
            let patch = patch();
            let mut output = egui::FullOutput::default();
            for _ in 0..2 {
                output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1800.0, 1100.0))),
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
                                    assert_eq!(rendered.inputs.len(), 5);
                                    assert_eq!(rendered.outputs.len(), 16);
                                }
                            });
                    },
                );
            }
            let text = labels(&output);
            for required in [
                "STOCK SEQ3",
                "TEMPO",
                "STEPS",
                "TEMPO CV",
                "STEPS CV",
                "TELEMETRY —",
                "Trigger pulses",
                "Rotate ←",
                "Rotate →",
            ] {
                assert!(
                    text.iter().any(|(s, _)| s == required),
                    "{scale} browser={browser}: missing {required}"
                );
            }
            for (i, (label, rect)) in text.iter().enumerate() {
                assert!(
                    bounds.expand(0.5).contains_rect(*rect),
                    "{scale} {label} outside panel"
                );
                for (other, other_rect) in &text[i + 1..] {
                    assert!(
                        !rect
                            .shrink(0.2 * zoom)
                            .intersects(other_rect.shrink(0.2 * zoom)),
                        "{scale}: {label} overlaps {other}"
                    );
                }
            }
            for input in [true, false] {
                let count = if input { 5 } else { 16 };
                for index in 0..count {
                    let center = fitted.min
                        + port_position("stock_seq3", input, index, count, size.x).to_vec2() * zoom;
                    let socket = Rect::from_center_size(center, Vec2::splat(26.0 * zoom));
                    assert!(bounds.contains_rect(socket), "scale={scale} browser={browser} input={input} index={index}: socket {socket:?} outside {bounds:?}");
                    for (label, rect) in &text {
                        assert!(
                            !socket.shrink(0.5 * zoom).intersects(*rect),
                            "{scale} {label} overlaps socket {input}/{index}"
                        );
                    }
                }
            }
            for column in 0..8 {
                for dx in [-8.0, 8.0] {
                    let center = fitted.min + vec2(238.0 + column as f32 * 67.0 + dx, 306.0) * zoom;
                    let lamp = Rect::from_center_size(
                        center,
                        Vec2::splat(crate::lighting::lamp_radius(2.4 * zoom) * 2.0),
                    );
                    for (label, rect) in &text {
                        assert!(
                            !lamp.shrink(0.2 * zoom).intersects(*rect),
                            "{scale}: {label} overlaps source step lamp"
                        );
                    }
                }
            }
            assert!(preview.renderer.construction_anchors.is_empty());
            assert!(preview.renderer.editing.is_none());
            assert!(preview.renderer.held_manual.is_none());
        }
    }
}
#[test]
fn stock_seq3_twenty_seven_actual_lamps_emit_in_darkness_and_unavailable_preview_stays_quiet() {
    let mut f = Fixture::new();
    let mut lights = ModuleLightState {
        count: 19,
        ..Default::default()
    };
    for pair in &mut lights.brightness[..19] {
        *pair = [1.0, 1.0];
    }
    f.rack
        .set_module_states(BTreeMap::from([("seq".into(), lights)]), BTreeMap::new());
    let mut signatures = vec![];
    for room in [1.0, 0.0] {
        f.rack.set_room_brightness(room);
        f.draw(vec![]);
        let mut lamps = vec![];
        for shape in &f.output.shapes {
            if let egui::Shape::Circle(c) = &shape.shape {
                if (c.radius - 5.04).abs() < 0.01 && (c.fill.g() > 150 || c.fill.r() > 200) {
                    lamps.push((c.center, c.fill));
                }
            }
        }
        assert_eq!(lamps.len(), 27);
        signatures.push(lamps);
    }
    assert_eq!(signatures[0], signatures[1]);
    f.rack.set_module_states(BTreeMap::new(), BTreeMap::new());
    f.draw(vec![]);
    assert!(labels(&f.output).iter().any(|(s, _)| s == "TELEMETRY —"));
    assert!(labels(&f.output).iter().any(|(s, _)| s == "Trigger pulses"));
}
#[test]
fn stock_seq3_fast_click_pairs_actions_and_gate_mode_retains_opening_revision() {
    for index in 0..11 {
        let mut f = Fixture::new();
        let at = f.held[&index].center();
        let action = f
            .draw(vec![
                egui::Event::PointerMoved(at),
                button(at, true),
                button(at, false),
            ])
            .expect("Fast click");
        let expected = if index < 3 {
            [
                ("clock_press", "clock_release"),
                ("run_press", "run_release"),
                ("reset_press", "reset_release"),
            ][index]
        } else {
            (
                STOCK_SEQ3_GATE_PRESS[index - 3],
                STOCK_SEQ3_GATE_RELEASE[index - 3],
            )
        };
        assert!(
            matches!(action,RackAction::ManualClick{id,press,release} if id=="seq"&&press==expected.0&&release==expected.1)
        );
        assert!(f.rack.held_manual.is_none());
    }
    let mut f = Fixture::new();
    let revision = f.session.revision();
    let at = labels(&f.output)
        .iter()
        .find(|(s, _)| s == "Trigger pulses")
        .unwrap()
        .1
        .center();
    f.click(at);
    f.draw(vec![]);
    f.session
        .execute(
            "set_parameter",
            json!({"id":"seq","parameter":"row3_8","value":1}),
        )
        .unwrap();
    f.draw(vec![]);
    let at = labels(&f.output)
        .iter()
        .find(|(s, _)| s == "Gate passthrough")
        .unwrap()
        .1
        .center();
    let mut actions = f.click(at);
    assert_eq!(actions.len(), 1);
    let RackAction::Command("set_parameter", args) = actions.remove(0) else {
        panic!()
    };
    assert_eq!(args["expected_revision"], revision);
    let before = f.session.snapshot();
    assert!(f.session.execute("set_parameter", args).is_err());
    assert_eq!(f.session.snapshot(), before);
}
#[test]
fn stock_seq3_tempo_fader_retains_fine_keyboard_adjustment() {
    let mut coarse = Fixture::new();
    let at = coarse.knobs["tempo"].center();
    coarse.click(at);
    let RackAction::Command("set_parameter", coarse_args) =
        coarse.draw(vec![key(egui::Key::ArrowUp)]).unwrap()
    else {
        panic!()
    };
    let mut fine = Fixture::new();
    let at = fine.knobs["tempo"].center();
    fine.click(at);
    let mut event = key(egui::Key::ArrowUp);
    if let egui::Event::Key { modifiers, .. } = &mut event {
        *modifiers = egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
    }
    let RackAction::Command("set_parameter", fine_args) = fine.draw(vec![event]).unwrap() else {
        panic!()
    };
    assert!(
        (fine_args["value"].as_f64().unwrap() - 1.0).abs()
            < (coarse_args["value"].as_f64().unwrap() - 1.0).abs()
    );
}
