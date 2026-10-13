//! Pair-path exactness against two independently advanced scalar trajectories.
use super::*;
use crate::analog::Vca;

fn same_floats(first: &[f64], second: &[f64]) {
    for (index, (a, b)) in first.iter().zip(second).enumerate() {
        assert_eq!(a.to_bits(), b.to_bits(), "state index {index}");
    }
}
fn same_resampler(first: &Oversampler, second: &Oversampler) {
    same_floats(&first.input, &second.input);
    same_floats(&first.control, &second.control);
    same_floats(&first.output, &second.output);
    assert_eq!(first.control_nonzero, second.control_nonzero);
    assert_eq!(first.input_head, second.input_head);
    assert_eq!(first.output_head, second.output_head);
}
fn same_ladder(first: &Ladder, second: &Ladder) {
    same_resampler(&first.oversampler, &second.oversampler);
    same_floats(&first.integrator.q, &second.integrator.q);
    same_floats(
        &first.integrator.previous_term,
        &second.integrator.previous_term,
    );
    assert_eq!(
        first.integrator.previous_input.to_bits(),
        second.integrator.previous_input.to_bits()
    );
    assert_eq!(first.last_cv.to_bits(), second.last_cv.to_bits());
    let a = first.diagnostics();
    let b = second.diagnostics();
    assert_eq!(a.invalid_inputs, b.invalid_inputs);
    assert_eq!(a.solver_failures, b.solver_failures);
    assert_eq!(a.max_iterations, b.max_iterations);
    assert_eq!(
        a.max_accepted_residual.to_bits(),
        b.max_accepted_residual.to_bits()
    );
}
fn circuit(signal: f64, control: f64) -> f64 {
    (signal * 0.13 + control * 0.27).tanh()
}
fn controls(frame: usize) -> [f64; 2] {
    match frame {
        0..100 => [0.0, ((frame % 9) as f64 - 4.0) * 0.3],
        100..200 => [((frame % 7) as f64 - 3.0) * 0.4, 0.0],
        200..400 => [0.0, 0.0],
        _ => [
            ((frame % 11) as f64 - 5.0) * 0.2,
            ((frame % 13) as f64 - 6.0) * 0.3,
        ],
    }
}

