//! Native game controller input (Linux evdev), for Steam Deck Game Mode.
//!
//! Steam Input's default Gamepad template exposes an Xbox 360-style virtual
//! pad. Its buttons become the keys OpenPatch already understands, so the
//! Patch Engine, rack and toolbar behave exactly as with a keyboard:
//!
//! | Control              | Key            | Patch Engine open      | Rack            |
//! |----------------------|----------------|------------------------|-----------------|
//! | D-pad / left stick   | arrows         | row / word             | pan             |
//! | A                    | Enter          | build                  | module library  |
//! | B                    | Esc            | close                  | cancel          |
//! | X                    | Space          | surprise               | play / stop     |
//! | Y                    | F1             | open / close           | open            |
//! | LB / RB              | Shift+Tab / Tab| move focus             | move focus      |
//! | LT / RT              | PageDown / Up  | zoom out / in          |                 |
//! | View (back)          | Home           | fit the rack           |                 |
//! | Menu (start)         | play / stop    |                        |                 |
//!
//! Input is read only while the OpenPatch window has focus.
use eframe::egui::{self, Key, Modifiers};
use std::collections::BTreeMap;
use std::sync::mpsc;

/// Linux input event codes (linux/input-event-codes.h); Xbox layout as reported by xpad and Steam Input.
mod code {
    pub const EV_KEY: u16 = 0x01;
    pub const EV_ABS: u16 = 0x03;
    pub const BTN_A: u16 = 0x130;
    pub const BTN_B: u16 = 0x131;
    pub const BTN_X: u16 = 0x133;
    pub const BTN_Y: u16 = 0x134;
    pub const BTN_TL: u16 = 0x136;
    pub const BTN_TR: u16 = 0x137;
    pub const BTN_TL2: u16 = 0x138;
    pub const BTN_TR2: u16 = 0x139;
    pub const BTN_SELECT: u16 = 0x13a;
    pub const BTN_START: u16 = 0x13b;
    pub const BTN_DPAD_UP: u16 = 0x220;
    pub const BTN_DPAD_DOWN: u16 = 0x221;
    pub const BTN_DPAD_LEFT: u16 = 0x222;
    pub const BTN_DPAD_RIGHT: u16 = 0x223;
    pub const ABS_X: u16 = 0x00;
    pub const ABS_Y: u16 = 0x01;
    pub const ABS_Z: u16 = 0x02;
    pub const ABS_RZ: u16 = 0x05;
    pub const ABS_HAT0X: u16 = 0x10;
    pub const ABS_HAT0Y: u16 = 0x11;
}

/// One controller event: an EV_KEY or EV_ABS code and its value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Input {
    pub kind: u16,
    pub code: u16,
    pub value: i32,
}

/// Which control holds a key down; a key is released when no control holds it.
const HAT: u8 = 1;
const STICK: u8 = 2;
const BUTTON: u8 = 4;

const REPEAT_DELAY: f64 = 0.35;
const REPEAT_EVERY: f64 = 0.11;
/// Stick deflection (of ±32767) that counts as a direction; triggers count past half of 0–255.
const STICK_THRESHOLD: i32 = 16_000;
const TRIGGER_THRESHOLD: i32 = 128;

/// Turns controller events into egui key events, with key repeat for directions.
#[derive(Default)]
pub(crate) struct Mapper {
    held: BTreeMap<KeyId, (u8, f64)>,
    play: bool,
}

/// `Key` is not `Ord`; arrows and other held keys are tracked by their name.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct KeyId(&'static str);

fn key_id(key: Key) -> KeyId {
    KeyId(key.name())
}

fn key_of(id: KeyId) -> Key {
    Key::from_name(id.0).expect("held keys are named egui keys")
}

fn repeats(key: Key) -> bool {
    matches!(
        key,
        Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight
    )
}

fn key_event(key: Key, pressed: bool, repeat: bool, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers,
    }
}

