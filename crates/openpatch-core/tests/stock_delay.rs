use openpatch_core::*;
use serde_json::json;

const PORTS: [(&str, &str); 2] = [("delay", "mix"), ("delay", "wet")];
const TWO_PORTS: [(&str, &str); 4] = [
    ("delay", "mix"),
    ("delay", "wet"),
    ("other_delay", "mix"),
    ("other_delay", "wet"),
];

fn patch(two_delays: bool, clock: bool) -> Patch {
    let mut patch: Patch = serde_json::from_value(json!({
        "version":2,"sample_rate":8000,
        "nodes":[
            {"id":"audio","kind":"oscillator","params":{"frequency":137,"amplitude":0.7}},
            {"id":"clock","kind":"gate","params":{"frequency":9,"duty":0.27}},
            {"id":"delay","kind":"stock_delay","params":{"time":0.45,"feedback":0.73,"tone":0.61,"mix":0.8}}
        ],
        "cables":[{"id":"audio_delay","from":"audio","to":"delay","input":"signal"}],
        "output":"delay","output_port":"mix"
    })).unwrap();
    if clock {
        patch.cables.push(
            serde_json::from_value(json!({
                "id":"clock_delay","from":"clock","to":"delay","input":"clock"
            }))
            .unwrap(),
        );
    }
    if two_delays {
        patch.nodes.push(
            serde_json::from_value(json!({
                "id":"other_delay","kind":"stock_delay",
                "params":{"time":0.31,"feedback":0.67,"tone":0.4,"mix":0.6}
            }))
            .unwrap(),
        );
        patch.cables.push(
            serde_json::from_value(json!({
                "id":"audio_other","from":"audio","to":"other_delay","input":"signal"
            }))
            .unwrap(),
        );
        if clock {
            patch.cables.push(
                serde_json::from_value(json!({
                    "id":"clock_other","from":"clock","to":"other_delay","input":"clock"
                }))
                .unwrap(),
            );
        }
    }
    patch
}

fn stable_patch(two_delays: bool, clock: bool) -> Patch {
    let mut patch = patch(two_delays, clock);
    patch.nodes[0] = serde_json::from_value(json!({
        "id":"audio","kind":"poly_constant","params":{"channels":3,"v1":2,"v2":-1,"v3":3}
    }))
    .unwrap();
    patch.nodes[1] = serde_json::from_value(json!({
        "id":"clock","kind":"constant","params":{"value":0}
    }))
    .unwrap();
    patch
}

fn trace<const N: usize>(
    engine: &mut Engine,
    frames: usize,
    ports: [(&str, &str); N],
) -> Vec<[TapSample; N]> {
    let mut taps = [None; N];
    for (tap, (node, port)) in taps.iter_mut().zip(ports) {
        *tap = Some(engine.resolve_tap(node, port, 0).unwrap());
    }
    let mut samples = vec![[TapSample::default(); N]; frames];
    engine.process_audio_block_with_taps(&mut vec![[0.0; 2]; frames], &taps, &mut samples);
    samples
}

fn advance(engine: &mut Engine, mut count: usize) {
    let mut frames = [PortFrame::default(); 128];
    while count != 0 {
        let block = count.min(frames.len());
        engine.process_poly_block(&mut frames[..block]);
        count -= block;
    }
}

fn one(engine: &mut Engine) -> PortFrame {
    let mut frames = [PortFrame::default()];
    engine.process_poly_block(&mut frames);
    frames[0]
}