fn controls_four(frame: usize) -> [f64; 4] {
    let first = ((frame % 11) as f64 - 5.0) * 0.2;
    let second = ((frame % 13) as f64 - 6.0) * 0.3;
    match frame {
        0..120 => [0.0, second, first, 0.0],
        120..240 => [first, 0.0, 0.0, second],
        240..360 => [0.0; 4],
        _ => [first, second, -first, -second],
    }
}
#[test]
fn paired_resampling_preserves_every_phase_tap_and_independent_ring_head() {
    let mut first = Oversampler::new();
    let mut second = Oversampler::new();
    for frame in 0..17 {
        first.process_pair(frame as f64 * 0.01, 0.0, circuit);
    }
    for frame in 0..203 {
        second.process_pair(-(frame as f64) * 0.013, 1.0, circuit);
    }
    let mut scalar_first = first.clone();
    let mut scalar_second = second.clone();
    assert_ne!(first.input_head, second.input_head);
    assert_ne!(first.output_head, second.output_head);
    let mut seen = [false; 4];
    for frame in 0..600 {
        let input = [
            (frame as f64 * 0.37).sin() * 9.0,
            (frame as f64 * 0.23).cos() * 7.0,
        ];
        let control = controls(frame);
        let mut expected_signal = [[0.0; 2]; OVERSAMPLING];
        let mut expected_control = expected_signal;
        let mut phase = 0;
        let a = scalar_first.process_pair(input[0], control[0], |x, cv| {
            expected_signal[phase][0] = x;
            expected_control[phase][0] = cv;
            phase += 1;
            circuit(x, cv)
        });
        assert_eq!(phase, OVERSAMPLING);
        phase = 0;
        let b = scalar_second.process_pair(input[1], control[1], |x, cv| {
            expected_signal[phase][1] = x;
            expected_control[phase][1] = cv;
            phase += 1;
            circuit(x, cv)
        });
        assert_eq!(phase, OVERSAMPLING);
        phase = 0;
        let actual = Oversampler::process_two(&mut first, &mut second, input, control, |x, cv| {
            same_floats(&x, &expected_signal[phase]);
            same_floats(&cv, &expected_control[phase]);
            phase += 1;
            [circuit(x[0], cv[0]), circuit(x[1], cv[1])]
        });
        assert_eq!(phase, OVERSAMPLING);
        same_floats(&actual, &[a, b]);
        same_resampler(&first, &scalar_first);
        same_resampler(&second, &scalar_second);
        seen[usize::from(first.control_nonzero > 0)
            + 2 * usize::from(second.control_nonzero > 0)] = true;
    }
    assert_eq!(seen, [true; 4]);
}
#[test]
fn paired_ladders_match_scalar_states_and_diagnostics_at_all_host_rates() {
    for rate in [8000.0_f64, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for (drive0, drive1, resonance0, resonance1) in
            [(1.0, 3.2, 0.3, 0.95), (0.0, 4.0, 1.1, 0.0)]
        {
            let mut first = Ladder::new(rate, 20.0, resonance0, drive0);
            let mut second = Ladder::new(rate, 18_000.0_f64.min(0.4 * rate), resonance1, drive1);
            for frame in 0..17 {
                first.process((frame as f32 * 0.1).sin(), 0.0);
            }
            for frame in 0..93 {
                second.process((frame as f32 * 0.2).cos(), 1.2);
            }
            let mut a = first.clone();
            let mut b = second.clone();
            for frame in 0..256 {
                let mut input = [
                    ((frame as f64 * 0.17).sin() * 12.0) as f32,
                    ((frame as f64 * 0.39).cos() * 20.0) as f32,
                ];
                let mut control = controls(frame).map(|v| v as f32);
                if frame == 37 {
                    input[0] = f32::NAN;
                    control[1] = f32::INFINITY;
                }
                if frame == 149 {
                    input[1] = f32::NEG_INFINITY;
                    control[0] = f32::NAN;
                }
                let expected = [
                    a.process(input[0], control[0]),
                    b.process(input[1], control[1]),
                ];
                let actual = Ladder::process_two(&mut first, &mut second, input, control);
                assert_eq!(
                    actual.map(f32::to_bits),
                    expected.map(f32::to_bits),
                    "{rate} Hz frame {frame}"
                );
                same_ladder(&first, &a);
                same_ladder(&second, &b);
            }
        }
    }
}
#[test]
fn paired_ladder_failure_recovery_and_parameter_adoption_retain_exact_scalar_state() {
    let mut first = Ladder::new(48_000.0, 1000.0, 0.9, 1.0);
    let mut second = Ladder::new(48_000.0, 5000.0, 0.5, 3.0);
    first.integrator.previous_term[0] = f64::INFINITY;
    let mut a = first.clone();
    let mut b = second.clone();
    for frame in 0..80 {
        let input = [frame as f32 * 0.01, -0.2];
        let control = [0.0, 1.0];
        let expected = [
            a.process(input[0], control[0]),
            b.process(input[1], control[1]),
        ];
        let actual = Ladder::process_two(&mut first, &mut second, input, control);
        assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
        same_ladder(&first, &a);
        same_ladder(&second, &b);
    }
    assert_eq!(first.diagnostics().solver_failures, 1);
    let mut changed0 = Ladder::new(48_000.0, 400.0, 0.2, 0.7);
    let mut changed1 = Ladder::new(48_000.0, 8000.0, 1.1, 4.0);
    let mut reference0 = changed0.clone();
    let mut reference1 = changed1.clone();
    changed0.copy_state_from(&first);
    changed1.copy_state_from(&second);
    reference0.copy_state_from(&a);
    reference1.copy_state_from(&b);
    for frame in 0..160 {
        let input = [(frame as f32 * 0.43).sin(), (frame as f32 * 0.71).cos()];
        let control = [0.0, 0.0];
        let actual = Ladder::process_two(&mut changed0, &mut changed1, input, control);
        let expected = [
            reference0.process(input[0], control[0]),
            reference1.process(input[1], control[1]),
        ];
        assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
        same_ladder(&changed0, &reference0);
        same_ladder(&changed1, &reference1);
    }
    assert_eq!(changed0.oversampler.control_nonzero, 0);
    assert_eq!(changed1.oversampler.control_nonzero, 0);
}
#[test]
fn paired_vca_distinct_gains_histories_and_adoption_match_independent_scalar_trajectories() {
    for gains in [[0.0, 1.0], [0.7, 3.2], [4.0, 0.1]] {
        let mut first = Vca::new(gains[0]);
        let mut second = Vca::new(gains[1]);
        for frame in 0..17 {
            first.process(frame as f32 * 0.1, 0.0);
        }
        for frame in 0..203 {
            second.process(-(frame as f32) * 0.03, 10.0);
        }
        let mut a = first.clone();
        let mut b = second.clone();
        for frame in 0..600 {
            let input = [
                (frame as f32 * 0.19).sin() * 30.0,
                (frame as f32 * 0.31).cos() * 20.0,
            ];
            let control = controls(frame).map(|v| (v * 8.0) as f32);
            let expected = [
                a.process(input[0], control[0]),
                b.process(input[1], control[1]),
            ];
            let actual = Vca::process_two(&mut first, &mut second, input, control);
            assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
        }
        let mut adopted0 = Vca::new(0.3);
        let mut adopted1 = Vca::new(2.1);
        let mut reference0 = adopted0.clone();
        let mut reference1 = adopted1.clone();
        adopted0.copy_state_from(&first);
        adopted1.copy_state_from(&second);
        reference0.copy_state_from(&a);
        reference1.copy_state_from(&b);
        // A scalar suffix makes every retained FIR slot observable after the
        // paired prefix and parameter-preserving history adoption.
        for frame in 0..600 {
            let input = [(frame as f32 * 0.53).sin(), (frame as f32 * 0.67).cos()];
            assert_eq!(
                adopted0.process(input[0], 0.0).to_bits(),
                reference0.process(input[0], 0.0).to_bits()
            );
            assert_eq!(
                adopted1.process(input[1], 0.0).to_bits(),
                reference1.process(input[1], 0.0).to_bits()
            );
        }
    }
}

