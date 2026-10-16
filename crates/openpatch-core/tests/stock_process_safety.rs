//! Local Rust boundary checks for the 28 declared Process safety inputs.
//! Undefined SDK exp2 exponent cases are not source-parity assertions.
use openpatch_core::{Patch, PortFrame};
use serde_json::json;

const NONFINITE_BITS: [u32; 4] = [0x7f80_0000, 0xff80_0000, 0x7fc0_0001, 0xffc0_0001];
const EXTREME_FINITE_BITS: [u32; 2] = [0x7f7f_ffff, 0xff7f_ffff];
const OUTSIDE_EXP2_PITCHES: [f32; 4] = [-200.0, -130.0, 160.0, 200.0];
const DECLARED_SAFETY_IDS: [&str; 28] = [
    "signal/7f800000",
    "signal/ff800000",
    "signal/7fc00001",
    "signal/ffc00001",
    "signal/7f7fffff",
    "signal/ff7fffff",
    "gate/7f800000",
    "gate/ff800000",
    "gate/7fc00001",
    "gate/ffc00001",
    "gate/7f7fffff",
    "gate/ff7fffff",
    "slew-cv/7f800000",
    "slew-cv/ff800000",
    "slew-cv/7fc00001",
    "slew-cv/ffc00001",
    "slew-cv/7f7fffff",
    "slew-cv/ff7fffff",
    "slew-param/7f800000",
    "slew-param/ff800000",
    "slew-param/7fc00001",
    "slew-param/ffc00001",
    "slew-param/7f7fffff",
    "slew-param/ff7fffff",
    "exp2-floor-outside/-200",
    "exp2-floor-outside/-130",
    "exp2-floor-outside/160",
    "exp2-floor-outside/200",
];

#[test]
fn exercised_safety_cases_match_all_frozen_declaration_ids() {
    let mut exercised = Vec::with_capacity(28);
    for role in ["signal", "gate", "slew-cv", "slew-param"] {
        for bits in NONFINITE_BITS.into_iter().chain(EXTREME_FINITE_BITS) {
            exercised.push(format!("{role}/{bits:08x}"));
        }
    }
    for pitch in OUTSIDE_EXP2_PITCHES {
        exercised.push(format!("exp2-floor-outside/{pitch}"));
    }
    assert_eq!(exercised.len(), DECLARED_SAFETY_IDS.len());
    for (actual, expected) in exercised.iter().zip(DECLARED_SAFETY_IDS) {
        assert_eq!(actual, expected);
    }
}

fn patch() -> Patch {
    serde_json::from_value(json!({
        "version":2,"sample_rate":48000,
        "nodes":[
            {"id":"signal_source","kind":"poly_constant","params":{"channels":1,"v1":1}},
            {"id":"gate_source","kind":"poly_constant","params":{"channels":1,"v1":0}},
            {"id":"slew_source","kind":"poly_constant","params":{"channels":1,"v1":0}},
            {"id":"process","kind":"stock_process","params":{"slew":0}}
        ],
        "cables":[
            {"from":"signal_source","to":"process","input":"signal"},
            {"from":"gate_source","to":"process","input":"gate"},
            {"from":"slew_source","to":"process","input":"slew_cv"}
        ],
        "output":"process","output_port":"sh1"
    }))
    .unwrap()
}

fn source_id(role: &str) -> &'static str {
    match role {
        "signal" => "signal_source",
        "gate" => "gate_source",
        "slew-cv" => "slew_source",
        _ => unreachable!("declared safety role"),
    }
}

fn safe_source_value(role: &str) -> f32 {
    if role == "signal" {
        1.0
    } else {
        0.0
    }
}

