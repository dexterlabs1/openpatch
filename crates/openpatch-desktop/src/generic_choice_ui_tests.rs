// Native egui interactions against real Session edits. Geometry is read from
// the canonical response IDs, never copied from module layout formulas.
use super::*;

const CASES: [(&str, &str, &[&str]); 5] = [
    ("analog_vco", "waveform", &["Saw", "Pulse"]),
    ("crossfade", "law", &["Linear", "Power"]),
    ("audio_stereo", "dc_block", &["Off", "On"]),
    ("score_sequencer", "running", &["Off", "On"]),
    ("drum_voice", "mode", &["Kick", "Clap", "Hi-hat"]),
];
struct ChoiceFixture {
    ctx: egui::Context,
    rack: Rack,
    session: openpatch::control::Session,
    _workspace: tempfile::TempDir,
    kind: &'static str,
    parameter: &'static str,
    default: f32,
    choices: usize,
    time: f64,
    choice_id: Option<egui::Id>,
    choice_rect: Option<Rect>,
    value_rect: Option<Rect>,
    double_clicked: bool,
    output: Option<egui::FullOutput>,
}
fn pointer(position: Pos2, button: PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: position,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}
fn key_event(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}
impl ChoiceFixture {
    fn new(kind: &'static str, parameter: &'static str, choices: usize, zoom: f32) -> Self {
        let catalog = openpatch_core::catalog();
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        let default = module["params"][parameter]["default"].as_f64().unwrap() as f32;
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"target","kind":kind},{"id":"other","kind":"constant","params":{"value":9}}]}))
        .unwrap();
        session.replace_patch(patch, None).unwrap();
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut fixture = Self {
            ctx,
            rack: Rack {
                zoom,
                ..Default::default()
            },
            session,
            _workspace: workspace,
            kind,
            parameter,
            default,
            choices,
            time: 0.0,
            choice_id: None,
            choice_rect: None,
            value_rect: None,
            double_clicked: false,
            output: None,
        };
        fixture.draw(vec![], true, true, false, true);
        fixture.draw(vec![], true, true, false, true);
        assert!(
            fixture.choice_rect.is_some(),
            "{kind}.{parameter}: canonical choice target absent"
        );
        assert!(
            fixture.value_rect.is_some(),
            "{kind}.{parameter}: numeric entry target absent"
        );
        fixture
    }
    fn value(&self) -> f32 {
        self.session
            .patch()
            .nodes
            .iter()
            .find(|node| node.id == "target")
            .unwrap()
            .params
            .get(self.parameter)
            .copied()
            .unwrap_or(self.default)
    }
    fn draw(
        &mut self,
        events: Vec<egui::Event>,
        enabled: bool,
        focused: bool,
        modal: bool,
        apply: bool,
    ) -> Option<Value> {
        let revision = self.session.revision();
        let mut action = None;
        let mut choice_id = None;
        let mut choice_rect = None;
        let mut value_rect = None;
        let mut double_clicked = false;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1800.0, 1100.0))),
                time: Some(self.time),
                events,
                focused,
                ..Default::default()
            },
            |ctx| {
                if modal {
                    egui::Modal::new(egui::Id::new("choice-blocking-modal")).show(ctx, |ui| {
                        ui.label("A blocking modal");
                    });
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        ui.add_enabled_ui(enabled, |ui| {
                            action = self.rack.show(
                                ui,
                                self.session.patch(),
                                &openpatch_core::catalog(),
                                48000,
                                revision,
                            );
                            let id = ui.id().with(("knob", "target", self.parameter));
                            choice_id = Some(id);
                            if let Some(response) = ctx.read_response(id) {
                                choice_rect = Some(response.rect);
                                double_clicked = response.double_clicked_by(PointerButton::Primary);
                            }
                            value_rect = ctx
                                .read_response(ui.id().with(("value", "target", self.parameter)))
                                .map(|response| response.rect);
                        });
                    });
            },
        );
        self.time += 0.05;
        self.choice_id = choice_id;
        self.choice_rect = choice_rect;
        self.value_rect = value_rect;
        self.double_clicked = double_clicked;
        self.output = Some(output);
        match action {
            None => None,
            Some(RackAction::Command("set_parameter", args)) => {
                assert_eq!(args["id"], "target");
                assert_eq!(args["parameter"], self.parameter);
                assert_eq!(args["expected_revision"], revision);
                if apply {
                    self.session.execute("set_parameter", args.clone()).unwrap();
                }
                Some(args)
            }
            Some(_) => panic!("{}: categorical input emitted unrelated action", self.kind),
        }
    }
    fn segment(&self, index: usize) -> Pos2 {
        let rect = self.choice_rect.expect("actual categorical response");
        pos2(
            rect.left() + rect.width() * (index as f32 + 0.5) / self.choices as f32,
            rect.center().y,
        )
    }
    fn click(&mut self, position: Pos2) -> usize {
        let first = self.draw(
            vec![
                egui::Event::PointerMoved(position),
                pointer(position, PointerButton::Primary, true),
            ],
            true,
            true,
            false,
            true,
        );
        let second = self.draw(
            vec![pointer(position, PointerButton::Primary, false)],
            true,
            true,
            false,
            true,
        );
        usize::from(first.is_some()) + usize::from(second.is_some())
    }
    fn key(&mut self, key: egui::Key) -> Option<Value> {
        let action = self.draw(vec![key_event(key, true)], true, true, false, true);
        assert_eq!(
            self.ctx.memory(|m| m.focused()),
            self.choice_id,
            "{}: focused choice lost arrow/activation ownership",
            self.kind
        );
        self.draw(vec![key_event(key, false)], true, true, false, true);
        action
    }
}
fn texts(shape: &egui::Shape, result: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Text(text) => result.push((
            text.galley.text().into(),
            text.galley.rect.translate(text.pos.to_vec2()),
        )),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                texts(shape, result)
            }
        }
        _ => {}
    }
}