#[test]
fn four_resamplers_match_independent_scalar_phases_and_ring_histories() {
    let mut grouped = std::array::from_fn::<_, 4, _>(|_| Oversampler::new());
    for (lane, warmup) in [17, 203, 41, 89].into_iter().enumerate() {
        for frame in 0..warmup {
            grouped[lane].process_pair(
                ((frame + lane * 19) as f64 * 0.013).sin() * 4.0,
                if lane % 2 == 0 { 0.0 } else { 1.0 },
                circuit,
            );
        }
    }
    let mut scalar = grouped.clone();
    for lane in 1..4 {
        assert_ne!(grouped[0].input_head, grouped[lane].input_head);
        assert_ne!(grouped[0].output_head, grouped[lane].output_head);
    }
    let mut seen_masks = [false; 16];
    for frame in 0..600 {
        let input = std::array::from_fn(|lane| {
            ((frame as f64 * (0.17 + lane as f64 * 0.11)).sin() * (lane as f64 + 3.0))
                + lane as f64 * 0.07
        });
        let control = controls_four(frame);
        let mut expected_signal = [[0.0; 4]; OVERSAMPLING];
        let mut expected_control = expected_signal;
        let mut expected = [0.0; 4];
        for lane in 0..4 {
            let mut phase = 0;
            expected[lane] = scalar[lane].process_pair(input[lane], control[lane], |x, cv| {
                expected_signal[phase][lane] = x;
                expected_control[phase][lane] = cv;
                phase += 1;
                circuit(x, cv)
            });
            assert_eq!(phase, OVERSAMPLING);
        }
        let mut phase = 0;
        let [first, second, third, fourth] = &mut grouped;
        let actual =
            Oversampler::process_four(first, second, third, fourth, input, control, |x, cv| {
                same_floats(&x, &expected_signal[phase]);
                same_floats(&cv, &expected_control[phase]);
                phase += 1;
                std::array::from_fn(|lane| circuit(x[lane], cv[lane]))
            });
        assert_eq!(phase, OVERSAMPLING);
        same_floats(&actual, &expected);
        for lane in 0..4 {
            same_resampler(&grouped[lane], &scalar[lane]);
        }
        let mask = grouped.iter().enumerate().fold(0, |mask, (lane, state)| {
            mask | (usize::from(state.control_nonzero > 0) << lane)
        });
        seen_masks[mask] = true;
    }
    assert!(seen_masks[0] && seen_masks[15]);
    assert!(seen_masks[0b0110] && seen_masks[0b1001]);
}

