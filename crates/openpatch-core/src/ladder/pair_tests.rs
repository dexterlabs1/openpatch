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