#[test]
fn catalog_controls_ports_defaults_and_v2_validation_are_explicit() {
    let catalog = catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "stock_delay")
        .unwrap();
    assert_eq!(
        module["inputs"],
        json!([
            "time_cv",
            "feedback_cv",
            "tone_cv",
            "mix_cv",
            "signal",
            "clock"
        ])
    );
    assert_eq!(module["outputs"], json!(["mix", "wet"]));
    assert_eq!(module["min_patch_version"], 2);
    let parameters = module["params"].as_object().unwrap();
    assert_eq!(parameters.len(), 8);
    assert!(
        (parameters["time"]["default"].as_f64().unwrap() - 500.0_f64.log10() / 4.0).abs() < 1e-7
    );
    for name in ["feedback", "tone", "mix"] {
        assert_eq!(parameters[name]["default"], 0.5);
    }
    for name in [
        "time_cv_amount",
        "feedback_cv_amount",
        "tone_cv_amount",
        "mix_cv_amount",
    ] {
        assert_eq!(parameters[name]["min"], -1.0);
        assert_eq!(parameters[name]["max"], 1.0);
        assert_eq!(parameters[name]["default"], 0.0);
        assert_eq!(parameters[name]["randomization"], "disabled");
    }
    let mut legacy = patch(false, false);
    legacy.version = 1;
    assert!(legacy.compile().is_err());
    for (name, spec) in parameters {
        for value in [
            spec["min"].as_f64().unwrap() as f32 - 0.001,
            spec["max"].as_f64().unwrap() as f32 + 0.001,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ] {
            let mut invalid = patch(false, false);
            invalid.nodes[2].params.insert(name.clone(), value);
            assert!(invalid.compile().is_err(), "accepted {name}={value}");
        }
    }
    let mut invalid = patch(false, false);
    invalid.nodes[2].params.insert("unknown".into(), 0.0);
    assert!(invalid.compile().is_err());
    let actions = ModuleParameterActions::new(8000);
    let mut node = patch(false, false).nodes.remove(2);
    for name in [
        "time_cv_amount",
        "feedback_cv_amount",
        "tone_cv_amount",
        "mix_cv_amount",
    ] {
        node.params.insert(name.into(), -0.75);
    }
    for seed in 0..16 {
        actions.randomize(&mut node, seed).unwrap();
        for name in [
            "time_cv_amount",
            "feedback_cv_amount",
            "tone_cv_amount",
            "mix_cv_amount",
        ] {
            assert_eq!(node.params[name], -0.75);
        }
    }
    actions.initialize(&mut node).unwrap();
    for name in [
        "time_cv_amount",
        "feedback_cv_amount",
        "tone_cv_amount",
        "mix_cv_amount",
    ] {
        assert_eq!(node.params[name], 0.0);
    }
}

#[test]
fn normal_audio_sums_channels_to_mono_on_both_outputs() {
    let poly = stable_patch(false, false);
    let mut mono = poly.clone();
    mono.nodes[0] = serde_json::from_value(json!({
        "id":"audio","kind":"constant","params":{"value":4}
    }))
    .unwrap();
    let expected = trace(&mut mono.compile().unwrap(), 2051, PORTS);
    let actual = trace(&mut poly.compile().unwrap(), 2051, PORTS);
    assert_eq!(actual, expected);
    assert!(actual.iter().any(|sample| sample[1].value.abs() > 0.001));
    for width in [0, 1, 5, 16] {
        for port in ["mix", "wet"] {
            let mut selected = poly.clone();
            selected.output_port = port.into();
            selected.nodes[0]
                .params
                .insert("channels".into(), width as f32);
            let mut engine = selected.compile().unwrap();
            let mut frames = [PortFrame::default(); 129];
            engine.process_poly_block(&mut frames);
            for frame in frames {
                assert_eq!(frame.channels, 1);
                assert_eq!(&frame.values[1..], &[0.0; 15]);
            }
        }
    }
}

#[test]
fn auxiliary_inputs_read_only_channel_zero() {
    let mut poly = stable_patch(false, false);
    for (index, name) in ["time_cv", "feedback_cv", "tone_cv", "mix_cv", "clock"]
        .into_iter()
        .enumerate()
    {
        let id = format!("aux{index}");
        poly.nodes.push(
            serde_json::from_value(json!({
                "id":id,"kind":"poly_constant",
                "params":{"channels":16,"v1":if index == 4 {0.0} else {0.3},"v2":9,"v7":-8,"v16":10}
            }))
            .unwrap(),
        );
        poly.cables.push(
            serde_json::from_value(json!({
                "id":format!("aux_edge{index}"),"from":id,"to":"delay","input":name
            }))
            .unwrap(),
        );
    }
    for name in [
        "time_cv_amount",
        "feedback_cv_amount",
        "tone_cv_amount",
        "mix_cv_amount",
    ] {
        poly.nodes[2].params.insert(name.into(), 1.0);
    }
    let mut mono = poly.clone();
    for node in &mut mono.nodes[3..] {
        node.params.insert("channels".into(), 1.0);
    }
    assert_eq!(
        trace(&mut poly.compile().unwrap(), 4099, PORTS),
        trace(&mut mono.compile().unwrap(), 4099, PORTS)
    );
}

