// Included inside rack::tests. Real egui events emit edits into the real Session;
// these tests do not call parameter_fader_input or synthesize its result.
use super::*;

struct FaderFixture {
    ctx: egui::Context,
    rack: Rack,
    session: openpatch::control::Session,
    _workspace: tempfile::TempDir,
    time: f64,
}
impl FaderFixture {
    fn new(kind: &str, parameter: &str, value: f32, zoom: f32) -> Self {
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[{"id":"target","kind":kind,"params":{parameter:value}},
                     {"id":"other","kind":"constant","params":{"value":9.0}}]}))
        .unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        let mut fixture = Self {
            ctx: egui::Context::default(),
            rack: Rack {
                zoom,
                ..Rack::default()
            },
            session,
            _workspace: workspace,
            time: 0.0,
        };
        fixture.draw(vec![], true);
        fixture
    }
    fn draw(&mut self, events: Vec<egui::Event>, enabled: bool) -> Option<Value> {
        let revision = self.session.revision();
        let action = frame_revision_state(
            &self.ctx,
            &mut self.rack,
            self.session.patch(),
            self.time,
            events,
            true,
            enabled,
            revision,
        )
        .0;
        self.time += 0.05;
        match action {
            None => None,
            Some(RackAction::Command("set_parameter", args)) => {
                assert_eq!(args["expected_revision"], revision);
                self.session.execute("set_parameter", args.clone()).unwrap();
                Some(args)
            }
            Some(_) => panic!("a parameter gesture emitted an unrelated rack action"),
        }
    }
    fn value(&self, id: &str, parameter: &str) -> f32 {
        self.session
            .patch()
            .nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap()
            .params[parameter]
    }
    fn undo_steps(&self) -> u64 {
        self.session.snapshot()["history"]["undo_steps"]
            .as_u64()
            .unwrap()
    }
    fn track(&self, id: &str) -> Rect {
        let node = self
            .session
            .patch()
            .nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap();
        let width = module_width(node);
        let (local, radius) = match node.kind.as_str() {
            "constant" => atlas_legacy_utility_control("constant", "value"),
            "octave" => atlas_legacy_utility_control("octave", "octaves"),
            "audio_stereo" => (pos2(width * 0.5, 130.0), 30.0),
            _ => parameter_position(1, 0, width),
        };
        let center = self.rack.viewport.unwrap().min
            + self.rack.pan
            + (self.rack.positions[id].to_vec2() + local.to_vec2()) * self.rack.zoom;
        if atlas_legacy_utility_kind(&node.kind) {
            // Rotary motion is relative, with 150 logical points per full range.
            // This span describes input travel, not painted fader geometry.
            Rect::from_center_size(center, vec2(radius * 2.0, 150.0) * self.rack.zoom)
        } else {
            crate::panel_skin::parameter_fader_track(center, radius, self.rack.zoom)
        }
    }
    fn readout(&self) -> Pos2 {
        self.rack.viewport.unwrap().min
            + self.rack.pan
            + (self.rack.positions["target"].to_vec2()
                + atlas_legacy_utility_value_center("constant", "value").to_vec2())
                * self.rack.zoom
    }
    fn gate_duty_rotary(&self) -> (Pos2, Pos2) {
        let node = self
            .session
            .patch()
            .nodes
            .iter()
            .find(|node| node.id == "target")
            .unwrap();
        assert_eq!(node.kind, "gate");
        let (local, radius) = atlas_signal_utility_control("gate", "duty");
        assert!(matches!(
            generic_parameter_hardware("gate", "duty", radius, false),
            GenericParameterHardware::Rotary { .. }
        ));
        let center = self.rack.viewport.unwrap().min
            + self.rack.pan
            + (self.rack.positions["target"].to_vec2() + local.to_vec2()) * self.rack.zoom;
        (
            center,
            self.rack.viewport.unwrap().min
                + self.rack.pan
                + (self.rack.positions["target"].to_vec2()
                    + atlas_signal_utility_value_center("gate", "duty").to_vec2())
                    * self.rack.zoom,
        )
    }
    fn oscillator_rotary_center(&self, kind: &str, parameter: &str) -> Pos2 {
        let node = self
            .session
            .patch()
            .nodes
            .iter()
            .find(|node| node.id == "target")
            .unwrap();
        assert_eq!(node.kind, kind);
        let local = match (kind, parameter) {
            ("multi_vco" | "multi_lfo", "frequency") => pos2(75.0, 125.0),
            ("multi_vco" | "multi_lfo", "pulse_width") => pos2(225.0, 125.0),
            ("wavetable_vco" | "wavetable_lfo", "position") => pos2(180.0, 194.0),
            _ => panic!("unsupported oscillator test target: {kind}.{parameter}"),
        };
        self.rack.viewport.unwrap().min
            + self.rack.pan
            + (self.rack.positions["target"].to_vec2() + local.to_vec2()) * self.rack.zoom
    }
    fn press(&mut self, at: Pos2, modifiers: egui::Modifiers) -> Option<Value> {
        self.draw(
            vec![egui::Event::PointerMoved(at), pointer(at, true, modifiers)],
            true,
        )
    }
    fn move_to(&mut self, at: Pos2, modifiers: egui::Modifiers) -> Option<Value> {
        self.draw(
            vec![egui::Event::PointerMoved(at), modifier_carrier(modifiers)],
            true,
        )
    }
    fn release(&mut self, at: Pos2, modifiers: egui::Modifiers) -> Option<Value> {
        self.draw(vec![pointer(at, false, modifiers)], true)
    }
    fn click(&mut self, at: Pos2) {
        self.press(at, egui::Modifiers::NONE);
        self.release(at, egui::Modifiers::NONE);
    }
}

#[test]
fn converted_gate_rotary_click_stays_put_then_relative_drag_has_one_undo() {
    for zoom in [0.5, 1.0, 2.0] {
        let mut f = FaderFixture::new("gate", "duty", 0.5, zoom);
        let (center, _) = f.gate_duty_rotary();
        let baseline = f.undo_steps();
        let revision = f.session.revision();
        f.time = 1.0;
        assert!(f.press(center, egui::Modifiers::NONE).is_none());
        assert!(f.release(center, egui::Modifiers::NONE).is_none());
        assert_eq!(f.value("target", "duty"), 0.5);
        assert_eq!(f.session.revision(), revision);
        assert_eq!(f.undo_steps(), baseline);

        f.time = 2.0;
        assert!(f.press(center, egui::Modifiers::NONE).is_none());
        let end = center - vec2(0.0, 15.0 * zoom);
        let edit = f
            .move_to(end, egui::Modifiers::NONE)
            .expect("relative rotary drag");
        assert_eq!(edit["parameter"], "duty");
        assert_eq!(edit["expected_revision"], revision);
        assert!(edit["history_group"].as_str().is_some());
        near(f.value("target", "duty"), 0.598);
        let after_drag = f.session.revision();
        assert!(f.release(end, egui::Modifiers::NONE).is_none());
        assert_eq!(f.session.revision(), after_drag);
        assert_eq!(f.undo_steps(), baseline + 1);
        assert_eq!(f.value("other", "value"), 9.0);
        f.session.execute("undo", json!({})).unwrap();
        assert_eq!(f.value("target", "duty"), 0.5);
        f.session.execute("redo", json!({})).unwrap();
        near(f.value("target", "duty"), 0.598);
    }
}

