//! Public-engine regressions for paired circuit voices. Each expected lane is
//! rendered by its own one-channel Engine, not by a copy of the paired code.
use openpatch_core::{Cable, Engine, Node, Patch, PortFrame};
use std::collections::BTreeMap;

fn source(id: &str, values: &[f32]) -> Node {
    assert!(values.len() <= 16);
    let mut params = BTreeMap::from([("channels".into(), values.len() as f32)]);
    for (index, value) in values.iter().enumerate() {
        params.insert(format!("v{}", index + 1), *value);
    }
    Node {
        id: id.into(),
        kind: "poly_constant".into(),
        bypassed: false,
        params,
    }
}

fn target(kind: &str, bypassed: bool, edited: bool) -> Node {
    let params = match (kind, edited) {
        ("ladder_vcf", false) => [("cutoff", 1450.0), ("resonance", 0.76), ("drive", 1.9)]
            .into_iter()
            .map(|(name, value)| (name.into(), value))
            .collect(),
        ("ladder_vcf", true) => [("cutoff", 820.0), ("resonance", 0.94), ("drive", 2.3)]
            .into_iter()
            .map(|(name, value)| (name.into(), value))
            .collect(),
        ("analog_vca", false) => BTreeMap::from([("gain".into(), 1.35)]),
        ("analog_vca", true) => BTreeMap::from([("gain".into(), 0.63)]),
        _ => panic!("unsupported circuit kind {kind}"),
    };
    Node {
        id: "circuit".into(),
        kind: kind.into(),
        bypassed,
        params,
    }
}

fn cable(from: &str, input: &str) -> Cable {
    Cable {
        id: None,
        from: from.into(),
        output: "out".into(),
        to: "circuit".into(),
        input: input.into(),
    }
}

fn fixture(kind: &str, signal: &[f32], cv: Option<&[f32]>, bypassed: bool, edited: bool) -> Patch {
    let mut nodes = vec![source("signal", signal), target(kind, bypassed, edited)];
    let mut cables = vec![cable("signal", "signal")];
    if let Some(cv) = cv {
        nodes.push(source("cv", cv));
        cables.push(cable(
            "cv",
            if kind == "ladder_vcf" {
                "cutoff_cv"
            } else {
                "cv"
            },
        ));
    }
    Patch {
        sample_rate: 48_000,
        nodes,
        cables,
        output: Some("circuit".into()),
        ..Patch::new_v2()
    }
}

fn lane_cv(cv: Option<&[f32]>, lane: usize) -> Option<[f32; 1]> {
    cv.map(|values| {
        [if values.len() == 1 {
            values[0]
        } else {
            values.get(lane).copied().unwrap_or(0.0)
        }]
    })
}

fn lane_fixture(
    kind: &str,
    signal: f32,
    cv: Option<&[f32]>,
    lane: usize,
    bypassed: bool,
    edited: bool,
) -> Patch {
    let cv = lane_cv(cv, lane);
    fixture(
        kind,
        &[signal],
        cv.as_ref().map(|value| &value[..]),
        bypassed,
        edited,
    )
}

fn render(engine: &mut Engine, frames: usize) -> Vec<PortFrame> {
    let mut output = vec![PortFrame::default(); frames];
    engine.process_poly_block(&mut output);
    output
}

fn render_partitioned(engine: &mut Engine, frames: usize) -> Vec<PortFrame> {
    let mut output = vec![PortFrame::default(); frames];
    let mut offset = 0;
    for size in [1, 3, 17, 2, 71, 5].into_iter().cycle() {
        let end = (offset + size).min(frames);
        engine.process_poly_block(&mut output[offset..end]);
        offset = end;
        if offset == frames {
            break;
        }
    }
    output
}

/// A lane above zero does not exist in the poly circuit until its signal
/// arrives through the v2 cable one sample later. Its mono reference must
/// likewise leave the circuit unprocessed on frame zero while still filling
/// the cable history. A bypassed target provides that public-engine fixture.
fn independent_lane(
    kind: &str,
    signal: f32,
    cv: Option<&[f32]>,
    lane: usize,
    edited: bool,
) -> (Engine, Option<PortFrame>) {
    if lane == 0 {
        return (
            lane_fixture(kind, signal, cv, lane, false, edited)
                .compile()
                .unwrap(),
            None,
        );
    }
    let mut initially_bypassed = lane_fixture(kind, signal, cv, lane, true, edited)
        .compile()
        .unwrap();
    let first = render(&mut initially_bypassed, 1)[0];
    let mut active = lane_fixture(kind, signal, cv, lane, false, edited)
        .compile()
        .unwrap();
    active.adopt_state_from(&initially_bypassed);
    (active, Some(first))
}

