//! UI-thread performance keyboard; these notes never become patch edits.
use eframe::egui::{self, Align2, Color32, FontId, Key, Rect, Sense, Stroke, StrokeKind, Vec2};
use openpatch_core::MidiEvent;

const KEYS: [Key; 13] = [
    Key::A,
    Key::W,
    Key::S,
    Key::E,
    Key::D,
    Key::F,
    Key::T,
    Key::G,
    Key::Y,
    Key::H,
    Key::U,
    Key::J,
    Key::K,
];
const LABELS: [&str; 13] = [
    "A", "W", "S", "E", "D", "F", "T", "G", "Y", "H", "U", "J", "K",
];
const WHITE: [usize; 8] = [0, 2, 4, 5, 7, 9, 11, 12];
const BLACK: [(usize, usize); 5] = [(1, 0), (3, 1), (6, 3), (8, 4), (10, 5)];

pub struct Keyboard {
    base: u8,
    held: [bool; 13],
    mouse_note: Option<u8>,
}
impl Default for Keyboard {
    fn default() -> Self {
        Self {
            base: 60,
            held: [false; 13],
            mouse_note: None,
        }
    }
}
fn note_on(note: u8) -> MidiEvent {
    MidiEvent::NoteOn {
        channel: 0,
        note,
        velocity: 100,
    }
}
fn note_off(note: u8) -> MidiEvent {
    MidiEvent::NoteOff {
        channel: 0,
        note,
        velocity: 0,
    }
}

