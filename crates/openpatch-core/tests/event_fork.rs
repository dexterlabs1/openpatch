use openpatch_core::{Engine, Patch, PortFrame, TapSample};
use serde_json::json;

fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"gate","kind":"constant","params":{"value":0}},
            {"id":"fork","kind":"event_fork","params":{"seed":17,"probability1":0.5,"probability2":0.25,"toggle1":1,"latch2":1}}],
        "cables":[{"from":"gate","output":"out","to":"fork","input":"gate1"}],
        "output":"fork","output_port":"a1"})).unwrap()
}
fn render(engine: &mut Engine, frames: usize) -> Vec<[TapSample; 4]> {
    let taps =
        ["a1", "b1", "a2", "b2"].map(|port| Some(engine.resolve_tap("fork", port, 0).unwrap()));
    let mut audio = vec![[0.0; 2]; frames];
    let mut samples = vec![[TapSample::default(); 4]; frames];
    engine.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    samples
}

#[test]
fn event_fork_live_graph_matches_original_firmware_with_declared_one_frame_cable_delay() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/event-fork-native-graph.json")).unwrap();
    let gates = fixture["gates"].as_array().unwrap();
    let expected = fixture["output_highs"].as_array().unwrap();
    let mut p = patch();
    p.nodes[0].params.insert("value".into(), 5.0);
    let mut engine = p.compile().unwrap();
    engine.set_port_lights_enabled(true);
    for (frame, values) in expected.iter().enumerate() {
        // Supply the following frame's physical input one frame early. The
        // source fixture remains unchanged; native graph cables have delay1.
        let next = gates
            .get(frame + 1)
            .unwrap_or(&gates[frame])
            .as_u64()
            .unwrap() as f32
            * 5.0;
        if p.nodes[0].params["value"] != next {
            p.nodes[0].params.insert("value".into(), next);
            let mut replacement = p.compile().unwrap();
            replacement.adopt_state_from(&engine);
            engine = replacement;
        }
        let actual = render(&mut engine, 1)[0];
        for (i, expected) in values.as_array().unwrap().iter().enumerate() {
            assert!(actual[i].active, "frame {frame}, port {i}");
            assert_eq!(
                actual[i].value,
                expected.as_u64().unwrap() as f32 * 5.0,
                "frame {frame}, port {i}"
            );
        }
    }
}

#[test]
fn event_fork_physical_zero_channel_gate2_breaks_normal_while_taps_do_not() {
    let mut p = patch();
    p.nodes[0].params.insert("value".into(), 5.0);
    p.nodes[1].params.insert("probability1".into(), 0.0);
    p.nodes[1].params.insert("probability2".into(), 1.0);
    p.nodes[1].params.insert("toggle1".into(), 0.0);
    let normal = render(&mut p.compile().unwrap(), 4);
    assert_eq!(normal[3].map(|s| s.value), [5.0, 0.0, 0.0, 5.0]);
    p.nodes.push(
        serde_json::from_value(
            json!({"id":"empty","kind":"poly_constant","params":{"channels":0}}),
        )
        .unwrap(),
    );
    p.cables.push(
        serde_json::from_value(json!({"from":"empty","output":"out","to":"fork","input":"gate2"}))
            .unwrap(),
    );
    let mut wired = p.compile().unwrap();
    wired.set_port_lights_enabled(true);
    assert_eq!(
        render(&mut wired, 4)[3].map(|s| s.value),
        [5.0, 0.0, 0.0, 0.0]
    );
}

#[test]
fn event_fork_observation_and_callback_partitions_do_not_change_random_decisions() {
    let mut p = patch();
    p.nodes[0] = serde_json::from_value(
        json!({"id":"gate","kind":"stock_lfo","params":{"rate":8,"pulse_width":0.3}}),
    )
    .unwrap();
    p.cables[0].output = "square".into();
    let mut whole = p.compile().unwrap();
    let expected = render(&mut whole, 1537);
    let mut split = p.compile().unwrap();
    split.set_port_lights_enabled(true);
    let mut actual = Vec::new();
    for count in [1, 7, 0, 128, 300, 1101] {
        actual.extend(render(&mut split, count));
    }
    assert_eq!(actual.len(), expected.len());
    assert!(actual.iter().zip(&expected).all(|(a, b)| a
        .iter()
        .zip(b)
        .all(|(a, b)| a.active == b.active && a.value.to_bits() == b.value.to_bits())));
    assert!(actual.iter().any(|frame| frame[0].value == 5.0));
    assert!(actual.iter().any(|frame| frame[1].value == 5.0));
}

#[test]
fn event_fork_bypass_outputs_mono_zero_and_freezes_routing_memory() {
    let p = patch();
    let mut old = p.compile().unwrap();
    render(&mut old, 257);
    let handle = old.resolve_module_lights("fork").unwrap();
    let before = old.module_lights(handle).unwrap();
    let mut bypassed = p.clone();
    bypassed.nodes[1].bypassed = true;
    let mut bypass = bypassed.compile().unwrap();
    bypass.adopt_state_from(&old);
    let mut outputs = [PortFrame::default(); 67];
    bypass.process_poly_block(&mut outputs);
    assert!(outputs
        .iter()
        .all(|p| p.channels == 1 && p.channel(0) == 0.0));
    let handle = bypass.resolve_module_lights("fork").unwrap();
    assert_eq!(
        bypass.module_lights(handle).unwrap().brightness,
        before.brightness
    );
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&bypass);
    assert_eq!(
        render(&mut resumed, 99)
            .iter()
            .map(|f| f.map(|s| s.value.to_bits()))
            .collect::<Vec<_>>(),
        render(&mut old, 99)
            .iter()
            .map(|f| f.map(|s| s.value.to_bits()))
            .collect::<Vec<_>>()
    );
}