#[test]
fn converted_rotary_fine_keyboard_and_exact_entry_share_session_authority() {
    let mut f = FaderFixture::new("gate", "duty", 0.5, 1.0);
    let (center, readout) = f.gate_duty_rotary();
    let baseline = f.undo_steps();
    f.time = 1.0;
    f.press(center, ctrl());
    let fine_end = center - vec2(0.0, 75.0);
    f.move_to(fine_end, ctrl()).expect("Ctrl rotary drag");
    near(f.value("target", "duty"), 0.549);
    let revision = f.session.revision();
    assert!(f.move_to(fine_end, egui::Modifiers::NONE).is_none());
    assert_eq!(f.session.revision(), revision);
    near(f.value("target", "duty"), 0.549);
    let end = fine_end - vec2(0.0, 15.0);
    f.move_to(end, egui::Modifiers::NONE)
        .expect("normal drag continues from fine value");
    near(f.value("target", "duty"), 0.647);
    f.release(end, egui::Modifiers::NONE);
    assert_eq!(f.undo_steps(), baseline + 1);

    let pan_before_keys = f.rack.pan;
    for (key, expected) in [
        (egui::Key::ArrowUp, 0.6568),
        (egui::Key::Home, 0.01),
        (egui::Key::End, 0.99),
    ] {
        let action = f
            .draw(
                vec![key_event(key, true, false, egui::Modifiers::NONE)],
                true,
            )
            .expect("focused rotary keyboard edit");
        assert_eq!(action["parameter"], "duty");
        near(f.value("target", "duty"), expected);
        f.draw(
            vec![key_event(key, false, false, egui::Modifiers::NONE)],
            true,
        );
    }
    let revision = f.session.revision();
    for key in [egui::Key::Space, egui::Key::Enter] {
        assert!(f
            .draw(
                vec![key_event(key, true, false, egui::Modifiers::NONE)],
                true
            )
            .is_none());
        assert_eq!(f.session.revision(), revision);
        f.draw(
            vec![key_event(key, false, false, egui::Modifiers::NONE)],
            true,
        );
    }

    assert_eq!(
        f.rack.pan, pan_before_keys,
        "focused rotary keys must not pan the rack"
    );
    assert_eq!(readout, f.gate_duty_rotary().1);
    f.time = 3.0;
    // Move from the focused knob to the value field before pressing, as a
    // separate pointer frame; this settles egui's prior keyboard ownership.
    assert!(f
        .draw(vec![egui::Event::PointerMoved(readout)], true)
        .is_none());
    f.click(readout);
    let edit = f.rack.editing.as_mut().expect("rotary exact value editor");
    assert_eq!((&*edit.id, &*edit.parameter), ("target", "duty"));
    edit.text = "0.375".into();
    f.draw(
        vec![key_event(
            egui::Key::Enter,
            true,
            false,
            egui::Modifiers::NONE,
        )],
        true,
    )
    .expect("exact value commits through Session");
    assert_eq!(f.value("target", "duty"), 0.375);
}
fn pointer(at: Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers,
    }
}
fn key_event(
    key: egui::Key,
    pressed: bool,
    repeat: bool,
    modifiers: egui::Modifiers,
) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers,
    }
}
fn modifier_carrier(modifiers: egui::Modifiers) -> egui::Event {
    // The shared frame helper derives RawInput.modifiers from button/key events.
    // A release-only unused function key carries modifiers without a key action.
    key_event(egui::Key::F12, false, false, modifiers)
}
fn ctrl() -> egui::Modifiers {
    egui::Modifiers {
        ctrl: true,
        ..Default::default()
    }
}
fn near(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 2e-4 * expected.abs().max(1.0),
        "{actual} versus {expected}"
    );
}

#[test]
fn generic_fader_absolute_track_clicks_reach_limits_at_rack_scales() {
    for zoom in [0.5, 1.0, 2.0] {
        let mut f = FaderFixture::new("audio_stereo", "gain", 1.25, zoom);
        let baseline = f.undo_steps();
        let track = f.track("target");
        f.time = 1.0;
        f.click(pos2(track.center().x, track.top()));
        assert_eq!(f.value("target", "gain"), 2.0);
        assert_eq!(f.value("other", "value"), 9.0);
        f.time = 2.0; // independent click, not a double-click reset
        f.click(pos2(track.center().x, track.bottom()));
        assert_eq!(f.value("target", "gain"), 0.0);
        assert_eq!(f.undo_steps(), baseline + 2);
    }
}

#[test]
fn multi_vco_rotary_geometric_frequency_drag_moves_one_octave_over_relative_travel() {
    let c4 = 261.625_58;
    let mut f = FaderFixture::new("multi_vco", "frequency", c4, 2.0);
    // The saved frequency spans nine octaves. Knob input has 150 logical
    // points of relative travel, independent of its painted body and target.
    let from = f.oscillator_rotary_center("multi_vco", "frequency");
    let to = from - vec2(0.0, 150.0 * f.rack.zoom / 9.0);
    f.time = 1.0;
    f.press(from, egui::Modifiers::NONE);
    let moved = f
        .move_to(to, egui::Modifiers::NONE)
        .expect("octave drag emits edit");
    assert_eq!(moved["parameter"], "frequency");
    f.release(to, egui::Modifiers::NONE);
    near(f.value("target", "frequency"), c4 * 2.0);
    assert_eq!(f.value("other", "value"), 9.0);
}

