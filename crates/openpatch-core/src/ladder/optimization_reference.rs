//! Frozen scalar optimization oracle from git 98edb73 (ladder.rs).
//!
//! Keep this independent of production FIR/solver helpers and arithmetic order.
//! This catches implementation drift; it does not replace the independent ODE
//! and physical-response tests. Exactness is within the same build/platform,
//! not a claim of cross-platform libm bit identity. Only observable resampler
//! outputs/callbacks are compared, permitting production history-layout changes.

use super::{Integrator, Oversampler};
use std::f64::consts::{PI, TAU};

const FROZEN_TEMPERATURE_K: f64 = 298.15;
const FROZEN_THERMAL_VOLTAGE: f64 = 1.380_649e-23 * FROZEN_TEMPERATURE_K / 1.602_176_634e-19;
const FROZEN_OUTPUT_GAIN: f64 = 100.0;
const FROZEN_OVERSAMPLING: usize = 8;
const FROZEN_TAPS: usize = 513;
const FROZEN_INPUT_TAPS: usize = FROZEN_TAPS.div_ceil(FROZEN_OVERSAMPLING);
const FROZEN_MAX_NEWTON: usize = 8;
const FROZEN_MAX_BACKTRACK: usize = 4;
const FROZEN_RESIDUAL_TOLERANCE: f64 = 2e-12;

// Taylor evaluation is only used at compile time; runtime processing uses the
// precomputed coefficients. Range reduction avoids large polynomial arguments.
const fn frozen_sin_const(mut x: f64) -> f64 {
    while x > PI {
        x -= TAU;
    }
    while x < -PI {
        x += TAU;
    }
    let mut term = x;
    let mut result = x;
    let mut n = 1;
    while n < 20 {
        term *= -x * x / ((2 * n) * (2 * n + 1)) as f64;
        result += term;
        n += 1;
    }
    result
}
const fn frozen_make_kernel() -> [f64; FROZEN_TAPS] {
    let mut h = [0.0; FROZEN_TAPS];
    let mut sum = 0.0;
    let mut i = 0;
    let cutoff = 0.45 / FROZEN_OVERSAMPLING as f64;
    while i < FROZEN_TAPS {
        let t = i as f64 - (FROZEN_TAPS - 1) as f64 / 2.0;
        let phase = TAU * i as f64 / (FROZEN_TAPS - 1) as f64;
        let window = 0.42 - 0.5 * frozen_sin_const(phase + PI / 2.0)
            + 0.08 * frozen_sin_const(2.0 * phase + PI / 2.0);
        let sinc = if i == (FROZEN_TAPS - 1) / 2 {
            2.0 * cutoff
        } else {
            frozen_sin_const(TAU * cutoff * t) / (PI * t)
        };
        h[i] = sinc * window;
        sum += h[i];
        i += 1;
    }
    i = 0;
    while i < FROZEN_TAPS {
        h[i] /= sum;
        i += 1;
    }
    h
}
const FROZEN_KERNEL: [f64; FROZEN_TAPS] = frozen_make_kernel();

/// Shared allocation-free oversampling for memoryless or stateful circuits.
/// The callback is invoked eight times in chronological order per host sample.
#[derive(Clone, Debug)]
struct FrozenOversampler {
    input: [f64; FROZEN_INPUT_TAPS],
    output: [f64; FROZEN_TAPS],
    control: [f64; FROZEN_INPUT_TAPS],
    control_nonzero: usize,
    input_head: usize,
    output_head: usize,
}
impl FrozenOversampler {
    fn new() -> Self {
        Self {
            input: [0.0; FROZEN_INPUT_TAPS],
            output: [0.0; FROZEN_TAPS],
            control: [0.0; FROZEN_INPUT_TAPS],
            control_nonzero: 0,
            input_head: 0,
            output_head: 0,
        }
    }
    /// Reconstruct signal and control with identical phase and group delay.
    /// The zero-control fast path skips control FIR work when its history is zero.
    #[inline]
    fn process_pair(
        &mut self,
        input: f64,
        control: f64,
        mut circuit: impl FnMut(f64, f64) -> f64,
    ) -> f64 {
        self.input_head = (self.input_head + 1) % FROZEN_INPUT_TAPS;
        self.input[self.input_head] = input;
        self.control_nonzero -= usize::from(self.control[self.input_head] != 0.0);
        self.control[self.input_head] = control;
        self.control_nonzero += usize::from(control != 0.0);
        let mut result = 0.0;
        for phase in 0..FROZEN_OVERSAMPLING {
            let mut interpolated = 0.0;
            let mut interpolated_control = 0.0;
            let mut tap = phase;
            let mut history = self.input_head;
            while tap < FROZEN_TAPS {
                interpolated += FROZEN_KERNEL[tap] * self.input[history];
                if self.control_nonzero > 0 {
                    interpolated_control += FROZEN_KERNEL[tap] * self.control[history];
                }
                history = if history == 0 {
                    FROZEN_INPUT_TAPS - 1
                } else {
                    history - 1
                };
                tap += FROZEN_OVERSAMPLING;
            }
            self.output_head = (self.output_head + 1) % FROZEN_TAPS;
            self.output[self.output_head] = circuit(
                interpolated * FROZEN_OVERSAMPLING as f64,
                interpolated_control * FROZEN_OVERSAMPLING as f64,
            );
            // Decimate at phase zero to retain the exact integer 64-sample delay.
            if phase == 0 {
                let mut history = self.output_head;
                for coefficient in FROZEN_KERNEL {
                    result += coefficient * self.output[history];
                    history = if history == 0 {
                        FROZEN_TAPS - 1
                    } else {
                        history - 1
                    };
                }
            }
        }
        result
    }
}