impl Mapper {
    /// Applies one controller event at time `now` (seconds), appending key events to `out`.
    pub(crate) fn feed(&mut self, input: Input, now: f64, out: &mut Vec<egui::Event>) {
        use code::*;
        let on = input.value != 0;
        match (input.kind, input.code) {
            (EV_KEY, BTN_A) => self.tap(Key::Enter, on, Modifiers::NONE, out),
            (EV_KEY, BTN_B) => self.tap(Key::Escape, on, Modifiers::NONE, out),
            (EV_KEY, BTN_X) => self.tap(Key::Space, on, Modifiers::NONE, out),
            (EV_KEY, BTN_Y) => self.tap(Key::F1, on, Modifiers::NONE, out),
            (EV_KEY, BTN_TL) => self.tap(Key::Tab, on, Modifiers::SHIFT, out),
            (EV_KEY, BTN_TR) => self.tap(Key::Tab, on, Modifiers::NONE, out),
            (EV_KEY, BTN_TL2) => self.hold(Key::PageDown, BUTTON, on, now, out),
            (EV_KEY, BTN_TR2) => self.hold(Key::PageUp, BUTTON, on, now, out),
            (EV_KEY, BTN_SELECT) => self.tap(Key::Home, on, Modifiers::NONE, out),
            (EV_KEY, BTN_START) => self.play |= on,
            (EV_KEY, BTN_DPAD_UP) => self.hold(Key::ArrowUp, BUTTON, on, now, out),
            (EV_KEY, BTN_DPAD_DOWN) => self.hold(Key::ArrowDown, BUTTON, on, now, out),
            (EV_KEY, BTN_DPAD_LEFT) => self.hold(Key::ArrowLeft, BUTTON, on, now, out),
            (EV_KEY, BTN_DPAD_RIGHT) => self.hold(Key::ArrowRight, BUTTON, on, now, out),
            (EV_ABS, ABS_HAT0X) => self.axis(
                Key::ArrowLeft,
                Key::ArrowRight,
                HAT,
                input.value.signum(),
                now,
                out,
            ),
            (EV_ABS, ABS_HAT0Y) => self.axis(
                Key::ArrowUp,
                Key::ArrowDown,
                HAT,
                input.value.signum(),
                now,
                out,
            ),
            (EV_ABS, ABS_X) => self.axis(
                Key::ArrowLeft,
                Key::ArrowRight,
                STICK,
                deflection(input.value),
                now,
                out,
            ),
            (EV_ABS, ABS_Y) => self.axis(
                Key::ArrowUp,
                Key::ArrowDown,
                STICK,
                deflection(input.value),
                now,
                out,
            ),
            (EV_ABS, ABS_Z) => self.hold(
                Key::PageDown,
                STICK,
                input.value > TRIGGER_THRESHOLD,
                now,
                out,
            ),
            (EV_ABS, ABS_RZ) => self.hold(
                Key::PageUp,
                STICK,
                input.value > TRIGGER_THRESHOLD,
                now,
                out,
            ),
            _ => {}
        }
    }

    /// Emits key repeats for held directions; returns the time of the next repeat, if any.
    pub(crate) fn tick(&mut self, now: f64, out: &mut Vec<egui::Event>) -> Option<f64> {
        let mut next: Option<f64> = None;
        for (&id, (_, due)) in self.held.iter_mut() {
            let key = key_of(id);
            if !repeats(key) {
                continue;
            }
            if now >= *due {
                out.push(key_event(key, true, true, Modifiers::NONE));
                *due = now + REPEAT_EVERY;
            }
            next = Some(next.map_or(*due, |n: f64| n.min(*due)));
        }
        next
    }

    /// True once per Menu (start) press.
    pub(crate) fn take_play(&mut self) -> bool {
        std::mem::take(&mut self.play)
    }

    /// Releases every held key, e.g. when the window loses focus.
    pub(crate) fn release_all(&mut self, out: &mut Vec<egui::Event>) {
        for (id, _) in std::mem::take(&mut self.held) {
            out.push(key_event(key_of(id), false, false, Modifiers::NONE));
        }
        self.play = false;
    }

    fn tap(&mut self, key: Key, on: bool, modifiers: Modifiers, out: &mut Vec<egui::Event>) {
        out.push(key_event(key, on, false, modifiers));
    }

    fn hold(&mut self, key: Key, source: u8, on: bool, now: f64, out: &mut Vec<egui::Event>) {
        let id = key_id(key);
        let before = self.held.get(&id).map_or(0, |held| held.0);
        let after = if on {
            before | source
        } else {
            before & !source
        };
        match (before, after) {
            (0, 0) => {}
            (0, _) => {
                self.held.insert(id, (after, now + REPEAT_DELAY));
                out.push(key_event(key, true, false, Modifiers::NONE));
            }
            (_, 0) => {
                self.held.remove(&id);
                out.push(key_event(key, false, false, Modifiers::NONE));
            }
            _ => {
                if let Some(held) = self.held.get_mut(&id) {
                    held.0 = after;
                }
            }
        }
    }

    fn axis(
        &mut self,
        negative: Key,
        positive: Key,
        source: u8,
        direction: i32,
        now: f64,
        out: &mut Vec<egui::Event>,
    ) {
        self.hold(negative, source, direction < 0, now, out);
        self.hold(positive, source, direction > 0, now, out);
    }
}

fn deflection(value: i32) -> i32 {
    if value <= -STICK_THRESHOLD {
        -1
    } else if value >= STICK_THRESHOLD {
        1
    } else {
        0
    }
}

/// Connected controllers and the events they send.
pub(crate) struct Gamepad {
    events: mpsc::Receiver<Input>,
    names: std::sync::Arc<std::sync::Mutex<BTreeMap<String, String>>>,
    mapper: Mapper,
    focused: bool,
}

