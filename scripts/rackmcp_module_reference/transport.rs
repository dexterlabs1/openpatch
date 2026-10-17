// SPDX-License-Identifier: MIT
// Public Patch/Engine transport, status, host-source and raw-tap APIs only.
extern crate openpatch_core;
extern crate serde_json;
use openpatch_core::{Engine, Patch, PortFrame, RackMcpTransport, ResolvedTap, TapSample};
use serde_json::{json, Value};
use std::{env, fs};
const PORTS: [&str; 18] = [
    "pitch1",
    "pitch2",
    "pitch3",
    "pitch4",
    "gate1",
    "gate2",
    "gate3",
    "gate4",
    "velocity1",
    "velocity2",
    "velocity3",
    "velocity4",
    "accent1",
    "accent2",
    "accent3",
    "accent4",
    "clock",
    "reset",
];
fn patch(fixture: &Value, controls: &Value) -> Patch {
    let mut cables =
        vec![json!({"id":"reset_route","from":"reset_source","to":"seq","input":"reset"})];
    if controls["external"].as_bool().unwrap() {
        cables.push(json!({"id":"clock_route","from":"clock_source","to":"seq","input":"clock"}));
    }
    serde_json::from_value(json!({"version":2,"sample_rate":fixture["rate"],
        "nodes":[{"id":"clock_source","kind":"poly_constant"},
            {"id":"reset_source","kind":"poly_constant"},
            {"id":"seq","kind":"rackmcp_sequencer","params":{
                "bpm":controls["bpm"],"swing":controls["swing"],"reset":controls["button"]},
                "rackmcp_sequence":{"data":fixture["data"]}}],
        "cables":cables,"output":"seq","output_port":"pitch1"}))
    .unwrap()
}
fn taps(engine: &Engine) -> [Option<ResolvedTap>; 288] {
    std::array::from_fn(|index| {
        Some(
            engine
                .resolve_tap("seq", PORTS[index / 16], index % 16)
                .unwrap(),
        )
    })
}
fn status(engine: &Engine, fixture: &Value) -> Value {
    let status = engine
        .rackmcp_sequence_status(engine.resolve_module_state("seq").unwrap())
        .unwrap();
    let name = |index: i32| -> Value {
        if index < 0 {
            Value::Null
        } else {
            fixture["data"]["patterns"]
                .get(index as usize)
                .map_or(Value::Null, |pattern| pattern["name"].clone())
        }
    };
    let bar = if status.beat > 0.0 {
        f64::from(status.beat) / 4.0
    } else {
        0.0
    };
    json!({"moduleId":"100","running":status.running,
        "bpm_bits":status.bpm.to_bits(),"swing_bits":status.swing.to_bits(),
        "clockSource":if status.external_clock {"external"} else {"internal"},
        "selectedPattern":name(i32::from(status.selected_pattern)),
        "activePattern":name(status.active_pattern),
        "pendingPattern":status.pending_pattern.map_or(Value::Null, |index| name(i32::from(index))),
        "step":status.step,"loop":status.loop_count,"bar_bits":bar.to_bits(),
        "beat_bits":status.beat.to_bits(),"patternCount":status.pattern_count,"laneCount":status.lane_count})
}
fn commands(engine: &mut Engine, packet: &Value) {
    let handle = engine.resolve_module_state("seq").unwrap();
    for command in packet["commands"].as_array().unwrap() {
        let request = match command["action"].as_str().unwrap() {
            "play" => RackMcpTransport::Play,
            "stop" => RackMcpTransport::Stop,
            "toggle" => RackMcpTransport::Toggle,
            "reset" => RackMcpTransport::Reset,
            "select" => RackMcpTransport::Select {
                pattern: command["pattern"].as_u64().unwrap().try_into().unwrap(),
                now: command["now"].as_bool().unwrap(),
            },
            "seek" => RackMcpTransport::Seek {
                bar: command["bar"].as_f64().unwrap(),
            },
            _ => panic!("unknown transport action"),
        };
        assert!(engine.rackmcp_sequence_transport(handle, request));
    }
}
fn main() {
    let args: Vec<_> = env::args().collect();
    assert_eq!(args.len(), 3);
    let fixture: Value = serde_json::from_str(&fs::read_to_string(&args[1]).unwrap()).unwrap();
    let mut controls = json!({"bpm":300,"swing":63.25,"button":0,"external":false});
    let mut engine = patch(&fixture, &controls).compile().unwrap();
    let mut observed = taps(&engine);
    for line in fs::read_to_string(&args[2]).unwrap().lines() {
        let packet: Value = serde_json::from_str(line).unwrap();
        let next = json!({"bpm":packet["bpm"],"swing":packet["swing"],
            "button":packet["button"],"external":packet["external"]});
        if next != controls {
            let mut replacement = patch(&fixture, &next).compile().unwrap();
            // These parameter/topology edits load no source data. Ordinary
            // equal-data adoption preserves queued transport and divider phase.
            replacement.adopt_state_from(&engine);
            replacement.inherit_parameter_values(&engine, &[], &[]);
            engine = replacement;
            observed = taps(&engine);
            controls = next;
        }
        commands(&mut engine, &packet);
        let before = status(&engine, &fixture);
        for (module, key) in [("clock_source", "clock"), ("reset_source", "reset")] {
            let source = engine.resolve_module_state(module).unwrap();
            assert!(engine.set_host_reference_source(
                source,
                PortFrame::mono(packet[key].as_f64().unwrap() as f32)
            ));
        }
        let mut audio = [[0.0; 2]; 1];
        let mut samples = [[TapSample::default(); 288]; 1];
        engine.process_audio_block_with_taps(&mut audio, &observed, &mut samples);
        let mut outputs = Vec::with_capacity(306);
        for port in 0..18 {
            let values = &samples[0][port * 16..port * 16 + 16];
            outputs.push(values.iter().filter(|value| value.active).count() as u32);
            outputs.extend(values.iter().map(|value| {
                if value.active {
                    value.value.to_bits()
                } else {
                    0
                }
            }));
        }
        println!(
            "{}",
            json!({"before":before,"after":status(&engine, &fixture),"outputs":outputs})
        );
    }
}
