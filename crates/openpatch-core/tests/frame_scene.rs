use openpatch_core::{
    Engine, FrameSceneData, FrameSceneEdit, FrameSceneKeyframe, ModuleContent, Patch, PortFrame,
    TapSample,
};
use serde_json::json;

fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"scene","kind":"frame_scene","params":{"level1":1,"level2":0,"level3":1,"level4":0}}],
        "output":"scene","output_port":"mix"})).unwrap()
}

fn render(engine: &mut Engine, count: usize) -> Vec<[TapSample; 6]> {
    let ports = ["out1", "out2", "out3", "out4", "mix", "step"];
    let taps = std::array::from_fn(|i| Some(engine.resolve_tap("scene", ports[i], 0).unwrap()));
    let mut output = vec![[0.0; 2]; count];
    let mut sampled = vec![[TapSample::default(); 6]; count];
    engine.process_audio_block_with_taps(&mut output, &taps, &mut sampled);
    sampled
}

#[test]
fn frame_scene_mix_uses_physical_cables_not_taps_or_selected_output() {
    let p = patch();
    let mut engine = p.compile().unwrap();
    for enabled in [false, true, false] {
        engine.set_port_lights_enabled(enabled);
        for sample in render(&mut engine, 8) {
            assert_eq!(sample.map(|s| s.value), [10.0, 0.0, 10.0, 0.0, 20.0, 0.0]);
            assert!(sample.iter().all(|s| s.active));
        }
    }
    let mut selected = p.clone();
    selected.output_port = "out1".into();
    for sample in render(&mut selected.compile().unwrap(), 8) {
        assert_eq!(sample[4].value, 20.0);
    }
    let mut wired = p;
    wired
        .nodes
        .push(serde_json::from_value(json!({"id":"meter","kind":"poly_viz"})).unwrap());
    wired.cables.push(
        serde_json::from_value(
            json!({"from":"scene","output":"out1","to":"meter","input":"signal"}),
        )
        .unwrap(),
    );
    for sample in render(&mut wired.compile().unwrap(), 8) {
        assert_eq!(sample[4].value, 10.0);
    }
    wired.cables.clear();
    for sample in render(&mut wired.compile().unwrap(), 8) {
        assert_eq!(sample[4].value, 20.0);
    }
}

#[test]
fn frame_scene_physical_zero_channel_inputs_break_all_and_individual_normaling() {
    let mut p = patch();
    p.nodes.push(
        serde_json::from_value(
            json!({"id":"empty","kind":"poly_constant","params":{"channels":0}}),
        )
        .unwrap(),
    );
    p.cables.push(
        serde_json::from_value(json!({"from":"empty","output":"out","to":"scene","input":"all"}))
            .unwrap(),
    );
    for sample in render(&mut p.compile().unwrap(), 8) {
        assert_eq!(sample.map(|s| s.value), [0.0; 6]);
    }
    p.cables[0].input = "in1".into();
    for sample in render(&mut p.compile().unwrap(), 8) {
        assert_eq!(sample.map(|s| s.value), [0.0, 0.0, 10.0, 0.0, 10.0, 0.0]);
    }
    p.cables.clear();
    p.nodes[0].params.insert("offset".into(), 5.0);
    for sample in render(&mut p.compile().unwrap(), 8) {
        assert_eq!(sample.map(|s| s.value), [5.0, 0.0, 5.0, 0.0, 10.0, 0.0]);
    }
}

fn with_keys() -> Patch {
    let mut p = patch();
    p.nodes[0].content = Some(ModuleContent::FrameScene(FrameSceneData {
        next_id: 2,
        keyframes: vec![
            FrameSceneKeyframe {
                id: 0,
                timestamp: 0,
                values: [0, 16384, 32768, 65535],
            },
            FrameSceneKeyframe {
                id: 1,
                timestamp: 65535,
                values: [65535, 32768, 16384, 0],
            },
        ],
    }));
    p
}

