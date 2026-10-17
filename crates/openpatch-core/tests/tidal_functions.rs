use openpatch_core::{catalog, Engine, ModuleLightState, Patch, PortFrame, TapSample};
use serde_json::{json, Value};

fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"tidal","kind":"tidal_functions","params":{
            "frequency":200.0,"shift":0.5,"output_mode":2.0}}],"output":"tidal","output_port":"out1"}))
    .unwrap()
}

fn render(engine: &mut Engine, count: usize) -> Vec<[TapSample; 4]> {
    let taps = std::array::from_fn(|i| {
        Some(
            engine
                .resolve_tap("tidal", ["out1", "out2", "out3", "out4"][i], 0)
                .unwrap(),
        )
    });
    let mut audio = vec![[0.0; 2]; count];
    let mut samples = vec![[TapSample::default(); 4]; count];
    engine.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    samples
}

fn lights(engine: &Engine) -> ModuleLightState {
    engine
        .module_lights(engine.resolve_module_lights("tidal").unwrap())
        .unwrap()
}

#[test]
fn tidal_default_first_output_is_audible_without_extra_configuration() {
    let mut p = patch();
    p.nodes[0].params.clear();
    let mut engine = p.compile().unwrap();
    let mut frames = vec![PortFrame::default(); 24006];
    engine.process_poly_block(&mut frames);
    let peak = frames
        .iter()
        .map(|f| f.channel(0).abs())
        .fold(0.0_f32, f32::max);
    assert!(peak > 1.0 && peak <= 8.0, "default output 1 peak={peak}");
}

#[test]
fn tidal_actual_engine_matches_retained_cpp_voltages_after_six_frame_native_delay() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/tidal_looping_slope_phase.json")).unwrap();
    let samples = render(&mut patch().compile().unwrap(), 2406);
    for sample in &samples[..6] {
        assert!(sample.iter().all(|s| s.active && s.value == 0.0));
    }
    for frame in fixture["frames"].as_array().unwrap() {
        let index = frame["frame"].as_u64().unwrap() as usize;
        for channel in 0..4 {
            assert!(samples[index + 6][channel].active);
            assert_eq!(
                samples[index + 6][channel].value.to_bits() as u64,
                frame["output_bits"][channel].as_u64().unwrap(),
                "source frame {index}, output {channel}"
            );
        }
    }
}

#[test]
fn tidal_chunk_boundaries_and_graph_adoption_preserve_all_four_outputs() {
    let p = patch();
    let expected = render(&mut p.compile().unwrap(), 519);
    let mut engine = p.compile().unwrap();
    let mut offset = 0;
    for count in [1, 2, 6, 0, 17, 128, 3, 97, 129, 136] {
        let samples = render(&mut engine, count);
        for (a, b) in samples
            .iter()
            .flatten()
            .zip(expected[offset..offset + count].iter().flatten())
        {
            assert_eq!(a.value.to_bits(), b.value.to_bits());
            assert_eq!(a.active, b.active);
        }
        offset += count;
        let mut replacement = p.compile().unwrap();
        replacement.adopt_state_from(&engine);
        engine = replacement;
    }
    assert_eq!(offset, 519);
}

#[test]
fn tidal_bypass_is_silent_and_freezes_runtime_initialize_matches_fresh_reopen() {
    let p = patch();
    let mut engine = p.compile().unwrap();
    render(&mut engine, 101);
    let before = lights(&engine);
    let mut bypass_patch = p.clone();
    bypass_patch.nodes[0].bypassed = true;
    let mut bypass = bypass_patch.compile().unwrap();
    bypass.adopt_state_from(&engine);
    let mut frames = [PortFrame::default(); 71];
    bypass.process_poly_block(&mut frames);
    assert!(frames
        .iter()
        .all(|f| f.channels == 1 && f.values == [0.0; 16]));
    assert_eq!(lights(&bypass).brightness, before.brightness);
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&bypass);
    let a = render(&mut resumed, 47);
    let b = render(&mut engine, 47);
    for (a, b) in a.iter().flatten().zip(b.iter().flatten()) {
        assert_eq!(a.value.to_bits(), b.value.to_bits());
    }
    let handle = resumed.resolve_module_lights("tidal").unwrap();
    assert!(resumed.initialize_module_state(handle));
    let a = render(&mut resumed, 103);
    let saved = serde_json::to_string(&p).unwrap();
    let reopened: Patch = serde_json::from_str(&saved).unwrap();
    let b = render(&mut reopened.compile().unwrap(), 103);
    for (a, b) in a.iter().flatten().zip(b.iter().flatten()) {
        assert_eq!(a.value.to_bits(), b.value.to_bits());
    }
}

#[test]
fn tidal_catalog_exposes_native_source_contract_and_rejects_invalid_edits() {
    let modules = catalog();
    let module = modules["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "tidal_functions")
        .unwrap();
    assert_eq!(module["outputs"], json!(["out1", "out2", "out3", "out4"]));
    assert_eq!(module["polyphony"]["maximum_channels"], 1);
    assert_eq!(
        module["tags"],
        json!(["Custom", "Modulation", "Envelopes", "Oscillators"])
    );
    assert!(module["processing_policy"]
        .as_str()
        .unwrap()
        .contains("six frames"));
    for (name, value) in [
        ("frequency", 0.0),
        ("frequency", 20001.0),
        ("clock_ratio", 8.5),
        ("ramp_mode", 0.5),
        ("output_mode", 4.0),
        ("unknown", 1.0),
    ] {
        let mut bad = patch();
        bad.nodes[0].params.insert(name.into(), value);
        assert!(bad.compile().is_err(), "invalid {name}={value}");
    }
}