#[test]
fn multi_and_wavetable_rotaries_click_without_jump_then_drag_through_session() {
    for (kind, parameter, expected) in [
        ("multi_vco", "pulse_width", 0.696),
        ("multi_lfo", "pulse_width", 0.696),
        ("wavetable_vco", "position", 0.7),
        ("wavetable_lfo", "position", 0.7),
    ] {
        for zoom in [0.35, 0.5, 1.0] {
            let mut f = FaderFixture::new(kind, parameter, 0.5, zoom);
            let center = f.oscillator_rotary_center(kind, parameter);
            let initial_revision = f.session.revision();
            let initial_history = f.undo_steps();
            f.time = 1.0;
            assert!(f.press(center, egui::Modifiers::NONE).is_none());
            assert!(f.release(center, egui::Modifiers::NONE).is_none());
            assert_eq!(f.value("target", parameter), 0.5);
            assert_eq!(f.session.revision(), initial_revision);

            f.time = 2.0;
            assert!(f.press(center, egui::Modifiers::NONE).is_none());
            let end = center - vec2(0.0, 30.0 * zoom);
            let edit = f
                .move_to(end, egui::Modifiers::NONE)
                .expect("relative knob edit");
            assert_eq!(edit["parameter"], parameter);
            assert_eq!(edit["expected_revision"], initial_revision);
            assert!(edit["history_group"].as_str().is_some());
            near(f.value("target", parameter), expected);
            f.release(end, egui::Modifiers::NONE);
            assert_eq!(f.undo_steps(), initial_history + 1);
            f.session.execute("undo", json!({})).unwrap();
            assert_eq!(f.value("target", parameter), 0.5);
        }
    }
}

#[test]
fn scalar_rotary_ctrl_drag_is_fine_and_modifier_transition_uses_current_position() {
    for transition in [false, true] {
        let mut f = FaderFixture::new("constant", "value", 0.0, 1.0);
        let track = f.track("target");
        let start = track.center();
        let baseline = f.undo_steps();
        f.time = 1.0;
        f.press(
            start,
            if transition {
                egui::Modifiers::NONE
            } else {
                ctrl()
            },
        );
        let (point, expected) = if transition {
            let normal = start - vec2(0.0, track.height() * 0.1);
            f.move_to(normal, egui::Modifiers::NONE);
            near(f.value("target", "value"), 3.2);
            (normal - vec2(0.0, track.height() * 0.25), 4.0)
        } else {
            // Full pointer travel produces 10% normalized travel in Ctrl mode.
            (start - vec2(0.0, track.height()), 3.2)
        };
        f.move_to(point, ctrl());
        near(f.value("target", "value"), expected);
        f.release(point, ctrl());
        near(f.value("target", "value"), expected);
        assert_eq!(f.undo_steps(), baseline + 1);
        f.session.execute("undo", json!({})).unwrap();
        assert_eq!(f.value("target", "value"), 0.0);
    }
}

#[test]
fn scalar_rotary_fine_to_normal_preserves_value_on_stationary_transition_and_mouseup() {
    let command = egui::Modifiers {
        command: true,
        mac_cmd: true,
        ..Default::default()
    };
    for modifiers in [ctrl(), command] {
        for zoom in [1.0, 2.0] {
            for finish in ["mouseup", "stationary_release", "normal_movement"] {
                let mut f = FaderFixture::new("constant", "value", 0.0, zoom);
                let track = f.track("target");
                let start = track.center();
                let fine_point = start - vec2(0.0, track.height() * 0.25);
                let baseline = f.undo_steps();
                f.time = 1.0;
                f.press(start, modifiers);
                f.move_to(fine_point, modifiers)
                    .expect("fine drag emits a parameter edit");
                near(f.value("target", "value"), 0.8);
                let fine_value = f.value("target", "value");
                let revision = f.session.revision();
                let mut end = fine_point;
                let mut expected = fine_value;
                if finish != "mouseup" {
                    // Releasing Ctrl/Cmd with the primary pointer still held
                    // must not reinterpret its distant absolute track position.
                    assert!(f.move_to(fine_point, egui::Modifiers::NONE).is_none());
                    assert_eq!(f.value("target", "value"), fine_value);
                    assert_eq!(f.session.revision(), revision);
                    if finish == "normal_movement" {
                        // Normal motion resumes at full speed from the fine
                        // value, retaining the gesture's pointer/value offset.
                        end = fine_point - vec2(0.0, track.height() * 0.125);
                        let args = f
                            .move_to(end, egui::Modifiers::NONE)
                            .expect("normal motion continues the existing gesture");
                        assert_eq!(args["id"], "target");
                        expected = fine_value + 4.0;
                        near(f.value("target", "value"), expected);
                    }
                }
                let before_mouseup = f.session.revision();
                // Includes release of the fine modifier on the same mouseup,
                // with no intervening frame in the first variant.
                assert!(f.release(end, egui::Modifiers::NONE).is_none());
                near(f.value("target", "value"), expected);
                assert_eq!(f.session.revision(), before_mouseup);
                assert_eq!(f.value("other", "value"), 9.0);
                assert_eq!(f.undo_steps(), baseline + 1);
                f.session.execute("undo", json!({})).unwrap();
                assert_eq!(f.value("target", "value"), 0.0);
            }
        }
    }
}

#[test]
fn scalar_rotary_ctrl_discrete_drag_accumulates_substeps_without_fractional_edits() {
    let mut f = FaderFixture::new("octave", "octaves", 0.0, 1.0);
    let start = f.track("target").center();
    let baseline = f.undo_steps();
    f.time = 1.0;
    f.press(start, ctrl());
    let mut updates = 0;
    for pixel in 1..=120 {
        if let Some(args) = f.move_to(start - vec2(0.0, pixel as f32), ctrl()) {
            assert_eq!(args["parameter"], "octaves");
            assert_eq!(args["value"].as_f64().unwrap().fract(), 0.0);
            updates += 1;
        }
        if pixel < 30 {
            assert_eq!(f.value("target", "octaves"), 0.0);
        }
    }
    f.release(start - vec2(0.0, 120.0), ctrl());
    assert!(
        updates > 0,
        "small fine motions must accumulate past a discrete step"
    );
    assert_eq!(f.value("target", "octaves"), 1.0);
    assert_eq!(f.undo_steps(), baseline + 1);
    assert_eq!(f.value("other", "value"), 9.0);
}

