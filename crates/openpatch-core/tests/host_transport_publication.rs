//! Production publication exits and synchronous node-order independence.
use openpatch_core::{Engine, HostModuleSnapshot, Patch, PortFrame, TapSample};
use serde_json::json;

fn patch(width: u8, quantizer_bypassed: bool) -> Patch {
    serde_json::from_value(json!({
        "version":2,"sample_rate":48000,
        "nodes":[
            {"id":"source","kind":"poly_constant","params":{"channels":width,"v1":0.125,"v2":-0.25,"v3":0.75}},
            {"id":"q","kind":"stock_quantizer","bypassed":quantizer_bypassed},
            {"id":"bypass","kind":"stock_vca","bypassed":true,"params":{"level":0}},
            {"id":"native","kind":"stock_mid_side"}
        ],
        "cables":[
            {"id":"source_q","from":"source","to":"q","input":"pitch"},
            {"id":"q_bypass","from":"q","output":"pitch","to":"bypass","input":"signal"},
            {"id":"q_native","from":"q","output":"pitch","to":"native","input":"left"},
            {"id":"bypass_native","from":"bypass","to":"native","input":"right"}
        ],"output":"native","output_port":"left"
    })).unwrap()
}

fn snapshot(engine: &Engine, id: &str) -> HostModuleSnapshot {
    engine
        .host_module_snapshot(engine.resolve_module_state(id).unwrap())
        .unwrap()
}

#[test]
fn quantizer_and_generic_bypass_publish_current_frames_at_both_early_exits() {
    for width in [0, 1, 3, 16] {
        for bypassed in [false, true] {
            let mut engine = patch(width, bypassed).compile().unwrap();
            let taps = [
                Some(engine.resolve_tap("q", "pitch", 0).unwrap()),
                Some(engine.resolve_tap("q", "pitch", 1).unwrap()),
                Some(engine.resolve_tap("q", "pitch", 15).unwrap()),
                Some(engine.resolve_tap("bypass", "out", 0).unwrap()),
                Some(engine.resolve_tap("bypass", "out", 1).unwrap()),
                Some(engine.resolve_tap("bypass", "out", 15).unwrap()),
            ];
            for _ in 0..6 {
                let mut samples = [[TapSample::default(); 6]; 1];
                engine.process_audio_block_with_taps(&mut [[0.0; 2]; 1], &taps, &mut samples);
                for (index, id) in ["q", "bypass"].into_iter().enumerate() {
                    let state = snapshot(&engine, id);
                    for (tap_index, lane) in [0, 1, 15].into_iter().enumerate() {
                        let actual = samples[0][index * 3 + tap_index];
                        assert_eq!(actual.active, lane < usize::from(state.outputs[0].channels));
                        if actual.active {
                            assert_eq!(
                                actual.value.to_bits(),
                                state.outputs[0].values[lane].to_bits()
                            );
                        }
                    }
                    let count = usize::from(state.outputs[0].channels);
                    assert!(state.outputs[0].values[count..]
                        .iter()
                        .all(|value| value.to_bits() == 0));
                }
            }
            let q = snapshot(&engine, "q");
            let bypass = snapshot(&engine, "bypass");
            assert_eq!(q.outputs[0].channels, width.max(1));
            assert_eq!(bypass.outputs[0], q.outputs[0]);
            if width != 0 {
                // The VCA level is zero: only the actual bypass route can
                // produce this nonzero value at its early publication exit.
                assert_ne!(bypass.outputs[0].values[0], 0.0);
            }
        }
    }
}

#[test]
fn publication_keeps_all_private_ports_independent_of_node_order() {
    let original = patch(3, false);
    let mut reference = original.compile().unwrap();
    let mut expected = Vec::new();
    for _ in 0..12 {
        let mut output = [PortFrame::default(); 1];
        reference.process_poly_block(&mut output);
        expected.push((
            output[0],
            ["source", "q", "bypass", "native"].map(|id| snapshot(&reference, id)),
        ));
    }
    // All 24 permutations exercise ordinary, quantizer, generic-bypass and
    // native exits before/after each other. Cable transfer must remain at END.
    for first in 0..4 {
        for second in 0..4 {
            for third in 0..4 {
                if first == second || first == third || second == third {
                    continue;
                }
                let fourth = 6 - first - second - third;
                let mut changed = original.clone();
                changed.nodes = [first, second, third, fourth]
                    .map(|index| original.nodes[index].clone())
                    .into();
                let mut engine = changed.compile().unwrap();
                for (expected_output, expected_ports) in &expected {
                    let mut output = [PortFrame::default(); 1];
                    engine.process_poly_block(&mut output);
                    assert_eq!(output[0], *expected_output);
                    for (id, ports) in ["source", "q", "bypass", "native"]
                        .into_iter()
                        .zip(expected_ports)
                    {
                        assert_eq!(snapshot(&engine, id), *ports);
                    }
                }
            }
        }
    }
}
