use super::*;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"lfo","kind":"stock_lfo"}]})).unwrap()
}
fn texts(shape: &egui::Shape, out: &mut Vec<(String, Rect, Color32)>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                texts(shape, out);
            }
        }
        egui::Shape::Text(t) => out.push((
            t.galley.text().into(),
            Rect::from_min_size(t.pos, t.galley.size()),
            t.override_text_color.unwrap_or(t.fallback_color),
        )),
        _ => {}
    }
}
fn drawn(output: &egui::FullOutput) -> Vec<(String, Rect, Color32)> {
    let mut out = vec![];
    for shape in &output.shapes {
        texts(&shape.shape, &mut out);
    }
    out
}
fn circles(shape: &egui::Shape, out: &mut Vec<(Pos2, f32)>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                circles(shape, out);
            }
        }
        egui::Shape::Circle(circle) => out.push((circle.center, circle.radius)),
        _ => {}
    }
}
fn assert_stock_lfo_port_clearance(f: &Fixture) {
    let labels = drawn(&f.output);
    let patch = f.session.patch();
    let scale = STOCK_OSCILLATOR_PORT_SCALE * f.rack.zoom;
    let mut painted_circles = vec![];
    for shape in &f.output.shapes {
        circles(&shape.shape, &mut painted_circles);
    }
    let inputs = [
        ("fm", "FM"),
        ("pwm", "PWM"),
        ("reset", "RESET"),
        ("clock", "CLK"),
    ];
    let outputs = [
        ("sine", "SIN"),
        ("triangle", "TRI"),
        ("saw", "SAW"),
        ("square", "SQR"),
    ];
    let locate = |port: &str, input: bool, _index: usize, text: &str| {
        let endpoint = fixture_jack(&f.rack, patch, "lfo", port, input);
        let expected = if input {
            endpoint - vec2(0.0, 25.0 * f.rack.zoom)
        } else {
            endpoint + vec2(36.0 * f.rack.zoom, 0.0)
        };
        let bounds = labels
            .iter()
            .filter(|(actual, _, _)| actual == text)
            .min_by(|a, b| {
                a.1.center()
                    .distance(expected)
                    .total_cmp(&b.1.center().distance(expected))
            })
            .expect("painted oscillator port label")
            .1;
        assert!(
            bounds.center().distance(expected) < 0.8 * f.rack.zoom,
            "{text}: {bounds:?} vs {expected:?}"
        );
        (endpoint, bounds)
    };
    let input_geometry = inputs
        .iter()
        .enumerate()
        .map(|(index, (_port, text))| {
            let (center, label) = locate(_port, true, index, text);
            let socket = Rect::from_center_size(center, vec2(20.6, 20.6) * scale);
            assert!(
                painted_circles
                    .iter()
                    .any(|(at, radius)| at.distance(center) < 0.1
                        && (*radius - 10.3 * scale).abs() < 0.01),
                "input ring radius is not enlarged at {center:?}"
            );
            assert!(
                !label.intersects(socket),
                "input label {text} intersects its larger socket: {label:?} {socket:?}"
            );
            (center, socket, label)
        })
        .collect::<Vec<_>>();
    for (index, (_port, text)) in outputs.into_iter().enumerate() {
        let (center, label) = locate(_port, false, index, text);
        let ring = Rect::from_center_size(center, vec2(26.0, 26.0) * scale);
        let legend_plate =
            Rect::from_center_size(label.center(), label.size() + vec2(8.0 * f.rack.zoom, 0.0));
        assert!(
            painted_circles
                .iter()
                .any(|(at, radius)| at.distance(center) < 0.1
                    && (*radius - 13.0 * scale).abs() < 0.01),
            "output ring radius is not enlarged at {center:?}"
        );
        assert!(
            !legend_plate.intersects(ring),
            "output legend {text} intersects its 17-point ring: {legend_plate:?} {ring:?}"
        );
        for (_, input_socket, _) in &input_geometry {
            assert!(
                !legend_plate.intersects(*input_socket),
                "output legend {text} overlaps an input socket: {legend_plate:?} {input_socket:?}"
            );
        }
    }
}
fn assert_stock_lfo_port_geometry(output: &egui::FullOutput, origin: Pos2, zoom: f32) {
    let labels = drawn(output);
    let mut painted_circles = vec![];
    for shape in &output.shapes {
        circles(&shape.shape, &mut painted_circles);
    }
    let scale = STOCK_OSCILLATOR_PORT_SCALE * zoom;
    let inputs = [
        ("fm", "FM"),
        ("pwm", "PWM"),
        ("reset", "RESET"),
        ("clock", "CLK"),
    ];
    let outputs = [
        ("sine", "SIN"),
        ("triangle", "TRI"),
        ("saw", "SAW"),
        ("square", "SQR"),
    ];
    let locate = |input: bool, index: usize, text: &str| {
        let center = origin
            + port_position("stock_lfo", input, index, 4, panel_width("stock_lfo")).to_vec2()
                * zoom;
        let expected = if input {
            center - vec2(0.0, 25.0 * zoom)
        } else {
            center + vec2(36.0 * zoom, 0.0)
        };
        let bounds = labels
            .iter()
            .filter(|(actual, _, _)| actual == text)
            .min_by(|a, b| {
                a.1.center()
                    .distance(expected)
                    .total_cmp(&b.1.center().distance(expected))
            })
            .expect("painted oscillator port label")
            .1;
        assert!(
            bounds.center().distance(expected) < 0.8 * zoom,
            "{text}: {bounds:?} vs {expected:?}"
        );
        (center, bounds)
    };
    let input_geometry = inputs
        .iter()
        .enumerate()
        .map(|(index, (_port, text))| {
            let (center, label) = locate(true, index, text);
            let socket = Rect::from_center_size(center, vec2(20.6, 20.6) * scale);
            assert!(painted_circles
                .iter()
                .any(|(at, radius)| at.distance(center) < 0.1
                    && (*radius - 10.3 * scale).abs() < 0.01));
            assert!(
                !label.intersects(socket),
                "input label {text} intersects socket at zoom {zoom}: {label:?} {socket:?}"
            );
            socket
        })
        .collect::<Vec<_>>();
    for (index, (_port, text)) in outputs.iter().enumerate() {
        let (center, label) = locate(false, index, text);
        let ring = Rect::from_center_size(center, vec2(26.0, 26.0) * scale);
        let legend_plate =
            Rect::from_center_size(label.center(), label.size() + vec2(8.0 * zoom, 0.0));
        assert!(
            painted_circles
                .iter()
                .any(|(at, radius)| at.distance(center) < 0.1
                    && (*radius - 13.0 * scale).abs() < 0.01)
        );
        assert!(
            !legend_plate.intersects(ring),
            "output legend {text} intersects ring at zoom {zoom}: {legend_plate:?} {ring:?}"
        );
        for socket in &input_geometry {
            assert!(
                !legend_plate.intersects(*socket),
                "output legend {text} overlaps input socket at zoom {zoom}"
            );
        }
    }
}
fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
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
    jacks: BTreeMap<String, Rect>,
}
impl Fixture {
    fn new(clock: Option<f32>) -> Self {
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch(), None).unwrap();
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut rack = Rack::default();
        if clock.is_some() {
            rack.module_lights.insert(
                "lfo".into(),
                ModuleLightState {
                    count: 5,
                    clock_frequency: clock,
                    ..Default::default()
                },
            );
        }
        let mut f = Self {
            ctx,
            rack,
            session,
            _workspace: workspace,
            time: 0.0,
            output: Default::default(),
            knobs: BTreeMap::new(),
            values: BTreeMap::new(),
            jacks: BTreeMap::new(),
        };
        f.draw(vec![]);
        f.draw(vec![]);
        f
    }
    fn draw(&mut self, events: Vec<egui::Event>) -> Option<RackAction> {
        let modifiers = events
            .iter()
            .rev()
            .find_map(|event| match event {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let mut action = None;
        self.output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1100.0))),
                time: Some(self.time),
                focused: true,
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
                        for parameter in ["rate", "pulse_width", "fm_amount", "pwm_amount"] {
                            if let Some(response) =
                                ctx.read_response(ui.id().with(("knob", "lfo", parameter)))
                            {
                                self.knobs.insert(parameter.into(), response.rect);
                            }
                            if let Some(response) =
                                ctx.read_response(ui.id().with(("value", "lfo", parameter)))
                            {
                                self.values.insert(parameter.into(), response.rect);
                            }
                        }
                        for port in ["fm", "pwm", "reset", "clock"] {
                            if let Some(response) =
                                ctx.read_response(ui.id().with(("input", "lfo", port)))
                            {
                                self.jacks.insert(format!("in:{port}"), response.rect);
                            }
                        }
                        for port in ["sine", "triangle", "saw", "square"] {
                            if let Some(response) =
                                ctx.read_response(ui.id().with(("output", "lfo", port)))
                            {
                                self.jacks.insert(format!("out:{port}"), response.rect);
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
            if let Some(action) =
                self.draw(vec![egui::Event::PointerMoved(at), button(at, pressed)])
            {
                out.push(action);
            }
        }
        out
    }
    fn execute(&mut self, action: RackAction) -> Value {
        let RackAction::Command("set_parameter", args) = action else {
            panic!("Expected parameter edit")
        };
        self.session.execute("set_parameter", args.clone()).unwrap();
        args
    }
    fn open_rate(&mut self) {
        let at = self.values["rate"].center();
        assert!(self.click(at).is_empty());
        self.draw(vec![]);
        self.draw(vec![]);
        assert_eq!(self.rack.editing.as_ref().unwrap().parameter, "rate");
    }
}
#[test]
fn stock_lfo_all_six_controls_use_real_pointer_targets_and_guarded_edits() {
    for parameter in ["rate", "pulse_width", "fm_amount", "pwm_amount"] {
        let mut f = Fixture::new(Some(2.0));
        let at = f.knobs[parameter].center();
        assert!(f
            .draw(vec![egui::Event::PointerMoved(at), button(at, true)])
            .is_none());
        let action = f
            .draw(vec![egui::Event::PointerMoved(at - vec2(0.0, 9.0))])
            .expect("Fader drag edit");
        let revision = f.session.revision();
        let args = f.execute(action);
        assert_eq!(args["parameter"], parameter);
        assert_eq!(args["expected_revision"], revision);
        f.draw(vec![button(at - vec2(0.0, 9.0), false)]);
    }
    for (label, parameter, expected) in [("0–10 V", "offset", 0.0), ("Invert", "invert", 1.0)] {
        let mut f = Fixture::new(Some(2.0));
        let at = drawn(&f.output)
            .into_iter()
            .find(|(text, _, _)| text == label)
            .unwrap()
            .1
            .center();
        let mut actions = f.click(at);
        assert_eq!(actions.len(), 1);
        let args = f.execute(actions.remove(0));
        assert_eq!(args["parameter"], parameter);
        assert_eq!(args["value"], expected);
    }
}
#[test]
fn stock_lfo_rate_units_follow_actual_clock_and_exact_entry_inverts_log2_correctly() {
    for (clock, typed, expected, scale) in [
        (Some(2.0), "4 Hz", 2.0, 1.0),
        (Some(4.0), "2x", 2.0, 0.5),
        (Some(1.0), "0.25x", -1.0, 0.5),
        (None, "0.5 Hz", -1.0, 1.0),
    ] {
        let mut f = Fixture::new(clock);
        let labels = drawn(&f.output);
        if clock == Some(2.0) {
            assert!(labels.iter().any(|(text, _, _)| text == "2.000 Hz"));
        } else if clock.is_some() {
            assert!(labels.iter().any(|(text, _, _)| text == "1.0000x"));
        } else {
            assert!(labels.iter().any(|(text, _, _)| text == "CLOCK —"));
            assert!(labels
                .iter()
                .any(|(text, _, _)| text == "2.000 Hz / 1.0000x"));
        }
        f.open_rate();
        assert_eq!(f.rack.editing.as_ref().unwrap().display_scale, scale);
        f.draw(vec![egui::Event::Text(typed.into())]);
        let action = f
            .draw(vec![key(egui::Key::Enter, egui::Modifiers::NONE)])
            .expect("Typed rate edit");
        let args = f.execute(action);
        assert!((args["value"].as_f64().unwrap() - expected).abs() < 1e-6);
    }
    for clock in [
        None,
        Some(0.0),
        Some(-1.0),
        Some(f32::NAN),
        Some(f32::INFINITY),
    ] {
        let state = ModuleLightState {
            clock_frequency: clock,
            ..Default::default()
        };
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
            assert_eq!(
                parsed_parameter_result(draft).unwrap(),
                2.0,
                "The open ratio draft still means log2(2 / 0.5)"
            );
            if newer_revision {
                let guarded = parameter_edit_action(draft, 2.0);
                f.session
                    .execute(
                        "set_parameter",
                        json!({"id":"lfo","parameter":"pulse_width","value":0.7}),
                    )
                    .unwrap();
                let before = f.session.snapshot();
                let RackAction::Command(tool, args) = guarded else {
                    unreachable!()
                };
                assert_eq!(args["expected_revision"], opening_revision);
                assert!(f.session.execute(tool, args).is_err());
                f.draw(vec![]);
                assert!(f.rack.editing.as_ref().unwrap().stale);
                assert!(f
                    .draw(vec![key(egui::Key::Enter, egui::Modifiers::NONE)])
                    .is_none());
                assert_eq!(f.session.snapshot(), before);
            } else {
                let action = f
                    .draw(vec![key(egui::Key::Enter, egui::Modifiers::NONE)])
                    .expect("Ratio draft should commit with its opening units");
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
        let mut f = Fixture::new(Some(2.0));
        f.open_rate();
        f.draw(vec![egui::Event::Text(text.into())]);
        assert!(f
            .draw(vec![key(egui::Key::Enter, egui::Modifiers::NONE)])
            .is_none());
        assert!(f.rack.editing.is_some());
    }
    let mut f = Fixture::new(Some(4.0));
    f.open_rate();
    let action = parameter_edit_action(f.rack.editing.as_ref().unwrap(), 2.0);
    f.session
        .execute(
            "set_parameter",
            json!({"id":"lfo","parameter":"pulse_width","value":0.7}),
        )
        .unwrap();
    let before = f.session.snapshot();
    let RackAction::Command(tool, args) = action else {
        unreachable!()
    };
    assert!(f.session.execute(tool, args).is_err());
    f.draw(vec![]);
    assert!(f.rack.editing.as_ref().unwrap().stale);
    assert!(f
        .draw(vec![key(egui::Key::Enter, egui::Modifiers::NONE)])
        .is_none());
    assert_eq!(f.session.snapshot(), before);
}
#[test]
fn stock_lfo_fine_keyboard_adjustment_is_smaller_and_all_eight_jacks_accept_pointer_starts() {
    let mut coarse = Fixture::new(Some(2.0));
    let at = coarse.knobs["rate"].center();
    coarse.click(at);
    let coarse_action = coarse
        .draw(vec![key(egui::Key::ArrowUp, egui::Modifiers::NONE)])
        .unwrap();
    let coarse_args = coarse.execute(coarse_action);
    let mut fine = Fixture::new(Some(2.0));
    let at = fine.knobs["rate"].center();
    fine.click(at);
    let fine_action = fine
        .draw(vec![key(
            egui::Key::ArrowUp,
            egui::Modifiers {
                ctrl: true,
                shift: true,
                ..Default::default()
            },
        )])
        .unwrap();
    let fine_args = fine.execute(fine_action);
    assert!(
        (fine_args["value"].as_f64().unwrap() - 1.0).abs()
            < (coarse_args["value"].as_f64().unwrap() - 1.0).abs()
    );
    for (input, ports) in [
        (true, ["fm", "pwm", "reset", "clock"]),
        (false, ["sine", "triangle", "saw", "square"]),
    ] {
        for port in ports {
            let mut f = Fixture::new(None);
            let at = fixture_jack(&f.rack, f.session.patch(), "lfo", port, input);
            assert!(f.click(at).is_empty());
            let expected = if input {
                CableEnd::Input {
                    node: "lfo".into(),
                    port: port.into(),
                }
            } else {
                CableEnd::Output {
                    node: "lfo".into(),
                    port: port.into(),
                }
            };
            assert_eq!(
                f.rack.pending.as_ref().map(|pending| &pending.origin),
                Some(&expected)
            );
        }
    }
}
fn primitive(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                primitive(shape, out);
            }
        }
        egui::Shape::Text(t) => out.push(format!(
            "text:{:?}:{:?}:{}:{:?}:{:?}:{:?}",
            t.pos,
            t.galley.size(),
            t.galley.text(),
            t.galley.job.sections,
            t.fallback_color,
            t.override_text_color
        )),
        egui::Shape::Rect(r) => out.push(format!("rect:{r:?}")),
        egui::Shape::Circle(c) => out.push(format!("circle:{c:?}")),
        egui::Shape::LineSegment { points, stroke } => {
            out.push(format!("line:{points:?}:{stroke:?}"))
        }
        egui::Shape::Mesh(m) => out.push(format!("mesh:{:?}:{:?}", m.vertices, m.indices)),
        other => out.push(format!("shape:{other:?}")),
    }
}
#[test]
fn stock_lfo_shared_rack_browser_geometry_keeps_controls_labels_and_jacks_separate() {
    assert_eq!(title("stock_lfo"), "STOCK LFO");
    assert_eq!(title("multi_lfo"), "MULTI LFO");
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "stock_lfo")
        .unwrap();
    for scale in [0.25, 0.42, 0.85, 1.0] {
        let mut surfaces = vec![];
        for browser in [false, true] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut preview = ModulePreview::new(module);
            let size = vec2(panel_width("stock_lfo"), PANEL_HEIGHT);
            let bounds = Rect::from_min_size(pos2(24.0, 12.0), size * scale);
            let zoom = (bounds.width() / size.x).min(bounds.height() / size.y);
            let fitted = Rect::from_center_size(bounds.center(), size * zoom);
            let mut rack = Rack {
                zoom,
                pan: Vec2::ZERO,
                ..Default::default()
            };
            rack.positions.insert("lfo".into(), Pos2::ZERO);
            let patch = patch();
            let mut output = egui::FullOutput::default();
            for _ in 0..2 {
                output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 1100.0))),
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
                                    assert_eq!(rendered.inputs.len(), 4);
                                    assert_eq!(rendered.outputs.len(), 4);
                                }
                            });
                    },
                );
            }
            let labels = drawn(&output);
            for required in [
                "STOCK LFO",
                "PHASE",
                "RATE",
                "WIDTH",
                "FM DEPTH",
                "PWM DEPTH",
                "CLOCK —",
                "0–10 V",
                "Invert",
                "FM",
                "PWM",
                "RESET",
                "CLK",
                "SIN",
                "TRI",
                "SAW",
                "SQR",
            ] {
                assert!(
                    labels.iter().any(|(text, _, _)| text == required),
                    "{scale} browser={browser}: missing {required}"
                );
            }
            for (i, (label, rect, _)) in labels.iter().enumerate() {
                assert!(
                    bounds.expand(0.5).contains_rect(*rect),
                    "{scale}: {label} outside panel"
                );
                for (other, other_rect, _) in &labels[i + 1..] {
                    assert!(
                        !rect
                            .shrink(0.2 * scale)
                            .intersects(other_rect.shrink(0.2 * scale)),
                        "{scale}: {label} overlaps {other}"
                    );
                }
            }
            assert_stock_lfo_port_geometry(
                &output,
                if browser { bounds.min } else { fitted.min },
                zoom,
            );
            let mut primitives = vec![];
            for shape in &output.shapes {
                primitive(&shape.shape, &mut primitives);
            }
            surfaces.push(primitives);
            assert!(preview.renderer.editing.is_none());
        }
        assert_eq!(
            surfaces[0], surfaces[1],
            "Stock LFO rack/browser mismatch at {scale}"
        );
    }
}
#[test]
fn stock_lfo_runtime_phase_and_mode_lamps_emit_in_darkness_without_invented_preview_values() {
    let mut f = Fixture::new(Some(4.0));
    let lamp_centers = [pos2(180.0, 285.0), pos2(132.0, 216.0), pos2(132.0, 244.0)]
        .map(|p| f.rack.pan.to_pos2() + p.to_vec2());
    let state = f.rack.module_lights.get_mut("lfo").unwrap();
    state.brightness[2][0] = 1.0;
    state.brightness[3][0] = 1.0;
    state.brightness[4][0] = 1.0;
    let mut signatures = vec![];
    for room in [1.0, 0.0] {
        f.rack.set_room_brightness(room);
        f.draw(vec![]);
        let mut lamp_circles = vec![];
        for shape in &f.output.shapes {
            circles(&shape.shape, &mut lamp_circles);
        }
        assert!(lamp_circles
            .iter()
            .any(|(center, radius)| center.distance(lamp_centers[0]) < 0.1
                && (*radius - 11.0).abs() < 0.01));
        assert!(lamp_circles
            .iter()
            .any(|(center, radius)| center.distance(lamp_centers[0]) < 0.1
                && (*radius - 8.5).abs() < 0.01));
        let mut emitting = vec![];
        for shape in &f.output.shapes {
            if let egui::Shape::Circle(circle) = &shape.shape {
                // Compare measured lens faces, not the additive hot core whose
                // brightness intentionally increases as the room darkens.
                if circle.fill.a() > 0
                    && (circle.fill.b() > 200 || circle.fill.g() > 150 || circle.fill.r() > 200)
                {
                    emitting.push((circle.center, circle.radius, circle.fill));
                }
            }
        }
        // Only compare known measured lamp centers, excluding reflective sockets.
        emitting.retain(|(center, _, _)| lamp_centers.iter().any(|p| center.distance(*p) < 0.1));
        assert_eq!(emitting.len(), 3, "phase and both measured mode lenses");
        signatures.push(emitting);
        assert!(drawn(&f.output)
            .iter()
            .any(|(text, _, color)| text == "CLOCK 4.000 Hz"
                && *color == Color32::from_rgb(155, 220, 197)));
    }
    assert_eq!(signatures[0], signatures[1]);
    f.rack.set_module_states(BTreeMap::new(), BTreeMap::new());
    f.draw(vec![]);
    assert!(drawn(&f.output)
        .iter()
        .any(|(text, _, _)| text == "CLOCK —"));
}

