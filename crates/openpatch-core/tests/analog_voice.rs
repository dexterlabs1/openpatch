use openpatch_core::{analog_demo_patch, catalog, render, Cable, Node, Patch};
use std::collections::BTreeMap;

fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
    }
}
fn cable(from: &str, to: &str, input: &str) -> Cable {
    Cable {
        from: from.into(),
        to: to.into(),
        input: input.into(),
        output: "out".into(),
    }
}
#[test]
fn catalog_defaults_compile_and_invalid_reference_parameters_fail() {
    for module in catalog()["modules"].as_array().unwrap() {
        let kind = module["kind"].as_str().unwrap();
        let patch = Patch {
            version: module["min_patch_version"].as_u64().unwrap() as u32,
            nodes: vec![node("test", kind, &[])],
            output: Some("test".into()),
            output_port: module["outputs"][0].as_str().unwrap().into(),
            ..Patch::default()
        };
        assert!(patch.compile().is_ok(), "{kind}");
        for (name, spec) in module["params"].as_object().unwrap() {
            for value in [f32::NAN, spec["max"].as_f64().unwrap() as f32 + 1.0] {
                let mut invalid = patch.clone();
                invalid.nodes[0].params = BTreeMap::from([(name.clone(), value)]);
                assert!(invalid.validate().is_err(), "{kind}.{name}={value}");
            }
        }
    }
}
#[test]
fn ota_control_gain_matches_static_circuit_and_audio_boundary() {
    let mut patch = Patch {
        nodes: vec![
            node("source", "constant", &[("value", 0.1)]),
            node("cv", "constant", &[("value", 5.0)]),
            node("vca", "analog_vca", &[]),
            node("out", "audio_out", &[]),
        ],
        cables: vec![
            cable("source", "vca", "signal"),
            cable("cv", "vca", "cv"),
            cable("vca", "out", "signal"),
        ],
        output: Some("out".into()),
        ..Patch::default()
    };
    for signal in [0.1, 1.0, 5.0, -5.0] {
        for control in [-2.0_f32, 0.0, 5.0, 10.0, 12.0] {
            patch.nodes[0].params.insert("value".into(), signal);
            patch.nodes[1].params.insert("value".into(), control);
            let result = render(&patch, 0.01).unwrap();
            let vt = 1.380649e-23 * 298.15 / 1.602176634e-19;
            let scale = 2.0 * vt / 0.01;
            let expected = scale
                * (signal as f64 / scale).tanh()
                * (control as f64 / 10.0).clamp(0.0, 1.0)
                * 0.1;
            assert!(
                (result.last().unwrap() - expected as f32).abs() < 2e-5,
                "{signal}/{control}"
            );
        }
    }
}
#[test]
fn reference_voice_is_audible_and_independent_of_callback_boundaries() {
    let patch = analog_demo_patch();
    let mut expected = vec![0.0; 8192];
    patch.compile().unwrap().process_block(&mut expected);
    let mut engine = patch.compile().unwrap();
    let mut actual = vec![0.0; expected.len()];
    let mut position = 0;
    for size in [1, 127, 3, 257, 512, 17].into_iter().cycle() {
        let end = (position + size).min(actual.len());
        engine.process_block(&mut actual[position..end]);
        position = end;
        if position == actual.len() {
            break;
        }
    }
    assert_eq!(actual, expected);
    assert_eq!(engine.solver_failures(), 0);
    let peak = actual.iter().fold(0.0_f32, |a, x| a.max(x.abs()));
    assert!(peak > 0.01 && peak < 1.0, "peak={peak}");
    assert!(actual.iter().all(|x| x.is_finite()));
}
#[test]
fn identical_reference_graph_adopts_all_analog_memory() {
    let patch = analog_demo_patch();
    let mut live = patch.compile().unwrap();
    live.process_block(&mut [0.0; 2000]);
    let mut replacement = patch.compile().unwrap();
    replacement.adopt_state_from(&live);
    let (mut a, mut b) = ([0.0; 1024], [0.0; 1024]);
    live.process_block(&mut a);
    replacement.process_block(&mut b);
    assert_eq!(a, b);
}