#[test]
fn generic_choices_use_explicit_catalog_states_and_real_segments_at_all_rack_scales() {
    let catalog = openpatch_core::catalog();
    for (kind, parameter, labels) in CASES {
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        assert_eq!(
            module["params"][parameter]["values"],
            json!((0..labels.len()).collect::<Vec<_>>())
        );
        assert_eq!(module["params"][parameter]["labels"], json!(labels));
        for zoom in [0.42, 0.85, 1.0, 2.0] {
            let mut f = ChoiceFixture::new(kind, parameter, labels.len(), zoom);
            let control = f.choice_rect.unwrap();
            assert!(
                (control.height() - 24.0 * zoom).abs() < 0.001,
                "{kind}.{parameter}: expected24-point categorical target, not fader"
            );
            assert!(
                !control.intersects(f.value_rect.unwrap()),
                "{kind}.{parameter}: choice and number target overlap"
            );
            let mut painted = vec![];
            for shape in &f.output.as_ref().unwrap().shapes {
                texts(&shape.shape, &mut painted);
            }
            for label in labels {
                let (_, bounds) = painted
                    .iter()
                    .find(|(text, bounds)| text == label && bounds.intersects(control))
                    .unwrap_or_else(|| {
                        panic!("{kind}.{parameter} at{zoom}: no painted {label:?} in choice")
                    });
                assert!(
                    control.expand(0.5).contains_rect(*bounds),
                    "{kind}.{parameter} at{zoom}: clipped state label {label}"
                );
            }
            for index in 0..labels.len() {
                f.time += 0.5;
                let before = f.session.snapshot();
                let old = f.value();
                let actions = f.click(f.segment(index));
                assert_eq!(
                    f.value(),
                    index as f32,
                    "{kind}.{parameter} segment{index} at{zoom}"
                );
                assert_eq!(actions, usize::from(old != index as f32));
                assert_eq!(
                    f.session.revision(),
                    before["revision"].as_u64().unwrap() + u64::from(old != index as f32)
                );
                let selected = f.session.snapshot();
                assert_eq!(
                    f.click(f.segment(index)),
                    0,
                    "click selected must be idempotent"
                );
                assert_eq!(
                    f.session.snapshot(),
                    selected,
                    "selected choice cannot materialize defaults or edit history"
                );
            }
        }
    }
}