#[test]
fn processing_continues_without_consumers_or_taps() {
    let selected = patch(false, true);
    let mut disconnected = selected.clone();
    disconnected.output = None;
    disconnected.output_port = "out".into();
    let mut audible = selected.compile().unwrap();
    let mut idle = disconnected.compile().unwrap();
    advance(&mut audible, 4099);
    advance(&mut idle, 4099);
    let expected = trace(&mut audible, 513, PORTS);
    assert!(expected.iter().any(|sample| sample[1].value.abs() > 0.001));
    assert_eq!(trace(&mut idle, 513, PORTS), expected);
}

#[test]
fn mutable_handoff_matches_independent_copy_with_clocks_partial_queues_and_unrelated_edits() {
    for offset in [1, 15, 17, 129] {
        let patch = patch(true, true);
        let mut donor = patch.compile().unwrap();
        advance(&mut donor, 4096 + offset);
        let mut edited = patch.clone();
        edited.nodes.push(
            serde_json::from_value(json!({
                "id":"unrelated","kind":"constant","params":{"value":7}
            }))
            .unwrap(),
        );
        edited.nodes.reverse();
        let mut expected = edited.compile().unwrap();
        expected.adopt_state_from(&donor);
        let mut moved = edited.compile().unwrap();
        assert!(moved.handoff_state_from(&mut donor));
        let expected = trace(&mut expected, 1027, TWO_PORTS);
        assert!(expected.iter().any(|sample| sample[1].value.abs() > 0.001));
        assert!(expected.iter().any(|sample| sample[3].value.abs() > 0.001));
        assert_eq!(trace(&mut moved, 1027, TWO_PORTS), expected);
    }
}

#[test]
fn immutable_adoption_is_an_independent_copy() {
    let patch = patch(false, true);
    let mut donor = patch.compile().unwrap();
    advance(&mut donor, 4099);
    let mut a = patch.compile().unwrap();
    let mut b = patch.compile().unwrap();
    a.adopt_state_from(&donor);
    b.adopt_state_from(&donor);
    advance(&mut donor, 1025);
    assert_eq!(trace(&mut a, 513, PORTS), trace(&mut b, 513, PORTS));
}

#[test]
fn handoff_retains_receiving_parameters() {
    let mut before = stable_patch(false, false);
    before.nodes[2].params.insert("mix".into(), 1.0);
    let mut donor = before.compile().unwrap();
    advance(&mut donor, 4099);
    let mut after = before.clone();
    after.nodes[2].params.insert("mix".into(), 0.0);
    let mut expected = after.compile().unwrap();
    expected.adopt_state_from(&donor);
    let mut moved = after.compile().unwrap();
    assert!(moved.handoff_state_from(&mut donor));
    let actual = trace(&mut moved, 257, PORTS);
    assert!(actual.iter().all(|sample| sample[0].value == 4.0));
    assert!(actual.iter().any(|sample| sample[1].value != 0.0));
    assert_eq!(actual, trace(&mut expected, 257, PORTS));
}

