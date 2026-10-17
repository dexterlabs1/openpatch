use openpatch_core::{Engine, Patch, PortFrame};
use serde_json::json;

fn patch(kind: &str) -> Patch {
    serde_json::from_value(json!({
        "version":2,"sample_rate":48000,
        "nodes":[{"id":"signal","kind":"poly_constant","params":{"v1":5}},
                 {"id":"target","kind":kind,"params":{}}],
        "cables":[{"from":"signal","to":"target","input":"signal"}],
        "output":"target","output_port":if kind=="stock_vca" {"out"} else {"lowpass"}
    }))
    .unwrap()
}

fn sample(engine: &mut Engine) -> f32 {
    let mut frames = [PortFrame::default(); 2];
    engine.process_poly_block(&mut frames);
    frames[1].values[0]
}

#[test]
fn live_vca_value_is_the_actual_rendered_gain_and_bypass_preserves_it() {
    let p = patch("stock_vca");
    let mut engine = p.compile().unwrap();
    let handle = engine.resolve_parameter("target", "level").unwrap();
    assert_eq!(sample(&mut engine), 5.0);
    assert!(engine.set_parameter_value(handle, 0.25));
    assert_eq!(engine.parameter_value(handle), Some(0.25));
    assert_eq!(sample(&mut engine), 1.25);
    let mut bypass = p.clone();
    bypass.nodes[1].bypassed = true;
    let mut b = bypass.compile().unwrap();
    b.adopt_state_from(&engine);
    assert_eq!(b.inherit_parameter_values(&engine, &[], &[]), 1);
    let h = b.resolve_parameter("target", "level").unwrap();
    assert_eq!(sample(&mut b), 5.0);
    assert!(b.set_parameter_value(h, 0.6));
    let mut unbypassed = p.compile().unwrap();
    unbypassed.adopt_state_from(&b);
    unbypassed.inherit_parameter_values(&b, &[], &[]);
    assert_eq!(sample(&mut unbypassed), 3.0);
}

#[test]
fn invalid_values_foreign_handles_and_unsupported_parameters_are_atomic() {
    for (kind, name) in [("stock_vca", "level"), ("rack264_vcf", "frequency")] {
        let mut engine = patch(kind).compile().unwrap();
        let handle = engine.resolve_parameter("target", name).unwrap();
        let descriptor = engine.parameter_descriptor(handle).unwrap();
        let before = engine.parameter_value(handle);
        for invalid in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            descriptor.min - 1.,
            descriptor.max + 1.,
        ] {
            assert!(!engine.set_parameter_value(handle, invalid));
            assert_eq!(engine.parameter_value(handle), before);
        }
        let foreign = patch(kind)
            .compile()
            .unwrap()
            .resolve_parameter("target", name)
            .unwrap();
        assert!(!engine.set_parameter_value(foreign, descriptor.min));
        assert_eq!(engine.parameter_value(foreign), None);
        assert_eq!(engine.parameter_descriptor(foreign), None);
        assert!(engine.resolve_parameter("missing", name).is_err());
        assert!(engine.resolve_parameter("target", "unsupported").is_err());
        assert!(engine.set_parameter_value(handle, descriptor.min));
        assert!(engine.set_parameter_value(handle, descriptor.max));
    }
}

#[test]
fn filter_live_frequency_matches_parameter_recompile_without_losing_history() {
    let mut p = patch("rack264_vcf");
    let mut live = p.compile().unwrap();
    let handle = live.resolve_parameter("target", "frequency").unwrap();
    let mut warm = [PortFrame::default(); 211];
    live.process_poly_block(&mut warm);
    for frequency in [0.1, 0.8, 0.35, 0.55] {
        p.nodes[1].params.insert("frequency".into(), frequency);
        let mut rebuilt = p.compile().unwrap();
        rebuilt.adopt_state_from(&live);
        assert!(live.set_parameter_value(handle, frequency));
        let mut a = [PortFrame::default(); 257];
        let mut b = a;
        live.process_poly_block(&mut a);
        rebuilt.process_poly_block(&mut b);
        assert_eq!(
            a, b,
            "direct parameter update changed filter state/rounding"
        );
        assert_eq!(live.parameter_value(handle), Some(frequency));
    }
}

#[test]
fn effective_values_survive_unrelated_edits_repeated_adoption_and_rate_changes() {
    for (kind, name, value) in [
        ("stock_vca", "level", 0.25),
        ("rack264_vcf", "frequency", 0.3),
    ] {
        let mut p = patch(kind);
        let mut old = p.compile().unwrap();
        let handle = old.resolve_parameter("target", name).unwrap();
        assert!(old.set_parameter_value(handle, value));
        for rate in [48000, 96000, 44100] {
            p.sample_rate = rate;
            // An unrelated edit does not give the stale authored target value authority.
            p.nodes[0].params.insert("v1".into(), 4.0);
            let mut next = p.compile().unwrap();
            next.adopt_state_from(&old);
            assert_eq!(next.inherit_parameter_values(&old, &[], &[]), 1);
            let h = next.resolve_parameter("target", name).unwrap();
            assert_eq!(next.parameter_value(h), Some(value));
            old = next;
        }
    }
}

#[test]
fn explicit_equal_assignment_and_fresh_incarnation_win_over_inherited_values() {
    let p = patch("stock_vca");
    let mut old = p.compile().unwrap();
    let old_handle = old.resolve_parameter("target", "level").unwrap();
    old.set_parameter_value(old_handle, 0.25);
    let mut assigned = p.compile().unwrap();
    let h = assigned.resolve_parameter("target", "level").unwrap();
    assert_eq!(assigned.inherit_parameter_values(&old, &[], &[h]), 0);
    assert_eq!(assigned.parameter_value(h), Some(1.0)); // explicit 1→1 authored assignment still wins
    assert_eq!(
        assigned.inherit_parameter_values(&old, &[h.module()], &[]),
        0
    );
    assert_eq!(assigned.parameter_value(h), Some(1.0));
    // A stale/foreign assignment cannot suppress valid receiving inheritance.
    assert_eq!(
        assigned.inherit_parameter_values(&old, &[], &[old_handle]),
        1
    );
    assert_eq!(assigned.parameter_value(h), Some(0.25));
    let other = patch("rack264_vcf").compile().unwrap();
    assert_eq!(assigned.inherit_parameter_values(&other, &[], &[]), 0);
}

#[test]
fn explicit_filter_assignment_preserves_the_other_modules_effective_value() {
    let mut p = patch("stock_vca");
    p.nodes.push(
        serde_json::from_value(
            json!({"id":"filter","kind":"rack264_vcf","params":{"frequency":0.5}}),
        )
        .unwrap(),
    );
    let mut old = p.compile().unwrap();
    let gain = old.resolve_parameter("target", "level").unwrap();
    let cutoff = old.resolve_parameter("filter", "frequency").unwrap();
    assert!(old.set_parameter_value(gain, 0.25));
    assert!(old.set_parameter_value(cutoff, 0.3));
    let mut next = p.compile().unwrap();
    let receiving_cutoff = next.resolve_parameter("filter", "frequency").unwrap();
    assert_eq!(
        next.inherit_parameter_values(&old, &[], &[receiving_cutoff]),
        1
    );
    assert_eq!(next.parameter_value(receiving_cutoff), Some(0.5));
    let receiving_gain = next.resolve_parameter("target", "level").unwrap();
    assert_eq!(next.parameter_value(receiving_gain), Some(0.25));
}
