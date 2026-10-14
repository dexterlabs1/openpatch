//! UI-thread performance keyboard; these notes never become patch edits.
use eframe::egui::{self, Align2, Color32, FontId, Key, Rect, Sense, Stroke, StrokeKind, Vec2};
use openpatch_core::MidiEvent;

// Pinned Rack physical-key layout. The two rows overlap on five pitches.
const KEYS: [(Key, u8); 37] = [
    (Key::Z, 0),
    (Key::S, 1),
    (Key::X, 2),
    (Key::D, 3),
    (Key::C, 4),
    (Key::V, 5),
    (Key::G, 6),
    (Key::B, 7),
    (Key::H, 8),
    (Key::N, 9),
    (Key::J, 10),
    (Key::M, 11),
    (Key::Comma, 12),
    (Key::L, 13),
    (Key::Period, 14),
    (Key::Semicolon, 15),
    (Key::Slash, 16),
    (Key::Q, 12),
    (Key::Num2, 13),
    (Key::W, 14),
    (Key::Num3, 15),
    (Key::E, 16),
    (Key::R, 17),
    (Key::Num5, 18),
    (Key::T, 19),
    (Key::Num6, 20),
    (Key::Y, 21),
    (Key::Num7, 22),
    (Key::U, 23),
    (Key::I, 24),
    (Key::Num9, 25),
    (Key::O, 26),
    (Key::Num0, 27),
    (Key::P, 28),
    (Key::OpenBracket, 29),
    (Key::Equals, 30),
    (Key::CloseBracket, 31),
];
const NOTE_SOURCES: usize = KEYS.len();
const SOURCE_COUNT: usize = NOTE_SOURCES + 2;

const LABELS: [&str; 32] = [
    "Z", "S", "X", "D", "C", "V", "G", "B", "H", "N", "J", "M", ",\nQ", "L\n2", ".\nW", ";\n3",
    "/\nE", "R", "5", "T", "6", "Y", "7", "U", "I", "9", "O", "0", "P", "[", "=", "]",
];
const WHITE: [usize; 19] = [
    0, 2, 4, 5, 7, 9, 11, 12, 14, 16, 17, 19, 21, 23, 24, 26, 28, 29, 31,
];
const BLACK: [(usize, usize); 13] = [
    (1, 0),
    (3, 1),
    (6, 3),
    (8, 4),
    (10, 5),
    (13, 7),
    (15, 8),
    (18, 10),
    (20, 11),
    (22, 12),
    (25, 14),
    (27, 15),
    (30, 17),
];

pub struct Keyboard {
    base: u8,
    // Store emitted pitch, rather than recomputing it when the octave changes.
    held: [Option<u8>; SOURCE_COUNT],
    blocked: [bool; SOURCE_COUNT],
    logical: [Option<Key>; SOURCE_COUNT],
    quarantined: [bool; SOURCE_COUNT],
    was_focused: bool,
    mouse_note: Option<u8>,
    mouse_offset: Option<usize>,
    mouse_drag_active: bool,
}
impl Default for Keyboard {
    fn default() -> Self {
        Self {
            base: 60,
            held: [None; SOURCE_COUNT],
            blocked: [false; SOURCE_COUNT],
            logical: [None; SOURCE_COUNT],
            quarantined: [false; SOURCE_COUNT],
            was_focused: true,
            mouse_note: None,
            mouse_offset: None,
            mouse_drag_active: false,
        }
    }
}
fn note_on(note: u8) -> MidiEvent {
    MidiEvent::NoteOn {
        channel: 0,
        note,
        velocity: 127,
    }
}
fn note_off(note: u8) -> MidiEvent {
    MidiEvent::NoteOff {
        channel: 0,
        note,
        velocity: 127,
    }
}
fn input_enabled(ctx: &egui::Context, enabled: bool) -> bool {
    enabled && ctx.input(|input| input.focused && input.modifiers.is_none())
}
fn note_at(base: u8, offset: usize) -> Option<u8> {
    let note = base as usize + offset;
    (note <= 127).then_some(note as u8)
}
fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", NAMES[note as usize % 12], note as i16 / 12 - 1)
}
fn key_rects(rect: Rect) -> [Rect; 32] {
    let white_width = rect.width() / WHITE.len() as f32;
    let mut keys = [Rect::NOTHING; 32];
    for (position, note) in WHITE.iter().enumerate() {
        keys[*note] = Rect::from_min_size(
            rect.min + Vec2::new(position as f32 * white_width, 0.0),
            Vec2::new(white_width - 1.0, rect.height()),
        );
    }
    for (note, position) in BLACK {
        keys[note] = Rect::from_min_size(
            rect.min + Vec2::new((position as f32 + 0.68) * white_width, 0.0),
            Vec2::new(white_width * 0.60, 51.0),
        );
    }
    keys
}

