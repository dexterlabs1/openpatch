//! End-to-end UI input → MIDI → CV qualification, without an audio device.
use crate::keyboard::Keyboard;
use eframe::egui::{self, Key};
use openpatch_core::{Engine, Patch, TapSample};
use serde_json::json;

fn key(key: Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers: Default::default(),
    }
}

struct Performance {
    ctx: egui::Context,
    keyboard: Keyboard,
    engine: Engine,
}
impl Performance {
    fn new() -> Self {
        let patch: Patch = serde_json::from_value(json!({
            "version": 2,
            "nodes": [{"id":"midi", "kind":"midi_cv", "params":{"channels":1}}],
            "output":"midi", "output_port":"gate"
        }))
        .unwrap();
        Self {
            ctx: egui::Context::default(),
            keyboard: Keyboard::default(),
            engine: patch.compile().unwrap(),
        }
    }
    fn frame(&mut self, events: Vec<egui::Event>, enabled: bool) -> (f32, f32) {
        let _ = self.ctx.run(
            egui::RawInput {
                events,
                focused: true,
                ..Default::default()
            },
            |ctx| {
                for event in self.keyboard.update(ctx, enabled) {
                    self.engine.handle_midi(event);
                }
            },
        );
        let taps = [
            Some(self.engine.resolve_tap("midi", "pitch", 0).unwrap()),
            Some(self.engine.resolve_tap("midi", "gate", 0).unwrap()),
        ];
        let mut audio = [[0.0; 2]; 1];
        let mut values = [[TapSample::default(); 2]; 1];
        self.engine
            .process_audio_block_with_taps(&mut audio, &taps, &mut values);
        assert!(values[0][0].active && values[0][1].active);
        (values[0][0].value, values[0][1].value)
    }
}

#[test]
fn full_keyboard_range_and_cross_octave_owners_drive_correct_volts_and_balanced_gates() {
    let mut performance = Performance::new();
    for (key_code, semitones) in [
        (Key::Z, 0),
        (Key::Q, 12),
        (Key::I, 24),
        (Key::CloseBracket, 31),
    ] {
        let (pitch, gate) = performance.frame(vec![key(key_code, true)], true);
        assert!((pitch - semitones as f32 / 12.0).abs() < 1e-6);
        assert_eq!(gate, 10.0);
        assert_eq!(performance.frame(vec![key(key_code, false)], true).1, 0.0);
    }

    // Q and comma are the same pitch. Transpose while they are held, then
    // press Z: three independent sources now own MIDI72 across two bases.
    assert_eq!(
        performance.frame(vec![key(Key::Q, true)], true),
        (1.0, 10.0)
    );
    assert_eq!(
        performance.frame(vec![key(Key::Comma, true)], true),
        (1.0, 10.0)
    );
    assert_eq!(
        performance.frame(vec![key(Key::Num1, true)], true),
        (1.0, 10.0)
    );
    assert_eq!(
        performance.frame(vec![key(Key::Z, true)], true),
        (1.0, 10.0)
    );
    assert_eq!(
        performance.frame(vec![key(Key::Q, false)], true),
        (1.0, 10.0)
    );
    assert_eq!(
        performance.frame(vec![key(Key::Comma, false)], true),
        (1.0, 10.0)
    );
    assert_eq!(performance.frame(vec![key(Key::Z, false)], true).1, 0.0);

    // A text field/modal taking input releases notes. Returning to the rack
    // does not play the still-physically-held key until a new press occurs.
    assert_eq!(
        performance.frame(vec![key(Key::Q, true)], true),
        (2.0, 10.0)
    );
    assert_eq!(performance.frame(vec![], false).1, 0.0);
    assert_eq!(performance.frame(vec![], true).1, 0.0);
    assert_eq!(performance.frame(vec![key(Key::Q, false)], true).1, 0.0);
    assert_eq!(
        performance.frame(vec![key(Key::Q, true)], true),
        (2.0, 10.0)
    );
    assert_eq!(performance.frame(vec![key(Key::Q, false)], true).1, 0.0);

    // Press/release coalesced into one UI frame cannot leave an open gate.
    assert_eq!(
        performance
            .frame(vec![key(Key::E, true), key(Key::E, false)], true)
            .1,
        0.0
    );
}

#[test]
fn mouse_octave_button_keeps_held_note_and_leaves_computer_performance_available() {
    let ctx = egui::Context::default();
    let mut keyboard = Keyboard::default();
    let mut events = Vec::new();
    {
        let mut draw = |input_events: Vec<egui::Event>| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(940.0, 150.0),
                    )),
                    focused: true,
                    events: input_events,
                    ..Default::default()
                },
                |ctx| {
                    events.clear();
                    egui::CentralPanel::default().show(ctx, |ui| {
                        events.extend(keyboard.show(ui, true));
                    });
                    // Production processes physical input after the dock and editors.
                    events.extend(keyboard.update(ctx, !ctx.wants_keyboard_input()));
                },
            )
        };
        draw(vec![]);
        let rendered = draw(vec![key(Key::Q, true)]);
        let button = rendered
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape {
                    (text.galley.text() == "+ octave").then(|| text.pos + text.galley.size() * 0.5)
                } else {
                    None
                }
            })
            .expect("octave button visible");
        for pressed in [true, false] {
            draw(vec![
                egui::Event::PointerMoved(button),
                egui::Event::PointerButton {
                    pos: button,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
    }
    assert!(
        events.is_empty(),
        "octave click must not release held Q: {events:?}"
    );
    let _ = ctx.run(
        egui::RawInput {
            focused: true,
            events: vec![key(Key::Q, false), key(Key::Z, true)],
            ..Default::default()
        },
        |ctx| events = keyboard.update(ctx, !ctx.wants_keyboard_input()),
    );
    assert_eq!(
        events,
        vec![
            openpatch_core::MidiEvent::NoteOff {
                channel: 0,
                note: 72,
                velocity: 127
            },
            openpatch_core::MidiEvent::NoteOn {
                channel: 0,
                note: 72,
                velocity: 127
            },
        ]
    );
}
