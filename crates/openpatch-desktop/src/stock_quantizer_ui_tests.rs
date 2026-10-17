use super::*;
fn patch(mask: u16) -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"q","kind":"stock_quantizer","params":{"scale_mask":mask}}]})).unwrap()
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
fn texts(output: &egui::FullOutput) -> Vec<(String, Rect)> {
    fn visit(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Vec(v) => {
                for s in v {
                    visit(s, out);
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
    for s in &output.shapes {
        visit(&s.shape, &mut out);
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
    knob: Rect,
    value: Rect,
    focused: bool,
}
impl Fixture {
    fn new(mask: u16) -> Self {
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch(mask), None).unwrap();
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut f = Self {
            ctx,
            rack: Rack::default(),
            session,
            _workspace: workspace,
            time: 0.0,
            output: Default::default(),
            knob: Rect::NOTHING,
            value: Rect::NOTHING,
            focused: true,
        };
        f.draw(vec![]);
        f.draw(vec![]);
        f
    }
    fn draw(&mut self, events: Vec<egui::Event>) -> Option<RackAction> {
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
        let mut action = None;
        self.output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 900.0))),
                time: Some(self.time),
                focused: self.focused,
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
                        if let Some(r) = ctx.read_response(ui.id().with(("knob", "q", "offset"))) {
                            self.knob = r.rect;
                        }
                        if let Some(r) = ctx.read_response(ui.id().with(("value", "q", "offset"))) {
                            self.value = r.rect;
                        }
                    });
            },
        );
        self.time += 0.1;
        action
    }
    fn note(&self, note: usize) -> Pos2 {
        let r = stock_quantizer_key_rect(note);
        pos2(24.0, 12.0)
            + vec2(
                r.center().x,
                if matches!(note, 1 | 3 | 6 | 8 | 10) {
                    r.center().y
                } else {
                    r.bottom() - 18.0
                },
            )
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
        match action {
            RackAction::ScalePaint { arguments, gesture } => {
                let result = self
                    .session
                    .execute("set_parameter", arguments.clone())
                    .map(|_| self.session.revision());
                self.rack.complete_scale_paint(gesture, result.clone());
                result.unwrap();
                arguments
            }
            RackAction::Command(tool, args) => {
                self.session.execute(tool, args.clone()).unwrap();
                args
            }
            _ => panic!("Expected real edit"),
        }
    }
    fn open_offset(&mut self) {
        let at = self.value.center();
        assert!(self.click(at).is_empty());
        self.draw(vec![]);
        self.draw(vec![]);
        assert_eq!(self.rack.editing.as_ref().unwrap().display_unit, "st");
    }
}
#[test]
fn stock_quantizer_all_twelve_real_note_targets_toggle_their_own_bits() {
    for note in 0..12 {
        let mut f = Fixture::new(4095);
        let at = f.note(note);
        let action = f
            .draw(vec![egui::Event::PointerMoved(at), button(at, true)])
            .unwrap();
        let args = f.execute(action);
        assert_eq!(args["value"], 4095_u16 & !(1 << note));
        assert_eq!(args["parameter"], "scale_mask");
        f.draw(vec![button(at, false)]);
        assert!(f.rack.scale_paint.is_none());
    }
}
#[test]
fn stock_quantizer_drag_paints_origin_value_including_black_keys_and_is_one_undo() {
    for initial in [0, 4095] {
        let mut f = Fixture::new(initial);
        let mut groups = vec![];
        for (index, note) in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
            .into_iter()
            .enumerate()
        {
            let at = f.note(note);
            let mut events = vec![egui::Event::PointerMoved(at)];
            if index == 0 {
                events.push(button(at, true));
            }
            let action = f.draw(events).unwrap();
            let revision = f.session.revision();
            let args = f.execute(action);
            assert_eq!(args["expected_revision"], revision);
            groups.push(args["history_group"].clone());
        }
        assert!(groups.windows(2).all(|p| p[0] == p[1]));
        let at = f.note(0);
        assert!(
            f.draw(vec![egui::Event::PointerMoved(at)]).is_none(),
            "Revisiting keeps the origin's painted value"
        );
        f.draw(vec![button(at, false)]);
        assert_eq!(
            stock_quantizer_mask(&f.session.patch().nodes[0]),
            initial ^ 4095
        );
        f.session.execute("undo", json!({})).unwrap();
        assert_eq!(stock_quantizer_mask(&f.session.patch().nodes[0]), initial);
        f.session.execute("redo", json!({})).unwrap();
        assert_eq!(
            stock_quantizer_mask(&f.session.patch().nodes[0]),
            initial ^ 4095
        );
    }
}
#[test]
fn stock_quantizer_paint_cancels_on_foreign_revision_focus_modal_escape_or_deletion() {
    for case in 0..5 {
        let mut f = Fixture::new(4095);
        let at = f.note(0);
        let action = f
            .draw(vec![egui::Event::PointerMoved(at), button(at, true)])
            .unwrap();
        f.execute(action);
        match case {
            0 => {
                f.session
                    .execute(
                        "set_parameter",
                        json!({"id":"q","parameter":"offset","value":0.25}),
                    )
                    .unwrap();
            }
            1 => f.focused = false,
            2 => f.rack.set_shortcuts_enabled(false),
            3 => {}
            _ => {
                f.session
                    .execute("remove_module", json!({"id":"q"}))
                    .unwrap();
            }
        }
        let mut events = vec![egui::Event::PointerMoved(f.note(2))];
        if case == 3 {
            events.push(key(egui::Key::Escape, egui::Modifiers::NONE));
        }
        assert!(f.draw(events).is_none());
        assert!(f.rack.scale_paint.is_none(), "case {case}");
    }
}
#[test]
fn stock_quantizer_unaccepted_or_stale_paint_cannot_continue_under_assumed_authority() {
    let mut f = Fixture::new(4095);
    let at = f.note(0);
    let action = f
        .draw(vec![egui::Event::PointerMoved(at), button(at, true)])
        .unwrap();
    let RackAction::ScalePaint { arguments, gesture } = action else {
        panic!()
    };
    f.session
        .execute(
            "set_parameter",
            json!({"id":"q","parameter":"offset","value":0.5}),
        )
        .unwrap();
    let before = f.session.snapshot();
    let result = f
        .session
        .execute("set_parameter", arguments)
        .map(|_| f.session.revision());
    assert!(result.is_err());
    f.rack.complete_scale_paint(gesture, result);
    assert!(f.rack.scale_paint.is_none());
    assert!(f.draw(vec![egui::Event::PointerMoved(f.note(2))]).is_none());
    assert_eq!(f.session.snapshot(), before);
}
#[test]
fn stock_quantizer_offset_exact_entry_uses_semitones_and_original_revision() {
    for semitones in [-12.0, -1.0, 0.0, 1.0, 12.0] {
        let mut f = Fixture::new(4095);
        f.open_offset();
        assert_eq!(
            (
                f.rack.editing.as_ref().unwrap().min,
                f.rack.editing.as_ref().unwrap().max
            ),
            (-12.0, 12.0)
        );
        f.draw(vec![egui::Event::Text(format!("{semitones} semitones"))]);
        let action = f
            .draw(vec![key(egui::Key::Enter, egui::Modifiers::NONE)])
            .unwrap();
        let args = f.execute(action);
        assert!((args["value"].as_f64().unwrap() - semitones / 12.0).abs() < 1e-6);
    }
    for invalid in ["-13 st", "13 st", "NaN", "1/0"] {
        let mut f = Fixture::new(4095);
        f.open_offset();
        f.draw(vec![egui::Event::Text(invalid.into())]);
        assert!(f
            .draw(vec![key(egui::Key::Enter, egui::Modifiers::NONE)])
            .is_none());
    }
    let mut f = Fixture::new(4095);
    f.open_offset();
    f.session
        .execute(
            "set_parameter",
            json!({"id":"q","parameter":"scale_mask","value":1}),
        )
        .unwrap();
    f.draw(vec![]);
    assert!(f.rack.editing.as_ref().unwrap().stale);
}
#[test]
fn stock_quantizer_offset_pointer_fine_keyboard_shifts_and_two_jacks_are_real() {
    let mut f = Fixture::new(4095);
    let at = f.knob.center();
    f.draw(vec![egui::Event::PointerMoved(at), button(at, true)]);
    let action = f
        .draw(vec![egui::Event::PointerMoved(at - vec2(0.0, 8.0))])
        .unwrap();
    let args = f.execute(action);
    assert_eq!(args["parameter"], "offset");
    let mut coarse = Fixture::new(4095);
    let at = coarse.knob.center();
    coarse.click(at);
    let a = coarse
        .draw(vec![key(egui::Key::ArrowUp, egui::Modifiers::NONE)])
        .unwrap();
    let c = coarse.execute(a)["value"].as_f64().unwrap();
    let mut fine = Fixture::new(4095);
    let at = fine.knob.center();
    fine.click(at);
    let a = fine
        .draw(vec![key(
            egui::Key::ArrowUp,
            egui::Modifiers {
                ctrl: true,
                shift: true,
                ..Default::default()
            },
        )])
        .unwrap();
    let v = fine.execute(a)["value"].as_f64().unwrap();
    assert!(v.abs() < c.abs());
    for (label, shift) in [("−1 semitone", -1), ("+1 semitone", 1)] {
        let mut f = Fixture::new(1);
        let at = texts(&f.output)
            .into_iter()
            .find(|(s, _)| s == label)
            .unwrap()
            .1
            .center();
        let mut actions = f.click(at);
        assert_eq!(actions.len(), 1);
        let args = f.execute(actions.remove(0));
        assert_eq!(args["semitones"], shift);
        assert_eq!(
            stock_quantizer_mask(&f.session.patch().nodes[0]),
            if shift < 0 { 2048 } else { 2 }
        );
    }
    for input in [true, false] {
        let mut f = Fixture::new(4095);
        let at = fixture_jack(&f.rack, f.session.patch(), "q", "pitch", input);
        assert!(f.click(at).is_empty());
        let expected = if input {
            CableEnd::Input {
                node: "q".into(),
                port: "pitch".into(),
            }
        } else {
            CableEnd::Output {
                node: "q".into(),
                port: "pitch".into(),
            }
        };
        assert_eq!(f.rack.pending.as_ref().map(|p| &p.origin), Some(&expected));
    }
}
#[test]
fn stock_quantizer_played_highlights_are_measured_separate_from_enabled_and_emit_in_darkness() {
    let mut f = Fixture::new(0);
    let mut signatures = vec![];
    for room in [1.0, 0.0] {
        f.rack.set_room_brightness(room);
        f.rack.set_module_states(
            BTreeMap::from([(
                "q".into(),
                ModuleLightState {
                    count: 0,
                    playing_notes: Some(1 | 16 | 128),
                    ..Default::default()
                },
            )]),
            BTreeMap::new(),
        );
        f.draw(vec![]);
        let rects = f
            .output
            .shapes
            .iter()
            .filter_map(|s| {
                if let egui::Shape::Rect(r) = &s.shape {
                    (r.fill == Color32::from_rgb(75, 232, 218)).then_some(r.rect)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(rects.len(), 3);
        signatures.push(rects);
        assert_eq!(stock_quantizer_mask(&f.session.patch().nodes[0]), 0);
    }
    assert_eq!(signatures[0], signatures[1]);
    for playing in [None, Some(0), Some(4096)] {
        f.rack.set_module_states(
            BTreeMap::from([(
                "q".into(),
                ModuleLightState {
                    count: 0,
                    playing_notes: playing,
                    ..Default::default()
                },
            )]),
            BTreeMap::new(),
        );
        f.draw(vec![]);
        assert!(!f.output.shapes.iter().any(
            |s| matches!(&s.shape,egui::Shape::Rect(r)if r.fill==Color32::from_rgb(75,232,218))
        ));
        if playing != Some(0) {
            assert!(texts(&f.output).iter().any(|(s, _)| s == "PLAYED —"));
        }
    }
}
fn primitives(output: &egui::FullOutput) -> Vec<String> {
    fn visit(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Vec(v) => {
                for s in v {
                    visit(s, out)
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
            s => out.push(format!("{s:?}")),
        }
    }
    let mut out = vec![];
    for s in &output.shapes {
        visit(&s.shape, &mut out);
    }
    out
}
#[test]
fn stock_quantizer_shared_panel_preserves_keyboard_text_and_jack_geometry_at_all_zooms() {
    assert_eq!(title("stock_quantizer"), "STOCK QUANTIZER");
    assert_eq!(title("quantizer"), "QUANTIZER");
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "stock_quantizer")
        .unwrap();
    for scale in [0.25, 0.42, 0.7, 1.0, 1.75] {
        let mut surfaces = vec![];
        for browser in [false, true] {
            let ctx = egui::Context::default();
            crate::panel_skin::install_fonts(&ctx);
            let size = vec2(panel_width("stock_quantizer"), PANEL_HEIGHT);
            let bounds = Rect::from_min_size(pos2(24.0, 12.0), size * scale);
            let zoom = (bounds.width() / size.x).min(bounds.height() / size.y);
            let fitted = Rect::from_center_size(bounds.center(), size * zoom);
            let mut preview = ModulePreview::new(module);
            let mut rack = Rack {
                zoom,
                pan: Vec2::ZERO,
                ..Default::default()
            };
            rack.positions.insert("q".into(), Pos2::ZERO);
            let patch = patch(4095);
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
                                        (1, 1)
                                    );
                                }
                            });
                    },
                );
            }
            let text = texts(&output);
            assert_eq!(
                text.iter().filter(|(label, _)| label == "V/OCT").count(),
                2,
                "Both real pitch ports retain their printed labels"
            );
            for required in [
                "STOCK QUANTIZER",
                "ENABLED NOTES",
                "PLAYED —",
                "AMBER: ENABLED  ·  CYAN: PLAYED",
                "OFFSET",
                "−1 semitone",
                "+1 semitone",
                "Presets",
            ] {
                assert!(
                    text.iter().any(|(s, _)| s == required),
                    "{scale} browser={browser} missing {required}"
                );
            }
            for note in STOCK_QUANTIZER_NOTES {
                assert!(text.iter().any(|(s, _)| s == note));
            }
            for (i, (s, r)) in text.iter().enumerate() {
                assert!(
                    bounds.expand(0.5).contains_rect(*r),
                    "{scale} {s} outside panel"
                );
                for (other, o) in &text[i + 1..] {
                    assert!(
                        !r.shrink(0.2 * zoom).intersects(o.shrink(0.2 * zoom)),
                        "{scale} {s} overlaps {other}"
                    );
                }
            }
            for input in [true, false] {
                let center = fitted.min
                    + port_position("stock_quantizer", input, 0, 1, size.x).to_vec2() * zoom;
                let socket = Rect::from_center_size(center, Vec2::splat(26.0 * zoom));
                assert!(bounds.contains_rect(socket));
                for (label, r) in &text {
                    assert!(
                        !socket.shrink(0.5 * zoom).intersects(*r),
                        "{scale} browser={browser} {label} {r:?} overlaps input={input} jack {socket:?}"
                    );
                }
            }
            assert!(preview.renderer.scale_paint.is_none());
            assert!(preview.renderer.construction_anchors.is_empty());
            surfaces.push(primitives(&output));
        }
        assert_eq!(
            surfaces[0], surfaces[1],
            "Shared renderer mismatch at {scale}"
        );
    }
}
#[test]
fn stock_quantizer_scale_presets_are_atomic_and_retain_dropdown_opening_revision() {
    let mut f = Fixture::new(4095);
    let at = texts(&f.output)
        .into_iter()
        .find(|(s, _)| s == "Presets")
        .unwrap()
        .1
        .center();
    assert!(f.click(at).is_empty());
    f.draw(vec![]);
    let at = texts(&f.output)
        .into_iter()
        .find(|(s, _)| s == "Major")
        .unwrap()
        .1
        .center();
    let mut actions = f.click(at);
    assert_eq!(actions.len(), 1);
    let args = f.execute(actions.remove(0));
    assert_eq!(args["parameter"], "scale_mask");
    assert_eq!(args["value"], 2741);
    f.session.execute("undo", json!({})).unwrap();
    assert_eq!(stock_quantizer_mask(&f.session.patch().nodes[0]), 4095);
    let mut f = Fixture::new(4095);
    let opening = f.session.revision();
    let at = texts(&f.output)
        .into_iter()
        .find(|(s, _)| s == "Presets")
        .unwrap()
        .1
        .center();
    f.click(at);
    f.draw(vec![]);
    f.session
        .execute(
            "set_parameter",
            json!({"id":"q","parameter":"offset","value":0.1}),
        )
        .unwrap();
    f.draw(vec![]);
    let at = texts(&f.output)
        .into_iter()
        .find(|(s, _)| s == "Minor")
        .unwrap()
        .1
        .center();
    let mut actions = f.click(at);
    let RackAction::Command(tool, args) = actions.remove(0) else {
        panic!()
    };
    assert_eq!(args["expected_revision"], opening);
    let before = f.session.snapshot();
    assert!(f.session.execute(tool, args).is_err());
    assert_eq!(f.session.snapshot(), before);
}
#[test]
fn stock_quantizer_paint_uses_one_origin_value_across_mixed_enabled_and_disabled_keys() {
    for initial in [16, 17] {
        let mut f = Fixture::new(initial);
        let c = f.note(0);
        let action = f
            .draw(vec![egui::Event::PointerMoved(c), button(c, true)])
            .unwrap();
        f.execute(action);
        let expected = initial ^ 1;
        assert_eq!(stock_quantizer_mask(&f.session.patch().nodes[0]), expected);
        let already = if initial == 16 { 4 } else { 2 };
        assert!(
            f.draw(vec![egui::Event::PointerMoved(f.note(already))])
                .is_none(),
            "An already matching key must not toggle away from the origin's painted value"
        );
        let change = if initial == 16 { 2 } else { 4 };
        let action = f
            .draw(vec![egui::Event::PointerMoved(f.note(change))])
            .unwrap();
        f.execute(action);
        assert_eq!(
            stock_quantizer_mask(&f.session.patch().nodes[0]),
            if initial == 16 { 21 } else { 0 }
        );
        let at = f.note(change);
        f.draw(vec![button(at, false)]);
        f.session.execute("undo", json!({})).unwrap();
        assert_eq!(stock_quantizer_mask(&f.session.patch().nodes[0]), initial);
    }
}
