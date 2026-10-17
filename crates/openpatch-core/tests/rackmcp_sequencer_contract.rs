//! Public native workflow checks. These exercise host scheduling and lifecycle;
//! they do not establish whole-module or whole-patch Rack equivalence.
use openpatch_core::{Engine, Node, Patch, PortFrame};
use serde_json::{json, Value};

fn node(value: Value) -> Node {
    serde_json::from_value(value).unwrap()
}

fn sequence(state: Value) -> Node {
    node(
        json!({"id":"seq","kind":"rackmcp_sequencer","params":{"running":1},
        "rackmcp_sequence":state}),
    )
}

fn notes() -> Value {
    json!({"data":{"patterns":[{"name":"A","length":2,"tracks":[{
        "steps":[{"step":0,"notes":[0]},{"step":1,"notes":[12]}]
    }]}]}})
}

fn automation(mode: &str, value: f32) -> Value {
    let mut state = notes();
    state["data"]["lanes"] = json!([{"moduleId":"42","paramId":0,"mode":mode,
        "points":[{"bar":0,"value":value}]}]);
    state["targets"] = json!([{"source_module_id":"42","source_param_id":0,
        "module":"vca","parameter":"level"}]);
    state
}

fn patch(state: Value) -> Patch {
    Patch {
        sample_rate: 8000,
        nodes: vec![sequence(state)],
        output: Some("seq".into()),
        output_port: "pitch1".into(),
        ..Patch::new_v2()
    }
}

fn with_vca(state: Value) -> Patch {
    let mut patch = patch(state);
    patch.nodes.push(node(
        json!({"id":"vca","kind":"stock_vca","params":{"level":0.8}}),
    ));
    patch
}

fn render(engine: &mut Engine, frames: usize) -> Vec<PortFrame> {
    let mut output = vec![PortFrame::default(); frames];
    engine.process_poly_block(&mut output);
    output
}

fn value(engine: &Engine, module: &str, parameter: &str) -> f32 {
    let handle = engine.resolve_parameter(module, parameter).unwrap();
    engine.parameter_value(handle).unwrap()
}

#[test]
fn all_eighteen_named_outputs_have_source_polyphony_and_voltages() {
    let tracks: Vec<_> = (0..4)
        .map(|track| {
            json!({"voices":track+1,"steps":[{"step":0,
            "notes":[track*12,(track+1)*12],"vel":(track+1) as f32*0.25,"accent":true}]})
        })
        .collect();
    let state = json!({"data":{"patterns":[{"name":"A","length":16,"tracks":tracks}]}});
    for group in ["pitch", "gate", "velocity", "accent"] {
        for track in 0..4 {
            let mut patch = patch(state.clone());
            patch.output_port = format!("{group}{}", track + 1);
            let frames = render(&mut patch.compile().unwrap(), 9);
            assert_eq!(frames[8].channels, track + 1, "{}", patch.output_port);
            for voice in 0..usize::from(track + 1) {
                let expected = match group {
                    "pitch" => f32::from(track) + (voice % 2) as f32,
                    "velocity" => f32::from(track + 1) * 2.5,
                    _ => 10.0,
                };
                assert_eq!(
                    frames[8].channel(voice),
                    expected,
                    "{} voice {voice}",
                    patch.output_port
                );
            }
            if matches!(group, "gate" | "accent") {
                assert_eq!(frames[0].channel(0), 0.0); // source's 1 ms retrigger gap
            }
            assert!(frames[8].values[usize::from(track + 1)..]
                .iter()
                .all(|v| *v == 0.0));
        }
    }
    for port in ["clock", "reset"] {
        let mut patch = patch(state.clone());
        patch.output_port = port.into();
        let frames = render(&mut patch.compile().unwrap(), 9);
        assert!(frames.iter().all(|frame| frame.channels == 1));
        assert_eq!(
            frames[0].channel(0),
            if port == "clock" { 10.0 } else { 0.0 }
        );
        assert_eq!(frames[8].channel(0), 0.0);
    }
}