#[test]
fn four_ladders_preserve_scalar_state_across_rates_and_parameter_adoption() {
    for rate in [8_000.0_f64, 48_000.0, 192_000.0] {
        let cutoff = [20.0, 700.0, 5_000.0, 18_000.0_f64.min(0.4 * rate)];
        let resonance = [0.3, 0.95, 1.1, 0.0];
        let drive = [1.0, 3.2, 0.0, 4.0];
        let mut grouped = std::array::from_fn::<_, 4, _>(|lane| {
            Ladder::new(rate, cutoff[lane], resonance[lane], drive[lane])
        });
        for (lane, warmup) in [17, 93, 41, 157].into_iter().enumerate() {
            for frame in 0..warmup {
                grouped[lane].process(((frame + 7 * lane) as f32 * 0.13).sin(), lane as f32);
            }
        }
        let mut scalar = grouped.clone();
        for frame in 0..160 {
            let mut input = std::array::from_fn(|lane| {
                ((frame as f32 * (0.17 + lane as f32 * 0.09)).sin() * (lane as f32 + 2.0))
                    - lane as f32
            });
            let mut cv = controls_four(frame).map(|value| value as f32);
            if frame == 37 {
                input[2] = f32::NAN;
                cv[3] = f32::INFINITY;
            }
            let expected = std::array::from_fn(|lane| scalar[lane].process(input[lane], cv[lane]));
            let [first, second, third, fourth] = &mut grouped;
            let actual = Ladder::process_four(first, second, third, fourth, input, cv);
            assert_eq!(
                actual.map(f32::to_bits),
                expected.map(f32::to_bits),
                "{rate} Hz frame {frame}"
            );
            for lane in 0..4 {
                same_ladder(&grouped[lane], &scalar[lane]);
            }
        }
        let mut adopted = std::array::from_fn::<_, 4, _>(|lane| {
            Ladder::new(
                rate,
                300.0 + lane as f64 * 1700.0,
                0.2 * lane as f64,
                0.7 + lane as f64,
            )
        });
        let mut references = adopted.clone();
        for lane in 0..4 {
            adopted[lane].copy_state_from(&grouped[lane]);
            references[lane].copy_state_from(&scalar[lane]);
        }
        for frame in 0..160 {
            let input = std::array::from_fn(|lane| {
                ((frame as f32 * (0.31 + lane as f32 * 0.07)).cos() * 5.0) - lane as f32
            });
            let cv = controls_four(frame + 240).map(|value| value as f32);
            let expected =
                std::array::from_fn(|lane| references[lane].process(input[lane], cv[lane]));
            let [first, second, third, fourth] = &mut adopted;
            let actual = Ladder::process_four(first, second, third, fourth, input, cv);
            assert_eq!(
                actual.map(f32::to_bits),
                expected.map(f32::to_bits),
                "{rate} Hz adopted frame {frame}"
            );
            for lane in 0..4 {
                same_ladder(&adopted[lane], &references[lane]);
            }
        }
    }
}