#[test]
fn stock_lfo_rotaries_match_the_stock_oscillator_layout_and_modulation_law() {
    let f = Fixture::new(None);
    for (parameter, center, half) in [
        ("rate", pos2(72.0, 120.0), 38.0),
        ("pulse_width", pos2(288.0, 120.0), 38.0),
        ("fm_amount", pos2(66.0, 239.0), 30.0),
        ("pwm_amount", pos2(294.0, 239.0), 30.0),
    ] {
        let target = f.knobs[parameter];
        assert_eq!(target.size(), Vec2::splat(half * 2.0));
        assert_eq!(target.center(), f.rack.pan.to_pos2() + center.to_vec2());
        assert!(!target.intersects(f.values[parameter]));
    }
    let labels = drawn(&f.output);
    for label in [
        "CONFIGURED 0–10 V",
        "FM × A → OCT",
        "PWM/10 × A → PW",
        "1/256 Hz",
        "1024 Hz",
    ] {
        assert!(
            labels.iter().any(|(text, _, _)| text == label),
            "missing {label}"
        );
    }
    for (direction, ports) in [
        ("in", ["fm", "pwm", "reset", "clock"]),
        ("out", ["sine", "triangle", "saw", "square"]),
    ] {
        for port in ports {
            assert!((f.jacks[&format!("{direction}:{port}")].width() - 37.7).abs() < 0.01);
        }
    }
    assert_stock_lfo_port_clearance(&f);
}