#[test]
fn excluded_module_starts_fresh_while_other_delay_continues() {
    let patch = patch(true, true);
    let mut donor = patch.compile().unwrap();
    advance(&mut donor, 4099);
    let mut continued = patch.compile().unwrap();
    continued.adopt_state_from(&donor);
    let mut expected = patch.compile().unwrap();
    let excluded = expected.resolve_module_state("delay").unwrap();
    expected.adopt_state_from_excluding(&donor, &[excluded]);
    let mut moved = patch.compile().unwrap();
    let excluded = moved.resolve_module_state("delay").unwrap();
    assert!(moved.handoff_state_from_excluding_routes(&mut donor, &[excluded], &[]));
    let actual = trace(&mut moved, 257, TWO_PORTS);
    let unchanged = trace(&mut continued, 257, TWO_PORTS);
    assert_eq!(actual, trace(&mut expected, 257, TWO_PORTS));
    assert_eq!(actual[0][1].value, 0.0);
    assert!(actual.iter().zip(&unchanged).any(|(a, b)| a[1] != b[1]));
    for (actual, unchanged) in actual.iter().zip(unchanged) {
        assert_eq!(&actual[2..], &unchanged[2..]);
    }
}

#[test]
fn foreign_module_and_cable_exclusions_do_not_discard_compatible_state() {
    let patch = patch(true, true);
    let mut donor = patch.compile().unwrap();
    advance(&mut donor, 4099);
    let foreign_module = donor.resolve_module_state("delay").unwrap();
    let foreign_cable = donor.resolve_cable_state("audio_delay").unwrap();
    let mut expected = patch.compile().unwrap();
    expected.adopt_state_from(&donor);
    let mut moved = patch.compile().unwrap();
    assert!(moved.handoff_state_from_excluding_routes(
        &mut donor,
        &[foreign_module],
        &[foreign_cable]
    ));
    assert_eq!(
        trace(&mut moved, 257, TWO_PORTS),
        trace(&mut expected, 257, TWO_PORTS)
    );
}

#[test]
fn excluded_or_replaced_cable_is_fresh_without_clearing_delay_runtime() {
    for replace_id in [false, true] {
        let mut patch = stable_patch(true, false);
        for node in patch
            .nodes
            .iter_mut()
            .filter(|node| node.kind == "stock_delay")
        {
            node.params.insert("mix".into(), 0.0);
        }
        let mut donor = patch.compile().unwrap();
        advance(&mut donor, 4099);
        let mut normal = patch.compile().unwrap();
        normal.adopt_state_from(&donor);
        let mut after = patch.clone();
        if replace_id {
            after.cables[0].id = Some("replacement_audio_delay".into());
        }
        let mut moved = after.compile().unwrap();
        let moved_state = if replace_id {
            moved.handoff_state_from(&mut donor)
        } else {
            let cable = moved.resolve_cable_state("audio_delay").unwrap();
            moved.handoff_state_from_excluding_routes(&mut donor, &[], &[cable])
        };
        assert!(moved_state);
        let actual = trace(&mut moved, 3, TWO_PORTS);
        let normal = trace(&mut normal, 3, TWO_PORTS);
        assert_eq!(actual[0][0].value, 0.0);
        assert_eq!(normal[0][0].value, 4.0);
        assert_eq!(actual[0][1], normal[0][1]);
        assert_ne!(actual[0][1].value, 0.0);
        assert_eq!(actual[1][0].value, 4.0);
        for (actual, normal) in actual.iter().zip(normal) {
            assert_eq!(&actual[2..], &normal[2..]);
        }
    }
}

#[test]
fn incompatible_rate_version_new_identity_and_all_exclusions_report_no_delay_move() {
    let patch = patch(false, false);
    let mut donor = patch.compile().unwrap();
    advance(&mut donor, 2053);
    let mut different_rate = patch.clone();
    different_rate.sample_rate = 16000;
    assert!(!different_rate
        .compile()
        .unwrap()
        .handoff_state_from(&mut donor));
    let mut renamed = patch.clone();
    renamed.nodes[2].id = "replacement".into();
    renamed.output = Some("replacement".into());
    renamed.cables[0].to = "replacement".into();
    let mut renamed = renamed.compile().unwrap();
    assert!(!renamed.handoff_state_from(&mut donor));
    assert_eq!(
        trace(&mut renamed, 1, [("replacement", "wet")])[0][0].value,
        0.0
    );
    let mut excluded = patch.compile().unwrap();
    let handle = excluded.resolve_module_state("delay").unwrap();
    assert!(!excluded.handoff_state_from_excluding_routes(&mut donor, &[handle], &[]));
    let legacy: Patch = serde_json::from_value(json!({
        "version":1,"sample_rate":8000,"nodes":[{"id":"audio","kind":"constant"}],"output":"audio"
    }))
    .unwrap();
    let mut legacy = legacy.compile().unwrap();
    let mut receiving = patch.compile().unwrap();
    assert!(!receiving.handoff_state_from(&mut legacy));
    assert!(!legacy.handoff_state_from(&mut donor));
    let mut gain = patch.clone();
    gain.nodes[2] = serde_json::from_value(json!({"id":"delay","kind":"gain"})).unwrap();
    gain.output_port = "out".into();
    assert!(!gain.compile().unwrap().handoff_state_from(&mut donor));
}