/// Numerical diagnostics are cumulative and require no audio-thread I/O.
#[derive(Clone, Copy, Debug, Default)]
struct FrozenDiagnostics {
    solver_failures: u64,
    invalid_inputs: u64,
    max_iterations: u8,
    max_accepted_residual: f64,
}

#[derive(Clone, Debug)]
struct FrozenIntegrator {
    q: [f64; 4],
    previous_term: [f64; 4],
    previous_input: f64,
    diagnostics: FrozenDiagnostics,
}
fn frozen_currents(q: [f64; 4], u: f64, k: f64) -> ([f64; 4], [f64; 4], f64) {
    let t = q.map(f64::tanh);
    let input = (u - k * q[3]).tanh();
    (
        [input - t[0], t[0] - t[1], t[1] - t[2], t[2] - t[3]],
        t.map(|x| 1.0 - x * x),
        1.0 - input * input,
    )
}
fn frozen_max_abs(a: [f64; 4]) -> f64 {
    a.into_iter().fold(0.0_f64, |m, x| {
        if x.is_finite() {
            m.max(x.abs())
        } else {
            f64::INFINITY
        }
    })
}
impl FrozenIntegrator {
    fn new() -> Self {
        Self {
            q: [0.0; 4],
            previous_term: [0.0; 4],
            previous_input: 0.0,
            diagnostics: FrozenDiagnostics::default(),
        }
    }
    #[inline]
    fn step(&mut self, input: f64, scale: f64, g: f64, k: f64) -> f64 {
        let u = input * scale;
        let old = self.q;
        // Start at the previous solution. The cyclic bidiagonal Jacobian is
        // solved analytically, including the instantaneous feedback corner.
        let mut candidate = old;
        let mut accepted_residual = f64::INFINITY;
        let mut iterations = 0;
        for iteration in 0..FROZEN_MAX_NEWTON {
            iterations = iteration + 1;
            let (f, derivative, input_derivative) = frozen_currents(candidate, u, k);
            let residual =
                std::array::from_fn(|i| candidate[i] - old[i] - self.previous_term[i] - g * f[i]);
            let norm = frozen_max_abs(residual);
            if norm <= FROZEN_RESIDUAL_TOLERANCE * (1.0 + frozen_max_abs(candidate)) {
                accepted_residual = norm;
                break;
            }
            let diagonal = derivative.map(|d| 1.0 + g * d);
            let corner = g * k * input_derivative;
            let mut a = [1.0; 4];
            let mut b = [0.0; 4];
            for i in 1..4 {
                let sub = g * derivative[i - 1];
                a[i] = sub * a[i - 1] / diagonal[i];
                b[i] = (-residual[i] + sub * b[i - 1]) / diagonal[i];
            }
            let first = (-residual[0] - corner * b[3]) / (diagonal[0] + corner * a[3]);
            let delta: [f64; 4] = std::array::from_fn(|i| a[i] * first + b[i]);
            let mut damping = 1.0;
            let mut improved = false;
            for _ in 0..FROZEN_MAX_BACKTRACK {
                let trial = std::array::from_fn(|i| candidate[i] + damping * delta[i]);
                if trial.iter().all(|x| x.is_finite()) {
                    let (trial_f, _, _) = frozen_currents(trial, u, k);
                    let trial_residual = frozen_max_abs(std::array::from_fn(|i| {
                        trial[i] - old[i] - self.previous_term[i] - g * trial_f[i]
                    }));
                    if trial_residual < norm {
                        candidate = trial;
                        if trial_residual
                            <= FROZEN_RESIDUAL_TOLERANCE * (1.0 + frozen_max_abs(trial))
                        {
                            accepted_residual = trial_residual;
                        }
                        improved = true;
                        break;
                    }
                }
                damping *= 0.5;
            }
            if accepted_residual.is_finite() || !improved {
                break;
            }
        }
        self.diagnostics.max_iterations = self.diagnostics.max_iterations.max(iterations as u8);
        if accepted_residual.is_finite() {
            self.q = candidate;
            self.diagnostics.max_accepted_residual = self
                .diagnostics
                .max_accepted_residual
                .max(accepted_residual);
        } else {
            // Bounded failure policy: retain the last valid capacitor voltages,
            // count the event, and resume from a consistent derivative next step.
            // No hidden clipping or unbounded Newton retries.
            self.diagnostics.solver_failures = self.diagnostics.solver_failures.saturating_add(1);
        }
        self.previous_term = frozen_currents(self.q, u, k).0.map(|f| g * f);
        self.previous_input = input;
        FROZEN_OUTPUT_GAIN * 2.0 * FROZEN_THERMAL_VOLTAGE * self.q[3]
    }
}

