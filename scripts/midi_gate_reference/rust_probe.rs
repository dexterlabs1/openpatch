// SPDX-License-Identifier: MIT
// Minimal fixture host types. All gate, timer and Learn behavior is production.
#![allow(dead_code)]
use std::{collections::BTreeMap, io::{self, BufRead, Write}};
#[path = __MIDI_GATE_SOURCE__]
mod midi_gate;
#[derive(Debug)]
struct Node { id: String, kind: String, params: BTreeMap<String, f32> }
fn parameter(node: &Node, name: &str, default: f32, min: f32, max: f32) -> Result<f32, String> {
    let value = node.params.get(name).copied().unwrap_or(default);
    if value.is_finite() && (min..=max).contains(&value) { Ok(value) } else { Err(name.into()) }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SavedModuleState { MidiGateMappings([i16; 16]), Other }
#[derive(Clone, Copy, Debug)]
struct MidiLearnCompletion { serial: u64, cell: u8, before: [i16; 16], after: [i16; 16] }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MidiEvent {
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8, velocity: u8 },
    ControlChange { channel: u8, controller: u8, value: u8 },
    Panic,
}
impl MidiEvent {
    fn is_valid(&self) -> bool {
        match *self {
            Self::NoteOn { channel, note, velocity } | Self::NoteOff { channel, note, velocity } => channel < 16 && note < 128 && velocity < 128,
            Self::ControlChange { channel, controller, value } => channel < 16 && controller < 128 && value < 128,
            Self::Panic => true,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct PortFrame { channels: u8, values: [f32; 16] }
#[derive(Clone, Copy, Debug, Default)]
struct ModuleLightState { input_channels: Option<u8>, count: u8, brightness: [[f32; 2]; 16], manual: bool }
fn node(config: &[i32; 19]) -> Node {
    let mut params = BTreeMap::new();
    for (name, value) in ["velocity", "mpe", "channel"].iter().zip(config) { params.insert(name.to_string(), *value as f32); }
    for i in 0..16 { params.insert(format!("note{}", i + 1), config[3 + i] as f32); }
    Node { id: "oracle".into(), kind: "midi_gate".into(), params }
}
fn main() {
    let rate = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut module = midi_gate::MidiGate::build(&Node { id: "oracle".into(), kind: "midi_gate".into(), params: BTreeMap::new() }, rate).unwrap();
    let mut previous = [-999; 19];
    let mut saved = None;
    let mut saved_config = [0; 19];
    let mut serial = 0;
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let values: Vec<i32> = line.unwrap().split_whitespace().map(|s| s.parse().unwrap()).collect();
        let mut config: [i32; 19] = values[..3].iter().chain(&values[6..22]).copied().collect::<Vec<_>>().try_into().unwrap();
        if config != previous {
            let mut next = midi_gate::MidiGate::build(&node(&config), rate).unwrap();
            next.adopt(&module); module = next; previous = config;
        }
        let operation = values[3];
        if operation == 1 { module.panic(); }
        if operation == 2 {
            saved = Some(module.saved_state()); saved_config = config;
            let SavedModuleState::MidiGateMappings(assignments) = module.saved_state() else { panic!() };
            for i in 0..16 { saved_config[3 + i] = assignments[i] as i32; }
        }
        if operation == 3 {
            config = saved_config;
            let mut next = midi_gate::MidiGate::build(&node(&config), rate).unwrap();
            next.adopt(&module); module = next;
            assert!(module.set_saved_state(saved.unwrap())); previous = config;
        }
        if values[4] > 0 { serial += 1; assert!(module.arm_learn(serial, (values[4] - 1) as u8)); }
        let mut cursor = 22;
        for _ in 0..values[5] {
            let (status, channel, note, velocity) = (values[cursor], values[cursor + 1] as u8, values[cursor + 2] as u8, values[cursor + 3] as u8);
            cursor += 4;
            module.handle_midi(match status {
                8 => MidiEvent::NoteOff { channel, note, velocity },
                9 => MidiEvent::NoteOn { channel, note, velocity },
                _ => MidiEvent::ControlChange { channel, controller: note, value: velocity },
            });
        }
        assert_eq!(cursor, values.len());
        let mut outputs = [PortFrame::default(); 16]; module.process(&mut outputs);
        let SavedModuleState::MidiGateMappings(assignments) = module.saved_state() else { panic!() };
        let learning = module.learning().map_or(-1, |(_, cell)| cell as i32);
        let completed = module.take_learn_completion().map_or(-1, |done| done.cell as i32);
        for number in assignments.into_iter().map(i32::from).chain([learning, completed]).chain(outputs.iter().map(|p| p.channels as i32)) {
            output.write_all(&number.to_le_bytes()).unwrap();
        }
        for port in outputs { for value in port.values { output.write_all(&value.to_le_bytes()).unwrap(); } }
    }
}