#[test]
fn frame_scene_curves_response_and_saved_content_drive_actual_bounded_volts() {
    let mut p = with_keys();
    p.nodes[0].params.insert("frame".into(), 0.5);
    let values = render(&mut p.compile().unwrap(), 1)[0];
    assert!((values[0].value - 5.0).abs() < 0.02);
    assert!((values[1].value - 3.75).abs() < 0.02);
    assert!((values[2].value - 3.75).abs() < 0.02);
    p.nodes[0].params.insert("response1".into(), 1.0);
    let exponential = render(&mut p.compile().unwrap(), 1)[0];
    assert!(exponential[0].value < values[0].value * 0.5);
    for mode in 0..6 {
        p.nodes[0].params.insert("easing1".into(), mode as f32);
        for frame in [0.0, 0.25, 0.5, 0.75, 1.0] {
            p.nodes[0].params.insert("frame".into(), frame);
            let serialized = serde_json::to_string(&p).unwrap();
            let reopened: Patch = serde_json::from_str(&serialized).unwrap();
            for sample in render(&mut reopened.compile().unwrap(), 3) {
                assert!(sample
                    .iter()
                    .all(|s| s.value.is_finite() && (0.0..=40.0).contains(&s.value)));
            }
        }
    }
}

#[test]
fn frame_scene_crossing_pulses_adopt_with_irregular_blocks_and_content_edits_prime() {
    let mut p = with_keys();
    let mut old = p.compile().unwrap();
    assert_eq!(render(&mut old, 1)[0][5].value, 0.0);
    p.nodes[0].params.insert("frame".into(), 1.0);
    let mut moved = p.compile().unwrap();
    moved.adopt_state_from(&old);
    let a = render(&mut moved, 7);
    assert!(a.iter().all(|s| s[5].value == 5.0));
    let mut replacement = p.compile().unwrap();
    replacement.adopt_state_from(&moved);
    let a = render(&mut replacement, 42);
    assert!(a[..41].iter().all(|s| s[5].value == 5.0));
    assert_eq!(a[41][5].value, 0.0);
    let Some(ModuleContent::FrameScene(data)) = &mut p.nodes[0].content else {
        unreachable!()
    };
    data.apply(FrameSceneEdit::Add {
        timestamp: 32000,
        values: [12345; 4],
    })
    .unwrap();
    let mut edited = p.compile().unwrap();
    edited.adopt_state_from(&replacement);
    assert_eq!(render(&mut edited, 1)[0][5].value, 0.0);
    let h = edited.resolve_module_lights("scene").unwrap();
    assert!(edited.initialize_module_state(h));
    let initialized = render(&mut edited, 1)[0];
    let fresh = render(&mut p.compile().unwrap(), 1)[0];
    assert_eq!(initialized.map(|s| s.value), fresh.map(|s| s.value));
}

#[test]
fn frame_scene_sixty_four_keys_and_source_boundary_extensions_remain_safe() {
    let mut p = patch();
    p.nodes[0].params.insert("frame".into(), 1.0);
    p.nodes[0].params.insert("easing1".into(), 5.0);
    p.nodes[0].content = Some(ModuleContent::FrameScene(FrameSceneData {
        next_id: 64,
        keyframes: (0..64)
            .map(|i| FrameSceneKeyframe {
                id: i,
                timestamp: i * 1000,
                values: if i % 2 == 0 { [65535; 4] } else { [0; 4] },
            })
            .collect(),
    }));
    for sample in render(&mut p.compile().unwrap(), 7) {
        assert!(sample.iter().all(|s| s.value.is_finite()));
    }
    p.nodes[0].params.insert("frame".into(), 32000.0 / 65535.0);
    for sample in render(&mut p.compile().unwrap(), 7) {
        assert!(sample.iter().all(|s| s.value.is_finite()));
    }
    p.nodes[0].params.insert("frame".into(), 32500.0 / 65535.0);
    for sample in render(&mut p.compile().unwrap(), 7) {
        assert!(sample.iter().all(|s| s.value.is_finite()));
    }
    let Some(ModuleContent::FrameScene(data)) = &mut p.nodes[0].content else {
        unreachable!()
    };
    data.keyframes.swap(0, 1);
    assert!(p.compile().is_err());
}

#[test]
fn frame_scene_bypass_freezes_real_observations_and_emits_mono_zero() {
    let p = with_keys();
    let mut old = p.compile().unwrap();
    render(&mut old, 6);
    let h = old.resolve_module_lights("scene").unwrap();
    let before = old.module_lights(h).unwrap();
    let mut bypassed = p.clone();
    bypassed.nodes[0].bypassed = true;
    let mut bypass = bypassed.compile().unwrap();
    bypass.adopt_state_from(&old);
    let mut frames = [PortFrame::default(); 17];
    bypass.process_poly_block(&mut frames);
    assert!(frames
        .iter()
        .all(|f| f.channels == 1 && f.values == [0.0; 16]));
    let h = bypass.resolve_module_lights("scene").unwrap();
    let after = bypass.module_lights(h).unwrap();
    assert_eq!(before.brightness, after.brightness);
    assert_eq!(before.waveform_position, after.waveform_position);
}