#[test]
fn four_vcas_with_distinct_gains_and_history_match_scalar_after_adoption() {
    let mut grouped = [Vca::new(0.37), Vca::new(1.0), Vca::new(0.7), Vca::new(3.2)];
    for (lane, warmup) in [17, 203, 41, 89].into_iter().enumerate() {
        for frame in 0..warmup {
            grouped[lane].process((frame as f32 * 0.13).sin() * 10.0, lane as f32 * 3.0);
        }
    }
    let mut scalar = grouped.clone();
    for frame in 0..400 {
        let input = std::array::from_fn(|lane| {
            (frame as f32 * (0.19 + lane as f32 * 0.07)).sin() * (lane as f32 + 5.0)
        });
        let cv = controls_four(frame).map(|value| (value * 8.0) as f32);
        let expected = std::array::from_fn(|lane| scalar[lane].process(input[lane], cv[lane]));
        let [first, second, third, fourth] = &mut grouped;
        let actual = Vca::process_four(first, second, third, fourth, input, cv);
        assert_eq!(
            actual.map(f32::to_bits),
            expected.map(f32::to_bits),
            "frame {frame}"
        );
    }
    let mut adopted = [Vca::new(0.3), Vca::new(2.1), Vca::new(4.0), Vca::new(0.1)];
    let mut references = adopted.clone();
    for lane in 0..4 {
        adopted[lane].copy_state_from(&grouped[lane]);
        references[lane].copy_state_from(&scalar[lane]);
    }
    for frame in 0..600 {
        let input = std::array::from_fn(|lane| {
            (frame as f32 * (0.53 + lane as f32 * 0.05)).sin() * (lane as f32 + 1.0)
        });
        let cv = controls_four(frame + 360).map(|value| (value * 8.0) as f32);
        let expected = std::array::from_fn(|lane| references[lane].process(input[lane], cv[lane]));
        let [first, second, third, fourth] = &mut adopted;
        let actual = Vca::process_four(first, second, third, fourth, input, cv);
        assert_eq!(
            actual.map(f32::to_bits),
            expected.map(f32::to_bits),
            "adopted frame {frame}"
        );
    }
}

// This oracle uses the published polyphase kernel but indexes every tap in
// chronological order per phase. It does not call either production
// interpolation implementation or depend on how its loops are grouped.
fn interpolation_oracle(
    signals: [&[f64]; 2],
    controls: [&[f64]; 2],
    active: [bool; 2],
) -> ([[f64; 2]; OVERSAMPLING], [[f64; 2]; OVERSAMPLING]) {
    let mut output = [[0.0; 2]; OVERSAMPLING];
    let mut control_output = output;
    for phase in 0..OVERSAMPLING {
        for lane in 0..2 {
            for tap in 0..INPUT_TAPS {
                let kernel_index = tap * OVERSAMPLING + phase;
                if kernel_index >= TAPS {
                    break;
                }
                let history_index = INPUT_TAPS - 1 - tap;
                output[phase][lane] += KERNEL[kernel_index] * signals[lane][history_index];
                if active[lane] {
                    control_output[phase][lane] +=
                        KERNEL[kernel_index] * controls[lane][history_index];
                }
            }
        }
    }
    (output, control_output)
}

fn assert_interpolation_bits<const FIRST_CONTROL: bool, const SECOND_CONTROL: bool>(
    signals: [&[f64]; 2],
    controls: [&[f64]; 2],
    context: &str,
) {
    let active = [FIRST_CONTROL, SECOND_CONTROL];
    let expected = interpolation_oracle(signals, controls, active);
    let actual = interpolate_two::<FIRST_CONTROL, SECOND_CONTROL>(signals, controls);
    for phase in 0..OVERSAMPLING {
        for lane in 0..2 {
            assert_eq!(
                actual.0[phase][lane].to_bits(),
                expected.0[phase][lane].to_bits(),
                "{context} signal phase {phase} lane {lane}"
            );
            assert_eq!(
                actual.1[phase][lane].to_bits(),
                expected.1[phase][lane].to_bits(),
                "{context} control phase {phase} lane {lane}"
            );
        }
    }
}

fn check_control_modes(signals: [&[f64]; 2], controls: [&[f64]; 2], context: &str) {
    assert_interpolation_bits::<false, false>(signals, [&[], &[]], context);
    assert_interpolation_bits::<true, false>(signals, [controls[0], &[]], context);
    assert_interpolation_bits::<false, true>(signals, [&[], controls[1]], context);
    assert_interpolation_bits::<true, true>(signals, controls, context);
}