fn assert_bits(actual: f64, expected: f64, label: &str, sample: usize) {
    assert_eq!(
        actual.to_bits(),
        expected.to_bits(),
        "{label} at sample {sample}: actual={actual:?}, expected={expected:?}"
    );
}

fn assert_solver_state(actual: &Integrator, expected: &FrozenIntegrator, sample: usize) {
    for channel in 0..4 {
        assert_bits(actual.q[channel], expected.q[channel], "capacitor", sample);
        assert_bits(
            actual.previous_term[channel],
            expected.previous_term[channel],
            "previous term",
            sample,
        );
    }
    assert_bits(
        actual.previous_input,
        expected.previous_input,
        "previous input",
        sample,
    );
    assert_eq!(
        actual.diagnostics.solver_failures,
        expected.diagnostics.solver_failures
    );
    assert_eq!(
        actual.diagnostics.invalid_inputs,
        expected.diagnostics.invalid_inputs
    );
    assert_eq!(
        actual.diagnostics.max_iterations,
        expected.diagnostics.max_iterations
    );
    assert_bits(
        actual.diagnostics.max_accepted_residual,
        expected.diagnostics.max_accepted_residual,
        "maximum residual",
        sample,
    );
}

fn stimulus(n: usize) -> (f64, f64) {
    let signal = match n % 389 {
        0 => 7.0,
        1..=64 => 0.0,
        65..=127 => {
            if n.is_multiple_of(2) {
                2.5
            } else {
                -2.5
            }
        }
        _ => (n as f64 * 0.071).sin() + 0.3 * (n as f64 * 0.173).cos(),
    };
    // Initial zero, nonzero, exact signed-zero drain, audio-rate CV, then
    // another >65-sample drain. This exercises both control fast-path states.
    let control = match n % 521 {
        0..=79 => 0.0,
        80..=159 => 0.75,
        160..=239 => -0.0,
        240..=399 => 1.2 * (n as f64 * 0.213).sin(),
        _ => 0.0,
    };
    (signal, control)
}

#[test]
fn frozen_resampler_matches_bits_callback_order_wraps_and_control_drain() {
    for nonlinear in [false, true] {
        let mut actual = Oversampler::new();
        let mut expected = FrozenOversampler::new();
        let (mut state_a, mut state_b) = (0.0_f64, 0.0_f64);
        for n in 0..1303 {
            // Copy at different ring phases while a nonzero control tail exists.
            if n == 193 || n == 777 {
                let mut replacement = Oversampler::new();
                replacement.copy_state_from(&actual);
                actual = replacement;
                expected = expected.clone();
            }
            let (signal, control) = stimulus(n);
            let mut calls_a = [(0_u64, 0_u64); 8];
            let mut calls_b = [(0_u64, 0_u64); 8];
            let (mut count_a, mut count_b) = (0, 0);
            let a = actual.process_pair(signal, control, |x, cv| {
                calls_a[count_a] = (x.to_bits(), cv.to_bits());
                count_a += 1;
                state_a = if nonlinear {
                    (0.81 * state_a + x + 0.17 * cv).tanh()
                } else {
                    0.81 * state_a + 0.13 * x + 0.17 * cv
                };
                state_a
            });
            let b = expected.process_pair(signal, control, |x, cv| {
                calls_b[count_b] = (x.to_bits(), cv.to_bits());
                count_b += 1;
                state_b = if nonlinear {
                    (0.81 * state_b + x + 0.17 * cv).tanh()
                } else {
                    0.81 * state_b + 0.13 * x + 0.17 * cv
                };
                state_b
            });
            assert_eq!((count_a, count_b), (8, 8));
            assert_eq!(calls_a, calls_b, "callback reconstruction/order at {n}");
            assert_bits(state_a, state_b, "callback state", n);
            assert_bits(a, b, "decimated output", n);
        }
    }
}