#[test]
fn selected_pattern_survives_serialization_and_lands_on_first_advance() {
    let state = json!({"data":{"seed":19,"selectedPattern":"B","patterns":[
        {"name":"A","length":2,"tracks":[{"steps":[{"step":0,"notes":[0]}]}]},
        {"name":"B","length":2,"tracks":[{"steps":[{"step":1,"notes":[24]}]}]}
    ]}});
    let original = patch(state);
    let saved = serde_json::to_value(&original).unwrap();
    assert_eq!(
        saved["nodes"][0]["rackmcp_sequence"]["data"]["selectedPattern"],
        "B"
    );
    let reopened: Patch = serde_json::from_value(saved).unwrap();
    assert_eq!(
        original.nodes[0].rackmcp_sequence,
        reopened.nodes[0].rackmcp_sequence
    );
    let frames = render(&mut reopened.compile().unwrap(), 1001);
    assert!(frames[..1000].iter().all(|frame| frame.channel(0) == 0.0));
    assert_eq!(frames[1000].channel(0), 2.0);
    assert_eq!(render(&mut original.compile().unwrap(), 1001), frames);
}

#[test]
fn explicit_source_pairs_map_normalized_endpoints_and_clamp_native_raw_values() {
    for (mode, gain, cutoff) in [
        ("normalized", 0.0, 1.0),
        ("normalized", 1.0, 0.0),
        ("raw", -2.0, 100.0),
    ] {
        let mut state = automation(mode, gain);
        state["data"]["lanes"].as_array_mut().unwrap().push(json!({
            "moduleId":"99","paramId":4,"mode":mode,"points":[{"bar":0,"value":cutoff}]
        }));
        // Saved target order deliberately differs from source lane order.
        state["targets"].as_array_mut().unwrap().insert(0, json!({
            "source_module_id":"99","source_param_id":4,"module":"filter","parameter":"frequency"
        }));
        let mut patch = with_vca(state);
        patch
            .nodes
            .push(node(json!({"id":"filter","kind":"rack264_vcf"})));
        let mut engine = patch.compile().unwrap();
        let gain_handle = engine.resolve_parameter("vca", "level").unwrap();
        let cutoff_handle = engine.resolve_parameter("filter", "frequency").unwrap();
        let gain_bounds = engine.parameter_descriptor(gain_handle).unwrap();
        let cutoff_bounds = engine.parameter_descriptor(cutoff_handle).unwrap();
        render(&mut engine, 31);
        assert_eq!(engine.parameter_value(gain_handle), Some(0.8));
        render(&mut engine, 1);
        let expected = |input: f32, lo: f32, hi: f32| {
            if mode == "normalized" {
                lo + input * (hi - lo)
            } else {
                input.clamp(lo, hi)
            }
        };
        assert_eq!(
            engine.parameter_value(gain_handle),
            Some(expected(gain, gain_bounds.min, gain_bounds.max))
        );
        assert_eq!(
            engine.parameter_value(cutoff_handle),
            Some(expected(cutoff, cutoff_bounds.min, cutoff_bounds.max))
        );
    }
}

#[test]
fn equal_authored_assignment_survives_lane_cache_until_changed_curve_writes() {
    let mut patch = with_vca(automation("normalized", 0.25));
    let mut old = patch.compile().unwrap();
    render(&mut old, 32);
    assert_eq!(value(&old, "vca", "level"), 0.25);

    // The authored knob is already .8. An explicit .8 -> .8 assignment still
    // overrides its current effective .25; assignment intent is host supplied.
    let mut assigned = patch.compile().unwrap();
    let assigned_handle = assigned.resolve_parameter("vca", "level").unwrap();
    assigned.adopt_state_from(&old);
    assert_eq!(
        assigned.inherit_parameter_values(&old, &[], &[assigned_handle]),
        1 // The unassigned sequencer running value also inherits.
    );
    render(&mut assigned, 96);
    assert_eq!(assigned.parameter_value(assigned_handle), Some(0.8));

    patch.nodes[0].rackmcp_sequence.as_mut().unwrap().data.lanes[0].points[0].value = 0.4;
    let mut edited = patch.compile().unwrap();
    edited.adopt_state_from(&assigned);
    edited.inherit_parameter_values(&assigned, &[], &[]);
    render(&mut edited, 31);
    assert_eq!(value(&edited, "vca", "level"), 0.8);
    render(&mut edited, 1);
    assert_eq!(value(&edited, "vca", "level"), 0.4);
}

