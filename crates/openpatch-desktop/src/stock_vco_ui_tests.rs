use super::*;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"vco","kind":"stock_vco"}]})).unwrap()
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
fn assert_stock_vco_port_clearance(f: &Fixture) {
    let labels = drawn(&f.output);
    let patch = f.session.patch();
    let scale = STOCK_OSCILLATOR_PORT_SCALE * f.rack.zoom;
    let mut painted_circles = vec![];
    for shape in &f.output.shapes {
        circles(&shape.shape, &mut painted_circles);
    }
    let inputs = [
        ("pitch", "V/OCT"),
        ("fm", "FM"),
        ("sync", "SYNC"),
        ("pwm", "PWM"),
    ];
    let outputs = [
        ("sine", "SIN"),
        ("triangle", "TRI"),
        ("saw", "SAW"),
        ("square", "SQR"),
    ];
    let locate = |port: &str, input: bool, _index: usize, text: &str| {
        let endpoint = fixture_jack(&f.rack, patch, "vco", port, input);
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
fn assert_stock_vco_port_geometry(output: &egui::FullOutput, origin: Pos2, zoom: f32) {
    let labels = drawn(output);
    let mut painted_circles = vec![];
    for shape in &output.shapes {
        circles(&shape.shape, &mut painted_circles);
    }
    let scale = STOCK_OSCILLATOR_PORT_SCALE * zoom;
    let inputs = [
        ("pitch", "V/OCT"),
        ("fm", "FM"),
        ("sync", "SYNC"),
        ("pwm", "PWM"),
    ];
    let outputs = [
        ("sine", "SIN"),
        ("triangle", "TRI"),
        ("saw", "SAW"),
        ("square", "SQR"),
    ];
    let locate = |input: bool, index: usize, text: &str| {
        let center = origin
            + port_position("stock_vco", input, index, 4, panel_width("stock_vco")).to_vec2()
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
                        for parameter in ["frequency", "pulse_width", "fm_amount", "pwm_amount"] {
                            if let Some(response) =
                                ctx.read_response(ui.id().with(("knob", "vco", parameter)))
                            {
                                self.knobs.insert(parameter.into(), response.rect);
                            }
                            if let Some(response) =
                                ctx.read_response(ui.id().with(("value", "vco", parameter)))
                            {
                                self.values.insert(parameter.into(), response.rect);
                            }
                        }
                        for port in ["pitch", "fm", "sync", "pwm"] {
                            if let Some(response) =
                                ctx.read_response(ui.id().with(("input", "vco", port)))
                            {
                                self.jacks.insert(format!("in:{port}"), response.rect);
                            }
                        }
                        for port in ["sine", "triangle", "saw", "square"] {
                            if let Some(response) =
                                ctx.read_response(ui.id().with(("output", "vco", port)))
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
    fn open_frequency(&mut self) {
        let at = self.values["frequency"].center();
        assert!(self.click(at).is_empty());
        self.draw(vec![]);
        self.draw(vec![]);
        let edit = self.rack.editing.as_ref().unwrap();
        assert_eq!(edit.parameter, "frequency");
        assert_eq!(edit.display_unit, "Hz");
    }
}
#[test]
fn stock_vco_all_six_controls_use_real_pointer_targets_and_guarded_edits() {
    for parameter in ["frequency", "pulse_width", "fm_amount", "pwm_amount"] {
        let mut f = Fixture::new();
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
    for (label, parameter) in [("Linear FM", "linear_fm"), ("Soft sync", "soft_sync")] {
        let mut f = Fixture::new();
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
        assert_eq!(args["value"], 1.0);
    }
}
#[test]
fn stock_vco_typed_frequency_is_hz_with_exact_semitone_inverse_and_inclusive_bounds() {
    for raw in [-76.0, -24.0, 0.0, 12.0, 76.0] {
        let mut f = Fixture::new();
        f.open_frequency();
        let edit = f.rack.editing.as_ref().unwrap();
        assert_eq!(edit.min, stock_vco_frequency_hz(-76.0));
        assert_eq!(edit.max, stock_vco_frequency_hz(76.0));
        f.draw(vec![egui::Event::Text(format!(
            "{} Hz",
            stock_vco_frequency_hz(raw)
        ))]);
        let action = f
            .draw(vec![key(egui::Key::Enter, egui::Modifiers::NONE)])
            .expect("Typed Hz edit");
        let args = f.execute(action);
        assert!((args["value"].as_f64().unwrap() - f64::from(raw)).abs() < 0.000_03);
    }
    assert_eq!(stock_vco_frequency_hz(0.0), 261.6256);
    assert!(
        !logarithmic_parameter("stock_vco", "frequency"),
        "Stored semitones already provide logarithmic Hz travel"
    );
}
#[test]
fn stock_vco_frequency_draft_rejects_invalid_hz_and_preserves_opening_revision() {
    for text in ["0 Hz", "-1 Hz", "1 Hz", "30000 Hz", "NaN", "1/0"] {
        let mut f = Fixture::new();
        f.open_frequency();
        f.draw(vec![egui::Event::Text(text.into())]);
        assert!(f
            .draw(vec![key(egui::Key::Enter, egui::Modifiers::NONE)])
            .is_none());
        assert!(f.rack.editing.is_some());
    }
    let mut f = Fixture::new();
    f.open_frequency();
    f.draw(vec![egui::Event::Text("523.2512 Hz".into())]);
    let stale = parameter_edit_action(f.rack.editing.as_ref().unwrap(), 12.0);
    f.session
        .execute(
            "set_parameter",
            json!({"id":"vco","parameter":"pulse_width","value":0.7}),
        )
        .unwrap();
    let before = f.session.snapshot();
    let RackAction::Command(tool, args) = stale else {
        unreachable!()
    };
    assert!(f.session.execute(tool, args).is_err());
    f.draw(vec![]);
    assert!(f.rack.editing.as_ref().unwrap().stale);
    assert_eq!(f.rack.editing.as_ref().unwrap().display_unit, "Hz");
    assert!(f
        .draw(vec![key(egui::Key::Enter, egui::Modifiers::NONE)])
        .is_none());
    assert_eq!(f.session.snapshot(), before);
}
#[test]
fn stock_vco_fine_keyboard_pitch_adjustment_and_all_eight_named_jacks_are_live_targets() {
    let mut coarse = Fixture::new();
    let at = coarse.knobs["frequency"].center();
    coarse.click(at);
    let action = coarse
        .draw(vec![key(egui::Key::ArrowUp, egui::Modifiers::NONE)])
        .unwrap();
    let coarse_args = coarse.execute(action);
    let mut fine = Fixture::new();
    let at = fine.knobs["frequency"].center();
    fine.click(at);
    let action = fine
        .draw(vec![key(
            egui::Key::ArrowUp,
            egui::Modifiers {
                ctrl: true,
                shift: true,
                ..Default::default()
            },
        )])
        .unwrap();
    let fine_args = fine.execute(action);
    assert!(
        fine_args["value"].as_f64().unwrap().abs() < coarse_args["value"].as_f64().unwrap().abs()
    );
    for (input, ports) in [
        (true, ["pitch", "fm", "sync", "pwm"]),
        (false, ["sine", "triangle", "saw", "square"]),
    ] {
        for port in ports {
            let mut f = Fixture::new();
            let at = fixture_jack(&f.rack, f.session.patch(), "vco", port, input);
            assert!(f.click(at).is_empty());
            let expected = if input {
                CableEnd::Input {
                    node: "vco".into(),
                    port: port.into(),
                }
            } else {
                CableEnd::Output {
                    node: "vco".into(),
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
fn stock_vco_shared_rack_browser_geometry_keeps_controls_labels_and_jacks_separate() {
    assert_eq!(title("stock_vco"), "STOCK VCO");
    assert_eq!(title("multi_vco"), "MULTI VCO");
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "stock_vco")
        .unwrap();
    for scale in [0.25, 0.42, 0.85, 1.0] {
        let mut surfaces = vec![];
        for browser in [false, true] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let mut preview = ModulePreview::new(module);
            let size = vec2(panel_width("stock_vco"), PANEL_HEIGHT);
            let bounds = Rect::from_min_size(pos2(24.0, 12.0), size * scale);
            let zoom = (bounds.width() / size.x).min(bounds.height() / size.y);
            let fitted = Rect::from_center_size(bounds.center(), size * zoom);
            let mut rack = Rack {
                zoom,
                pan: Vec2::ZERO,
                ..Default::default()
            };
            rack.positions.insert("vco".into(), Pos2::ZERO);
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
                "STOCK VCO",
                "PHASE",
                "FREQUENCY",
                "WIDTH",
                "FM DEPTH",
                "PWM DEPTH",
                "TELEMETRY —",
                "Linear FM",
                "Soft sync",
                "V/OCT",
                "FM",
                "SYNC",
                "PWM",
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
            assert_stock_vco_port_geometry(
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
fn stock_vco_actual_rgb_and_mode_lamps_emit_in_darkness_without_preview_invention() {
    let mut f = Fixture::new();
    let lamp_centers = [180.0, 38.0, 322.0].map(|x| f.rack.pan.to_pos2() + vec2(x, 279.0));
    let mut lights = ModuleLightState {
        count: 5,
        ..Default::default()
    };
    lights.brightness[0][0] = -0.01;
    lights.brightness[2][0] = 1.0;
    lights.brightness[3][0] = 1.0;
    lights.brightness[4][0] = 1.0;
    f.rack
        .set_module_states(BTreeMap::from([("vco".into(), lights)]), BTreeMap::new());
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
                if circle.fill.b() > 200 || circle.fill.g() > 150 || circle.fill.r() > 200 {
                    emitting.push((circle.center, circle.radius, circle.fill));
                }
            }
        }
        emitting.retain(|(center, _, _)| lamp_centers.iter().any(|p| center.distance(*p) < 0.1));
        assert!(!emitting.is_empty());
        signatures.push(emitting);
        assert!(drawn(&f.output)
            .iter()
            .any(|(text, _, _)| text == "RUNTIME STATE"));
        assert!(!drawn(&f.output)
            .iter()
            .any(|(text, _, _)| text.starts_with("CLOCK")));
    }
    assert_eq!(signatures[0], signatures[1]);
    f.rack.set_module_states(BTreeMap::new(), BTreeMap::new());
    f.draw(vec![]);
    assert!(drawn(&f.output)
        .iter()
        .any(|(text, _, _)| text == "TELEMETRY —"));
}

#[test]
fn stock_vco_rotaries_are_large_separate_targets_and_both_fm_laws_are_printed() {
    let f = Fixture::new();
    for (index, parameter) in ["frequency", "pulse_width", "fm_amount", "pwm_amount"]
        .iter()
        .enumerate()
    {
        let target = f.knobs[*parameter];
        assert_eq!(target.size(), vec2(68.0, 68.0));
        assert_eq!(
            target.center(),
            f.rack.pan.to_pos2() + vec2(45.0 + index as f32 * 90.0, 196.0)
        );
        assert!(!target.intersects(f.values[*parameter]));
    }
    let labels = drawn(&f.output);
    assert!(labels
        .iter()
        .any(|(text, _, _)| text == "EXP target: C4·2^(T/12+V+F·A)"));
    assert!(labels
        .iter()
        .any(|(text, _, _)| text == "LIN target: C4·2^(T/12+V)+C4·F·A"));
    assert!(labels
        .iter()
        .any(|(text, _, _)| text == "PWM/10×DEPTH→SQUARE WIDTH · SYNC→PHASE"));
    for port in ["pitch", "fm", "sync", "pwm"] {
        assert!((f.jacks[&format!("in:{port}")].width() - 37.7).abs() < 0.01);
    }
    for port in ["sine", "triangle", "saw", "square"] {
        assert!((f.jacks[&format!("out:{port}")].width() - 37.7).abs() < 0.01);
    }
    assert_stock_vco_port_clearance(&f);
}
