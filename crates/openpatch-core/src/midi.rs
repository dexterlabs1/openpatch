//! Original MIDI decoding and fixed-storage MIDI-to-voltage conversion.
//! Wire channels are zero based. Poly pressure is accepted but intentionally
//! ignored: this subset's aftertouch output follows channel pressure only.
//! Rack's pinned implementation also handles key pressure; see the manual review.
use crate::{parameter, Node, PortFrame, MAX_CHANNELS};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MidiEvent {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
    },
    PitchBend {
        channel: u8,
        value: u16,
    },
    ChannelPressure {
        channel: u8,
        pressure: u8,
    },
    PolyPressure {
        channel: u8,
        note: u8,
        pressure: u8,
    },
    Clock,
    Start,
    Continue,
    Stop,
    Panic,
}
// Empty struct wire variants make deny_unknown_fields effective for realtime
// messages too (serde unit variants otherwise accept extra tagged fields).
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WireEvent {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
    },
    PitchBend {
        channel: u8,
        value: u16,
    },
    ChannelPressure {
        channel: u8,
        pressure: u8,
    },
    PolyPressure {
        channel: u8,
        note: u8,
        pressure: u8,
    },
    Clock {},
    Start {},
    Continue {},
    Stop {},
    Panic {},
}
impl<'de> Deserialize<'de> for MidiEvent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let event = match WireEvent::deserialize(deserializer)? {
            WireEvent::NoteOn {
                channel,
                note,
                velocity,
            } => Self::NoteOn {
                channel,
                note,
                velocity,
            },
            WireEvent::NoteOff {
                channel,
                note,
                velocity,
            } => Self::NoteOff {
                channel,
                note,
                velocity,
            },
            WireEvent::ControlChange {
                channel,
                controller,
                value,
            } => Self::ControlChange {
                channel,
                controller,
                value,
            },
            WireEvent::PitchBend { channel, value } => Self::PitchBend { channel, value },
            WireEvent::ChannelPressure { channel, pressure } => {
                Self::ChannelPressure { channel, pressure }
            }
            WireEvent::PolyPressure {
                channel,
                note,
                pressure,
            } => Self::PolyPressure {
                channel,
                note,
                pressure,
            },
            WireEvent::Clock {} => Self::Clock,
            WireEvent::Start {} => Self::Start,
            WireEvent::Continue {} => Self::Continue,
            WireEvent::Stop {} => Self::Stop,
            WireEvent::Panic {} => Self::Panic,
        };
        if !event.is_valid() {
            return Err(serde::de::Error::custom(
                "MIDI channel or data is out of range",
            ));
        }
        Ok(event)
    }
}
impl MidiEvent {
    pub fn is_valid(&self) -> bool {
        match *self {
            Self::NoteOn {
                channel,
                note,
                velocity,
            }
            | Self::NoteOff {
                channel,
                note,
                velocity,
            } => channel < 16 && note < 128 && velocity < 128,
            Self::ControlChange {
                channel,
                controller,
                value,
            } => channel < 16 && controller < 128 && value < 128,
            Self::PitchBend { channel, value } => channel < 16 && value < 16384,
            Self::ChannelPressure { channel, pressure } => channel < 16 && pressure < 128,
            Self::PolyPressure {
                channel,
                note,
                pressure,
            } => channel < 16 && note < 128 && pressure < 128,
            _ => true,
        }
    }
    /// Validate events deserialized from JSON before scheduling them.
    pub fn validate(&self) -> Result<(), String> {
        if self.is_valid() {
            Ok(())
        } else {
            Err("MIDI channel or data is out of range".into())
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimedMidiEvent {
    /// Absolute sample frame from the start of a render or audio session.
    pub frame: u64,
    pub event: MidiEvent,
}
/// Decode exactly one complete supported MIDI 1.0 message. Running status,
/// SysEx, unsupported system messages, trailing bytes and data >127 are rejected.
pub fn parse_midi(bytes: &[u8]) -> Option<MidiEvent> {
    let status = *bytes.first()?;
    let channel = status & 15;
    let event = match status {
        0xf8 if bytes.len() == 1 => MidiEvent::Clock,
        0xfa if bytes.len() == 1 => MidiEvent::Start,
        0xfb if bytes.len() == 1 => MidiEvent::Continue,
        0xfc if bytes.len() == 1 => MidiEvent::Stop,
        0xff if bytes.len() == 1 => MidiEvent::Panic,
        0x80..=0xef => {
            let kind = status >> 4;
            let len = if kind == 0xc || kind == 0xd { 2 } else { 3 };
            if bytes.len() != len || bytes[1..].iter().any(|v| *v > 127) {
                return None;
            }
            let a = bytes[1];
            let b = if len == 3 { bytes[2] } else { 0 };
            match kind {
                8 => MidiEvent::NoteOff {
                    channel,
                    note: a,
                    velocity: b,
                },
                9 if b == 0 => MidiEvent::NoteOff {
                    channel,
                    note: a,
                    velocity: 0,
                },
                9 => MidiEvent::NoteOn {
                    channel,
                    note: a,
                    velocity: b,
                },
                10 => MidiEvent::PolyPressure {
                    channel,
                    note: a,
                    pressure: b,
                },
                11 => MidiEvent::ControlChange {
                    channel,
                    controller: a,
                    value: b,
                },
                13 => MidiEvent::ChannelPressure {
                    channel,
                    pressure: a,
                },
                14 => MidiEvent::PitchBend {
                    channel,
                    value: u16::from(a) | (u16::from(b) << 7),
                },
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(event)
}

pub(crate) const OUTPUTS: [&str; 12] = [
    "pitch",
    "gate",
    "velocity",
    "retrigger",
    "aftertouch",
    "pitchwheel",
    "modwheel",
    "clock",
    "clock_div",
    "start",
    "stop",
    "continue",
];
#[derive(Clone, Copy, Debug, Default)]
struct Held {
    count: u16,
    velocity: u8,
    order: u64,
}
#[derive(Clone, Copy, Debug, Default)]
struct Voice {
    channel: u8,
    note: u8,
    velocity: u8,
    used: bool,
    down: bool,
    gate: bool,
    retrigger: u32,
}
#[derive(Clone, Debug)]
pub(crate) struct MidiCv {
    channels: u8,
    channel: u8,
    allocation: u8,
    division: u8,
    pulse: u32,
    voices: [Voice; MAX_CHANNELS],
    held: [[Held; 128]; 16],
    sustain: [bool; 16],
    pressure: [u8; 16],
    wheel: [u16; 16],
    modulation: [u8; 16],
    next: usize,
    order: u64,
    clocks: u8,
    triggers: [u32; 5],
}
impl MidiCv {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for key in node.params.keys() {
            if !["channels", "channel", "allocation", "clock_div"].contains(&key.as_str()) {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let integer = |name, default, min, max| {
            let v = parameter(node, name, default, min, max)?;
            if v.fract() != 0.0 {
                return Err(format!(
                    "node '{}': parameter '{name}' must be an integer",
                    node.id
                ));
            }
            Ok(v as u8)
        };
        Ok(Self {
            channels: integer("channels", 4.0, 1.0, 16.0)?,
            channel: integer("channel", 0.0, 0.0, 16.0)?,
            allocation: integer("allocation", 0.0, 0.0, 2.0)?,
            division: integer("clock_div", 24.0, 1.0, 96.0)?,
            pulse: rate.div_ceil(1000),
            voices: [Voice::default(); 16],
            held: [[Held::default(); 128]; 16],
            sustain: [false; 16],
            pressure: [0; 16],
            wheel: [8192; 16],
            modulation: [0; 16],
            next: 0,
            order: 0,
            clocks: 0,
            triggers: [0; 5],
        })
    }
    pub(crate) fn compatible(&self, old: &Self) -> bool {
        self.channels == old.channels
            && self.channel == old.channel
            && self.allocation == old.allocation
    }
    pub(crate) fn adopt(&mut self, old: &Self) -> bool {
        if !self.compatible(old) {
            return false;
        }
        // All members are fixed storage. Keep the newly compiled clock division.
        let division = self.division;
        self.clone_from(old);
        self.division = division;
        self.clocks %= division;
        true
    }
    fn accepts(&self, channel: u8) -> bool {
        self.channel == 0 || self.channel == channel + 1
    }
    fn panic(&mut self) {
        self.voices.fill(Voice::default());
        self.held.fill([Held::default(); 128]);
        self.sustain.fill(false);
        self.pressure.fill(0);
        self.wheel.fill(8192);
        self.modulation.fill(0);
        self.next = 0;
        self.order = 0;
        self.clocks = 0;
        self.triggers.fill(0);
    }
    fn compact(&mut self) {
        if self.allocation != 2 || self.channels == 1 {
            return;
        }
        let mut write = 0;
        for read in 0..self.channels as usize {
            if self.voices[read].gate {
                if write != read {
                    self.voices[write] = self.voices[read];
                }
                write += 1;
            }
        }
        for voice in &mut self.voices[write..self.channels as usize] {
            voice.gate = false;
            voice.down = false;
            voice.retrigger = 0;
        }
    }
    fn mono_fallback(&mut self) {
        let mut latest = None;
        for (channel, notes) in self.held.iter().enumerate() {
            for (note, held) in notes.iter().enumerate() {
                if held.count > 0
                    && latest
                        .map(|(_, _, h): (usize, usize, Held)| held.order > h.order)
                        .unwrap_or(true)
                {
                    latest = Some((channel, note, *held));
                }
            }
        }
        if let Some((channel, note, held)) = latest {
            self.voices[0] = Voice {
                channel: channel as u8,
                note: note as u8,
                velocity: held.velocity,
                used: true,
                down: true,
                gate: true,
                retrigger: self.voices[0].retrigger,
            };
        } else {
            self.voices[0].down = false;
            if !self.sustain[self.voices[0].channel as usize] {
                self.voices[0].gate = false;
            }
        }
    }
    fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        self.order = self.order.saturating_add(1);
        let held = &mut self.held[channel as usize][note as usize];
        held.count = held.count.saturating_add(1);
        held.velocity = velocity;
        held.order = self.order;
        let n = self.channels as usize;
        let index = if n == 1 {
            0
        } else if self.allocation == 1 {
            self.voices[..n]
                .iter()
                .position(|v| v.used && v.channel == channel && v.note == note)
                .or_else(|| self.voices[..n].iter().position(|v| !v.gate))
                .unwrap_or(0)
        } else if self.allocation == 2 {
            self.voices[..n].iter().position(|v| !v.gate).unwrap_or(0)
        } else {
            (0..n)
                .map(|offset| (self.next + offset) % n)
                .find(|i| !self.voices[*i].gate)
                .unwrap_or(self.next)
        };
        self.voices[index] = Voice {
            channel,
            note,
            velocity,
            used: true,
            down: true,
            gate: true,
            retrigger: self.pulse,
        };
        self.next = (index + 1) % n;
    }
    fn note_off(&mut self, channel: u8, note: u8) {
        let held = &mut self.held[channel as usize][note as usize];
        held.count = held.count.saturating_sub(1);
        if self.channels == 1 {
            self.mono_fallback();
            return;
        }
        // A reused voice remains held until all overlapping note-ons are released.
        let remaining = held.count as usize;
        let pressed = self.voices[..self.channels as usize]
            .iter()
            .filter(|v| v.down && v.channel == channel && v.note == note)
            .count();
        if pressed <= remaining {
            return;
        }
        // One note-off releases the earliest lane containing a matching voice.
        if let Some(v) = self.voices[..self.channels as usize]
            .iter_mut()
            .find(|v| v.down && v.channel == channel && v.note == note)
        {
            v.down = false;
            if !self.sustain[channel as usize] {
                v.gate = false;
            }
        }
        self.compact();
    }
    fn release_sustain(&mut self, channel: u8) {
        for v in &mut self.voices[..self.channels as usize] {
            if v.channel == channel && !v.down {
                v.gate = false;
            }
        }
        if self.channels == 1 {
            self.mono_fallback();
        }
        self.compact();
    }
    pub(crate) fn handle(&mut self, event: MidiEvent) {
        use MidiEvent::*;
        let channel = match event {
            NoteOn { channel, .. }
            | NoteOff { channel, .. }
            | ControlChange { channel, .. }
            | PitchBend { channel, .. }
            | ChannelPressure { channel, .. }
            | PolyPressure { channel, .. } => Some(channel),
            _ => None,
        };
        if channel.is_some_and(|c| !self.accepts(c)) {
            return;
        }
        match event {
            Panic => self.panic(),
            NoteOn {
                channel,
                note,
                velocity: 0,
            }
            | NoteOff { channel, note, .. } => self.note_off(channel, note),
            NoteOn {
                channel,
                note,
                velocity,
            } => self.note_on(channel, note, velocity),
            PitchBend { channel, value } => self.wheel[channel as usize] = value,
            ChannelPressure { channel, pressure } => self.pressure[channel as usize] = pressure,
            PolyPressure { .. } => {}
            ControlChange {
                channel,
                controller,
                value,
            } => {
                let c = channel as usize;
                match controller {
                    1 => self.modulation[c] = value,
                    64 => {
                        self.sustain[c] = value >= 64;
                        if !self.sustain[c] {
                            self.release_sustain(channel);
                        }
                    }
                    120 => {
                        self.held[c].fill(Held::default());
                        self.sustain[c] = false;
                        self.pressure[c] = 0;
                        self.wheel[c] = 8192;
                        self.modulation[c] = 0;
                        for v in &mut self.voices[..self.channels as usize] {
                            if v.channel == channel {
                                v.gate = false;
                                v.down = false;
                                v.retrigger = 0;
                            }
                        }
                        if self.channels == 1 {
                            self.mono_fallback();
                        }
                        self.compact();
                    }
                    123 => {
                        self.held[c].fill(Held::default());
                        for v in &mut self.voices[..self.channels as usize] {
                            if v.channel == channel {
                                v.down = false;
                                if !self.sustain[c] {
                                    v.gate = false;
                                }
                            }
                        }
                        if self.channels == 1 {
                            self.mono_fallback();
                        }
                        self.compact();
                    }
                    121 => {
                        self.sustain[c] = false;
                        self.pressure[c] = 0;
                        self.wheel[c] = 8192;
                        self.modulation[c] = 0;
                        self.release_sustain(channel);
                    }
                    _ => {}
                }
            }
            Clock => {
                self.triggers[0] = self.pulse;
                if self.clocks == 0 {
                    self.triggers[1] = self.pulse;
                }
                self.clocks = (self.clocks + 1) % self.division;
            }
            Start => {
                self.clocks = 0;
                self.triggers[2] = self.pulse;
            }
            Stop => self.triggers[3] = self.pulse,
            Continue => self.triggers[4] = self.pulse,
        }
    }
    pub(crate) fn process(&mut self, outputs: &mut [PortFrame]) {
        outputs.fill(PortFrame {
            channels: self.channels,
            ..PortFrame::default()
        });
        for (i, voice) in self.voices[..self.channels as usize].iter_mut().enumerate() {
            let c = if voice.used {
                voice.channel as usize
            } else {
                self.channel.saturating_sub(1) as usize
            };
            outputs[0].values[i] = if voice.used {
                (voice.note as f32 - 60.0) / 12.0
            } else {
                0.0
            };
            outputs[1].values[i] = if voice.gate { 10.0 } else { 0.0 };
            outputs[2].values[i] = voice.velocity as f32 * (10.0 / 127.0);
            outputs[3].values[i] = if voice.retrigger > 0 { 10.0 } else { 0.0 };
            outputs[4].values[i] = self.pressure[c] as f32 * (10.0 / 127.0);
            let bend = self.wheel[c] as i32 - 8192;
            outputs[5].values[i] = if bend < 0 {
                bend as f32 * (5.0 / 8192.0)
            } else {
                bend as f32 * (5.0 / 8191.0)
            };
            outputs[6].values[i] = self.modulation[c] as f32 * (10.0 / 127.0);
            for trigger in 0..5 {
                outputs[7 + trigger].values[i] = if self.triggers[trigger] > 0 {
                    10.0
                } else {
                    0.0
                };
            }
            voice.retrigger = voice.retrigger.saturating_sub(1);
        }
        for trigger in &mut self.triggers {
            *trigger = trigger.saturating_sub(1);
        }
    }
}