impl Keyboard {
    pub fn release(&mut self) -> Vec<MidiEvent> {
        let mut events = Vec::new();
        for (index, held) in self.held.iter_mut().enumerate() {
            if *held {
                events.push(note_off(self.base + index as u8));
                *held = false;
            }
        }
        if let Some(note) = self.mouse_note.take() {
            events.push(note_off(note));
        }
        events
    }
    pub fn update(&mut self, ctx: &egui::Context, enabled: bool) -> Vec<MidiEvent> {
        let enabled = enabled
            && ctx.input(|input| input.focused && !input.modifiers.command && !input.modifiers.alt);
        if !enabled {
            return self.release();
        }
        let mut events = Vec::new();
        for (index, key) in KEYS.iter().enumerate() {
            let (pressed, down) = ctx.input(|input| (input.events.iter().any(|event| matches!(event, egui::Event::Key { key: pressed_key, pressed: true, repeat: false, .. } if pressed_key == key)), input.key_down(*key)));
            if self.held[index] && !down {
                self.held[index] = false;
                events.push(note_off(self.base + index as u8));
            } else if !self.held[index] && pressed {
                self.held[index] = true;
                events.push(note_on(self.base + index as u8));
            }
        }
        events
    }
    pub fn show(&mut self, ui: &mut egui::Ui, playable: bool) -> Vec<MidiEvent> {
        let mut events = Vec::new();
        ui.horizontal(|ui| {
            ui.strong("Keyboard");
            if ui
                .add_enabled(self.base >= 36, egui::Button::new("− octave"))
                .clicked()
            {
                events.extend(self.release());
                self.base -= 12;
            }
            ui.label(format!("C{}–C{}", self.base / 12 - 1, self.base / 12));
            if ui
                .add_enabled(self.base <= 84, egui::Button::new("+ octave"))
                .clicked()
            {
                events.extend(self.release());
                self.base += 12;
            }
            if ui.button("All notes off").clicked() {
                events.extend(self.release());
                events.push(MidiEvent::Panic);
            }
            ui.label(if playable {
                "A W S E D F T G Y H U J K · MIDI channel 1"
            } else {
                "Press Play to perform · MIDI channel 1"
            });
        });
        let width = ui.available_width().min(640.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 82.0), Sense::hover());
        let white_width = width / 8.0;
        let mut keys = [Rect::NOTHING; 13];
        for (position, note) in WHITE.iter().enumerate() {
            keys[*note] = Rect::from_min_size(
                rect.min + Vec2::new(position as f32 * white_width, 0.0),
                Vec2::new(white_width - 2.0, 82.0),
            );
        }
        for (note, position) in BLACK {
            keys[note] = Rect::from_min_size(
                rect.min + Vec2::new((position as f32 + 0.68) * white_width, 0.0),
                Vec2::new(white_width * 0.60, 51.0),
            );
        }
        let (pointer, down, pressed) = ui.input(|input| {
            (
                input.pointer.interact_pos(),
                input.pointer.primary_down(),
                input.pointer.primary_pressed(),
            )
        });
        let hovered = pointer.and_then(|point| {
            BLACK
                .iter()
                .map(|(note, _)| *note)
                .chain(WHITE)
                .find(|note| keys[*note].contains(point))
        });
        let next = if playable && down && (pressed || self.mouse_note.is_some()) {
            hovered.map(|note| self.base + note as u8)
        } else {
            None
        };
        if next != self.mouse_note {
            if let Some(note) = self.mouse_note.take() {
                events.push(note_off(note));
            }
            if let Some(note) = next {
                events.push(note_on(note));
                self.mouse_note = Some(note);
            }
        }
        for note in WHITE.into_iter().chain(BLACK.iter().map(|(note, _)| *note)) {
            let black = BLACK.iter().any(|(index, _)| *index == note);
            let held = self.held[note] || self.mouse_note == Some(self.base + note as u8);
            let fill = if held {
                Color32::from_rgb(113, 185, 157)
            } else if black {
                Color32::from_rgb(34, 38, 42)
            } else {
                Color32::from_rgb(219, 223, 214)
            };
            ui.painter().rect_filled(keys[note], 3.0, fill);
            ui.painter().rect_stroke(
                keys[note],
                3.0,
                Stroke::new(1.0_f32, Color32::from_gray(75)),
                StrokeKind::Inside,
            );
            ui.painter().text(
                keys[note].center_bottom() - Vec2::new(0.0, 10.0),
                Align2::CENTER_CENTER,
                LABELS[note],
                FontId::proportional(11.0),
                if black {
                    Color32::WHITE
                } else {
                    Color32::from_gray(35)
                },
            );
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(
        ctx: &egui::Context,
        keyboard: &mut Keyboard,
        events: Vec<egui::Event>,
        enabled: bool,
    ) -> Vec<MidiEvent> {
        let mut output = Vec::new();
        let _ = ctx.run(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ctx| {
                output = keyboard.update(ctx, enabled);
            },
        );
        output
    }
    fn key(key: Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: Default::default(),
        }
    }
    #[test]
    fn chords_do_not_repeat_and_focus_loss_releases_without_retriggering_held_keys() {
        let ctx = egui::Context::default();
        let mut keyboard = Keyboard::default();
        let events = frame(
            &ctx,
            &mut keyboard,
            vec![key(Key::A, true), key(Key::D, true), key(Key::G, true)],
            true,
        );
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], MidiEvent::NoteOn { note: 60, .. }));
        assert!(frame(&ctx, &mut keyboard, vec![], true).is_empty());
        assert_eq!(frame(&ctx, &mut keyboard, vec![], false).len(), 3);
        assert!(frame(&ctx, &mut keyboard, vec![], true).is_empty());
        let repeated = egui::Event::Key {
            key: Key::A,
            physical_key: Some(Key::A),
            pressed: true,
            repeat: true,
            modifiers: Default::default(),
        };
        assert!(frame(&ctx, &mut keyboard, vec![repeated], true).is_empty());
        assert!(frame(
            &ctx,
            &mut keyboard,
            vec![key(Key::A, false), key(Key::D, false), key(Key::G, false)],
            true
        )
        .is_empty());
        assert_eq!(
            frame(&ctx, &mut keyboard, vec![key(Key::A, true)], true).len(),
            1
        );
    }
}