fn assert_bounded_then_recovered(
    engine: &mut openpatch_core::Engine,
    process: openpatch_core::ModuleHandle,
    source: openpatch_core::ModuleHandle,
    role: &str,
) {
    // Completing this actual callback is the no-hang assertion. The SDK's
    // undefined exponent domain does not promise finite values during it.
    let mut extreme_frames = [PortFrame::default(); 4];
    engine.process_poly_block(&mut extreme_frames);
    assert!(extreme_frames.iter().all(|frame| frame.channels <= 16));

    assert!(engine.set_host_reference_source(source, PortFrame::mono(safe_source_value(role)),));
    assert!(engine.reset_module_state(process));
    let mut recovered = [0.0_f32; 8];
    engine.process_block(&mut recovered);
    assert!(recovered.iter().all(|value| value.is_finite()));
    let raw = engine.host_module_snapshot(process).unwrap();
    assert!(raw.outputs[..6]
        .iter()
        .all(|port| port.channels <= 16 && port.values.iter().all(|value| value.is_finite())));
}

#[test]
fn nonfinite_cable_sources_are_rejected_by_the_public_controller_and_audio_continues() {
    for role in ["signal", "gate", "slew-cv"] {
        let mut engine = patch().compile().unwrap();
        let source = engine.resolve_module_state(source_id(role)).unwrap();
        let before = engine.get_host_reference_source(source).unwrap();
        for bits in NONFINITE_BITS {
            assert!(
                !engine.set_host_reference_source(source, PortFrame::mono(f32::from_bits(bits)))
            );
            assert_eq!(engine.get_host_reference_source(source).unwrap(), before);
            let mut audio = [0.0_f32; 4];
            engine.process_block(&mut audio);
            assert!(audio.iter().all(|value| value.is_finite()));
        }
    }
}

#[test]
fn finite_extreme_signal_gate_and_slew_cv_complete_then_fresh_process_recovers() {
    for role in ["signal", "gate", "slew-cv"] {
        for bits in EXTREME_FINITE_BITS {
            let mut engine = patch().compile().unwrap();
            let process = engine.resolve_module_state("process").unwrap();
            let source = engine.resolve_module_state(source_id(role)).unwrap();
            assert!(engine.set_host_reference_source(source, PortFrame::mono(f32::from_bits(bits))));
            assert_bounded_then_recovered(&mut engine, process, source, role);
        }
    }
}

#[test]
fn raw_gate_nonfinite_values_complete_without_poisoning_safe_audio() {
    for bits in NONFINITE_BITS {
        let mut engine = patch().compile().unwrap();
        let process = engine.resolve_module_state("process").unwrap();
        assert!(engine.set_stock_process_gate_value(process, f32::from_bits(bits)));
        let mut audio = [0.0_f32; 8];
        engine.process_block(&mut audio);
        assert!(audio.iter().all(|value| value.is_finite()));
        assert!(engine.reset_module_state(process));
        engine.process_block(&mut audio);
        assert!(audio.iter().all(|value| value.is_finite()));
    }
}

#[test]
fn nonfinite_slew_parameter_is_rejected_and_finite_extremes_recover_after_rebuild() {
    for bits in NONFINITE_BITS {
        let mut invalid = patch();
        invalid.nodes[3]
            .params
            .insert("slew".into(), f32::from_bits(bits));
        assert!(invalid.compile().is_err());
    }
    for bits in EXTREME_FINITE_BITS {
        let mut extreme = patch();
        extreme.nodes[3]
            .params
            .insert("slew".into(), f32::from_bits(bits));
        let mut engine = extreme.compile().unwrap();
        let mut frames = [PortFrame::default(); 4];
        engine.process_poly_block(&mut frames);
        assert!(frames.iter().all(|frame| frame.channels <= 16));

        // The receiving patch has safe controls and is a fresh incarnation;
        // adopting a poisoned Process history would defeat this recovery path.
        let mut recovered = patch().compile().unwrap();
        let mut audio = [0.0_f32; 8];
        recovered.process_block(&mut audio);
        assert!(audio.iter().all(|value| value.is_finite()));
    }
}

#[test]
fn outside_exp2_floor_domain_completes_and_recovers_without_sdk_parity_claim() {
    for pitch in OUTSIDE_EXP2_PITCHES {
        let mut engine = patch().compile().unwrap();
        let process = engine.resolve_module_state("process").unwrap();
        let source = engine.resolve_module_state("slew_source").unwrap();
        assert!(engine.set_host_reference_source(source, PortFrame::mono(pitch)));
        assert_bounded_then_recovered(&mut engine, process, source, "slew-cv");
    }
}