#[test]
fn stock_lfo_configured_shapes_match_real_engine_modes_and_widths() {
    for offset in [0.0, 1.0] {
        for invert in [0.0, 1.0] {
            for width in [0.01, 0.37, 0.99] {
                let mut node = patch().nodes.remove(0);
                node.params.extend([
                    ("rate".into(), 10.0),
                    ("offset".into(), offset),
                    ("invert".into(), invert),
                    ("pulse_width".into(), width),
                ]);
                let mut probe = openpatch_core::StockLfoProbe::new(&node, 48000).unwrap();
                for _ in 0..256 {
                    let out = probe.step(&[openpatch_core::PortFrame::default(); 4], 15);
                    let phase = probe.snapshot().phases[0];
                    for (wave, output) in out.iter().enumerate() {
                        assert!(
                            (output.values[0] - stock_lfo_guide_sample(&node, wave, phase)).abs()
                                < 1e-6,
                            "wave{wave} phase{phase}, offset{offset} invert{invert} width{width}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn stock_lfo_mcp_motion_follows_repositioned_controls_and_clock_units() {
    use crate::construction_motion::{Marker, MotionMode};
    for (parameter, old, new) in [
        ("rate", 1.0, 2.0),
        ("pulse_width", 0.5, 0.75),
        ("fm_amount", 0.0, 0.5),
        ("pwm_amount", 0.0, 0.5),
        ("offset", 1.0, 0.0),
        ("invert", 0.0, 1.0),
    ] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let before = patch();
        let mut after = before.clone();
        after.nodes[0].params.insert(parameter.into(), new);
        let mut lights = ModuleLightState {
            count: 5,
            ..Default::default()
        };
        lights.clock_frequency = Some(4.0);
        rack.set_module_states(BTreeMap::from([("lfo".into(), lights)]), BTreeMap::new());
        frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
        let now = std::time::Instant::now();
        let receipt = json!({"plan_id":19,"completed_steps":1,"last_revision":10,"current_revision":10,
            "last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":19,"step_index":0,"before_revision":9,"revision":10,
            "label":"Shape modulation","new_modules":[],"new_cables":[],"parameter_changes":[{"id":"lfo","kind":"stock_lfo","parameter":parameter,"old_effective":old,"new_effective":new}]}});
        rack.observe_construction_transition_at(
            &before,
            &after,
            9,
            10,
            &receipt,
            MotionMode::Full,
            now,
        );
        rack.set_construction_clock_override(Some(now + std::time::Duration::from_millis(150)));
        assert!(
            frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10)
                .0
                .is_none()
        );
        let anchor = &rack.construction_anchors[&("lfo".into(), parameter.into())];
        let origin = rack.viewport.unwrap().min + rack.pan + rack.positions["lfo"].to_vec2();
        if matches!(parameter, "offset" | "invert") {
            assert_eq!(
                anchor.rect,
                stock_vco_mode_rect(parameter).translate(origin.to_vec2())
            );
            assert!(anchor.marker.is_none());
        } else {
            let Some(Marker::Rotary {
                center,
                radius,
                old: from,
                new: to,
            }) = anchor.marker.as_ref()
            else {
                panic!("{parameter}: missing angular cue")
            };
            let (local, body) = stock_vco_control(parameter);
            assert!(center.distance(origin + local.to_vec2()) < 0.001);
            assert!((*radius - (body + 3.0)).abs() < 0.001);
            assert!(from < to);
            assert_eq!(
                anchor.old_text,
                stock_lfo_parameter_value(parameter, old, Some(4.0))
            );
            assert_eq!(
                anchor.new_text,
                stock_lfo_parameter_value(parameter, new, Some(4.0))
            );
        }
    }
}