fn render_independent_lane(
    kind: &str,
    signal: f32,
    cv: Option<&[f32]>,
    lane: usize,
    edited: bool,
    frames: usize,
) -> Vec<PortFrame> {
    assert!(frames > 0);
    let (mut engine, first) = independent_lane(kind, signal, cv, lane, edited);
    let mut output = Vec::with_capacity(frames);
    if let Some(first) = first {
        output.push(first);
    }
    output.extend(render(&mut engine, frames - output.len()));
    output
}

fn advance_independent_lane(
    kind: &str,
    signal: f32,
    cv: Option<&[f32]>,
    lane: usize,
    frames: usize,
) -> Engine {
    assert!(frames > 0);
    let (mut engine, first) = independent_lane(kind, signal, cv, lane, false);
    render(&mut engine, frames - usize::from(first.is_some()));
    engine
}

fn assert_each_lane(
    actual: &[PortFrame],
    expected: &[Vec<PortFrame>],
    context: &str,
    fresh_graph: bool,
) {
    for (frame_index, frame) in actual.iter().enumerate() {
        let width = if fresh_graph && frame_index == 0 {
            1
        } else {
            expected.len()
        };
        assert_eq!(
            usize::from(frame.channels),
            width,
            "{context} frame {frame_index} width"
        );
        for (lane, mono) in expected.iter().enumerate() {
            assert_eq!(
                frame.values[lane], mono[frame_index].values[0],
                "{context} frame {frame_index} lane {lane}"
            );
        }
        assert!(frame.values[expected.len()..]
            .iter()
            .all(|value| *value == 0.0));
    }
}

fn signal_values(count: usize) -> Vec<f32> {
    (0..count).map(|lane| (lane as f32 - 6.5) * 0.31).collect()
}

fn cv_values(kind: &str, count: usize) -> Vec<f32> {
    (0..count)
        .map(|lane| {
            if kind == "ladder_vcf" {
                -0.8 + lane as f32 * 0.16
            } else {
                0.8 + lane as f32 * 0.57
            }
        })
        .collect()
}

#[test]
fn paired_ladder_and_vca_match_independent_voices_at_odd_even_and_limit_widths() {
    for kind in ["ladder_vcf", "analog_vca"] {
        for width in [1, 2, 3, 15, 16] {
            let signal = signal_values(width);
            let cv = cv_values(kind, width);
            let patch = fixture(kind, &signal, Some(&cv), false, false);
            let actual = render(&mut patch.compile().unwrap(), 257);
            let expected = (0..width)
                .map(|lane| {
                    render_independent_lane(
                        kind,
                        signal[lane],
                        Some(&cv),
                        lane,
                        false,
                        actual.len(),
                    )
                })
                .collect::<Vec<_>>();
            assert_each_lane(&actual, &expected, &format!("{kind} width {width}"), true);
            assert!(
                actual
                    .iter()
                    .skip(8)
                    .any(|frame| frame.values[..width].iter().any(|sample| *sample != 0.0)),
                "{kind} width {width} produced only silence"
            );
        }
    }
}

#[test]
fn primary_signal_width_and_cv_broadcast_or_zero_padding_match_mono_engines() {
    for kind in ["ladder_vcf", "analog_vca"] {
        let signal = signal_values(3);
        for cv in [
            None,
            Some(cv_values(kind, 1)),
            Some(cv_values(kind, 2)),
            Some(cv_values(kind, 4)),
        ] {
            let patch = fixture(kind, &signal, cv.as_deref(), false, false);
            let actual = render(&mut patch.compile().unwrap(), 193);
            let expected = (0..3)
                .map(|lane| {
                    render_independent_lane(
                        kind,
                        signal[lane],
                        cv.as_deref(),
                        lane,
                        false,
                        actual.len(),
                    )
                })
                .collect::<Vec<_>>();
            assert_each_lane(
                &actual,
                &expected,
                &format!("{kind} CV width {:?}", cv.as_ref().map(Vec::len)),
                true,
            );
        }
    }
}

#[test]
fn parameter_edit_adoption_and_callback_partition_preserve_each_circuit_lane() {
    for kind in ["ladder_vcf", "analog_vca"] {
        let signal = signal_values(3);
        let cv = cv_values(kind, 2);
        let old_patch = fixture(kind, &signal, Some(&cv), false, false);
        let changed_patch = fixture(kind, &signal, Some(&cv), false, true);
        let mut old = old_patch.compile().unwrap();
        render(&mut old, 89);
        let mut full = changed_patch.compile().unwrap();
        full.adopt_state_from(&old);
        let mut partitioned = changed_patch.compile().unwrap();
        partitioned.adopt_state_from(&old);
        let actual = render(&mut full, 311);
        assert_eq!(
            render_partitioned(&mut partitioned, 311),
            actual,
            "{kind} partition"
        );
        let expected = (0..signal.len())
            .map(|lane| {
                let previous = advance_independent_lane(kind, signal[lane], Some(&cv), lane, 89);
                let mut edited = lane_fixture(kind, signal[lane], Some(&cv), lane, false, true)
                    .compile()
                    .unwrap();
                edited.adopt_state_from(&previous);
                render(&mut edited, actual.len())
            })
            .collect::<Vec<_>>();
        assert_each_lane(
            &actual,
            &expected,
            &format!("{kind} edited adoption"),
            false,
        );
    }
}