#[test]
fn scalar_rotary_repeated_arrows_retain_focus_and_home_end_select_limits() {
    let mut f = FaderFixture::new("constant", "value", 4.0, 1.0);
    let track = f.track("target");
    let thumb = Rect::from_center_size(track.center(), Vec2::splat(1.0));
    f.time = 1.0;
    f.click(thumb.center());
    let focus = f.ctx.memory(|m| m.focused()).expect("fader takes focus");
    for (key, repeat, expected) in [
        (egui::Key::ArrowUp, false, 4.32),
        (egui::Key::ArrowUp, true, 4.64),
        (egui::Key::ArrowDown, false, 4.32),
        (egui::Key::Home, false, -16.0),
        (egui::Key::End, false, 16.0),
    ] {
        let args = f
            .draw(
                vec![key_event(key, true, repeat, egui::Modifiers::NONE)],
                true,
            )
            .expect("focused fader key emits edit");
        assert_eq!(args["parameter"], "value");
        near(f.value("target", "value"), expected);
        assert_eq!(f.ctx.memory(|m| m.focused()), Some(focus));
        f.draw(
            vec![key_event(key, false, false, egui::Modifiers::NONE)],
            true,
        );
    }
    // A focused slider is not a push button. Space/Enter must not synthesize
    // an absolute click using an unrelated pointer below/outside the track.
    let before = f.value("target", "value");
    let revision = f.session.revision();
    for key in [egui::Key::Space, egui::Key::Enter] {
        assert!(f
            .draw(
                vec![
                    egui::Event::PointerMoved(pos2(1000.0, 650.0)),
                    key_event(key, true, false, egui::Modifiers::NONE),
                ],
                true
            )
            .is_none());
        assert_eq!(f.value("target", "value"), before);
        assert_eq!(f.session.revision(), revision);
        assert_eq!(f.ctx.memory(|m| m.focused()), Some(focus));
        f.draw(
            vec![key_event(key, false, false, egui::Modifiers::NONE)],
            true,
        );
    }
    assert_eq!(f.value("other", "value"), 9.0);
}

#[test]
fn scalar_rotary_drag_has_one_undo_and_double_click_reset_restores_pre_click_value() {
    {
        let mut f = FaderFixture::new("constant", "value", 7.0, 1.0);
        let track = f.track("target");
        let baseline = f.undo_steps();
        let start = track.center();
        f.time = 1.0;
        f.press(start, egui::Modifiers::NONE);
        for distance in [10.0, 20.0, 30.0] {
            let point = start - vec2(0.0, distance);
            f.move_to(point, egui::Modifiers::NONE);
        }
        let end = start - vec2(0.0, 30.0);
        f.release(end, egui::Modifiers::NONE);
        near(f.value("target", "value"), 13.4);
        assert_eq!(f.undo_steps(), baseline + 1);
        f.session.execute("undo", json!({})).unwrap();
        assert_eq!(f.value("target", "value"), 7.0);
        f.session.execute("redo", json!({})).unwrap();
        near(f.value("target", "value"), 13.4);
    }
    {
        let mut f = FaderFixture::new("constant", "value", 7.0, 1.0);
        let track = f.track("target");
        let baseline = f.undo_steps();
        let at = track.center();
        f.time = 1.0;
        f.click(at);
        near(f.value("target", "value"), 7.0);
        // Both clicks go through Session, so the test catches a second history
        // group that would undo to the first click rather than the original value 7.
        f.click(at);
        assert_eq!(f.value("target", "value"), 0.0);
        assert_eq!(f.undo_steps(), baseline + 1);
        f.session.execute("undo", json!({})).unwrap();
        assert_eq!(f.value("target", "value"), 7.0);
    }
}

#[test]
fn scalar_rotary_numeric_entry_context_menu_and_preview_keep_their_separate_authority() {
    let mut f = FaderFixture::new("constant", "value", 4.0, 1.0);
    let revision = f.session.revision();
    let history = f.undo_steps();
    f.time = 1.0;
    f.click(f.readout());
    let edit = f
        .rack
        .editing
        .as_mut()
        .expect("clicking number opens editor");
    assert_eq!((&*edit.id, &*edit.parameter), ("target", "value"));
    edit.text = "3.25".into();
    assert_eq!(f.session.revision(), revision);
    assert_eq!(f.undo_steps(), history);
    f.draw(
        vec![key_event(
            egui::Key::Enter,
            true,
            false,
            egui::Modifiers::NONE,
        )],
        true,
    )
    .expect("number editor submits real parameter edit");
    assert_eq!(f.value("target", "value"), 3.25);
    f.draw(
        vec![key_event(
            egui::Key::Enter,
            false,
            false,
            egui::Modifiers::NONE,
        )],
        true,
    );
    let at = f.track("target").center();
    f.time = 2.0;
    for pressed in [true, false] {
        f.draw(
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            true,
        );
    }
    let edit = f
        .rack
        .editing
        .as_ref()
        .expect("right-click rotary opens parameter menu");
    assert_eq!(edit.surface, ParameterEditSurface::KnobMenu);
    assert_eq!(f.value("target", "value"), 3.25);
    assert_eq!(f.undo_steps(), history + 1);

    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "constant")
        .unwrap();
    let mut preview = ModulePreview::new(module);
    let before = serde_json::to_value(&preview.patch).unwrap();
    let ctx = egui::Context::default();
    let outside = egui::Id::new("preview-outside-focus");
    for (time, events) in [
        (0.0, vec![]),
        (
            1.0,
            vec![
                egui::Event::PointerMoved(pos2(90.0, 160.0)),
                button(pos2(90.0, 160.0), true),
            ],
        ),
        (1.1, vec![egui::Event::PointerMoved(pos2(90.0, 100.0))]),
        (
            1.2,
            vec![
                button(pos2(90.0, 100.0), false),
                key_event(egui::Key::End, true, false, egui::Modifiers::NONE),
            ],
        ),
    ] {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 700.0))),
                focused: true,
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = ui.interact(
                        Rect::from_min_size(pos2(900.0, 10.0), vec2(80.0, 30.0)),
                        outside,
                        Sense::click(),
                    );
                    ctx.memory_mut(|m| m.request_focus(outside));
                    preview.show(
                        ui,
                        Rect::from_min_size(
                            pos2(12.0, 12.0),
                            vec2(panel_width("constant"), PANEL_HEIGHT),
                        ),
                        &catalog,
                    );
                    assert_eq!(ctx.memory(|m| m.focused()), Some(outside));
                });
            },
        );
    }
    assert_eq!(serde_json::to_value(&preview.patch).unwrap(), before);
    assert!(preview.renderer.editing.is_none());
    assert!(preview.renderer.selection.is_empty());
    assert_eq!(preview.renderer.gesture, 0);
}