impl Gamepad {
    /// Starts watching for controllers; `None` where native controller input is not supported.
    pub(crate) fn start(ctx: &egui::Context) -> Option<Self> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        let (sender, events) = mpsc::channel();
        let names = std::sync::Arc::default();
        if !linux::watch(ctx.clone(), sender, std::sync::Arc::clone(&names)) {
            return None;
        }
        Some(Self {
            events,
            names,
            mapper: Mapper::default(),
            focused: true,
        })
    }

    /// Adds controller key events to this frame's input; returns true for a Menu (play) press.
    pub(crate) fn service(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) -> bool {
        let now = input.time.unwrap_or_else(|| ctx.input(|i| i.time));
        let focused = input.focused;
        let mut out = Vec::new();
        if !focused && self.focused {
            self.mapper.release_all(&mut out);
        }
        self.focused = focused;
        for event in self.events.try_iter() {
            if focused {
                self.mapper.feed(event, now, &mut out);
            }
        }
        if let Some(due) = self.mapper.tick(now, &mut out) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((due - now).max(0.0)));
        }
        input.events.extend(out);
        focused && self.mapper.take_play()
    }

    /// Names of connected controllers, for Settings.
    pub(crate) fn names(&self) -> Vec<String> {
        self.names
            .lock()
            .map(|names| names.values().cloned().collect())
            .unwrap_or_default()
    }
}

/// evdev, plain std; only started on Linux, but compiled (and tested) everywhere.
mod linux {
    use super::{code, Input};
    use eframe::egui;
    use std::collections::BTreeMap;
    use std::io::Read;
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::Duration;

    /// Size of `struct input_event` on 64-bit Linux: a 16-byte timeval, type, code and value.
    const EVENT_BYTES: usize = 24;

    /// True when the device's key capabilities include the gamepad A button.
    pub(super) fn is_gamepad(capabilities: &str) -> bool {
        // Space-separated hexadecimal words, most significant first, each one `long` (64 bits) wide.
        let words: Vec<&str> = capabilities.split_whitespace().rev().collect();
        let bit = code::BTN_A as usize;
        words
            .get(bit / 64)
            .and_then(|word| u64::from_str_radix(word, 16).ok())
            .is_some_and(|word| word & (1 << (bit % 64)) != 0)
    }

    pub(super) fn watch(
        ctx: egui::Context,
        sender: mpsc::Sender<Input>,
        names: Arc<Mutex<BTreeMap<String, String>>>,
    ) -> bool {
        std::thread::Builder::new()
            .name("openpatch-gamepad".into())
            .spawn(move || loop {
                scan(&ctx, &sender, &names);
                std::thread::sleep(Duration::from_secs(2));
            })
            .is_ok()
    }

    fn scan(
        ctx: &egui::Context,
        sender: &mpsc::Sender<Input>,
        names: &Arc<Mutex<BTreeMap<String, String>>>,
    ) {
        let Ok(entries) = std::fs::read_dir("/sys/class/input") else {
            return;
        };
        for entry in entries.flatten() {
            let node = entry.file_name().to_string_lossy().into_owned();
            if !node.starts_with("event")
                || names.lock().is_ok_and(|names| names.contains_key(&node))
            {
                continue;
            }
            let device = entry.path().join("device");
            let capabilities =
                std::fs::read_to_string(device.join("capabilities/key")).unwrap_or_default();
            if !is_gamepad(&capabilities) {
                continue;
            }
            let Ok(file) = std::fs::File::open(format!("/dev/input/{node}")) else {
                continue;
            };
            let name = std::fs::read_to_string(device.join("name"))
                .map_or_else(|_| node.clone(), |n| n.trim().to_owned());
            if let Ok(mut names) = names.lock() {
                names.insert(node.clone(), name);
            }
            let (ctx, sender, names) = (ctx.clone(), sender.clone(), Arc::clone(names));
            let spawned = std::thread::Builder::new()
                .name(format!("openpatch-{node}"))
                .spawn(move || {
                    read(file, &ctx, &sender);
                    if let Ok(mut names) = names.lock() {
                        names.remove(&node);
                    }
                });
            if spawned.is_err() {
                return;
            }
        }
    }