#[test]
fn frozen_solver_matches_bits_across_drive_feedback_rates_and_coefficient_changes() {
    for rate in [8_000.0_f64, 44_100.0, 48_000.0, 192_000.0] {
        for (pole, resonance, drive) in [
            (20.0, 0.0, 0.01),
            (1000.0, 0.8, 1.0),
            (18_000.0, 1.1, 4.0),
            (1000.0, 0.9, 0.0),
        ] {
            let mut actual = Integrator::new();
            let mut expected = FrozenIntegrator::new();
            let scale = 0.01 * drive / (2.0 * FROZEN_THERMAL_VOLTAGE);
            let k = 4.0 * resonance;
            for n in 0..1024 {
                let (signal, cv) = stimulus(n);
                let frequency = (pole * cv.exp2()).clamp(20.0, 18_000.0_f64.min(0.4 * rate));
                let g = PI * frequency / (rate * 8.0);
                let a = actual.step(signal, scale, g, k);
                let b = expected.step(signal, scale, g, k);
                assert_bits(a, b, "solver output", n);
                assert_solver_state(&actual, &expected, n);
            }
            assert_eq!(actual.diagnostics.solver_failures, 0);
        }
    }
}

#[test]
fn frozen_resampler_and_solver_composition_matches_each_host_sample() {
    // Covers coupled history without copying Ladder or replacing its separate
    // physical tests. Reconstructed CV sets the same pole for both solvers.
    let mut resampler_a = Oversampler::new();
    let mut resampler_b = FrozenOversampler::new();
    let mut solver_a = Integrator::new();
    let mut solver_b = FrozenIntegrator::new();
    let scale = 0.01 * 4.0 / (2.0 * FROZEN_THERMAL_VOLTAGE);
    for n in 0..769 {
        let (signal, control) = stimulus(n);
        let a = resampler_a.process_pair(signal, control, |x, cv| {
            let pole = (2700.0 * cv.exp2()).clamp(20.0, 18_000.0);
            solver_a.step(x, scale, PI * pole / (48_000.0 * 8.0), 4.4)
        });
        let b = resampler_b.process_pair(signal, control, |x, cv| {
            let pole = (2700.0 * cv.exp2()).clamp(20.0, 18_000.0);
            solver_b.step(x, scale, PI * pole / (48_000.0 * 8.0), 4.4)
        });
        assert_bits(a, b, "combined output", n);
        assert_solver_state(&solver_a, &solver_b, n);
    }
    assert_eq!(solver_a.diagnostics.solver_failures, 0);
}

#[test]
fn frozen_solver_failure_retention_saturation_and_recovery_match() {
    let mut actual = Integrator::new();
    let mut expected = FrozenIntegrator::new();
    for n in 0..80 {
        let signal = (n as f64 * 0.2).sin();
        assert_bits(
            actual.step(signal, 0.4, 0.1, 4.4),
            expected.step(signal, 0.4, 0.1, 4.4),
            "warmup",
            n,
        );
    }
    for corruption in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let before = actual.q.map(f64::to_bits);
        let failures = actual.diagnostics.solver_failures;
        actual.previous_term[0] = corruption;
        expected.previous_term[0] = corruption;
        let a = actual.step(0.25, 0.4, 0.1, 4.4);
        let b = expected.step(0.25, 0.4, 0.1, 4.4);
        assert_bits(a, b, "failure output", 0);
        assert_eq!(actual.q.map(f64::to_bits), before);
        assert_eq!(actual.diagnostics.solver_failures, failures + 1);
        assert_solver_state(&actual, &expected, 0);
        for n in 1..17 {
            assert_bits(
                actual.step(-0.5, 0.4, 0.1, 4.4),
                expected.step(-0.5, 0.4, 0.1, 4.4),
                "recovery",
                n,
            );
            assert_solver_state(&actual, &expected, n);
        }
        assert_eq!(actual.diagnostics.solver_failures, failures + 1);
    }
    actual.diagnostics.solver_failures = u64::MAX;
    expected.diagnostics.solver_failures = u64::MAX;
    actual.previous_term[2] = f64::INFINITY;
    expected.previous_term[2] = f64::INFINITY;
    assert_bits(
        actual.step(0.0, 1.0, 0.1, 3.6),
        expected.step(0.0, 1.0, 0.1, 3.6),
        "saturated counter",
        0,
    );
    assert_solver_state(&actual, &expected, 0);
    assert_eq!(actual.diagnostics.solver_failures, u64::MAX);
}