#[test]
fn shrink_retires_inactive_circuits_and_regrowth_starts_them_fresh() {
    for kind in ["ladder_vcf", "analog_vca"] {
        let signal = signal_values(3);
        let cv = cv_values(kind, 3);
        let wide_patch = fixture(kind, &signal, Some(&cv), false, false);
        let narrow_patch = fixture(kind, &signal[..1], Some(&cv[..1]), false, false);
        let mut wide = wide_patch.compile().unwrap();
        render(&mut wide, 117);
        let mut narrow = narrow_patch.compile().unwrap();
        narrow.adopt_state_from(&wide);
        let narrowed = render(&mut narrow, 83);
        assert_eq!(narrowed[0].channels, 3, "{kind} delayed prior width");
        assert!(narrowed[2..].iter().all(|frame| frame.channels == 1));

        let mut old_mono = lane_fixture(kind, signal[0], Some(&cv), 0, false, false)
            .compile()
            .unwrap();
        render(&mut old_mono, 117);
        let mut narrow_mono = lane_fixture(kind, signal[0], Some(&cv[..1]), 0, false, false)
            .compile()
            .unwrap();
        narrow_mono.adopt_state_from(&old_mono);
        let expected_narrow = render(&mut narrow_mono, 83);
        for (frame, expected) in narrowed.iter().zip(&expected_narrow) {
            assert_eq!(frame.values[0], expected.values[0], "{kind} surviving lane");
        }

        let mut regrown = wide_patch.compile().unwrap();
        regrown.adopt_state_from(&narrow);
        let actual = render(&mut regrown, 103);
        assert_eq!(actual[0].channels, 1, "{kind} delayed narrow width");
        assert!(actual[2..].iter().all(|frame| frame.channels == 3));
        let expected = (0..3)
            .map(|lane| {
                if lane == 0 {
                    let mut continuing = lane_fixture(kind, signal[0], Some(&cv), 0, false, false)
                        .compile()
                        .unwrap();
                    continuing.adopt_state_from(&narrow_mono);
                    render(&mut continuing, actual.len())
                } else {
                    render_independent_lane(
                        kind,
                        signal[lane],
                        Some(&cv),
                        lane,
                        false,
                        actual.len(),
                    )
                }
            })
            .collect::<Vec<_>>();
        for (index, frame) in actual.iter().enumerate() {
            assert_eq!(
                frame.values[0], expected[0][index].values[0],
                "{kind} continuing lane frame {index}"
            );
            if index >= 1 {
                for (lane, expected) in expected.iter().enumerate().skip(1) {
                    assert_eq!(
                        frame.values[lane], expected[index].values[0],
                        "{kind} reactivated lane {lane} frame {index}"
                    );
                }
            }
        }
    }
}

#[test]
fn bypass_then_resume_matches_independent_voices_without_advancing_circuit_state() {
    for kind in ["ladder_vcf", "analog_vca"] {
        let signal = signal_values(3);
        let cv = cv_values(kind, 3);
        let active_patch = fixture(kind, &signal, Some(&cv), false, false);
        let bypass_patch = fixture(kind, &signal, Some(&cv), true, false);
        let mut active = active_patch.compile().unwrap();
        render(&mut active, 67);
        let mut bypassed = bypass_patch.compile().unwrap();
        bypassed.adopt_state_from(&active);
        let bypass_actual = render(&mut bypassed, 59);
        let mut mono_bypassed = Vec::new();
        for (lane, value) in signal.iter().copied().enumerate() {
            let mono_active = advance_independent_lane(kind, value, Some(&cv), lane, 67);
            let mut mono = lane_fixture(kind, value, Some(&cv), lane, true, false)
                .compile()
                .unwrap();
            mono.adopt_state_from(&mono_active);
            mono_bypassed.push(mono);
        }
        let expected = mono_bypassed
            .iter_mut()
            .map(|engine| render(engine, 59))
            .collect::<Vec<_>>();
        assert_each_lane(&bypass_actual, &expected, &format!("{kind} bypass"), false);
        let mut resumed = active_patch.compile().unwrap();
        resumed.adopt_state_from(&bypassed);
        let actual = render(&mut resumed, 101);
        let expected = mono_bypassed
            .iter()
            .enumerate()
            .map(|(lane, old)| {
                let mut next = lane_fixture(kind, signal[lane], Some(&cv), lane, false, false)
                    .compile()
                    .unwrap();
                next.adopt_state_from(old);
                render(&mut next, actual.len())
            })
            .collect::<Vec<_>>();
        assert_each_lane(&actual, &expected, &format!("{kind} resumed"), false);
    }
}