#[test]
fn unresolved_native_targets_skip_but_present_unsupported_targets_are_rejected() {
    let state = automation("normalized", 0.25);
    let mut missing = patch(state.clone()).compile().unwrap();
    assert!(render(&mut missing, 1050)
        .iter()
        .all(|frame| frame.channel(0).is_finite()));

    let mut invalid = with_vca(state);
    invalid.nodes[0].rackmcp_sequence.as_mut().unwrap().targets[0].parameter = "unsupported".into();
    assert!(invalid
        .compile()
        .unwrap_err()
        .contains("live native parameter capability"));
    invalid.nodes[0].rackmcp_sequence.as_mut().unwrap().targets[0].parameter = "level".into();
    invalid.nodes[1] = node(json!({"id":"vca","kind":"constant"}));
    assert!(invalid
        .compile()
        .unwrap_err()
        .contains("live native parameter capability"));
}

#[test]
fn bypass_freezes_transport_divider_and_lane_cache_until_resumption() {
    let patch = with_vca(automation("normalized", 0.25));
    let mut original = patch.compile().unwrap();
    render(&mut original, 901);
    let gain = original.resolve_parameter("vca", "level").unwrap();
    assert!(original.set_parameter_value(gain, 0.8));
    let mut bypass_patch = patch.clone();
    bypass_patch.nodes[0].bypassed = true;
    let mut bypass = bypass_patch.compile().unwrap();
    bypass.adopt_state_from(&original);
    bypass.inherit_parameter_values(&original, &[], &[]);
    assert!(render(&mut bypass, 5000)
        .iter()
        .all(|frame| *frame == PortFrame::mono(0.0)));
    assert_eq!(value(&bypass, "vca", "level"), 0.8);
    let mut resumed = patch.compile().unwrap();
    resumed.adopt_state_from(&bypass);
    resumed.inherit_parameter_values(&bypass, &[], &[]);
    let expected = render(&mut original, 128);
    let actual = render(&mut resumed, 128);
    assert_eq!(actual, expected);
    assert_eq!(actual[98].channel(0), 0.0);
    assert_eq!(actual[99].channel(0), 1.0);
    assert_eq!(value(&resumed, "vca", "level"), 0.8);
}

#[test]
fn native_node_order_controls_same_sample_or_next_sample_destination_visibility() {
    for sequence_first in [true, false] {
        let sequence = sequence(automation("normalized", 0.25));
        let vca = node(json!({"id":"vca","kind":"stock_vca","params":{"level":1}}));
        let mut patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":8000,
            "nodes":[],"cables":[{"from":"signal","output":"out","to":"vca","input":"signal"}],
            "output":"vca","output_port":"out"}))
        .unwrap();
        patch.nodes.push(node(
            json!({"id":"signal","kind":"poly_constant","params":{"v1":5}}),
        ));
        if sequence_first {
            patch.nodes.extend([sequence, vca]);
        } else {
            patch.nodes.extend([vca, sequence]);
        }
        let frames = render(&mut patch.compile().unwrap(), 33);
        assert_eq!(frames[30].channel(0), 5.0);
        assert_eq!(
            frames[31].channel(0),
            if sequence_first { 1.25 } else { 5.0 }
        );
        assert_eq!(frames[32].channel(0), 1.25);
    }
}