#[test]
fn paired_ladders_match_independent_frozen_circuits_sample_for_sample() {
    for rate in [8_000.0_f64, 48_000.0, 192_000.0] {
        let poles = [20.0, 18_000.0_f64.min(rate * 0.4)];
        let drives = [0.25, 4.0];
        let feedback = [0.0, 4.4];
        let mut actual = [
            super::Ladder::new(rate, poles[0], feedback[0] / 4.0, drives[0]),
            super::Ladder::new(rate, poles[1], feedback[1] / 4.0, drives[1]),
        ];
        let mut resamplers = [FrozenOversampler::new(), FrozenOversampler::new()];
        let mut solvers = [FrozenIntegrator::new(), FrozenIntegrator::new()];
        for n in 0..1025 {
            let signals = [stimulus(n).0 as f32, stimulus(n + 319).0 as f32];
            let controls = [stimulus(n).1 as f32, stimulus(n + 71).1 as f32];
            let [first, second] = &mut actual;
            let output = super::Ladder::process_two(first, second, signals, controls);
            for lane in 0..2 {
                let mut expected_cv = 0.0;
                let expected = resamplers[lane].process_pair(
                    f64::from(signals[lane]),
                    f64::from(controls[lane]),
                    |signal, cv| {
                        expected_cv = cv;
                        let g = if cv == 0.0 {
                            PI * poles[lane] / (rate * FROZEN_OVERSAMPLING as f64)
                        } else {
                            let frequency = (poles[lane] * cv.clamp(-32.0, 32.0).exp2())
                                .clamp(20.0, 18_000.0_f64.min(0.4 * rate));
                            PI * frequency / (rate * FROZEN_OVERSAMPLING as f64)
                        };
                        solvers[lane].step(
                            signal,
                            0.01 * drives[lane] / (2.0 * FROZEN_THERMAL_VOLTAGE),
                            g,
                            feedback[lane],
                        )
                    },
                ) as f32;
                assert_eq!(
                    output[lane].to_bits(),
                    expected.to_bits(),
                    "rate {rate} lane {lane} sample {n}"
                );
                assert_bits(actual[lane].last_cv, expected_cv, "paired last CV", n);
                assert_solver_state(&actual[lane].integrator, &solvers[lane], n);
            }
        }
    }
}

#[test]
fn paired_vcas_match_independent_frozen_resamplers_and_ota_equations() {
    let gains = [0.37, 2.0];
    let mut actual = [
        crate::analog::Vca::new(gains[0]),
        crate::analog::Vca::new(gains[1]),
    ];
    let mut expected = [FrozenOversampler::new(), FrozenOversampler::new()];
    for n in 0..2049 {
        let signals = [stimulus(n).0 as f32, stimulus(n + 349).0 as f32];
        let controls = [
            stimulus(n).1 as f32 * 10.0,
            5.0 + stimulus(n + 37).1 as f32 * 7.0,
        ];
        let [first, second] = &mut actual;
        let output = crate::analog::Vca::process_two(first, second, signals, controls);
        for lane in 0..2 {
            let value = expected[lane].process_pair(
                f64::from(signals[lane]),
                f64::from(controls[lane]),
                |signal, control| {
                    let scale = 2.0 * FROZEN_THERMAL_VOLTAGE / 0.01;
                    scale * (signal / scale).tanh() * gains[lane] * (control / 10.0).clamp(0.0, 1.0)
                },
            ) as f32;
            assert_eq!(
                output[lane].to_bits(),
                value.to_bits(),
                "lane {lane} sample {n}"
            );
        }
    }
}