#[test]
fn generic_choice_double_and_rapid_repeat_clicks_never_reset_or_add_history() {
    for (kind, parameter, labels) in CASES {
        let mut f = ChoiceFixture::new(kind, parameter, labels.len(), 1.0);
        let chosen = if f.default as usize == labels.len() - 1 {
            0
        } else {
            labels.len() - 1
        };
        let before = f.session.snapshot();
        f.time = 1.0;
        assert_eq!(f.click(f.segment(chosen)), 1);
        let selected = f.session.snapshot();
        let mut saw_double = false;
        for _ in 0..3 {
            assert_eq!(f.click(f.segment(chosen)), 0);
            saw_double |= f.double_clicked;
            assert_eq!(f.value(), chosen as f32);
            assert_eq!(f.session.snapshot(), selected);
        }
        assert!(
            saw_double,
            "{kind}: fixture must produce an actual double-click response"
        );
        assert_ne!(
            chosen as f32, f.default,
            "non-default state required to catch hidden reset"
        );
        f.session.execute("undo", json!({})).unwrap();
        assert_eq!(f.session.snapshot()["patch"], before["patch"]);
        f.session.execute("redo", json!({})).unwrap();
        assert_eq!(f.session.snapshot()["patch"], selected["patch"]);
    }
}

#[test]
fn generic_choice_dragging_across_segments_is_not_a_selection() {
    for (kind, parameter, labels) in CASES {
        let mut f = ChoiceFixture::new(kind, parameter, labels.len(), 1.0);
        let selected = f.default as usize;
        let other = (selected + 1) % labels.len();
        let before = f.session.snapshot();
        let from = f.segment(selected);
        let to = f.segment(other) + vec2(0.0, 40.0);
        for events in [
            vec![
                egui::Event::PointerMoved(from),
                pointer(from, PointerButton::Primary, true),
            ],
            vec![egui::Event::PointerMoved(f.segment(other))],
            vec![egui::Event::PointerMoved(to)],
            vec![pointer(to, PointerButton::Primary, false)],
        ] {
            assert!(
                f.draw(events, true, true, false, true).is_none(),
                "{kind}: dragging cannot pick a state"
            );
            assert_eq!(f.session.snapshot(), before);
        }
    }
}

#[test]
fn generic_choice_keyboard_navigation_clamps_preserves_focus_and_never_pans_camera() {
    for (kind, parameter, labels) in CASES {
        let mut f = ChoiceFixture::new(kind, parameter, labels.len(), 1.0);
        assert_eq!(f.click(f.segment(f.default as usize)), 0);
        f.draw(vec![egui::Event::PointerGone], true, true, false, true);
        assert!(f.ctx.input(|input| input.pointer.hover_pos().is_none()));
        let pan = f.rack.pan;
        let last = labels.len() - 1;
        let mut expected = f.default as usize;
        for key in [
            egui::Key::Home,
            egui::Key::ArrowLeft,
            egui::Key::ArrowRight,
            egui::Key::ArrowUp,
            egui::Key::ArrowRight,
            egui::Key::ArrowDown,
            egui::Key::ArrowLeft,
            egui::Key::End,
            egui::Key::ArrowUp,
            egui::Key::ArrowRight,
            egui::Key::Home,
        ] {
            let prior = expected;
            expected = match key {
                egui::Key::Home => 0,
                egui::Key::End => last,
                egui::Key::ArrowLeft | egui::Key::ArrowDown => expected.saturating_sub(1),
                _ => (expected + 1).min(last),
            };
            let action = f.key(key);
            assert_eq!(action.is_some(), prior != expected, "{kind} {key:?}");
            assert_eq!(f.value(), expected as f32, "{kind} {key:?}");
            assert_eq!(
                f.rack.pan, pan,
                "{kind}: focused choice keys cannot pan rack"
            );
        }
        // OS repeats keep the key held; they are not separate press/release
        // gestures. Each repeat advances once, then remains at the last state.
        for (index, repeat) in [false, true, true].into_iter().enumerate() {
            let mut event = key_event(egui::Key::ArrowRight, true);
            if let egui::Event::Key {
                repeat: event_repeat,
                ..
            } = &mut event
            {
                *event_repeat = repeat;
            }
            let prior = expected;
            expected = (index + 1).min(last);
            let action = f.draw(vec![event], true, true, false, true);
            assert_eq!(action.is_some(), prior != expected);
            assert_eq!(f.value(), expected as f32);
            assert_eq!(f.rack.pan, pan);
            assert_eq!(f.ctx.memory(|m| m.focused()), f.choice_id);
        }
        f.draw(
            vec![key_event(egui::Key::ArrowRight, false)],
            true,
            true,
            false,
            true,
        );
        let selected = f.session.snapshot();
        for key in [egui::Key::Space, egui::Key::Enter, egui::Key::Space] {
            assert!(
                f.key(key).is_none(),
                "{kind}: radio activation reaffirms, not cycles"
            );
            assert_eq!(f.session.snapshot(), selected);
            assert_eq!(f.rack.pan, pan);
        }
    }
}

