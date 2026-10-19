use openpatch_core::{
    Engine, ModuleContent, Patch, PortFrame, RecallRandomCaptureStatus, RecallRandomData,
    RecallRandomDegree, RecallRandomScale, TapSample,
};
use serde_json::{json, Value};
fn patch(rate: u32) -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":rate,"nodes":[
        {"id":"pitch","kind":"constant","params":{"value":0}},
        {"id":"clock","kind":"constant","params":{"value":0}},
        {"id":"recall","kind":"recall_random","params":{"seed":17,"t_rate":60,"t_range":2,"t_jitter":0.6,"t_repeat":1,"x_repeat":1,"x_steps":0.8}}],
        "cables":[],"output":"recall","output_port":"x1"})).unwrap()
}
fn render(engine: &mut Engine, n: usize) -> Vec<[TapSample; 7]> {
    let taps = ["t1", "t2", "t3", "x1", "x2", "x3", "y"]
        .map(|p| Some(engine.resolve_tap("recall", p, 0).unwrap()));
    let mut audio = vec![[0.; 2]; n];
    let mut samples = vec![[TapSample::default(); 7]; n];
    engine.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    samples
}
fn values(frames: Vec<[TapSample; 7]>) -> Vec<[u32; 7]> {
    frames
        .into_iter()
        .map(|f| f.map(|s| s.value.to_bits()))
        .collect()
}
fn replace(p: &Patch, engine: &mut Engine) {
    let mut next = p.compile().unwrap();
    next.adopt_state_from(engine);
    *engine = next;
}
fn wire(p: &mut Patch, from: &str, input: &str) {
    p.cables.push(
        serde_json::from_value(json!({"from":from,"output":"out","to":"recall","input":input}))
            .unwrap(),
    );
}
#[test]
fn callback_partitions_observation_and_compatible_graph_replacement_preserve_coupled_streams() {
    for rate in [8000, 32000, 44100, 48000, 192000] {
        let p = patch(rate);
        let expected = values(render(&mut p.compile().unwrap(), 4097));
        let mut engine = p.compile().unwrap();
        engine.set_port_lights_enabled(true);
        let mut actual = Vec::new();
        for n in [1, 4, 0, 31, 129, 500, 3432] {
            actual.extend(values(render(&mut engine, n)));
            replace(&p, &mut engine);
        }
        assert_eq!(actual, expected, "rate {rate}");
        assert!(expected.iter().any(|f| f[3] != 0));
    }
}
#[test]
fn bypass_freezes_all_bridge_rng_and_loop_state_and_fresh_lifecycle_restarts() {
    let p = patch(48000);
    let mut original = p.compile().unwrap();
    render(&mut original, 2137);
    let mut bypass_patch = p.clone();
    bypass_patch.nodes[2].bypassed = true;
    let mut bypass = bypass_patch.compile().unwrap();
    bypass.adopt_state_from(&original);
    assert!(render(&mut bypass, 137)
        .iter()
        .flatten()
        .all(|s| s.active && s.value == 0.));
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&bypass);
    assert_eq!(
        values(render(&mut resumed, 2048)),
        values(render(&mut original, 2048))
    );
    let handle = resumed.resolve_module_lights("recall").unwrap();
    assert!(resumed.initialize_module_state(handle));
    assert_eq!(
        values(render(&mut resumed, 2048)),
        values(render(&mut p.compile().unwrap(), 2048))
    );
    let mut changed = p.clone();
    changed.nodes[2].params.insert("seed".into(), 23.);
    replace(&changed, &mut resumed);
    assert_eq!(
        values(render(&mut resumed, 2048)),
        values(render(&mut changed.compile().unwrap(), 2048))
    );
    let reopened: Patch = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(
        values(render(&mut p.compile().unwrap(), 2048)),
        values(render(&mut reopened.compile().unwrap(), 2048))
    );
}
#[test]
fn receiving_controls_scales_and_selector_survive_runtime_adoption() {
    let p = patch(32000);
    let mut old = p.compile().unwrap();
    render(&mut old, 997);
    let mut changed = p.clone();
    changed.nodes[2].params.insert("x_steps".into(), 1.);
    changed.nodes[2].params.insert("y_steps".into(), 1.);
    let mut data = RecallRandomData::default();
    data.active_slot = 5;
    data.scales[5] = RecallRandomScale {
        base_interval: 1.,
        degrees: vec![
            RecallRandomDegree {
                voltage: -0.01,
                weight: 255,
            },
            RecallRandomDegree {
                voltage: 0.37,
                weight: 128,
            },
        ],
    };
    changed.nodes[2].content = Some(ModuleContent::RecallRandom(data));
    let mut receiving = changed.compile().unwrap();
    receiving.adopt_state_from(&old);
    let a = render(&mut receiving, 8192);
    let b = render(&mut old, 8192);
    assert!(
        a.iter()
            .zip(&b)
            .all(|(a, b)| a[..3].iter().zip(&b[..3]).all(|(a, b)| a.value == b.value)),
        "scale edits retain T/RNG traversal"
    );
    assert!(a.iter().zip(&b).any(|(a, b)| a[3].value != b[3].value));
}
#[test]
fn weighted_content_validates_negative_degrees_and_rejects_empty_masks_and_bad_order() {
    let mut data = RecallRandomData::default();
    data.scales[0].degrees[0].voltage = -0.001;
    data.validate().unwrap();
    let content = ModuleContent::RecallRandom(data.clone());
    let json = serde_json::to_value(&content).unwrap();
    assert_eq!(json["type"], "recall_random");
    assert_eq!(
        serde_json::from_value::<ModuleContent>(json).unwrap(),
        content
    );
    data.scales[0]
        .degrees
        .iter_mut()
        .for_each(|d| d.weight = 254);
    assert!(data.validate().is_err());
    data = RecallRandomData::default();
    data.scales[0].degrees[1].voltage = -1.;
    assert!(data.validate().is_err());
    data = RecallRandomData::default();
    data.scales[0].base_interval = f32::NAN;
    assert!(data.validate().is_err());
    data = RecallRandomData::default();
    data.active_slot = 6;
    assert!(data.validate().is_err());
    let mut p = patch(32000);
    let mut data = RecallRandomData::default();
    data.scales[0].base_interval = f32::MIN_POSITIVE;
    p.nodes[2].content = Some(ModuleContent::RecallRandom(data));
    p.nodes[2].params.insert("x_steps".into(), 1.);
    assert!(render(&mut p.compile().unwrap(), 4096)
        .iter()
        .flatten()
        .all(|s| s.value.is_finite()));
}
#[test]
fn capture_is_bounded_transient_tagged_and_survives_compatible_replacement() {
    let mut p = patch(32000);
    wire(&mut p, "pitch", "register_cv");
    wire(&mut p, "clock", "x_clock");
    let mut engine = p.compile().unwrap();
    let h = engine.resolve_module_lights("recall").unwrap();
    assert!(engine.begin_recall_random_capture(h, 2, 42));
    assert!(!engine.begin_recall_random_capture(h, 3, 43));
    for voltage in [0.0, 0.25, 0.5, 0.75] {
        p.nodes[0].params.insert("value".into(), voltage);
        p.nodes[1].params.insert("value".into(), 0.);
        replace(&p, &mut engine);
        render(&mut engine, 300);
        p.nodes[1].params.insert("value".into(), 5.);
        replace(&p, &mut engine);
        render(&mut engine, 100);
        p.nodes[1].params.insert("value".into(), 0.);
        replace(&p, &mut engine);
        render(&mut engine, 100);
    }
    let h = engine.resolve_module_lights("recall").unwrap();
    assert!(engine.stop_recall_random_capture(h));
    let receipt = engine.recall_random_capture(h).unwrap();
    assert_eq!(receipt.status, RecallRandomCaptureStatus::Complete);
    assert_eq!((receipt.slot, receipt.revision), (2, 42));
    assert!((2..=16).contains(&receipt.scale.num_degrees));
    receipt.scale.to_scale().validate().unwrap();
    assert!(p.nodes[2].content.is_none());
    assert!(engine.begin_recall_random_capture(h, 1, 43));
    assert!(engine.cancel_recall_random_capture(h));
    let cancelled = engine.recall_random_capture(h).unwrap();
    assert_eq!(cancelled.status, RecallRandomCaptureStatus::Cancelled);
    assert!(cancelled.capture_id > receipt.capture_id);
    assert!(!engine.stop_recall_random_capture(h));
    assert!(engine.begin_recall_random_capture(h, 0, 44));
    assert!(engine.stop_recall_random_capture(h));
    assert_eq!(
        engine.recall_random_capture(h).unwrap().status,
        RecallRandomCaptureStatus::Empty
    );
    let empty_id = engine.recall_random_capture(h).unwrap().capture_id;
    assert!(engine.initialize_module_state(h));
    assert!(engine.recall_random_capture(h).is_none());
    assert!(engine.begin_recall_random_capture(h, 3, 44));
    assert!(engine.recall_random_capture(h).unwrap().capture_id > empty_id);
    assert!(engine.cancel_recall_random_capture(h));
    let mut next = p.compile().unwrap();
    assert!(!next.begin_recall_random_capture(h, 0, 44));
    let h2 = next.resolve_module_lights("recall").unwrap();
    assert!(next.begin_recall_random_capture(h2, 0, 44));
    let mut outputs = [PortFrame::default(); 1];
    next.process_poly_block(&mut outputs);
}
#[test]
fn physical_x_clock_zero_channels_breaks_internal_normal_and_disconnected_reset_stays_low() {
    let mut p = patch(32000);
    p.nodes[1] = serde_json::from_value(
        json!({"id":"clock","kind":"poly_constant","params":{"channels":0}}),
    )
    .unwrap();
    let normal = values(render(&mut p.compile().unwrap(), 8192));
    wire(&mut p, "clock", "x_clock");
    let external = values(render(&mut p.compile().unwrap(), 8192));
    assert_ne!(normal, external);
    assert!(
        external
            .iter()
            .all(|f| f[3..].iter().all(|v| *v == 0.0_f32.to_bits())),
        "unclocked external X/Y remains silent"
    );
}
#[test]
fn catalog_declares_typed_weighted_content_and_functional_custom_identity() {
    let catalog: Value = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "recall_random")
        .unwrap();
    assert_eq!(module["model"], "functional");
    assert!(module["tags"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "Custom"));
    assert_eq!(
        module["content_schema"]["properties"]["scales"]["maxItems"],
        6
    );
}