    fn read(mut file: std::fs::File, ctx: &egui::Context, sender: &mpsc::Sender<Input>) {
        let mut buffer = [0u8; EVENT_BYTES];
        while file.read_exact(&mut buffer).is_ok() {
            let kind = u16::from_ne_bytes([buffer[16], buffer[17]]);
            let code = u16::from_ne_bytes([buffer[18], buffer[19]]);
            let value = i32::from_ne_bytes([buffer[20], buffer[21], buffer[22], buffer[23]]);
            if kind != code::EV_KEY && kind != code::EV_ABS {
                continue;
            }
            if sender.send(Input { kind, code, value }).is_err() {
                return;
            }
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(events: &[egui::Event]) -> Vec<(Key, bool, bool, bool)> {
        events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    modifiers,
                    ..
                } => Some((*key, *pressed, *repeat, modifiers.shift)),
                _ => None,
            })
            .collect()
    }

    fn feed(
        mapper: &mut Mapper,
        kind: u16,
        code: u16,
        value: i32,
        now: f64,
    ) -> Vec<(Key, bool, bool, bool)> {
        let mut out = vec![];
        mapper.feed(Input { kind, code, value }, now, &mut out);
        keys(&out)
    }

    #[test]
    fn face_buttons_are_the_patch_engine_keys() {
        let mut m = Mapper::default();
        for (button, key) in [
            (code::BTN_A, Key::Enter),
            (code::BTN_B, Key::Escape),
            (code::BTN_X, Key::Space),
            (code::BTN_Y, Key::F1),
            (code::BTN_SELECT, Key::Home),
        ] {
            assert_eq!(
                feed(&mut m, code::EV_KEY, button, 1, 0.0),
                [(key, true, false, false)]
            );
            assert_eq!(
                feed(&mut m, code::EV_KEY, button, 0, 0.1),
                [(key, false, false, false)]
            );
        }
        assert_eq!(
            feed(&mut m, code::EV_KEY, code::BTN_TL, 1, 0.0),
            [(Key::Tab, true, false, true)]
        );
        assert_eq!(
            feed(&mut m, code::EV_KEY, code::BTN_TR, 1, 0.0),
            [(Key::Tab, true, false, false)]
        );
        assert!(!m.take_play());
        assert!(feed(&mut m, code::EV_KEY, code::BTN_START, 1, 0.0).is_empty());
        assert!(m.take_play());
        assert!(!m.take_play());
    }

    #[test]
    fn hat_and_stick_share_arrows_and_repeat_while_held() {
        let mut m = Mapper::default();
        assert_eq!(
            feed(&mut m, code::EV_ABS, code::ABS_HAT0Y, 1, 0.0),
            [(Key::ArrowDown, true, false, false)]
        );
        // The stick joining the same direction neither presses again nor releases.
        assert!(feed(&mut m, code::EV_ABS, code::ABS_Y, 30_000, 0.05).is_empty());
        assert!(feed(&mut m, code::EV_ABS, code::ABS_HAT0Y, 0, 0.1).is_empty());
        let mut out = vec![];
        assert!(m.tick(0.2, &mut out).is_some());
        assert!(out.is_empty(), "no repeat before the delay");
        m.tick(0.4, &mut out);
        assert_eq!(keys(&out), [(Key::ArrowDown, true, true, false)]);
        assert_eq!(
            feed(&mut m, code::EV_ABS, code::ABS_Y, 2_000, 0.5),
            [(Key::ArrowDown, false, false, false)]
        );
        assert_eq!(m.tick(0.6, &mut vec![]), None);
        // Flipping the hat releases one arrow and presses the other.
        assert_eq!(
            feed(&mut m, code::EV_ABS, code::ABS_HAT0X, -1, 1.0),
            [(Key::ArrowLeft, true, false, false)]
        );
        assert_eq!(
            feed(&mut m, code::EV_ABS, code::ABS_HAT0X, 1, 1.1),
            [
                (Key::ArrowLeft, false, false, false),
                (Key::ArrowRight, true, false, false)
            ]
        );
    }

    #[test]
    fn triggers_zoom_without_repeating_and_focus_loss_releases_everything() {
        let mut m = Mapper::default();
        assert!(feed(&mut m, code::EV_ABS, code::ABS_RZ, 60, 0.0).is_empty());
        assert_eq!(
            feed(&mut m, code::EV_ABS, code::ABS_RZ, 255, 0.0),
            [(Key::PageUp, true, false, false)]
        );
        assert_eq!(m.tick(5.0, &mut vec![]), None);
        feed(&mut m, code::EV_ABS, code::ABS_X, -32_000, 0.0);
        let mut out = vec![];
        m.release_all(&mut out);
        let mut released = keys(&out);
        released.sort_by_key(|k| k.0.name());
        assert_eq!(
            released,
            [
                (Key::ArrowLeft, false, false, false),
                (Key::PageUp, false, false, false)
            ]
        );
    }

    #[test]
    fn gamepads_are_recognised_by_their_a_button() {
        // An Xbox 360 pad (Steam Input's virtual controller) and a keyboard.
        assert!(linux::is_gamepad("7cdb000000000000 0 0 0 0"));
        assert!(!linux::is_gamepad("0 0 0 0 0 fffffffffffffffe"));
    }
}
