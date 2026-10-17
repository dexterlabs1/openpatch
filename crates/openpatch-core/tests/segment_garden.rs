use openpatch_core::{Engine, Patch, TapSample};
use serde_json::json;

fn patch(rate: u32) -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":rate,
        "nodes":[{"id":"garden","kind":"segment_garden","params":{"level1":0.73,"shape1":0.43,"loop1":1}}],
        "output":"garden","output_port":"out1"})).unwrap()
}
fn render(engine: &mut Engine, frames: usize) -> Vec<[TapSample; 2]> {
    let taps = [
        Some(engine.resolve_tap("garden", "out1", 0).unwrap()),
        Some(engine.resolve_tap("garden", "active1", 0).unwrap()),
    ];
    let mut audio = vec![[0.0; 2]; frames];
    let mut samples = vec![[TapSample::default(); 2]; frames];
    engine.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    samples
}

#[test]
fn segment_garden_native_bridge_matches_frozen_cpp_with_declared_block_latency() {
    let source: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/segment-garden-free-lfo-bits.json")).unwrap();
    let output = source["output_bits"].as_array().unwrap();
    let segments = source["segments"].as_array().unwrap();
    let mut engine = patch(31250).compile().unwrap();
    let mut samples = Vec::new();
    // Irregular native blocks exercise persistent source-clock/block cursors.
    for frames in [1, 3, 17, 64, 129, 306] {
        samples.extend(render(&mut engine, frames));
    }
    assert_eq!(samples.len(), 520);
    assert!(samples[..8].iter().all(|s| s[0].value == 0.0));
    for (i, expected) in output.iter().enumerate() {
        let raw = f32::from_bits(expected.as_u64().unwrap() as u32);
        assert_eq!(
            samples[i + 8][0].value.to_bits(),
            (raw * 8.0).to_bits(),
            "source frame {i}"
        );
        let active = if segments[i] == 0 { 5.0 } else { 0.0 };
        assert_eq!(samples[i + 8][1].value, active, "source activity frame {i}");
    }
}

#[test]
fn segment_garden_zero_channel_physical_gate_starts_a_group_while_taps_do_not() {
    let mut p = patch(48000);
    let mut independent = p.compile().unwrap();
    independent.set_port_lights_enabled(true);
    assert!(render(&mut independent, 4096)
        .iter()
        .any(|s| s[0].value > 0.01));
    p.nodes.push(
        serde_json::from_value(
            json!({"id":"empty","kind":"poly_constant","params":{"channels":0}}),
        )
        .unwrap(),
    );
    p.cables.push(
        serde_json::from_value(
            json!({"from":"empty","output":"out","to":"garden","input":"gate1"}),
        )
        .unwrap(),
    );
    let samples = render(&mut p.compile().unwrap(), 4096);
    assert!(samples.iter().all(|s| s[0].value == 0.0));
    p.cables[0].input = "gate3".into();
    assert!(render(&mut p.compile().unwrap(), 4096)
        .iter()
        .any(|s| s[0].value > 0.01));
}

#[test]
fn segment_garden_group_and_slave_contours_match_cpp_through_live_gate_edits() {
    let source: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/segment-garden-group-bits.json")).unwrap();
    let mut p = patch(31250);
    for i in 0..6 {
        p.nodes[0].params.insert(
            format!("type{}", i + 1),
            source["types"][i].as_u64().unwrap() as f32,
        );
        p.nodes[0].params.insert(
            format!("loop{}", i + 1),
            if i == 1 || i == 4 { 1.0 } else { 0.0 },
        );
        p.nodes[0].params.insert(
            format!("level{}", i + 1),
            source["primary"][i].as_f64().unwrap() as f32,
        );
        p.nodes[0].params.insert(format!("shape{}", i + 1), 0.12);
    }
    p.nodes.push(
        serde_json::from_value(
            json!({"id":"gate","kind":"poly_constant","params":{"channels":1,"v1":0}}),
        )
        .unwrap(),
    );
    p.cables.push(
        serde_json::from_value(json!({"from":"gate","output":"out","to":"garden","input":"gate1"}))
            .unwrap(),
    );
    let mut engine = p.compile().unwrap();
    let mut actual = Vec::new();
    // Every native cable delays by one host frame. Drive the upstream constant
    // one sample before each frozen source gate edge. Bridge latency is separate.
    for (length, voltage) in [
        (12, 0.0),
        (619, 5.0),
        (832, 0.0),
        (619, 5.0),
        (832, 0.0),
        (619, 5.0),
        (571, 0.0),
    ] {
        p.nodes[1].params.insert("v1".into(), voltage);
        let mut next = p.compile().unwrap();
        next.adopt_state_from(&engine);
        let taps = std::array::from_fn::<_, 12, _>(|i| {
            Some(
                next.resolve_tap(
                    "garden",
                    &format!("{}{}", if i < 6 { "out" } else { "active" }, i % 6 + 1),
                    0,
                )
                .unwrap(),
            )
        });
        let mut audio = vec![[0.0; 2]; length];
        let mut samples = vec![[TapSample::default(); 12]; length];
        next.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
        actual.extend(samples);
        engine = next;
    }
    assert_eq!(actual.len(), 4104);
    let rows = source["output_bits"].as_array().unwrap();
    for (frame, row) in rows.iter().enumerate() {
        let segment = source["segments"][frame].as_u64().unwrap() as usize;
        for slot in 0..6 {
            let raw = f32::from_bits(row[slot].as_u64().unwrap() as u32);
            let expected = raw.clamp(-1.0, 1.0) * 8.0;
            assert_eq!(
                actual[frame + 8][slot].value.to_bits(),
                expected.to_bits(),
                "source frame{frame} slot{slot}"
            );
            assert_eq!(
                actual[frame + 8][slot + 6].value,
                if segment == slot { 5.0 } else { 0.0 },
                "source activity frame{frame} slot{slot}"
            );
        }
    }
}

#[test]
fn segment_garden_adoption_bypass_initialize_and_rate_change_preserve_declared_lifecycle() {
    let p = patch(48000);
    let mut uninterrupted = p.compile().unwrap();
    let mut old = p.compile().unwrap();
    assert_eq!(render(&mut old, 1237), render(&mut uninterrupted, 1237));
    let mut next = p.compile().unwrap();
    next.adopt_state_from(&old);
    assert_eq!(render(&mut next, 257), render(&mut uninterrupted, 257));
    let mut bypassed_patch = p.clone();
    bypassed_patch.nodes[0].bypassed = true;
    let mut bypassed = bypassed_patch.compile().unwrap();
    bypassed.adopt_state_from(&next);
    let handle = bypassed.resolve_module_lights("garden").unwrap();
    let before = bypassed.module_lights(handle).unwrap();
    assert!(render(&mut bypassed, 129)
        .iter()
        .all(|s| s.iter().all(|v| v.active && v.value == 0.0)));
    assert_eq!(bypassed.module_lights(handle).unwrap(), before);
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&bypassed);
    assert_eq!(render(&mut resumed, 257), render(&mut uninterrupted, 257));
    let handle = resumed.resolve_module_lights("garden").unwrap();
    assert!(resumed.initialize_module_state(handle));
    assert_eq!(
        render(&mut resumed, 520),
        render(&mut p.compile().unwrap(), 520)
    );
    let changed_rate = patch(44100);
    let mut different = changed_rate.compile().unwrap();
    different.adopt_state_from(&resumed);
    assert_eq!(
        render(&mut different, 520),
        render(&mut changed_rate.compile().unwrap(), 520)
    );
}