#[test]
fn scalar_rotary_disabled_modal_and_removed_target_never_redirect_gestures() {
    for guard in ["disabled", "modal", "removed"] {
        let mut f = FaderFixture::new("constant", "value", 4.0, 1.0);
        let track = f.track("target");
        let other = f.track("other");
        f.time = 1.0;
        let original = f.session.revision();
        if guard == "modal" {
            f.click(f.readout());
            assert!(f.rack.editing.is_some());
        } else {
            f.press(track.center(), egui::Modifiers::NONE);
        }
        if guard == "removed" {
            f.session
                .execute("remove_module", json!({"id":"target"}))
                .unwrap();
        }
        let enabled = guard != "disabled";
        let destination = other.center();
        let mut events = vec![egui::Event::PointerMoved(destination)];
        if guard == "modal" {
            events.push(pointer(destination, true, egui::Modifiers::NONE));
        }
        let action = f.draw(events, enabled);
        assert!(
            action.is_none(),
            "{guard} gesture must not change another control"
        );
        let action = f.draw(
            vec![pointer(other.center(), false, egui::Modifiers::NONE)],
            enabled,
        );
        assert!(
            action.is_none(),
            "{guard} release must not become a new edit"
        );
        assert_eq!(f.value("other", "value"), 9.0);
        assert_eq!(
            f.session.revision(),
            original + u64::from(guard == "removed")
        );
    }
}

