use super::*;
fn gate_host(path: &Path) -> OpenPatch {
    let mut app = host(path);
    assert!(app.command(
        "replace_patch",
        json!({"patch":{"version":2,"nodes":[{"id":"gate","kind":"midi_gate"}]}})
    ));
    app
}
fn labels(output: &egui::FullOutput) -> Vec<String> {
    fn visit(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().into()),
            egui::Shape::Vec(children) => {
                for child in children {
                    visit(child, out);
                }
            }
            _ => {}
        }
    }
    let mut out = vec![];
    for shape in &output.shapes {
        visit(&shape.shape, &mut out);
    }
    out
}
fn panel_labels(app: &mut OpenPatch) -> Vec<String> {
    let ctx = egui::Context::default();
    labels(&ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            focused: true,
            ..Default::default()
        },
        |ctx| app.rack_workspace(ctx, None),
    ))
}
#[test]
fn midi_gate_app_actual_offline_learning_snapshot_is_preserved_only_for_smoke() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = gate_host(dir.path());
    app.rack.set_midi_gate_states(BTreeMap::from([("gate".into(),json!({"assignments":[60,49,50,51,44,45,46,47,40,41,42,43,36,37,38,39],"learning":{"cell":16,"serial":23}}))]));
    app.smoke_test = Some((dir.path().join("unused.png"), 0, 0));
    app.sync_midi_gate_states();
    let before = panel_labels(&mut app);
    assert!(before.iter().any(|s| s == "60 C4"));
    assert!(before.iter().any(|s| s == "LRN"));
    app.smoke_test = None;
    app.sync_midi_gate_states();
    let after = panel_labels(&mut app);
    assert!(!after.iter().any(|s| s == "LRN"));
    assert!(after.iter().any(|s| s == "48 C3"));
}
#[test]
fn midi_gate_app_keyboard_reaches_default_notes_and_note_entry_keeps_focus() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = gate_host(dir.path());
    app.keyboard_open = true;
    let before = app.shared.lock().unwrap().snapshot();
    let ctx = egui::Context::default();
    let mut events = vec![];
    let raw = |time, events| egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 800.0),
        )),
        time: Some(time),
        focused: true,
        events,
        ..Default::default()
    };
    let k = |key, pressed| performance_key(key, pressed, egui::Modifiers::NONE);
    let _ = ctx.run(
        raw(
            0.0,
            vec![
                k(egui::Key::Backtick, true),
                k(egui::Key::Backtick, false),
                k(egui::Key::Backtick, true),
                k(egui::Key::Backtick, false),
                k(egui::Key::Z, true),
            ],
        ),
        |ctx| {
            assert!(app.performance_keyboard_enabled(ctx, true));
            events = app.update_performance_keyboard(ctx, true);
        },
    );
    assert_eq!(
        events,
        vec![MidiEvent::NoteOn {
            channel: 0,
            note: 36,
            velocity: 127
        }],
        "two octave-down taps reach the default Gate bank"
    );
    app.rack
        .request_precision_control_for_smoke("gate", "note1");
    let _ = ctx.run(raw(1.0, vec![]), |ctx| app.rack_workspace(ctx, None));
    let _ = ctx.run(raw(1.1, vec![]), |ctx| {
        app.rack_workspace(ctx, None);
        assert!(ctx.wants_keyboard_input());
        assert!(!app.performance_keyboard_enabled(ctx, true));
        events = app.update_performance_keyboard(ctx, true);
    });
    assert_eq!(
        events,
        vec![MidiEvent::NoteOff {
            channel: 0,
            note: 36,
            velocity: 127
        }]
    );
    let _ = ctx.run(
        raw(
            1.2,
            vec![k(egui::Key::X, true), egui::Event::Text("72".into())],
        ),
        |ctx| {
            app.rack_workspace(ctx, None);
            assert!(ctx.wants_keyboard_input());
            assert!(app.update_performance_keyboard(ctx, true).is_empty());
        },
    );
    assert!(app.rack.precision_is_open_for_smoke());
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
}
#[test]
fn midi_gate_app_panic_while_bypassed_is_single_transient_module_action() {
    use openpatch::control::LiveModuleEvents;
    use std::sync::{Arc, Mutex};
    struct Backend(Mutex<Vec<openpatch_core::ModuleAction>>);
    impl LiveModuleEvents for Backend {
        fn trigger(&self, id: &str, action: openpatch_core::ModuleAction) -> Result<Value, String> {
            assert_eq!(id, "gate");
            self.0.lock().unwrap().push(action);
            Ok(json!({"queued":true}))
        }
        fn snapshot(&self) -> Value {
            json!({})
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut app = gate_host(dir.path());
    assert!(app.command("set_bypass", json!({"id":"gate","bypassed":true})));
    let backend = Arc::new(Backend(Mutex::new(vec![])));
    app.shared
        .lock()
        .unwrap()
        .set_live_module_events(Some(backend.clone()))
        .unwrap();
    let before = app.shared.lock().unwrap().snapshot();
    assert!(app.command("trigger_module", json!({"id":"gate","action":"midi_panic"})));
    assert_eq!(
        *backend.0.lock().unwrap(),
        vec![openpatch_core::ModuleAction::MidiPanic]
    );
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
}
#[test]
fn midi_gate_app_settings_label_the_endpoint_as_global() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = gate_host(dir.path());
    app.settings_open = true;
    let ctx = egui::Context::default();
    let mut rendered = vec![];
    for _ in 0..2 {
        rendered = labels(&ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                ..Default::default()
            },
            |ctx| app.audio_settings(ctx),
        ));
    }
    assert!(rendered.iter().any(|s| s == "Global MIDI input"));
    assert!(rendered.iter().any(|s| s.contains("on-screen keyboard")));
}