#[test]
fn callback_partitioning_preserves_both_outputs() {
    let patch = patch(false, true);
    let mut whole = patch.compile().unwrap();
    let mut partitioned = patch.compile().unwrap();
    let expected = trace(&mut whole, 2049, PORTS);
    let mut actual = Vec::new();
    for count in [0, 1, 3, 127, 128, 129, 513, 1025, 123] {
        actual.extend(trace(&mut partitioned, count, PORTS));
    }
    assert_eq!(actual, expected);
}

#[test]
fn initialize_preserves_existing_feedback_tail() {
    let before = patch(false, true);
    let mut donor = before.compile().unwrap();
    advance(&mut donor, 4099);
    let mut after = before.clone();
    ModuleParameterActions::new(8000)
        .initialize(&mut after.nodes[2])
        .unwrap();
    let mut expected = after.compile().unwrap();
    expected.adopt_state_from(&donor);
    let mut actual = after.compile().unwrap();
    assert!(actual.handoff_state_from(&mut donor));
    let handle = actual.resolve_module_state("delay").unwrap();
    assert!(actual.initialize_module_state(handle));
    let expected = trace(&mut expected, 513, PORTS);
    assert!(expected.iter().any(|sample| sample[1].value.abs() > 0.001));
    assert_eq!(trace(&mut actual, 513, PORTS), expected);
}

#[test]
fn bypass_copies_raw_signal_to_both_ports_and_freezes_runtime_through_resume_clock_edge() {
    let before = stable_patch(false, true);
    let mut donor = before.compile().unwrap();
    advance(&mut donor, 4099);
    let mut expected = before.compile().unwrap();
    expected.adopt_state_from(&donor);
    let mut bypass_patch = before.clone();
    bypass_patch.nodes[2].bypassed = true;
    let mut bypassed = bypass_patch.compile().unwrap();
    assert!(bypassed.handoff_state_from(&mut donor));
    for width in [0, 1, 5, 16] {
        for port in ["mix", "wet"] {
            let mut selected = bypass_patch.clone();
            selected.output_port = port.into();
            selected.nodes[0]
                .params
                .insert("channels".into(), width as f32);
            for channel in 4..=16 {
                selected.nodes[0].params.insert(format!("v{channel}"), 0.0);
            }
            selected.nodes[0].params.insert("v5".into(), 7.0);
            selected.nodes[0].params.insert("v16".into(), -2.0);
            let mut engine = selected.compile().unwrap();
            one(&mut engine);
            let frame = one(&mut engine);
            assert_eq!(frame.channels, width.max(1));
            let mut expected = [0.0; 16];
            if width != 0 {
                for (index, value) in [(0, 2.0), (1, -1.0), (2, 3.0), (4, 7.0), (15, -2.0)] {
                    if index < width as usize {
                        expected[index] = value;
                    }
                }
            }
            assert_eq!(frame.values, expected);
        }
    }
    advance(&mut bypassed, 1373);
    let mut resumed_patch = before.clone();
    resumed_patch.nodes[1].params.insert("value".into(), 10.0);
    let mut continued = resumed_patch.compile().unwrap();
    continued.adopt_state_from(&expected);
    let mut resumed = resumed_patch.compile().unwrap();
    assert!(resumed.handoff_state_from(&mut bypassed));
    assert_eq!(
        trace(&mut resumed, 2049, PORTS),
        trace(&mut continued, 2049, PORTS)
    );
}