#[test]
fn signal_atlas_voice_controls_support_successive_drag_and_one_undo() {
    for (kind, parameter, initial, local) in [
        ("attenuverter8", "gain1", 0.0, pos2(88.0, 107.0)),
        ("attenuverter8", "gain8", 0.0, pos2(88.0, 352.0)),
        ("cv_mix", "level1", 0.0, pos2(88.0, 132.0)),
        ("cv_mix", "level3", 0.0, pos2(88.0, 272.0)),
        ("oscillator", "frequency", 220.0, pos2(60.0, 117.0)),
        ("oscillator", "amplitude", 0.2, pos2(60.0, 225.0)),
        ("constant", "value", 0.0, pos2(60.0, 128.0)),
        ("octave", "octaves", 0.0, pos2(90.0, 118.0)),
        ("sample_hold", "initial", 0.0, pos2(75.0, 135.0)),
        ("fade", "position", 0.5, pos2(65.0, 121.0)),
        ("fade", "cv_amount", 0.0, pos2(175.0, 121.0)),
        ("compare", "b_offset", 0.0, pos2(120.0, 125.0)),
        ("process", "slew", -4.0, pos2(120.0, 116.0)),
        ("gate", "frequency", 2.0, pos2(75.0, 125.0)),
        ("gates", "length", 0.1, pos2(78.0, 189.0)),
        ("score_sequencer", "tempo", 120.0, pos2(65.0, 120.0)),
        ("score_sequencer", "swing", 50.0, pos2(185.0, 120.0)),
        ("white_noise", "level", 1.0, pos2(90.0, 135.0)),
        ("random", "rate", 1.0, pos2(45.0, 127.0)),
        ("random", "probability", 0.5, pos2(135.0, 127.0)),
        ("random", "spread", 0.5, pos2(225.0, 127.0)),
        ("random", "shape", 0.5, pos2(315.0, 127.0)),
        ("random", "rate_cv", 0.0, pos2(45.0, 234.0)),
        ("random", "probability_cv", 0.0, pos2(135.0, 234.0)),
        ("random", "spread_cv", 0.0, pos2(225.0, 234.0)),
        ("random", "shape_cv", 0.0, pos2(315.0, 234.0)),
        ("stereo_delay", "time_left", 0.234375, pos2(42.0, 111.0)),
        ("stereo_delay", "wet", 0.4, pos2(294.0, 253.0)),
        ("stereo_reverb", "size", 0.6, pos2(180.0, 112.0)),
        ("stereo_delay", "time_right", 0.46875, pos2(126.0, 111.0)),
        ("stereo_delay", "feedback", 0.15, pos2(210.0, 111.0)),
        ("stereo_delay", "crossfeed", 0.5, pos2(294.0, 111.0)),
        ("stereo_delay", "mod_rate", 4.0, pos2(378.0, 111.0)),
        ("stereo_delay", "low_cut", 300.0, pos2(42.0, 253.0)),
        ("stereo_delay", "high_cut", 8000.0, pos2(126.0, 253.0)),
        ("stereo_delay", "dry", 0.6, pos2(210.0, 253.0)),
        ("stereo_delay", "mod_depth", 0.2, pos2(378.0, 253.0)),
        ("stereo_reverb", "predelay", 0.01, pos2(60.0, 112.0)),
        ("stereo_reverb", "decay", 0.65, pos2(300.0, 112.0)),
        ("stereo_reverb", "damping", 6000.0, pos2(60.0, 263.0)),
        ("stereo_reverb", "dry", 1.0, pos2(180.0, 263.0)),
        ("stereo_reverb", "wet", 0.35, pos2(300.0, 263.0)),
        ("sidechain_compressor", "makeup_db", 0.0, pos2(42.0, 297.0)),
        ("drum_voice", "pitch", 48.0, pos2(180.0, 112.0)),
        ("macro_voice", "frequency", 261.62558, pos2(50.0, 143.0)),
        (
            "sidechain_compressor",
            "threshold_db",
            -10.5,
            pos2(70.0, 103.0),
        ),
        ("sidechain_compressor", "ratio", 4.0, pos2(210.0, 103.0)),
        ("sidechain_compressor", "knee_db", 6.0, pos2(350.0, 103.0)),
        ("sidechain_compressor", "attack_ms", 2.0, pos2(70.0, 210.0)),
        (
            "sidechain_compressor",
            "release_ms",
            200.0,
            pos2(210.0, 210.0),
        ),
        (
            "sidechain_compressor",
            "detector_ms",
            10.0,
            pos2(350.0, 210.0),
        ),
        ("drum_voice", "decay", 0.2, pos2(300.0, 112.0)),
        ("drum_voice", "tone", 0.5, pos2(60.0, 253.0)),
        ("drum_voice", "drive", 0.2, pos2(180.0, 253.0)),
        ("drum_voice", "level", 1.0, pos2(300.0, 253.0)),
        ("macro_voice", "harmonics", 0.3, pos2(150.0, 143.0)),
        ("macro_voice", "timbre", 0.45, pos2(250.0, 143.0)),
        ("macro_voice", "morph", 0.4, pos2(50.0, 245.0)),
        ("macro_voice", "decay", 0.5, pos2(150.0, 245.0)),
        ("macro_voice", "level", 0.8, pos2(250.0, 245.0)),
        ("modal_resonator", "frequency", 220.0, pos2(44.0, 139.0)),
        ("seq3", "tempo", 120.0, pos2(62.0, 118.0)),
        ("seq3", "tempo_amount", 0.5, pos2(62.0, 214.0)),
        ("seq3", "steps_amount", 0.5, pos2(153.0, 214.0)),
        ("stock_seq3", "tempo", 1.0, pos2(62.0, 118.0)),
        ("stock_seq3", "tempo_amount", 0.5, pos2(62.0, 214.0)),
        ("stock_seq3", "steps_amount", 0.5, pos2(153.0, 214.0)),
        ("seq3", "row1_1", 0.0, pos2(238.0, 112.0)),
        ("seq3", "row3_8", 0.0, pos2(707.0, 256.0)),
        ("stock_seq3", "row1_1", 0.0, pos2(238.0, 112.0)),
        ("stock_seq3", "row3_8", 0.0, pos2(707.0, 256.0)),
        ("crossfade", "mix", 0.5, pos2(90.0, 125.0)),
        ("attenuverter", "gain", 0.0, pos2(45.0, 132.0)),
        ("attenuverter", "offset", 0.0, pos2(135.0, 132.0)),
        ("rescale", "gain", 0.0, pos2(60.0, 127.0)),
        ("rescale", "offset", 0.0, pos2(180.0, 127.0)),
        ("rescale", "min", -10.0, pos2(60.0, 243.0)),
        ("rescale", "max", 5.0, pos2(180.0, 243.0)),
        ("quantizer", "offset", 0.0, pos2(60.0, 263.0)),
        ("stock_quantizer", "offset", 0.0, pos2(70.0, 307.0)),
        ("poly_sum", "level", 0.5, pos2(75.0, 145.0)),
        ("poly_constant", "v1", -4.0, pos2(50.0, 117.0)),
        ("poly_constant", "v16", -7.25, pos2(310.0, 318.0)),
        ("mid_side", "encode_width", 1.0, pos2(75.0, 133.0)),
        ("stock_mid_side", "encode_width", 1.0, pos2(75.0, 133.0)),
        ("mid_side", "decode_width", 1.0, pos2(225.0, 133.0)),
        ("stock_mid_side", "decode_width", 1.0, pos2(225.0, 133.0)),
        ("mix6", "level", 0.5, pos2(120.0, 125.0)),
        ("stock_mix", "level", 0.5, pos2(120.0, 125.0)),
        ("lowpass", "cutoff", 1000.0, pos2(90.0, 99.0)),
        ("gain", "gain", 1.0, pos2(75.0, 99.0)),
        ("analog_vca", "gain", 1.0, pos2(75.0, 99.0)),
        ("analog_vco", "frequency", 261.62558, pos2(65.5, 93.5)),
        ("analog_vco", "pulse_width", 0.5, pos2(163.0, 198.0)),
        ("adsr", "attack", 0.01, pos2(53.714287, 91.3)),
        ("adsr", "decay", 0.2, pos2(186.28572, 91.3)),
        ("adsr", "sustain", 0.5, pos2(53.714287, 198.0)),
        ("adsr", "release", 0.3, pos2(186.28572, 198.0)),
        ("ladder_vcf", "cutoff", 1200.0, pos2(65.5, 93.5)),
        ("ladder_vcf", "resonance", 0.3, pos2(47.0, 198.0)),
        ("ladder_vcf", "drive", 1.0, pos2(163.0, 198.0)),
        ("adsr_eg", "attack", 0.5, pos2(58.0, 173.0)),
        ("lfo", "frequency", 2.0, pos2(56.142857, 93.5)),
        ("lfo", "amplitude", 0.5, pos2(40.285713, 198.0)),
        ("lfo", "offset", 0.5, pos2(139.71428, 198.0)),
        ("multi_vco", "frequency", 261.62558, pos2(75.0, 125.0)),
        ("multi_vco", "pulse_width", 0.5, pos2(225.0, 125.0)),
        ("multi_vco", "fm_amount", 0.0, pos2(75.0, 225.0)),
        ("multi_vco", "pwm_amount", 0.0, pos2(225.0, 225.0)),
        ("multi_lfo", "frequency", 2.0, pos2(75.0, 125.0)),
        ("multi_lfo", "pulse_width", 0.5, pos2(225.0, 125.0)),
        ("multi_lfo", "fm_amount", 0.0, pos2(75.0, 225.0)),
        ("multi_lfo", "pwm_amount", 0.0, pos2(225.0, 225.0)),
        ("wavetable_vco", "tune", 0.0, pos2(60.0, 194.0)),
        ("wavetable_vco", "position", 0.0, pos2(180.0, 194.0)),
        ("wavetable_vco", "fm_amount", 0.0, pos2(300.0, 194.0)),
        (
            "wavetable_vco",
            "position_cv_amount",
            0.0,
            pos2(60.0, 288.0),
        ),
        ("wavetable_lfo", "rate", 0.0, pos2(60.0, 194.0)),
        ("wavetable_lfo", "position", 0.0, pos2(180.0, 194.0)),
        ("wavetable_lfo", "fm_amount", 0.0, pos2(300.0, 194.0)),
        (
            "wavetable_lfo",
            "position_cv_amount",
            0.0,
            pos2(60.0, 288.0),
        ),
        ("svf_filter", "cutoff", 1000.0, pos2(67.5, 139.0)),
        ("svf_filter", "resonance", 0.2, pos2(45.0, 255.0)),
        ("svf_filter", "drive", 0.0, pos2(135.0, 255.0)),
        ("svf_filter", "cutoff_cv_amount", 0.0, pos2(225.0, 255.0)),
        ("modal_resonator", "frequency", 220.0, pos2(44.0, 139.0)),
        ("modal_resonator", "stiffness", 0.2, pos2(114.0, 139.0)),
        ("modal_resonator", "decay", 2.0, pos2(184.0, 139.0)),
        ("modal_resonator", "damping", 0.35, pos2(254.0, 139.0)),
        (
            "modal_resonator",
            "strike_position",
            0.23,
            pos2(44.0, 254.0),
        ),
        (
            "modal_resonator",
            "pickup_position",
            0.61,
            pos2(114.0, 254.0),
        ),
        (
            "modal_resonator",
            "strike_strength",
            1.0,
            pos2(184.0, 254.0),
        ),
        ("modal_resonator", "level", 1.0, pos2(254.0, 254.0)),
    ] {
        let mut f = FaderFixture::new(kind, parameter, initial, 1.0);
        let center = f.rack.viewport.unwrap().min
            + f.rack.pan
            + f.rack.positions["target"].to_vec2()
            + local.to_vec2();
        let baseline = f.undo_steps();
        f.time = 1.0;
        assert!(
            f.press(center, egui::Modifiers::NONE).is_none(),
            "{kind}: no click jump"
        );
        let travel = if kind == "octave" { 30.0 } else { 10.0 };
        let first = center - vec2(0.0, travel);
        let edit = f.move_to(first, egui::Modifiers::NONE).expect(kind);
        assert_eq!(edit["parameter"], parameter);
        let after_first = f.value("target", parameter);
        assert!(after_first > initial, "{kind}: first relative move");
        let second = center - vec2(0.0, travel * 2.0);
        f.move_to(second, egui::Modifiers::NONE).expect(kind);
        let after_second = f.value("target", parameter);
        assert!(
            after_second > after_first,
            "{kind}: current revision on second move"
        );
        f.release(second, egui::Modifiers::NONE);
        assert_eq!(
            f.undo_steps(),
            baseline + 1,
            "{kind}: one gesture, one undo"
        );
        f.session.execute("undo", json!({})).unwrap();
        near(f.value("target", parameter), initial);
        f.session.execute("redo", json!({})).unwrap();
        near(f.value("target", parameter), after_second);
    }
}