#[test]
fn unrelated_reorder_and_rate_adoption_preserve_cache_even_when_target_is_recreated() {
    let mut patch = with_vca(automation("normalized", 0.25));
    patch.nodes.push(node(
        json!({"id":"unrelated","kind":"constant","params":{"value":1}}),
    ));
    let mut old = patch.compile().unwrap();
    render(&mut old, 1100);
    let gain = old.resolve_parameter("vca", "level").unwrap();
    assert!(old.set_parameter_value(gain, 0.8));
    for (rate, unrelated) in [(8000, 2.0), (48000, 3.0), (96000, 4.0)] {
        patch.sample_rate = rate;
        patch.nodes.reverse();
        patch
            .nodes
            .iter_mut()
            .find(|node| node.id == "unrelated")
            .unwrap()
            .params
            .insert("value".into(), unrelated);
        let mut next = patch.compile().unwrap();
        next.adopt_state_from(&old);
        assert_eq!(next.inherit_parameter_values(&old, &[], &[]), 2);
        assert_eq!(value(&next, "seq", "running"), 1.0);
        let expected = render(&mut old, 64);
        let actual = render(&mut next, 64);
        assert_eq!(actual, expected);
        assert!(actual.iter().all(|frame| frame.channel(0) == 1.0));
        assert_eq!(value(&next, "vca", "level"), 0.8);
        old = next;
    }
    let mut fresh = patch.compile().unwrap();
    let target = fresh.resolve_parameter("vca", "level").unwrap();
    fresh.adopt_state_from_excluding(&old, &[target.module()]);
    assert_eq!(
        fresh.inherit_parameter_values(&old, &[target.module()], &[]),
        1 // Fresh VCA is excluded; sequencer running remains continuous.
    );
    // 1100 + 3*64 samples leaves division at 12; the next write is 20 away.
    render(&mut fresh, 19);
    assert_eq!(fresh.parameter_value(target), Some(0.8));
    render(&mut fresh, 1);
    // The actual source keeps its last successful lane value even across
    // target disappearance/recreation. A constant lane leaves fresh defaults.
    assert_eq!(fresh.parameter_value(target), Some(0.8));
    let sequence = fresh.resolve_module_state("seq").unwrap();
    assert!(fresh.reload_rackmcp_sequence_data(sequence));
    render(&mut fresh, 32);
    assert_eq!(fresh.parameter_value(target), Some(0.25));
}

#[test]
fn explicit_equal_load_rearms_cache_without_resetting_playhead_and_handles_are_fenced() {
    let mut patch = with_vca(automation("normalized", 0.25));
    let mut loaded = patch.compile().unwrap();
    render(&mut loaded, 1100);
    let sequence = loaded.resolve_module_state("seq").unwrap();
    let gain = loaded.resolve_parameter("vca", "level").unwrap();
    assert!(loaded.set_parameter_value(gain, 0.75));
    let mut unchanged = patch.compile().unwrap();
    unchanged.adopt_state_from(&loaded);
    unchanged.inherit_parameter_values(&loaded, &[], &[]);
    let receiver = unchanged.resolve_module_state("seq").unwrap();
    assert!(unchanged.rackmcp_sequence_data_matches(receiver, &loaded, sequence));
    assert!(!unchanged.rackmcp_sequence_data_matches(sequence, &loaded, sequence));
    assert!(!unchanged.reload_rackmcp_sequence_data(sequence));
    assert!(!loaded.reload_rackmcp_sequence_data(gain.module()));
    assert!(loaded.reload_rackmcp_sequence_data(sequence));
    assert_eq!(render(&mut loaded, 1000), render(&mut unchanged, 1000));
    assert_eq!(value(&loaded, "vca", "level"), 0.25);
    assert_eq!(value(&unchanged, "vca", "level"), 0.75);
    // Native binding metadata is not source data, and parameter-only graph
    // edits must not masquerade as dataFromJson lifecycle events.
    patch.nodes[0].rackmcp_sequence.as_mut().unwrap().targets[0].module = "missing".into();
    let routing = patch.compile().unwrap();
    assert!(loaded.rackmcp_sequence_data_matches(
        sequence,
        &routing,
        routing.resolve_module_state("seq").unwrap()
    ));
    patch.nodes[0]
        .rackmcp_sequence
        .as_mut()
        .unwrap()
        .data
        .patterns[0]
        .length = 4;
    let changed = patch.compile().unwrap();
    assert!(!loaded.rackmcp_sequence_data_matches(
        sequence,
        &changed,
        changed.resolve_module_state("seq").unwrap()
    ));
}