impl Keyboard {
    fn owns(&self, note: u8) -> bool {
        self.mouse_note == Some(note) || self.held.contains(&Some(note))
    }
    fn octave(&mut self, up: bool) {
        self.base = if up {
            self.base.saturating_add(12).min(108)
        } else {
            self.base.saturating_sub(12)
        };
    }
    pub fn release(&mut self) -> Vec<MidiEvent> {
        let mut notes = [false; 128];
        for (index, held) in self.held.iter_mut().enumerate() {
            if let Some(note) = held.take() {
                notes[note as usize] = true;
            }
            if self.logical[index].is_some() {
                self.blocked[index] = true;
            }
        }
        if let Some(note) = self.mouse_note.take() {
            notes[note as usize] = true;
        }
        self.mouse_offset = None;
        self.mouse_drag_active = false;
        notes
            .into_iter()
            .enumerate()
            .filter_map(|(note, held)| held.then_some(note_off(note as u8)))
            .collect()
    }
    pub fn update(&mut self, ctx: &egui::Context, enabled: bool) -> Vec<MidiEvent> {
        let (focused, events) = ctx.input(|input| (input.focused, input.events.clone()));
        let blurred = !focused
            || events
                .iter()
                .any(|event| matches!(event, egui::Event::WindowFocused(false)));
        let continuous_focus = self.was_focused && focused && !blurred;
        self.was_focused = focused && !blurred;
        let enabled = input_enabled(ctx, enabled) && !blurred;
        let mut output = if enabled { Vec::new() } else { self.release() };
        if blurred {
            for index in 0..SOURCE_COUNT {
                self.quarantined[index] |= self.blocked[index];
            }
        }
        // Raw ordering retains quick taps and octave changes within one frame.
        for event in events {
            let egui::Event::Key {
                key,
                physical_key,
                pressed,
                repeat,
                modifiers,
            } = event
            else {
                continue;
            };
            let physical = physical_key.unwrap_or(key);
            let index = KEYS
                .iter()
                .position(|(key, _)| *key == physical)
                .or(match physical {
                    Key::Backtick => Some(NOTE_SOURCES),
                    Key::Num1 => Some(NOTE_SOURCES + 1),
                    _ => None,
                });
            if !pressed {
                if let Some(index) = index {
                    self.blocked[index] = false;
                    self.quarantined[index] = false;
                    self.logical[index] = None;
                    if let Some(note) = self.held[index].take() {
                        if !self.owns(note) {
                            output.push(note_off(note));
                        }
                    }
                }
                continue;
            }
            if !enabled || !modifiers.is_none() {
                output.extend(self.release());
                if let Some(index) = index {
                    self.blocked[index] = true;
                    self.logical[index] = Some(key);
                    self.quarantined[index] |= blurred;
                }
                continue;
            }
            if repeat {
                continue;
            }
            if let Some(index) = index {
                if self.quarantined[index] {
                    continue;
                }
                // During uninterrupted focus egui marks another press as a
                // repeat until it observes a release, even if a TextEdit later
                // consumes that release before this keyboard sees its event.
                if self.blocked[index] && !continuous_focus {
                    continue;
                }
                self.blocked[index] = false;
                if let Some(note) = self.held[index].take() {
                    if !self.owns(note) {
                        output.push(note_off(note));
                    }
                }
                self.logical[index] = Some(key);
                if index >= NOTE_SOURCES {
                    self.octave(index == NOTE_SOURCES + 1);
                    continue;
                }
                if let Some(note) = note_at(self.base, KEYS[index].1 as usize) {
                    if !self.owns(note) {
                        output.push(note_on(note));
                    }
                    self.held[index] = Some(note);
                }
            }
        }
        if continuous_focus {
            for index in 0..SOURCE_COUNT {
                if !self.quarantined[index]
                    && self.logical[index]
                        .is_some_and(|key| !ctx.input(|input| input.key_down(key)))
                {
                    self.logical[index] = None;
                    self.blocked[index] = false;
                    if let Some(note) = self.held[index].take() {
                        if !self.owns(note) {
                            output.push(note_off(note));
                        }
                    }
                }
            }
        }
        output
    }