#[test]
fn approved_voice_numeric_fields_commit_physical_units_and_undo() {
    for (kind, parameter, initial, local, text, expected) in [
        (
            "attenuverter8",
            "gain8",
            0.5,
            pos2(138.0, 352.0),
            "-3.5",
            -0.35,
        ),
        ("cv_mix", "level3", 0.5, pos2(162.0, 272.0), "2.5", 0.25),
        (
            "oscillator",
            "frequency",
            220.0,
            pos2(60.0, 164.0),
            "440",
            440.0,
        ),
        (
            "oscillator",
            "amplitude",
            0.2,
            pos2(60.0, 272.0),
            "0.4",
            0.4,
        ),
        ("constant", "value", 0.0, pos2(60.0, 176.0), "-3.25", -3.25),
        ("octave", "octaves", 0.0, pos2(90.0, 166.0), "-2", -2.0),
        (
            "svf_filter",
            "resonance",
            0.2,
            pos2(45.0, 292.0),
            "75",
            0.75,
        ),
        ("svf_filter", "drive", 0.0, pos2(135.0, 292.0), "25", 0.25),
        (
            "svf_filter",
            "cutoff_cv_amount",
            0.0,
            pos2(225.0, 292.0),
            "-2",
            -2.0,
        ),
        ("lfo", "frequency", 2.0, pos2(56.142857, 151.15), "0", 0.0),
        (
            "lfo",
            "amplitude",
            0.5,
            pos2(40.285713, 249.65),
            "0.001",
            0.001,
        ),
        (
            "lfo",
            "offset",
            0.5,
            pos2(139.71428, 249.65),
            "-0.25",
            -0.25,
        ),
        ("lowpass", "cutoff", 1000.0, pos2(90.0, 163.15), "80", 80.0),
        ("gain", "gain", 1.0, pos2(75.0, 163.15), "-4", -4.0),
        (
            "analog_vco",
            "frequency",
            261.62558,
            pos2(65.5, 151.15),
            "440",
            440.0,
        ),
        (
            "analog_vco",
            "pulse_width",
            0.5,
            pos2(163.0, 249.65),
            "25",
            0.25,
        ),
        ("analog_vco", "waveform", 0.0, pos2(47.0, 249.65), "1", 1.0),
        ("analog_vca", "gain", 1.0, pos2(75.0, 163.15), "1.5", 1.5),
        (
            "adsr",
            "attack",
            0.01,
            pos2(53.714287, 145.2),
            "0.075",
            0.075,
        ),
        ("adsr", "decay", 0.2, pos2(186.28572, 145.2), "0.8", 0.8),
        ("adsr", "sustain", 0.5, pos2(53.714287, 251.9), "75", 0.75),
        ("adsr", "release", 0.3, pos2(186.28572, 251.9), "1.2", 1.2),
        (
            "ladder_vcf",
            "cutoff",
            1200.0,
            pos2(65.5, 151.15),
            "2500",
            2500.0,
        ),
        (
            "ladder_vcf",
            "resonance",
            0.3,
            pos2(47.0, 249.65),
            "80",
            0.8,
        ),
        ("ladder_vcf", "drive", 1.0, pos2(163.0, 249.65), "2", 2.0),
    ] {
        let mut f = FaderFixture::new(kind, parameter, initial, 1.0);
        let target = f.rack.viewport.unwrap().min
            + f.rack.pan
            + f.rack.positions["target"].to_vec2()
            + local.to_vec2();
        let baseline = f.undo_steps();
        f.time = 1.0;
        f.draw(vec![egui::Event::PointerMoved(target)], true);
        f.click(target);
        let edit = f
            .rack
            .editing
            .as_mut()
            .expect("approved numeric field opens");
        assert_eq!(edit.parameter, parameter, "{kind}");
        edit.text = text.into();
        f.draw(
            vec![key_event(
                egui::Key::Enter,
                true,
                false,
                egui::Modifiers::NONE,
            )],
            true,
        )
        .expect("typed setting commits through Session");
        near(f.value("target", parameter), expected);
        assert_eq!(f.undo_steps(), baseline + 1);
        f.session.execute("undo", json!({})).unwrap();
        near(f.value("target", parameter), initial);
    }
}

#[test]
fn detailed_lfo_zero_hz_hold_remains_keyboard_editable() {
    let mut f = FaderFixture::new("lfo", "frequency", 2.0, 1.0);
    let local = pos2(56.142857, 93.5);
    let center = f.rack.viewport.unwrap().min
        + f.rack.pan
        + f.rack.positions["target"].to_vec2()
        + local.to_vec2();
    f.time = 1.0;
    f.click(center);
    f.draw(
        vec![key_event(
            egui::Key::Home,
            true,
            false,
            egui::Modifiers::NONE,
        )],
        true,
    )
    .expect("Home sets a true zero-Hz hold");
    assert_eq!(f.value("target", "frequency"), 0.0);
    f.draw(
        vec![key_event(
            egui::Key::Home,
            false,
            false,
            egui::Modifiers::NONE,
        )],
        true,
    );
    f.draw(
        vec![key_event(
            egui::Key::ArrowUp,
            true,
            false,
            egui::Modifiers::NONE,
        )],
        true,
    )
    .expect("a held LFO can restart with an arrow key");
    let hz = f.value("target", "frequency");
    assert!(hz.is_finite() && hz > 0.0 && hz < 0.1);
    f.session.execute("undo", json!({})).unwrap();
    assert_eq!(f.value("target", "frequency"), 0.0);
}