#[test]
fn generic_choice_right_click_opens_precise_entry_without_selecting_or_mutating() {
    for (kind, parameter, labels) in CASES {
        let mut f = ChoiceFixture::new(kind, parameter, labels.len(), 1.0);
        f.rack.select_only("other".into());
        f.draw(vec![], true, true, false, true);
        let selection = f.rack.selection.clone();
        let before = f.session.snapshot();
        let at = f.segment(labels.len() - 1);
        for pressed in [true, false] {
            assert!(f
                .draw(
                    vec![
                        egui::Event::PointerMoved(at),
                        pointer(at, PointerButton::Secondary, pressed)
                    ],
                    true,
                    true,
                    false,
                    true
                )
                .is_none());
        }
        f.draw(vec![], true, true, false, true);
        let edit = f
            .rack
            .editing
            .as_ref()
            .expect("actual categorical parameter menu");
        assert_eq!(edit.id, "target");
        assert_eq!(edit.parameter, parameter);
        assert_eq!(edit.surface, ParameterEditSurface::KnobMenu);
        assert_eq!(edit.step, Some(1.0));
        assert_eq!((edit.min, edit.max), (0.0, (labels.len() - 1) as f32));
        assert_eq!(f.rack.selection, selection);
        assert_eq!(f.session.snapshot(), before);
    }
    let mut f = ChoiceFixture::new("crossfade", "law", 2, 1.0);
    let before = f.session.snapshot();
    let at = f.value_rect.unwrap().center();
    f.click(at);
    assert_eq!(
        f.rack.editing.as_ref().unwrap().surface,
        ParameterEditSurface::Window
    );
    f.draw(vec![], true, true, false, true);
    f.draw(
        vec![egui::Event::Text("2/2".into())],
        true,
        true,
        false,
        true,
    );
    assert!(f
        .draw(
            vec![key_event(egui::Key::Enter, true)],
            true,
            true,
            false,
            true
        )
        .is_some());
    assert_eq!(f.value(), 1.0);
    f.session.execute("undo", json!({})).unwrap();
    assert_eq!(f.session.snapshot()["patch"], before["patch"]);
}

#[test]
fn generic_choice_disabled_modal_and_precision_guards_block_pointer_and_key_mutations() {
    for guard in ["disabled", "blur", "modal", "precision", "shortcuts"] {
        let mut f = ChoiceFixture::new("drum_voice", "mode", 3, 1.0);
        f.click(f.segment(0));
        if guard == "precision" {
            f.click(f.value_rect.unwrap().center());
            assert!(f.rack.editing.is_some());
            f.draw(vec![], true, true, false, true);
        }
        if guard == "shortcuts" {
            f.rack.set_shortcuts_enabled(false);
        }
        let enabled = guard != "disabled";
        let focused = guard != "blur";
        let modal = guard == "modal";
        f.draw(vec![], enabled, focused, modal, true);
        let before = f.session.snapshot();
        let pan = f.rack.pan;
        let at = f.segment(2);
        for events in [
            vec![
                egui::Event::PointerMoved(at),
                pointer(at, PointerButton::Primary, true),
            ],
            vec![pointer(at, PointerButton::Primary, false)],
            vec![key_event(egui::Key::End, true)],
        ] {
            assert!(
                f.draw(events, enabled, focused, modal, true).is_none(),
                "{guard}: categorical input leaked"
            );
            assert_eq!(f.session.snapshot(), before);
            assert_eq!(f.rack.pan, pan);
        }
    }
}