#[test]
fn actual_engine_matches_genuine_cpp_all_t_models_and_six_weighted_scales_at_declared_latency() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/recall-random-native-host.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut p = patch(32000);
        p.nodes[2].params = serde_json::from_value(case["params"].clone()).unwrap();
        let mut data = RecallRandomData::default();
        data.active_slot = case["active_slot"].as_u64().unwrap() as u8;
        p.nodes[2].content = Some(ModuleContent::RecallRandom(data));
        let expected = case["source_gate_highs_and_cv_bits"].as_array().unwrap();
        let actual = render(&mut p.compile().unwrap(), expected.len() + 8);
        for (source_frame, expected) in expected.iter().enumerate() {
            for port in 0..3 {
                assert_eq!(
                    actual[source_frame + 7][port].value,
                    expected[port].as_u64().unwrap() as f32 * 5.,
                    "{},source frame{source_frame},gate{port}",
                    case["id"]
                );
            }
            for port in 3..7 {
                assert_eq!(
                    actual[source_frame + 6][port].value.to_bits(),
                    expected[port].as_u64().unwrap() as u32,
                    "{},source frame{source_frame},CV{port}",
                    case["id"]
                );
            }
        }
    }
}

#[test]
fn randomize_and_initialize_keep_authored_weighted_scales_and_randomize_keeps_seed() {
    let actions = openpatch_core::ModuleParameterActions::new(48000);
    let mut p = patch(48000);
    let mut data = RecallRandomData::default();
    data.active_slot = 4;
    data.scales[4].degrees[0].voltage = -0.02;
    p.nodes[2].content = Some(ModuleContent::RecallRandom(data));
    let content = p.nodes[2].content.clone();
    actions.randomize(&mut p.nodes[2], 397).unwrap();
    assert_eq!(p.nodes[2].params["seed"], 17.);
    assert_eq!(p.nodes[2].content, content);
    p.compile().unwrap();
    actions.initialize(&mut p.nodes[2]).unwrap();
    assert_eq!(p.nodes[2].content, content);
    p.compile().unwrap();
}