fn large_bubble_texts(output: &egui::FullOutput) -> Vec<(String, Rect, f32)> {
    fn visit(shape: &egui::Shape, out: &mut Vec<(String, Rect, f32)>) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, out)
                }
            }
            egui::Shape::Text(t) => {
                let size = t
                    .galley
                    .job
                    .sections
                    .iter()
                    .map(|s| s.format.font_id.size)
                    .fold(0.0, f32::max);
                if size >= 22.0 {
                    out.push((
                        t.galley.text().into(),
                        Rect::from_min_size(t.pos, t.galley.size()),
                        size,
                    ));
                }
            }
            _ => {}
        }
    }
    let mut texts = vec![];
    for s in &output.shapes {
        visit(&s.shape, &mut texts)
    }
    texts
}
#[test]
fn knob_value_bubble_uses_human_units_above_the_control_without_stealing_drag_or_undo() {
    for (kind, parameter, initial, local, unit) in [
        ("gain", "gain", 1.0, pos2(75.0, 99.0), "×"),
        ("stock_vca", "level", 0.5, pos2(90.0, 106.0), "%"),
        (
            "stock_delay",
            "time",
            stock_delay_default(0),
            pos2(46.0, 120.0),
            "ms",
        ),
        ("stock_vca_mix", "mix_level", 1.0, pos2(235.0, 154.0), "×"),
        (
            "macro_voice",
            "frequency",
            261.62558,
            pos2(50.0, 143.0),
            "Hz",
        ),
        (
            "modal_resonator",
            "frequency",
            220.0,
            pos2(44.0, 139.0),
            "Hz",
        ),
    ] {
        let mut f = FaderFixture::new(kind, parameter, initial, 1.0);
        assert!(f.rack.knob_value_bubble.is_none());
        let center = f.rack.viewport.unwrap().min
            + f.rack.pan
            + f.rack.positions["target"].to_vec2()
            + local.to_vec2();
        let undo = f.undo_steps();
        f.press(center, egui::Modifiers::NONE);
        assert!(
            f.rack.knob_value_bubble.is_none(),
            "focus alone must not show a value change"
        );
        f.move_to(center - vec2(0.0, 10.0), egui::Modifiers::NONE)
            .unwrap();
        f.time += 0.12;
        let (action, output) = frame_revision_state(
            &f.ctx,
            &mut f.rack,
            f.session.patch(),
            f.time,
            vec![],
            true,
            true,
            f.session.revision(),
        );
        assert!(action.is_none());
        f.time += 0.05;
        let bubble = f.rack.knob_value_bubble.as_ref().unwrap();
        assert!(bubble.text.contains(unit), "{kind}: {}", bubble.text);
        let text = large_bubble_texts(&output)
            .into_iter()
            .find(|(s, _, _)| s == &bubble.text)
            .expect(kind);
        assert!(
            text.1.bottom() < bubble.anchor.top(),
            "{kind}: bubble above knob"
        );
        let first = f.value("target", parameter);
        // Continue the held drag through the floating overlay's region.
        f.move_to(center - vec2(0.0, 70.0), egui::Modifiers::NONE)
            .unwrap();
        assert!(f.value("target", parameter) > first);
        f.release(center - vec2(0.0, 70.0), egui::Modifiers::NONE);
        assert_eq!(f.undo_steps(), undo + 1, "one drag still has one undo");
        let last = f.rack.knob_value_bubble.as_ref().unwrap().last_active;
        f.time = last + 0.58;
        f.draw(vec![], true);
        assert!(f.rack.knob_value_bubble.is_some(), "fading after release");
        f.time = last + 0.80;
        f.draw(vec![], true);
        assert!(f.rack.knob_value_bubble.is_none(), "does not stick around");
        f.session.execute("undo", json!({})).unwrap();
        near(f.value("target", parameter), initial);
    }
}
#[test]
fn knob_value_bubble_supports_keyboard_and_clears_for_disabled_or_deleted_owner() {
    let mut f = FaderFixture::new("stock_vca", "level", 0.5, 1.0);
    let at = f.rack.viewport.unwrap().min
        + f.rack.pan
        + f.rack.positions["target"].to_vec2()
        + vec2(90.0, 106.0);
    f.press(at, egui::Modifiers::NONE);
    f.release(at, egui::Modifiers::NONE);
    f.draw(
        vec![egui::Event::Key {
            key: egui::Key::ArrowUp,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        true,
    )
    .unwrap();
    f.draw(vec![], true);
    assert_eq!(f.rack.knob_value_bubble.as_ref().unwrap().text, "51%");
    f.draw(vec![], false);
    assert!(f.rack.knob_value_bubble.is_none());
    let mut f = FaderFixture::new("stock_vca", "level", 0.5, 1.0);
    f.press(at, egui::Modifiers::NONE);
    f.release(at, egui::Modifiers::NONE);
    f.draw(
        vec![egui::Event::Key {
            key: egui::Key::ArrowUp,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        true,
    )
    .unwrap();
    let mut deleted = f.session.patch().clone();
    deleted.nodes.retain(|n| n.id != "target");
    frame_revision_state(
        &f.ctx,
        &mut f.rack,
        &deleted,
        f.time + 0.1,
        vec![],
        true,
        true,
        f.session.revision(),
    );
    assert!(f.rack.knob_value_bubble.is_none());
}
#[test]
fn knob_value_bubble_stays_readable_at_small_zoom_and_inside_canvas_edges() {
    for (zoom, pan) in [(0.35, vec2(24.0, 12.0)), (1.0, vec2(-60.0, -70.0))] {
        let mut f = FaderFixture::new("gain", "gain", 1.0, zoom);
        f.rack.pan = pan;
        f.draw(vec![], true);
        let at = f.rack.viewport.unwrap().min
            + pan
            + (f.rack.positions["target"].to_vec2() + vec2(75.0, 99.0)) * zoom;
        f.press(at, egui::Modifiers::NONE);
        f.move_to(at - vec2(0.0, 8.0), egui::Modifiers::NONE)
            .unwrap();
        let (_, output) = frame_revision_state(
            &f.ctx,
            &mut f.rack,
            f.session.patch(),
            f.time + 0.12,
            vec![],
            true,
            true,
            f.session.revision(),
        );
        let b = f.rack.knob_value_bubble.as_ref().unwrap();
        let text = large_bubble_texts(&output)
            .into_iter()
            .find(|(t, _, _)| t == &b.text)
            .unwrap();
        assert!(f.ctx.content_rect().shrink(5.0).contains_rect(text.1));
    }
}