#[test]
fn generic_choice_proposal_rejects_a_concurrent_session_revision_without_partial_edit() {
    let mut f = ChoiceFixture::new("analog_vco", "waveform", 2, 1.0);
    let at = f.segment(1);
    assert!(f
        .draw(
            vec![
                egui::Event::PointerMoved(at),
                pointer(at, PointerButton::Primary, true)
            ],
            true,
            true,
            false,
            false
        )
        .is_none());
    let args = f
        .draw(
            vec![pointer(at, PointerButton::Primary, false)],
            true,
            true,
            false,
            false,
        )
        .expect("actual choice edit proposal");
    assert_eq!(f.value(), 0.0, "proposal does not modify Session");
    f.session
        .execute(
            "set_parameter",
            json!({"id":"other","parameter":"value","value":10}),
        )
        .unwrap();
    let concurrent = f.session.snapshot();
    assert!(f.session.execute("set_parameter", args).is_err());
    assert_eq!(f.session.snapshot(), concurrent);
    assert_eq!(f.value(), 0.0);
}

#[test]
fn generic_choice_browser_previews_paint_states_but_never_select_or_edit_them() {
    let catalog = openpatch_core::catalog();
    for (kind, parameter, labels) in CASES {
        for scale in [0.42, 0.85, 1.0, 2.0] {
            let f = ChoiceFixture::new(kind, parameter, labels.len(), scale);
            let control = f.choice_rect.unwrap();
            let other = f.segment((f.default as usize + 1) % labels.len());
            let module = catalog["modules"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["kind"] == kind)
                .unwrap();
            let mut preview = ModulePreview::new(module);
            let before = serde_json::to_value(&preview.patch).unwrap();
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let outside = egui::Id::new("outside-preview-focus");
            for (time, events) in [
                (0.0, vec![]),
                (
                    1.0,
                    vec![
                        egui::Event::PointerMoved(other),
                        pointer(other, PointerButton::Primary, true),
                    ],
                ),
                (
                    1.05,
                    vec![
                        pointer(other, PointerButton::Primary, false),
                        key_event(egui::Key::End, true),
                    ],
                ),
                (
                    1.1,
                    vec![
                        key_event(egui::Key::Space, true),
                        key_event(egui::Key::Enter, true),
                    ],
                ),
            ] {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1800.0, 1100.0))),
                        events,
                        time: Some(time),
                        focused: true,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE)
                            .show(ctx, |ui| {
                                ui.interact(
                                    Rect::from_min_size(pos2(1700.0, 10.0), vec2(80.0, 30.0)),
                                    outside,
                                    Sense::click(),
                                );
                                ctx.memory_mut(|memory| memory.request_focus(outside));
                                preview.show(
                                    ui,
                                    Rect::from_min_size(
                                        pos2(24.0, 12.0),
                                        vec2(panel_width(kind), PANEL_HEIGHT) * scale,
                                    ),
                                    &catalog,
                                );
                            });
                    },
                );
                let mut painted = vec![];
                for shape in output.shapes {
                    texts(&shape.shape, &mut painted);
                }
                for label in labels {
                    assert!(
                        painted
                            .iter()
                            .any(|(text, bounds)| text == label && bounds.intersects(control)),
                        "{kind}.{parameter} preview{scale}: missing {label}"
                    );
                }
                assert_eq!(ctx.memory(|m| m.focused()), Some(outside));
                assert!(!egui::Popup::is_any_open(&ctx));
                assert!(preview.renderer.editing.is_none());
                assert!(preview.renderer.selection.is_empty());
                assert_eq!(preview.renderer.gesture, 0);
                assert_eq!(serde_json::to_value(&preview.patch).unwrap(), before);
            }
        }
    }
}
