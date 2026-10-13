use openpatch_core::{Cable, Node, Patch, PortFrame};
use std::collections::BTreeMap;

fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        bypassed: false,
        id: id.into(),
        kind: kind.into(),
        params: params
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect::<BTreeMap<_, _>>(),
    }
}
fn cable(from: &str, to: &str, input: &str) -> Cable {
    Cable {
        id: None,
        from: from.into(),
        output: "out".into(),
        to: to.into(),
        input: input.into(),
    }
}
fn poly_voice() -> Patch {
    Patch {
        sample_rate: 8000,
        nodes: vec![
            node(
                "pitch",
                "poly_constant",
                &[
                    ("channels", 4.0),
                    ("v1", -1.0),
                    ("v2", -0.5),
                    ("v3", 0.0),
                    ("v4", 0.5),
                ],
            ),
            node("source", "analog_vco", &[("frequency", 110.0)]),
            node(
                "filter",
                "ladder_vcf",
                &[("cutoff", 1000.0), ("resonance", 0.8)],
            ),
            node("clock", "gate", &[("frequency", 10.0)]),
            node("env", "adsr", &[("attack", 0.005)]),
            node("vca", "analog_vca", &[]),
            node("out", "audio_out", &[]),
        ],
        cables: vec![
            cable("pitch", "source", "pitch"),
            cable("source", "filter", "signal"),
            cable("clock", "env", "gate"),
            cable("filter", "vca", "signal"),
            cable("env", "vca", "cv"),
            cable("vca", "out", "signal"),
        ],
        output: Some("out".into()),
        ..Patch::new_v2()
    }
}
#[test]
fn all_active_analog_voices_and_edge_histories_survive_reordered_graph_replacement() {
    let patch = poly_voice();
    let mut old = patch.compile().unwrap();
    old.process_poly_block(&mut [PortFrame::default(); 113]);
    let mut reordered = patch.clone();
    reordered.nodes.reverse();
    reordered.cables.reverse();
    let mut current = reordered.compile().unwrap();
    current.adopt_state_from(&old);
    let mut expected = vec![PortFrame::default(); 2049];
    let mut actual = vec![PortFrame::default(); 2049];
    old.process_poly_block(&mut expected);
    for frames in actual.chunks_mut(37) {
        current.process_poly_block(frames);
    }
    assert_eq!(actual, expected);
    assert!(actual.iter().all(|frame| frame.channels == 4));
    assert!(actual
        .iter()
        .any(|frame| frame.values[0] != frame.values[3]));
    assert!(actual
        .iter()
        .all(|frame| frame.values.iter().all(|sample| sample.is_finite())));
    assert_eq!(current.solver_failures(), 0);
}
#[test]
fn sample_rate_and_scheduler_version_changes_start_with_fresh_state() {
    let patch = poly_voice();
    let mut old = patch.compile().unwrap();
    old.process_poly_block(&mut [PortFrame::default(); 317]);
    let mut changed = patch.clone();
    changed.sample_rate = 48000;
    let mut current = changed.compile().unwrap();
    current.adopt_state_from(&old);
    let mut fresh = changed.compile().unwrap();
    let mut expected = [PortFrame::default(); 257];
    let mut actual = [PortFrame::default(); 257];
    fresh.process_poly_block(&mut expected);
    current.process_poly_block(&mut actual);
    assert_eq!(actual, expected);

    let mut legacy = openpatch_core::demo_patch().compile().unwrap();
    legacy.process_block(&mut [0.0; 317]);
    let mut delayed = openpatch_core::demo_patch();
    delayed.version = 2;
    let mut current = delayed.compile().unwrap();
    current.adopt_state_from(&legacy);
    let mut fresh = delayed.compile().unwrap();
    current.process_poly_block(&mut actual);
    fresh.process_poly_block(&mut expected);
    assert_eq!(actual, expected);
}
#[test]
fn polyphonic_ladder_channels_match_separate_mono_ladders() {
    let mut patch = Patch {
        nodes: vec![
            node(
                "dc",
                "poly_constant",
                &[("channels", 3.0), ("v1", -0.5), ("v2", 0.25), ("v3", 1.0)],
            ),
            node(
                "filter",
                "ladder_vcf",
                &[("cutoff", 1500.0), ("resonance", 0.7), ("drive", 2.0)],
            ),
        ],
        cables: vec![cable("dc", "filter", "signal")],
        output: Some("filter".into()),
        ..Patch::new_v2()
    };
    let mut poly = patch.compile().unwrap();
    let mut actual = vec![PortFrame::default(); 1025];
    poly.process_poly_block(&mut actual);
    for (channel, value) in [-0.5, 0.25, 1.0].into_iter().enumerate() {
        patch.nodes[0].params = BTreeMap::from([("channels".into(), 1.0), ("v1".into(), value)]);
        let mut mono = patch.compile().unwrap();
        let mut expected = vec![0.0; actual.len()];
        mono.process_block(&mut expected);
        for n in 1..actual.len() {
            assert_eq!(
                actual[n].values[channel], expected[n],
                "channel={channel}, sample={n}"
            );
        }
    }
}