    fn mouse(
        &mut self,
        hovered: Option<usize>,
        down: bool,
        pressed: bool,
        playable: bool,
    ) -> Vec<MidiEvent> {
        if !playable {
            return self.release();
        }
        if !down {
            self.mouse_drag_active = false;
        }
        if pressed {
            self.mouse_drag_active = hovered
                .and_then(|offset| note_at(self.base, offset))
                .is_some();
        }
        let next_offset = if down && self.mouse_drag_active {
            hovered
        } else {
            None
        };
        let next = if next_offset == self.mouse_offset {
            self.mouse_note
        } else {
            next_offset.and_then(|offset| note_at(self.base, offset))
        };
        let mut output = Vec::new();
        if next != self.mouse_note {
            if let Some(note) = self.mouse_note.take() {
                if !self.owns(note) {
                    output.push(note_off(note));
                }
            }
            if let Some(note) = next {
                if !self.owns(note) {
                    output.push(note_on(note));
                }
                self.mouse_note = Some(note);
            }
        }
        self.mouse_offset = next_offset;
        output
    }
    pub fn show(&mut self, ui: &mut egui::Ui, playable: bool) -> Vec<MidiEvent> {
        let playable = input_enabled(ui.ctx(), playable && ui.is_enabled());
        let mut output = if playable { Vec::new() } else { self.release() };
        let show_hint = ui.available_width() >= 700.0;
        ui.horizontal(|ui| {
            ui.strong("Keyboard").on_hover_text("Physical keys: Z S X D C V G B H N J M , L . ; / and Q 2 W 3 E R 5 T 6 Y 7 U I 9 O 0 P [ = ]. Backtick lowers the octave; 1 raises it.");
            if ui
                .add_enabled(playable && self.base >= 12, egui::Button::new("− octave"))
                .clicked()
            {
                self.octave(false);
            }
            ui.label(format!(
                "{}–{}",
                note_name(self.base),
                note_name((self.base as usize + 31).min(127) as u8)
            ));
            if ui
                .add_enabled(playable && self.base < 108, egui::Button::new("+ octave"))
                .clicked()
            {
                self.octave(true);
            }
            if ui.button("All notes off").clicked() {
                output.extend(self.release());
                output.push(MidiEvent::Panic);
            }
            if show_hint {
                ui.label(if playable { "` / 1 octaves · MIDI channel 1" }
                    else { "Press Play to perform · MIDI channel 1" });
            }
        });
        let width = ui.available_width().min(900.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 82.0), Sense::hover());
        let keys = key_rects(rect);
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
        output.extend(self.mouse(hovered, down, pressed, playable));
        for offset in WHITE.into_iter().chain(BLACK.iter().map(|(note, _)| *note)) {
            let black = BLACK.iter().any(|(index, _)| *index == offset);
            let note = note_at(self.base, offset);
            let held = note.is_some_and(|note| self.owns(note));
            let fill = if held {
                Color32::from_rgb(113, 185, 157)
            } else if note.is_none() {
                if black {
                    Color32::from_gray(26)
                } else {
                    Color32::from_gray(104)
                }
            } else if black {
                Color32::from_rgb(34, 38, 42)
            } else {
                Color32::from_rgb(219, 223, 214)
            };
            ui.painter().rect_filled(keys[offset], 3.0, fill);
            ui.painter().rect_stroke(
                keys[offset],
                3.0,
                Stroke::new(1.0_f32, Color32::from_gray(75)),
                StrokeKind::Inside,
            );
            ui.painter().text(
                keys[offset].center_bottom()
                    - Vec2::new(
                        0.0,
                        if (12..=16).contains(&offset) {
                            15.0
                        } else {
                            10.0
                        },
                    ),
                Align2::CENTER_CENTER,
                LABELS[offset],
                FontId::proportional(if black { 9.0 } else { 10.0 }),
                if note.is_none() {
                    Color32::from_gray(75)
                } else if black {
                    Color32::WHITE
                } else {
                    Color32::from_gray(35)
                },
            );
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(key: Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }
    fn frame_state(
        ctx: &egui::Context,
        keyboard: &mut Keyboard,
        events: Vec<egui::Event>,
        enabled: bool,
        focused: bool,
        modifiers: egui::Modifiers,
    ) -> Vec<MidiEvent> {
        let mut output = Vec::new();
        let _ = ctx.run(
            egui::RawInput {
                events,
                focused,
                modifiers,
                ..Default::default()
            },
            |ctx| {
                output = keyboard.update(ctx, enabled);
            },
        );
        output
    }
    fn frame(
        ctx: &egui::Context,
        keyboard: &mut Keyboard,
        events: Vec<egui::Event>,
        enabled: bool,
    ) -> Vec<MidiEvent> {
        frame_state(ctx, keyboard, events, enabled, true, egui::Modifiers::NONE)
    }
    #[test]
    fn all_37_physical_bindings_cover_32_notes_with_full_velocity_and_logical_fallback() {
        let mut offsets = std::collections::BTreeSet::new();
        for (physical, offset) in KEYS {
            let ctx = egui::Context::default();
            let mut keyboard = Keyboard::default();
            assert_eq!(
                frame(&ctx, &mut keyboard, vec![key(physical, true)], true),
                vec![note_on(60 + offset)]
            );
            assert_eq!(
                frame(&ctx, &mut keyboard, vec![key(physical, false)], true),
                vec![note_off(60 + offset)]
            );
            offsets.insert(offset);
        }
        assert_eq!(
            offsets.into_iter().collect::<Vec<_>>(),
            (0..32).collect::<Vec<_>>()
        );
        let ctx = egui::Context::default();
        let mut keyboard = Keyboard::default();
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![egui::Event::Key {
                    key: Key::A,
                    physical_key: Some(Key::Z),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE
                }],
                true
            ),
            vec![note_on(60)]
        );
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![egui::Event::Key {
                    key: Key::A,
                    physical_key: Some(Key::Z),
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE
                }],
                true
            ),
            vec![note_off(60)]
        );
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![egui::Event::Key {
                    key: Key::X,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE
                }],
                true
            ),
            vec![note_on(62)]
        );
        assert_eq!(
            note_on(60),
            MidiEvent::NoteOn {
                channel: 0,
                note: 60,
                velocity: 127
            }
        );
        assert_eq!(
            note_off(60),
            MidiEvent::NoteOff {
                channel: 0,
                note: 60,
                velocity: 127
            }
        );
    }
    #[test]
    fn ordered_quick_taps_and_octave_events_preserve_press_pitch_and_ignore_repeats() {
        let ctx = egui::Context::default();
        let mut keyboard = Keyboard::default();
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![
                    key(Key::Z, true),
                    key(Key::Num1, true),
                    key(Key::Z, false),
                    key(Key::X, true),
                    key(Key::X, false)
                ],
                true
            ),
            vec![note_on(60), note_off(60), note_on(74), note_off(74)]
        );
        let repeated = egui::Event::Key {
            key: Key::Num1,
            physical_key: Some(Key::Num1),
            pressed: true,
            repeat: true,
            modifiers: egui::Modifiers::NONE,
        };
        assert!(frame(&ctx, &mut keyboard, vec![repeated], true).is_empty());
        assert_eq!(keyboard.base, 72);
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![key(Key::Z, true), key(Key::Backtick, true)],
                true
            ),
            vec![note_on(72)]
        );
        assert_eq!(keyboard.base, 60);
        assert_eq!(
            frame(&ctx, &mut keyboard, vec![key(Key::Z, false)], true),
            vec![note_off(72)]
        );
        assert_eq!(
            frame(&ctx, &mut keyboard, vec![key(Key::Z, true)], true),
            vec![note_on(60)]
        );
        assert!(frame(
            &ctx,
            &mut keyboard,
            vec![egui::Event::Key {
                key: Key::Z,
                physical_key: Some(Key::Z),
                pressed: true,
                repeat: true,
                modifiers: egui::Modifiers::NONE
            }],
            true
        )
        .is_empty());
        assert_eq!(
            frame(&ctx, &mut keyboard, vec![key(Key::Z, false)], true),
            vec![note_off(60)]
        );
    }
    #[test]
    fn octave_bounds_signed_names_and_out_of_range_notes_are_safe() {
        let ctx = egui::Context::default();
        let mut keyboard = Keyboard::default();
        frame(
            &ctx,
            &mut keyboard,
            (0..20)
                .flat_map(|_| [key(Key::Backtick, true), key(Key::Backtick, false)])
                .collect(),
            true,
        );
        assert_eq!(keyboard.base, 0);
        assert_eq!(note_name(0), "C-1");
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![key(Key::Z, true), key(Key::Z, false)],
                true
            ),
            vec![note_on(0), note_off(0)]
        );
        frame(
            &ctx,
            &mut keyboard,
            (0..20)
                .flat_map(|_| [key(Key::Num1, true), key(Key::Num1, false)])
                .collect(),
            true,
        );
        assert_eq!(keyboard.base, 108);
        assert!(frame(
            &ctx,
            &mut keyboard,
            vec![key(Key::CloseBracket, true), key(Key::CloseBracket, false)],
            true
        )
        .is_empty());
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![key(Key::T, true), key(Key::T, false)],
                true
            ),
            vec![note_on(127), note_off(127)]
        );
        assert!(keyboard.mouse(Some(31), true, true, true).is_empty());
        assert_eq!(note_name(127), "G9");
    }
    #[test]
    fn aliases_mouse_and_cross_octave_owners_emit_one_on_and_only_final_off() {
        let ctx = egui::Context::default();
        let mut keyboard = Keyboard::default();
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![key(Key::Q, true), key(Key::Comma, true)],
                true
            ),
            vec![note_on(72)]
        );
        assert!(keyboard.mouse(Some(12), true, true, true).is_empty());
        assert!(frame(
            &ctx,
            &mut keyboard,
            vec![key(Key::Q, false), key(Key::Comma, false)],
            true
        )
        .is_empty());
        assert_eq!(keyboard.mouse(None, false, false, true), vec![note_off(72)]);
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![key(Key::Q, true), key(Key::Num1, true), key(Key::Z, true)],
                true
            ),
            vec![note_on(72)]
        );
        assert!(frame(&ctx, &mut keyboard, vec![key(Key::Q, false)], true).is_empty());
        assert_eq!(
            frame(&ctx, &mut keyboard, vec![key(Key::Z, false)], true),
            vec![note_off(72)]
        );
        assert_eq!(
            frame(&ctx, &mut keyboard, vec![key(Key::Z, true)], true),
            vec![note_on(72)]
        );
        assert!(keyboard.mouse(Some(0), true, true, true).is_empty());
        assert_eq!(
            keyboard.release(),
            vec![note_off(72)],
            "panic releases shared pitch exactly once"
        );
    }
    #[test]
    fn mouse_gliss_keeps_held_pitch_across_octaves_and_reenters_without_extra_attack() {
        let mut keyboard = Keyboard::default();
        assert_eq!(keyboard.mouse(Some(0), true, true, true), vec![note_on(60)]);
        keyboard.octave(true);
        assert!(keyboard.mouse(Some(0), true, false, true).is_empty());
        assert_eq!(
            keyboard.mouse(Some(2), true, false, true),
            vec![note_off(60), note_on(74)]
        );
        assert_eq!(keyboard.mouse(None, true, false, true), vec![note_off(74)]);
        assert_eq!(
            keyboard.mouse(Some(4), true, false, true),
            vec![note_on(76)]
        );
        assert_eq!(keyboard.mouse(None, false, false, true), vec![note_off(76)]);
        assert!(
            keyboard.mouse(Some(0), true, false, true).is_empty(),
            "an outside press cannot activate keys by entering the dock"
        );
    }
    #[test]
    fn focus_disabled_and_all_modifier_forms_release_without_held_key_resurrection() {
        for modifiers in [
            egui::Modifiers::COMMAND,
            egui::Modifiers::CTRL,
            egui::Modifiers::SHIFT,
            egui::Modifiers::ALT,
        ] {
            let ctx = egui::Context::default();
            let mut keyboard = Keyboard::default();
            assert_eq!(
                frame(&ctx, &mut keyboard, vec![key(Key::Z, true)], true),
                vec![note_on(60)]
            );
            assert_eq!(
                frame_state(&ctx, &mut keyboard, vec![], true, true, modifiers),
                vec![note_off(60)]
            );
            assert!(frame(&ctx, &mut keyboard, vec![], true).is_empty());
            let modified = egui::Event::Key {
                key: Key::X,
                physical_key: Some(Key::X),
                pressed: true,
                repeat: false,
                modifiers,
            };
            assert!(
                frame(&ctx, &mut keyboard, vec![modified], true).is_empty(),
                "event modifiers matter even with clean global state"
            );
            assert!(
                frame(&ctx, &mut keyboard, vec![key(Key::X, true)], true).is_empty(),
                "suppressed physical press stays blocked until release"
            );
            frame(
                &ctx,
                &mut keyboard,
                vec![key(Key::Z, false), key(Key::X, false)],
                true,
            );
            assert_eq!(
                frame(&ctx, &mut keyboard, vec![key(Key::Z, true)], true),
                vec![note_on(60)]
            );
        }
        for focused in [false, true] {
            let ctx = egui::Context::default();
            let mut keyboard = Keyboard::default();
            frame(&ctx, &mut keyboard, vec![key(Key::Z, true)], true);
            assert_eq!(
                frame_state(
                    &ctx,
                    &mut keyboard,
                    vec![],
                    !focused,
                    focused,
                    egui::Modifiers::NONE
                ),
                vec![note_off(60)]
            );
            assert!(frame(&ctx, &mut keyboard, vec![], true).is_empty());
            assert!(frame(
                &ctx,
                &mut keyboard,
                vec![egui::Event::Key {
                    key: Key::Z,
                    physical_key: Some(Key::Z),
                    pressed: true,
                    repeat: true,
                    modifiers: egui::Modifiers::NONE
                }],
                true
            )
            .is_empty());
        }
    }
    #[test]
    fn actual_focus_reset_quarantines_notes_and_octave_commands_until_observed_release() {
        let ctx = egui::Context::default();
        let mut keyboard = Keyboard::default();
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![key(Key::Z, true), key(Key::Num1, true)],
                true
            ),
            vec![note_on(60)]
        );
        assert_eq!(keyboard.base, 72);
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![egui::Event::WindowFocused(false)],
                true
            ),
            vec![note_off(60)]
        );
        // egui resets keys_down on actual blur, so these backend repeat events
        // can arrive at this layer marked as fresh presses. Quarantine wins.
        assert!(frame(
            &ctx,
            &mut keyboard,
            vec![
                egui::Event::WindowFocused(true),
                egui::Event::Key {
                    key: Key::Z,
                    physical_key: Some(Key::Z),
                    pressed: true,
                    repeat: true,
                    modifiers: egui::Modifiers::NONE
                },
                egui::Event::Key {
                    key: Key::Num1,
                    physical_key: Some(Key::Num1),
                    pressed: true,
                    repeat: true,
                    modifiers: egui::Modifiers::NONE
                }
            ],
            true
        )
        .is_empty());
        assert_eq!(keyboard.base, 72);
        assert!(frame(
            &ctx,
            &mut keyboard,
            vec![key(Key::Z, false), key(Key::Num1, false)],
            true
        )
        .is_empty());
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![key(Key::Z, true), key(Key::Num1, true)],
                true
            ),
            vec![note_on(72)]
        );
        assert_eq!(keyboard.base, 84);
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![key(Key::Z, false), key(Key::Num1, false)],
                true
            ),
            vec![note_off(72)]
        );
    }
    #[test]
    fn consumed_release_during_continuous_focus_does_not_leave_notes_or_commands_blocked() {
        let ctx = egui::Context::default();
        let mut keyboard = Keyboard::default();
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![key(Key::Z, true), key(Key::Num1, true)],
                true
            ),
            vec![note_on(60)]
        );
        let mut output = Vec::new();
        let _ = ctx.run(
            egui::RawInput {
                events: vec![key(Key::Z, false), key(Key::Num1, false)],
                focused: true,
                ..Default::default()
            },
            |ctx| {
                // Equivalent to an editor consuming release events after egui has
                // updated its logical key set and before performance dispatch.
                ctx.input_mut(|input| {
                    input
                        .events
                        .retain(|event| !matches!(event, egui::Event::Key { pressed: false, .. }))
                });
                output = keyboard.update(ctx, false);
            },
        );
        assert_eq!(output, vec![note_off(60)]);
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![key(Key::Z, true), key(Key::Num1, true)],
                true
            ),
            vec![note_on(72)]
        );
        assert_eq!(keyboard.base, 84);
        assert_eq!(
            frame(
                &ctx,
                &mut keyboard,
                vec![key(Key::Z, false), key(Key::Num1, false)],
                true
            ),
            vec![note_off(72)]
        );
    }

    #[test]
    fn narrow_dock_draws_all_keys_without_growing_height_and_disabled_ui_releases() {
        let ctx = egui::Context::default();
        let mut keyboard = Keyboard::default();
        frame(&ctx, &mut keyboard, vec![key(Key::Z, true)], true);
        let mut output = Vec::new();
        let mut extent = Vec2::ZERO;
        let rendered = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(500.0, 140.0),
                )),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        ui.add_enabled_ui(false, |ui| {
                            output = keyboard.show(ui, true);
                            extent = ui.min_size();
                        });
                    });
            },
        );
        assert_eq!(output, vec![note_off(60)]);
        assert!(
            extent.x <= 500.0 && extent.y <= 115.0,
            "dock extent {extent:?}"
        );
        let key_shapes = rendered
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect)
                    if rect.rect.height() == 82.0 || rect.rect.height() == 51.0 =>
                {
                    Some(rect)
                }
                _ => None,
            })
            .count();
        assert_eq!(key_shapes, 64, "fill and stroke for all 32 keys");
        assert_eq!(WHITE.len(), 19);
        assert_eq!(BLACK.len(), 13);
    }
}
