use openpatch_core::{Engine, ModuleAction, Patch, PortFrame, TapSample};
use serde_json::{json, Value};

fn patch(rate: u32, params: Value) -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":rate,
        "nodes":[{"id":"clock","kind":"constant","params":{"value":0}},
                 {"id":"reset","kind":"constant","params":{"value":0}},
                 {"id":"terrain","kind":"rhythm_terrain","params":params}],
        "cables":[{"from":"clock","output":"out","to":"terrain","input":"clock"},
                  {"from":"reset","output":"out","to":"terrain","input":"reset"}],
        "output":"terrain","output_port":"trig1"}))
    .unwrap()
}
fn render(engine: &mut Engine, frames: usize) -> Vec<[TapSample; 8]> {
    let taps = [
        "trig1", "trig2", "trig3", "aux1", "aux2", "aux3", "clock", "random",
    ]
    .map(|p| Some(engine.resolve_tap("terrain", p, 0).unwrap()));
    let mut audio = vec![[0.0; 2]; frames];
    let mut samples = vec![[TapSample::default(); 8]; frames];
    engine.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    samples
}

#[test]
fn rhythm_terrain_actual_engine_matches_original_clock_reset_map_and_euclidean_scans() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/rhythm-terrain-native-host.json")).unwrap();
    let mut frames = Vec::new();
    for segment in fixture["segments"].as_array().unwrap() {
        for stream in segment["inputs"].as_array().unwrap() {
            for _ in 0..stream[0].as_u64().unwrap() {
                frames.push((
                    segment["params"].clone(),
                    [
                        stream[1].as_u64().unwrap() as f32 * 5.0,
                        stream[2].as_u64().unwrap() as f32 * 5.0,
                    ],
                ));
            }
        }
    }
    let expected = fixture["state_and_next_step"].as_array().unwrap();
    assert_eq!(frames.len(), expected.len());
    let mut p = patch(8000, frames[0].0.clone());
    let mut engine = p.compile().unwrap();
    for (frame, (params, _)) in frames.iter().enumerate() {
        let next = frames.get(frame + 1).unwrap_or(&frames[frame]).1;
        let mut updated = p.clone();
        updated.nodes[2].params = serde_json::from_value(params.clone()).unwrap();
        for (node, volts) in updated.nodes[..2].iter_mut().zip(next) {
            node.params.insert("value".into(), volts);
        }
        if updated
            .nodes
            .iter()
            .zip(&p.nodes)
            .any(|(a, b)| a.params != b.params)
        {
            let mut replacement = updated.compile().unwrap();
            replacement.adopt_state_from(&engine);
            engine = replacement;
            p = updated;
        }
        // Only input cable values are supplied one host frame early. Authored
        // controls change on the actual source frame. Expected records are the
        // untouched original helper's state/next-step publication.
        let actual = render(&mut engine, 1)[0];
        let state = expected[frame][0].as_u64().unwrap() as u8;
        for (i, sample) in actual.iter().enumerate() {
            assert!(sample.active, "frame {frame},port {i}");
            assert_eq!(
                sample.value,
                if state & (1 << i) != 0 { 5.0 } else { 0.0 },
                "frame {frame},port {i}"
            );
        }
        let lights = engine
            .module_lights(engine.resolve_module_lights("terrain").unwrap())
            .unwrap();
        assert_eq!(
            lights.waveform_position.unwrap() * 32.0,
            expected[frame][1].as_u64().unwrap() as f32,
            "next step at frame {frame}"
        );
    }
}

#[test]
fn rhythm_terrain_resampling_is_identical_across_callback_partitions_and_observation() {
    for rate in [8000, 44100, 192000] {
        let mut p = patch(
            rate,
            json!({"tempo":0,"gate_mode":1,"alternate":1,"chaos":0.2,"seed":33}),
        );
        p.nodes[0] = serde_json::from_value(
            json!({"id":"clock","kind":"stock_lfo","params":{"rate":6,"pulse_width":0.2}}),
        )
        .unwrap();
        p.cables[0].output = "square".into();
        let mut whole = p.compile().unwrap();
        let expected = render(&mut whole, 2049);
        let mut split = p.compile().unwrap();
        split.set_port_lights_enabled(true);
        let mut actual = Vec::new();
        for count in [1, 0, 7, 137, 511, 1393] {
            actual.extend(render(&mut split, count));
        }
        assert_eq!(actual.len(), expected.len());
        assert!(
            actual.iter().zip(&expected).all(|(a, b)| a
                .iter()
                .zip(b)
                .all(|(a, b)| a.active == b.active && a.value.to_bits() == b.value.to_bits())),
            "rate {rate}"
        );
    }
}

#[test]
fn rhythm_terrain_tap_releases_in_bypass_and_source_clock_resumes_frozen_state() {
    let p = patch(48000, json!({"tempo":480}));
    let mut old = p.compile().unwrap();
    let handle = old.resolve_module_control("terrain").unwrap();
    assert!(old.handle_module_event(handle, ModuleAction::TapPress));
    render(&mut old, 193);
    assert!(old.module_control_state(handle).unwrap().pressed);
    let mut paused = p.clone();
    paused.nodes[2].bypassed = true;
    let mut bypass = paused.compile().unwrap();
    bypass.adopt_state_from(&old);
    let handle = bypass.resolve_module_control("terrain").unwrap();
    assert!(bypass.handle_module_event(handle, ModuleAction::TapRelease));
    assert!(!bypass.handle_module_event(handle, ModuleAction::TapPress));
    let mut output = [PortFrame::default(); 129];
    bypass.process_poly_block(&mut output);
    assert!(output
        .iter()
        .all(|p| p.channels == 1 && p.channel(0) == 0.0));
    assert!(!bypass.module_control_state(handle).unwrap().pressed);
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&bypass);
    let a = render(&mut resumed, 997);
    let b = render(&mut old, 997);
    assert!(a.iter().zip(&b).all(|(a, b)| a
        .iter()
        .zip(b)
        .all(|(a, b)| a.value.to_bits() == b.value.to_bits())));
}