#[test]
fn paired_interpolation_matches_independent_tap_oracle_for_all_control_modes_and_offsets() {
    for (first_offset, second_offset) in [(0, 0), (1, 3), (7, 2), (3, 11)] {
        let mut first_signal = vec![f64::NAN; first_offset + INPUT_TAPS + 3];
        let mut second_signal = vec![f64::NAN; second_offset + INPUT_TAPS + 5];
        let mut first_control = vec![f64::NAN; first_signal.len()];
        let mut second_control = vec![f64::NAN; second_signal.len()];
        for tap in 0..INPUT_TAPS {
            first_signal[first_offset + tap] =
                ((tap * 37 % 101) as f64 - 50.0) * 0.037 + tap as f64 * 0.0001;
            second_signal[second_offset + tap] =
                ((tap * 19 % 97) as f64 - 48.0) * -0.083 - tap as f64 * 0.0003;
            first_control[first_offset + tap] = ((tap * 11 % 53) as f64 - 26.0) * 0.17;
            second_control[second_offset + tap] = ((tap * 23 % 71) as f64 - 35.0) * -0.11;
        }
        let signals = [
            &first_signal[first_offset..first_offset + INPUT_TAPS],
            &second_signal[second_offset..second_offset + INPUT_TAPS],
        ];
        let controls = [
            &first_control[first_offset..first_offset + INPUT_TAPS],
            &second_control[second_offset..second_offset + INPUT_TAPS],
        ];
        check_control_modes(
            signals,
            controls,
            &format!("signal offsets {first_offset}/{second_offset}"),
        );
    }
}

#[test]
fn paired_interpolation_keeps_signed_zero_subnormals_and_large_finite_lanes_separate() {
    let signed_zeros = [0.0, -0.0];
    let subnormals = [
        f64::from_bits(1),
        f64::from_bits((1_u64 << 63) | 3),
        f64::from_bits((1_u64 << 52) - 1),
        f64::from_bits((1_u64 << 63) | ((1_u64 << 52) - 1)),
    ];
    let mixed_finite = [
        0.0,
        -0.0,
        f64::from_bits(1),
        f64::from_bits((1_u64 << 63) | 3),
        f64::MIN_POSITIVE,
        -f64::MIN_POSITIVE,
        1.0e180,
        -1.0e180,
        0.125,
        -0.75,
    ];
    for (name, values) in [
        ("signed zeros only", &signed_zeros[..]),
        ("subnormals only", &subnormals[..]),
        ("mixed finite extremes", &mixed_finite[..]),
    ] {
        let mut first_signal = [0.0; INPUT_TAPS];
        let mut second_signal = [0.0; INPUT_TAPS];
        let mut first_control = [0.0; INPUT_TAPS];
        let mut second_control = [0.0; INPUT_TAPS];
        for tap in 0..INPUT_TAPS {
            first_signal[tap] = values[tap % values.len()];
            second_signal[tap] = values[(tap * 3 + 7) % values.len()];
            first_control[tap] = values[(tap * 7 + 1) % values.len()];
            second_control[tap] = values[(tap * 9 + 4) % values.len()];
        }
        check_control_modes(
            [&first_signal, &second_signal],
            [&first_control, &second_control],
            name,
        );
    }
}

#[test]
fn paired_interpolation_includes_the_final_phase_zero_tap() {
    assert_ne!(KERNEL[TAPS - 1], 0.0, "final tap must be observable");
    let mut first_signal = [0.0; INPUT_TAPS];
    let mut second_signal = [0.0; INPUT_TAPS];
    let mut first_control = [0.0; INPUT_TAPS];
    let mut second_control = [0.0; INPUT_TAPS];
    first_signal[0] = 1.0e200;
    second_signal[0] = -3.0e199;
    first_control[0] = -2.0e200;
    second_control[0] = 4.0e199;
    let expected_first = KERNEL[TAPS - 1] * first_signal[0];
    let expected_second = KERNEL[TAPS - 1] * second_signal[0];
    assert!(expected_first.is_finite() && expected_first != 0.0);
    assert!(expected_second.is_finite() && expected_second != 0.0);
    check_control_modes(
        [&first_signal, &second_signal],
        [&first_control, &second_control],
        "tap 512",
    );
    let actual = interpolate_two::<true, true>(
        [&first_signal, &second_signal],
        [&first_control, &second_control],
    );
    assert_eq!(
        actual.0[0].map(f64::to_bits),
        [expected_first, expected_second].map(f64::to_bits)
    );
    for phase in 1..OVERSAMPLING {
        assert_eq!(actual.0[phase], [0.0; 2]);
        assert_eq!(actual.1[phase], [0.0; 2]);
    }
}
