use openpatch_core::*;
fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        bypassed: false,
        params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
    }
}
fn cable(from: &str, port: &str, to: &str, input: &str) -> Cable {
    Cable {
        id: None,
        from: from.into(),
        output: port.into(),
        to: to.into(),
        input: input.into(),
    }
}
fn one(e: &mut Engine) -> PortFrame {
    let mut out = [PortFrame::default()];
    e.process_poly_block(&mut out);
    out[0]
}
fn pair(e: &mut Engine, a: (&str, &str), b: (&str, &str)) -> [TapSample; 2] {
    let taps = [
        Some(e.resolve_tap(a.0, a.1, 0).unwrap()),
        Some(e.resolve_tap(b.0, b.1, 0).unwrap()),
    ];
    let mut out = [[0.0; 2]];
    let mut samples = [[TapSample::default(); 2]];
    e.process_audio_block_with_taps(&mut out, &taps, &mut samples);
    samples[0]
}
#[test]
fn recreated_oscillator_starts_fresh_while_unaffected_phase_is_preserved_in_both_versions() {
    for version in [1, 2] {
        let p = Patch {
            version,
            sample_rate: 8000,
            nodes: vec![
                node("a", "oscillator", &[("frequency", 7.0)]),
                node("b", "oscillator", &[("frequency", 11.0)]),
            ],
            output: Some("a".into()),
            ..Patch::default()
        };
        let mut old = p.compile().unwrap();
        for _ in 0..379 {
            one(&mut old);
        }
        let mut new = p.compile().unwrap();
        let h = new.resolve_module_state("a").unwrap();
        assert!(new.resolve_module_state("missing").is_err());
        new.adopt_state_from_excluding(&old, &[h]);
        let mut cold = p.compile().unwrap();
        let expected_fresh = pair(&mut cold, ("a", "out"), ("b", "out"));
        let expected_continued = pair(&mut old, ("a", "out"), ("b", "out"));
        let actual = pair(&mut new, ("a", "out"), ("b", "out"));
        assert_eq!(actual[0], expected_fresh[0]);
        assert_eq!(actual[1], expected_continued[1]);
        assert_ne!(actual[0], expected_continued[0]);
    }
}
#[test]
fn recreated_envelope_is_fresh_but_matching_unexcluded_envelope_keeps_history() {
    for version in [1, 2] {
        let p = Patch {
            version,
            sample_rate: 8000,
            nodes: vec![
                node("gate", "constant", &[("value", 10.0)]),
                node("a", "adsr", &[("attack", 0.01)]),
                node("b", "adsr", &[("attack", 0.01)]),
            ],
            cables: vec![
                cable("gate", "out", "a", "gate"),
                cable("gate", "out", "b", "gate"),
            ],
            output: Some("a".into()),
            ..Patch::default()
        };
        let mut old = p.compile().unwrap();
        for _ in 0..100 {
            one(&mut old);
        }
        let mut new = p.compile().unwrap();
        let h = new.resolve_module_state("a").unwrap();
        new.adopt_state_from_excluding(&old, &[h]);
        let mut cold = p.compile().unwrap();
        let fresh = pair(&mut cold, ("a", "out"), ("b", "out"));
        let continued = pair(&mut old, ("a", "out"), ("b", "out"));
        let actual = pair(&mut new, ("a", "out"), ("b", "out"));
        assert_eq!(actual[0], fresh[0]);
        assert_eq!(actual[1], continued[1]);
        assert!(actual[1].value > actual[0].value);
    }
}
#[test]
fn recreated_midi_drops_held_notes_without_resetting_an_unaffected_generator() {
    let p = Patch {
        sample_rate: 8000,
        nodes: vec![
            node("m", "midi_cv", &[("channels", 1.0)]),
            node("osc", "oscillator", &[("frequency", 11.0)]),
        ],
        output: Some("m".into()),
        output_port: "gate".into(),
        ..Patch::new_v2()
    };
    let mut old = p.compile().unwrap();
    old.handle_midi(MidiEvent::NoteOn {
        channel: 0,
        note: 60,
        velocity: 100,
    });
    for _ in 0..123 {
        one(&mut old);
    }
    let mut new = p.compile().unwrap();
    let h = new.resolve_module_state("m").unwrap();
    new.adopt_state_from_excluding(&old, &[h]);
    let actual = pair(&mut new, ("m", "gate"), ("osc", "out"));
    let expected = pair(&mut old, ("m", "gate"), ("osc", "out"));
    assert_eq!(actual[0].value, 0.0);
    assert_eq!(expected[0].value, 10.0);
    assert_eq!(actual[1], expected[1]);
    new.handle_midi(MidiEvent::NoteOn {
        channel: 0,
        note: 64,
        velocity: 100,
    });
    assert_eq!(one(&mut new).values[0], 10.0);
}
#[test]
fn recreated_source_clears_its_outgoing_history_but_unrelated_edge_still_adopts() {
    let mut p = Patch {
        nodes: vec![
            node("source", "constant", &[("value", 3.0)]),
            node("other", "constant", &[("value", 7.0)]),
            node("a", "gain", &[]),
            node("b", "gain", &[]),
        ],
        cables: vec![
            cable("source", "out", "a", "signal"),
            cable("other", "out", "b", "signal"),
        ],
        ..Patch::new_v2()
    };
    let mut old = p.compile().unwrap();
    one(&mut old);
    one(&mut old);
    p.nodes.reverse(); // Resolution uses compiled indices, not old ordering.
    let mut new = p.compile().unwrap();
    let h = new.resolve_module_state("source").unwrap();
    new.adopt_state_from_excluding(&old, &[h]);
    let first = pair(&mut new, ("a", "out"), ("b", "out"));
    assert_eq!([first[0].value, first[1].value], [0.0, 7.0]);
    let second = pair(&mut new, ("a", "out"), ("b", "out"));
    assert_eq!([second[0].value, second[1].value], [3.0, 7.0]);
}
#[test]
fn excluded_state_clears_incoming_outgoing_and_feedback_frames() {
    let p = Patch {
        sample_rate: 8000,
        nodes: vec![
            node(
                "s",
                "seq3",
                &[("tempo", 15.0), ("row1_1", 10.0), ("row1_2", 10.0)],
            ),
            node("tail", "gain", &[]),
        ],
        cables: vec![
            cable("s", "cv1", "s", "clock"),
            cable("s", "cv1", "tail", "signal"),
        ],
        output: Some("s".into()),
        output_port: "cv1".into(),
        ..Patch::new_v2()
    };
    let mut old = p.compile().unwrap();
    one(&mut old);
    one(&mut old);
    assert_eq!(
        old.module_telemetry(old.resolve_module_event("s").unwrap())
            .unwrap()
            .step,
        2
    );
    let mut new = p.compile().unwrap();
    let h = new.resolve_module_state("s").unwrap();
    new.adopt_state_from_excluding(&old, &[h]);
    let first = pair(&mut new, ("s", "cv1"), ("tail", "out"));
    assert_eq!([first[0].value, first[1].value], [10.0, 0.0]);
    assert_eq!(new.module_telemetry(h).unwrap().step, 1);
    let second = pair(&mut new, ("s", "cv1"), ("tail", "out"));
    assert_eq!(second[1].value, 10.0);
    assert_eq!(new.module_telemetry(h).unwrap().step, 2);
}
#[test]
fn foreign_exclusion_handles_are_ignored_and_empty_api_matches_original_adoption() {
    for version in [1, 2] {
        let p = Patch {
            version,
            sample_rate: 8000,
            nodes: vec![node("osc", "oscillator", &[("frequency", 7.0)])],
            output: Some("osc".into()),
            ..Patch::default()
        };
        let mut old = p.compile().unwrap();
        for _ in 0..123 {
            one(&mut old);
        }
        let foreign = old.resolve_module_state("osc").unwrap();
        let mut plain = p.compile().unwrap();
        let mut empty = p.compile().unwrap();
        let mut stale = p.compile().unwrap();
        plain.adopt_state_from(&old);
        empty.adopt_state_from_excluding(&old, &[]);
        stale.adopt_state_from_excluding(&old, &[foreign, foreign]);
        let mut a = [0.0; 33];
        let mut b = [0.0; 33];
        let mut c = [0.0; 33];
        plain.process_block(&mut a);
        empty.process_block(&mut b);
        stale.process_block(&mut c);
        assert_eq!(a, b);
        assert_eq!(a, c);
    }
}
