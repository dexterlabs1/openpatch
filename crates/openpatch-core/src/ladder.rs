//! Original matched-transistor ladder reference circuit, derived from the ODE in
//! A. Huovilainen, DAFx 2004, section 3 (https://www.dafx.de/paper-archive/2004/P_061.PDF).
//! We use implicit trapezoidal integration, not that paper's Euler implementation.
//!
//! Assumptions: matched BJTs, infinite beta, no Early effect, constant 25 °C,
//! ideal buffers and output amplifier; four 10 nF capacitors. Our chosen input
//! divider 0.01 and output gain 100 are reference-circuit interface values,
//! not measurements or a calibration of a commercial instrument. With
//! q_i = v_i/(2 V_T), u = 0.01 * drive * Vin/(2 V_T), and k = 4 * resonance:
//! q1' = omega * (tanh(u-k*q4)-tanh(q1));
//! qi' = omega * (tanh(q[i-1])-tanh(qi)), i=2..4.
//! The physical half-bias current is Ictl = 4*pi*V_T*C*pole_hz.
//! Output volts = 100 * 2*V_T*q4. No output limiter is part of this circuit.
//!
//! `cutoff` denotes each stage's small-signal pole, not the complete ladder's
//! -3 dB point. At resonance=0 the response is -12.0412 dB at cutoff; the
//! complete -3.0103 dB point is 0.43497944 * cutoff. For small signals,
//! H(s) = drive / ((1+s/omega)^4+k); DC gain is drive/(1+k).
//!
//! Eightfold oversampling uses actual polyphase sinc interpolation and FIR
//! antialias decimation. Both 513-tap Blackman-windowed FIRs have cutoff
//! 0.45*host_rate and group delay 32 host samples, for 64 samples total latency.
//! Signal and cutoff CV use matched interpolation, including its 32-sample delay.
//! Their passband extends approximately to 0.40*host_rate; the transition band
//! is deliberately below Nyquist. The tests quantify stopband attenuation.
//! Finite oversampling cannot remove harmonics already aliased above the
//! internal Nyquist frequency; this is a numerical reference circuit, not a
//! claim of alias-free operation or measured-hardware equivalence.

use std::f64::consts::{PI, TAU};

pub const TEMPERATURE_K: f64 = 298.15;
pub const THERMAL_VOLTAGE: f64 = 1.380_649e-23 * TEMPERATURE_K / 1.602_176_634e-19;
pub const CAPACITANCE_F: f64 = 10e-9;
pub const INPUT_DIVIDER: f64 = 0.01;
pub const OUTPUT_GAIN: f64 = 100.0;
pub const OVERSAMPLING: usize = 8;
pub const LATENCY_SAMPLES: usize = 64;
const TAPS: usize = 513;
const INPUT_TAPS: usize = TAPS.div_ceil(OVERSAMPLING);
const MAX_NEWTON: usize = 8;
const MAX_BACKTRACK: usize = 4;
const RESIDUAL_TOLERANCE: f64 = 2e-12;

// Taylor evaluation is only used at compile time; runtime processing uses the
// precomputed coefficients. Range reduction avoids large polynomial arguments.
const fn sin_const(mut x: f64) -> f64 {
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
const fn make_kernel() -> [f64; TAPS] {
    let mut h = [0.0; TAPS];
    let mut sum = 0.0;
    let mut i = 0;
    let cutoff = 0.45 / OVERSAMPLING as f64;
    while i < TAPS {
        let t = i as f64 - (TAPS - 1) as f64 / 2.0;
        let phase = TAU * i as f64 / (TAPS - 1) as f64;
        let window =
            0.42 - 0.5 * sin_const(phase + PI / 2.0) + 0.08 * sin_const(2.0 * phase + PI / 2.0);
        let sinc = if i == (TAPS - 1) / 2 {
            2.0 * cutoff
        } else {
            sin_const(TAU * cutoff * t) / (PI * t)
        };
        h[i] = sinc * window;
        sum += h[i];
        i += 1;
    }
    i = 0;
    while i < TAPS {
        h[i] /= sum;
        i += 1;
    }
    h
}
const KERNEL: [f64; TAPS] = make_kernel();

/// Independent voice lanes permit parallel multiply/add instructions without
/// changing the chronological additions within either reconstructed phase.
/// Const branches avoid touching a lane's zero control history.
#[inline]
fn interpolate_two<const FIRST_CONTROL: bool, const SECOND_CONTROL: bool>(
    signals: [&[f64]; 2],
    controls: [&[f64]; 2],
) -> ([[f64; 2]; OVERSAMPLING], [[f64; 2]; OVERSAMPLING]) {
    let mut signal = [[0.0; 2]; OVERSAMPLING];
    let mut control = [[0.0; 2]; OVERSAMPLING];
    for (tap, coefficients) in KERNEL.as_chunks::<OVERSAMPLING>().0.iter().enumerate() {
        let index = INPUT_TAPS - 1 - tap;
        for phase in 0..OVERSAMPLING {
            signal[phase][0] += coefficients[phase] * signals[0][index];
            signal[phase][1] += coefficients[phase] * signals[1][index];
            if FIRST_CONTROL {
                control[phase][0] += coefficients[phase] * controls[0][index];
            }
            if SECOND_CONTROL {
                control[phase][1] += coefficients[phase] * controls[1][index];
            }
        }
    }
    signal[0][0] += KERNEL[TAPS - 1] * signals[0][0];
    signal[0][1] += KERNEL[TAPS - 1] * signals[1][0];
    if FIRST_CONTROL {
        control[0][0] += KERNEL[TAPS - 1] * controls[0][0];
    }
    if SECOND_CONTROL {
        control[0][1] += KERNEL[TAPS - 1] * controls[1][0];
    }
    (signal, control)
}

/// Shared allocation-free oversampling for memoryless or stateful circuits.
/// The callback is invoked eight times in chronological order per host sample.
#[derive(Clone, Debug)]
pub(crate) struct Oversampler {
    // Mirrored rings expose chronological histories as contiguous slices. Each
    // write updates both halves; FIR reads no longer wrap at every coefficient.
    input: [f64; INPUT_TAPS * 2],
    output: [f64; TAPS * 2],
    control: [f64; INPUT_TAPS * 2],
    control_nonzero: usize,
    input_head: usize,
    output_head: usize,
}
impl Oversampler {
    pub(crate) fn new() -> Self {
        Self {
            input: [0.0; INPUT_TAPS * 2],
            output: [0.0; TAPS * 2],
            control: [0.0; INPUT_TAPS * 2],
            control_nonzero: 0,
            input_head: 0,
            output_head: 0,
        }
    }
    pub(crate) fn copy_state_from(&mut self, previous: &Self) {
        self.input = previous.input;
        self.output = previous.output;
        self.control = previous.control;
        self.control_nonzero = previous.control_nonzero;
        self.input_head = previous.input_head;
        self.output_head = previous.output_head;
    }
    /// Process two independent voices. Their ring heads may differ. Only the
    /// local arithmetic lanes are paired; storage and per-voice tap order stay
    /// identical to process_pair, including phase-zero decimation timing.
    #[inline]
    pub(crate) fn process_two(
        first: &mut Self,
        second: &mut Self,
        inputs: [f64; 2],
        controls: [f64; 2],
        mut circuits: impl FnMut([f64; 2], [f64; 2]) -> [f64; 2],
    ) -> [f64; 2] {
        for (voice, (input, control)) in [
            (&mut *first, (inputs[0], controls[0])),
            (&mut *second, (inputs[1], controls[1])),
        ] {
            voice.input_head = (voice.input_head + 1) % INPUT_TAPS;
            voice.input[voice.input_head] = input;
            voice.input[voice.input_head + INPUT_TAPS] = input;
            voice.control_nonzero -= usize::from(voice.control[voice.input_head] != 0.0);
            voice.control[voice.input_head] = control;
            voice.control[voice.input_head + INPUT_TAPS] = control;
            voice.control_nonzero += usize::from(control != 0.0);
        }
        let signals = [
            &first.input[first.input_head + 1..first.input_head + 1 + INPUT_TAPS],
            &second.input[second.input_head + 1..second.input_head + 1 + INPUT_TAPS],
        ];
        let controls = [
            &first.control[first.input_head + 1..first.input_head + 1 + INPUT_TAPS],
            &second.control[second.input_head + 1..second.input_head + 1 + INPUT_TAPS],
        ];
        let (interpolated, interpolated_control) =
            match (first.control_nonzero > 0, second.control_nonzero > 0) {
                (false, false) => interpolate_two::<false, false>(signals, controls),
                (true, false) => interpolate_two::<true, false>(signals, controls),
                (false, true) => interpolate_two::<false, true>(signals, controls),
                (true, true) => interpolate_two::<true, true>(signals, controls),
            };
        let mut result = [0.0; 2];
        for phase in 0..OVERSAMPLING {
            first.output_head = (first.output_head + 1) % TAPS;
            second.output_head = (second.output_head + 1) % TAPS;
            let output = circuits(
                interpolated[phase].map(|sample| sample * OVERSAMPLING as f64),
                interpolated_control[phase].map(|sample| sample * OVERSAMPLING as f64),
            );
            first.output[first.output_head] = output[0];
            first.output[first.output_head + TAPS] = output[0];
            second.output[second.output_head] = output[1];
            second.output[second.output_head + TAPS] = output[1];
            if phase == 0 {
                let first_history =
                    &first.output[first.output_head + 1..first.output_head + 1 + TAPS];
                let second_history =
                    &second.output[second.output_head + 1..second.output_head + 1 + TAPS];
                // Two serial tap sums, with independent accumulators. Never
                // combine taps within one voice or use a fused multiply-add.
                for ((coefficient, first_sample), second_sample) in KERNEL
                    .iter()
                    .zip(first_history.iter().rev())
                    .zip(second_history.iter().rev())
                {
                    result[0] += coefficient * first_sample;
                    result[1] += coefficient * second_sample;
                }
            }
        }
        result
    }
    /// Reconstruct one independent pair without retaining four voices' FIR
    /// accumulators at once. Used by process_four; the original pair path stays
    /// unchanged for two voices and quartet remainders.
    #[inline]
    fn interpolate_pair(
        first: &Self,
        second: &Self,
    ) -> ([[f64; 2]; OVERSAMPLING], [[f64; 2]; OVERSAMPLING]) {
        let signals = [
            &first.input[first.input_head + 1..first.input_head + 1 + INPUT_TAPS],
            &second.input[second.input_head + 1..second.input_head + 1 + INPUT_TAPS],
        ];
        let controls = [
            &first.control[first.input_head + 1..first.input_head + 1 + INPUT_TAPS],
            &second.control[second.input_head + 1..second.input_head + 1 + INPUT_TAPS],
        ];
        match (first.control_nonzero > 0, second.control_nonzero > 0) {
            (false, false) => interpolate_two::<false, false>(signals, controls),
            (true, false) => interpolate_two::<true, false>(signals, controls),
            (false, true) => interpolate_two::<false, true>(signals, controls),
            (true, true) => interpolate_two::<true, true>(signals, controls),
        }
    }
    /// Four independent voice histories share one coefficient traversal during
    /// decimation. Each voice retains its original tap order and eight scalar
    /// circuit evaluations; all phase-zero sums finish before phase-one writes.
    #[inline]
    pub(crate) fn process_four(
        first: &mut Self,
        second: &mut Self,
        third: &mut Self,
        fourth: &mut Self,
        inputs: [f64; 4],
        controls: [f64; 4],
        mut circuits: impl FnMut([f64; 4], [f64; 4]) -> [f64; 4],
    ) -> [f64; 4] {
        for (lane, voice) in [&mut *first, &mut *second, &mut *third, &mut *fourth]
            .into_iter()
            .enumerate()
        {
            voice.input_head = (voice.input_head + 1) % INPUT_TAPS;
            voice.input[voice.input_head] = inputs[lane];
            voice.input[voice.input_head + INPUT_TAPS] = inputs[lane];
            voice.control_nonzero -= usize::from(voice.control[voice.input_head] != 0.0);
            voice.control[voice.input_head] = controls[lane];
            voice.control[voice.input_head + INPUT_TAPS] = controls[lane];
            voice.control_nonzero += usize::from(controls[lane] != 0.0);
        }
        let (signal01, control01) = Self::interpolate_pair(first, second);
        let (signal23, control23) = Self::interpolate_pair(third, fourth);
        let mut result = [0.0; 4];
        for phase in 0..OVERSAMPLING {
            for voice in [&mut *first, &mut *second, &mut *third, &mut *fourth] {
                voice.output_head = (voice.output_head + 1) % TAPS;
            }
            let output = circuits(
                [
                    signal01[phase][0],
                    signal01[phase][1],
                    signal23[phase][0],
                    signal23[phase][1],
                ]
                .map(|value| value * OVERSAMPLING as f64),
                [
                    control01[phase][0],
                    control01[phase][1],
                    control23[phase][0],
                    control23[phase][1],
                ]
                .map(|value| value * OVERSAMPLING as f64),
            );
            for (lane, voice) in [&mut *first, &mut *second, &mut *third, &mut *fourth]
                .into_iter()
                .enumerate()
            {
                voice.output[voice.output_head] = output[lane];
                voice.output[voice.output_head + TAPS] = output[lane];
            }
            if phase == 0 {
                let history0 = &first.output[first.output_head + 1..first.output_head + 1 + TAPS];
                let history1 =
                    &second.output[second.output_head + 1..second.output_head + 1 + TAPS];
                let history2 = &third.output[third.output_head + 1..third.output_head + 1 + TAPS];
                let history3 =
                    &fourth.output[fourth.output_head + 1..fourth.output_head + 1 + TAPS];
                for ((((coefficient, sample0), sample1), sample2), sample3) in KERNEL
                    .iter()
                    .zip(history0.iter().rev())
                    .zip(history1.iter().rev())
                    .zip(history2.iter().rev())
                    .zip(history3.iter().rev())
                {
                    result[0] += coefficient * sample0;
                    result[1] += coefficient * sample1;
                    result[2] += coefficient * sample2;
                    result[3] += coefficient * sample3;
                }
            }
        }
        result
    }
    #[cfg(test)]
    #[inline]
    pub(crate) fn process(&mut self, input: f64, mut circuit: impl FnMut(f64) -> f64) -> f64 {
        self.process_pair(input, 0.0, |signal, _| circuit(signal))
    }
    /// Reconstruct signal and control with identical phase and group delay.
    /// The zero-control fast path skips control FIR work when its history is zero.
    #[inline]
    pub(crate) fn process_pair(
        &mut self,
        input: f64,
        control: f64,
        mut circuit: impl FnMut(f64, f64) -> f64,
    ) -> f64 {
        self.input_head = (self.input_head + 1) % INPUT_TAPS;
        self.input[self.input_head] = input;
        self.input[self.input_head + INPUT_TAPS] = input;
        self.control_nonzero -= usize::from(self.control[self.input_head] != 0.0);
        self.control[self.input_head] = control;
        self.control[self.input_head + INPUT_TAPS] = control;
        self.control_nonzero += usize::from(control != 0.0);
        let input_history = &self.input[self.input_head + 1..self.input_head + 1 + INPUT_TAPS];
        let control_history = &self.control[self.input_head + 1..self.input_head + 1 + INPUT_TAPS];
        // Accumulate independent phase lanes together so the compiler can use
        // SIMD without reordering the additions within any reconstructed sample.
        // The first 512 taps cover all eight lanes; tap 512 belongs to phase 0.
        let mut interpolated = [0.0; OVERSAMPLING];
        let mut interpolated_control = [0.0; OVERSAMPLING];
        if self.control_nonzero > 0 {
            for ((coefficients, signal), control) in KERNEL
                .as_chunks::<OVERSAMPLING>()
                .0
                .iter()
                .zip(input_history.iter().rev())
                .zip(control_history.iter().rev())
            {
                for phase in 0..OVERSAMPLING {
                    interpolated[phase] += coefficients[phase] * signal;
                    interpolated_control[phase] += coefficients[phase] * control;
                }
            }
            interpolated_control[0] += KERNEL[TAPS - 1] * control_history[0];
        } else {
            for (coefficients, signal) in KERNEL
                .as_chunks::<OVERSAMPLING>()
                .0
                .iter()
                .zip(input_history.iter().rev())
            {
                for phase in 0..OVERSAMPLING {
                    interpolated[phase] += coefficients[phase] * signal;
                }
            }
        }
        interpolated[0] += KERNEL[TAPS - 1] * input_history[0];
        let mut result = 0.0;
        for phase in 0..OVERSAMPLING {
            self.output_head = (self.output_head + 1) % TAPS;
            let output = circuit(
                interpolated[phase] * OVERSAMPLING as f64,
                interpolated_control[phase] * OVERSAMPLING as f64,
            );
            self.output[self.output_head] = output;
            self.output[self.output_head + TAPS] = output;
            // Decimate at phase zero to retain the exact integer 64-sample delay.
            if phase == 0 {
                let history = &self.output[self.output_head + 1..self.output_head + 1 + TAPS];
                for (coefficient, sample) in KERNEL.iter().zip(history.iter().rev()) {
                    result += coefficient * sample;
                }
            }
        }
        result
    }
}

/// Numerical diagnostics are cumulative and require no audio-thread I/O.
#[derive(Clone, Copy, Debug, Default)]
pub struct Diagnostics {
    pub solver_failures: u64,
    pub invalid_inputs: u64,
    pub max_iterations: u8,
    pub max_accepted_residual: f64,
}

#[derive(Clone, Debug)]
struct Integrator {
    q: [f64; 4],
    previous_term: [f64; 4],
    previous_input: f64,
    diagnostics: Diagnostics,
}
fn currents(q: [f64; 4], u: f64, k: f64) -> ([f64; 4], [f64; 4], f64) {
    let t = q.map(f64::tanh);
    let input = (u - k * q[3]).tanh();
    (
        [input - t[0], t[0] - t[1], t[1] - t[2], t[2] - t[3]],
        t.map(|x| 1.0 - x * x),
        1.0 - input * input,
    )
}
fn max_abs(a: [f64; 4]) -> f64 {
    a.into_iter().fold(0.0_f64, |m, x| {
        if x.is_finite() {
            m.max(x.abs())
        } else {
            f64::INFINITY
        }
    })
}
impl Integrator {
    fn new() -> Self {
        Self {
            q: [0.0; 4],
            previous_term: [0.0; 4],
            previous_input: 0.0,
            diagnostics: Diagnostics::default(),
        }
    }
    #[inline]
    fn step(&mut self, input: f64, scale: f64, g: f64, k: f64) -> f64 {
        let u = input * scale;
        let old = self.q;
        // Start at the previous solution. The cyclic bidiagonal Jacobian is
        // solved analytically, including the instantaneous feedback corner.
        let mut candidate = old;
        // Retain the currents already evaluated at each accepted Newton trial.
        // Re-evaluating tanh at the identical state adds work without changing
        // the residual, Jacobian, or derivative used by the next host step.
        let initial_evaluation = currents(old, u, k);
        let mut evaluation = initial_evaluation;
        let mut accepted_residual = f64::INFINITY;
        let mut iterations = 0;
        for iteration in 0..MAX_NEWTON {
            iterations = iteration + 1;
            let (f, derivative, input_derivative) = evaluation;
            let residual =
                std::array::from_fn(|i| candidate[i] - old[i] - self.previous_term[i] - g * f[i]);
            let norm = max_abs(residual);
            if norm <= RESIDUAL_TOLERANCE * (1.0 + max_abs(candidate)) {
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
            for _ in 0..MAX_BACKTRACK {
                let trial = std::array::from_fn(|i| candidate[i] + damping * delta[i]);
                if trial.iter().all(|x| x.is_finite()) {
                    let trial_evaluation = currents(trial, u, k);
                    let trial_f = trial_evaluation.0;
                    let trial_residual = max_abs(std::array::from_fn(|i| {
                        trial[i] - old[i] - self.previous_term[i] - g * trial_f[i]
                    }));
                    if trial_residual < norm {
                        candidate = trial;
                        evaluation = trial_evaluation;
                        if trial_residual <= RESIDUAL_TOLERANCE * (1.0 + max_abs(trial)) {
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
            evaluation = initial_evaluation;
        }
        self.previous_term = evaluation.0.map(|f| g * f);
        self.previous_input = input;
        OUTPUT_GAIN * 2.0 * THERMAL_VOLTAGE * self.q[3]
    }
}

fn coefficient(cutoff: f64, rate: f64, fixed_g: f64, cv: f64) -> f64 {
    if cv == 0.0 {
        return fixed_g;
    }
    let maximum = 18_000.0_f64.min(0.4 * rate);
    // Bound CV before exp2 to prevent overflow; the physical pole bounds
    // determine the final value, including FIR overshoot at abrupt CV edges.
    let frequency = (cutoff * cv.clamp(-32.0, 32.0).exp2()).clamp(20.0, maximum);
    PI * frequency / (rate * OVERSAMPLING as f64)
}

/// Parameters are validated by the patch compiler: rate 8..192 kHz, pole
/// 20..min(18000,0.4*rate) Hz, resonance 0..1.1, drive 0..4 (unity default).
#[derive(Clone, Debug)]
pub struct Ladder {
    sample_rate: f64,
    cutoff: f64,
    feedback: f64,
    input_scale: f64,
    fixed_g: f64,
    last_cv: f64,
    oversampler: Oversampler,
    integrator: Integrator,
}
impl Ladder {
    pub fn new(sample_rate: f64, cutoff: f64, resonance: f64, drive: f64) -> Self {
        Self {
            sample_rate,
            cutoff,
            feedback: 4.0 * resonance,
            input_scale: INPUT_DIVIDER * drive / (2.0 * THERMAL_VOLTAGE),
            fixed_g: PI * cutoff / (sample_rate * OVERSAMPLING as f64),
            last_cv: 0.0,
            oversampler: Oversampler::new(),
            integrator: Integrator::new(),
        }
    }
    fn g_for_cv(&self, cv: f64) -> f64 {
        coefficient(self.cutoff, self.sample_rate, self.fixed_g, cv)
    }
    pub fn process(&mut self, input_volts: f32, cutoff_cv: f32) -> f32 {
        let input = if input_volts.is_finite() {
            input_volts as f64
        } else {
            self.integrator.diagnostics.invalid_inputs =
                self.integrator.diagnostics.invalid_inputs.saturating_add(1);
            0.0
        };
        let cv = if cutoff_cv.is_finite() {
            cutoff_cv as f64
        } else {
            self.integrator.diagnostics.invalid_inputs =
                self.integrator.diagnostics.invalid_inputs.saturating_add(1);
            0.0
        };
        let integrator = &mut self.integrator;
        let scale = self.input_scale;
        let k = self.feedback;
        let cutoff = self.cutoff;
        let rate = self.sample_rate;
        let fixed_g = self.fixed_g;
        let last_cv = &mut self.last_cv;
        self.oversampler
            .process_pair(input, cv, |x, interpolated_cv| {
                *last_cv = interpolated_cv;
                integrator.step(
                    x,
                    scale,
                    coefficient(cutoff, rate, fixed_g, interpolated_cv),
                    k,
                )
            }) as f32
    }
    /// Pair independent voices without altering each integrator's eight
    /// chronological scalar steps or coefficient precision.
    pub(crate) fn process_two(
        first: &mut Self,
        second: &mut Self,
        inputs: [f32; 2],
        cvs: [f32; 2],
    ) -> [f32; 2] {
        let mut inputs = inputs.map(f64::from);
        let mut cvs = cvs.map(f64::from);
        for (lane, voice) in [&mut *first, &mut *second].into_iter().enumerate() {
            if !inputs[lane].is_finite() {
                voice.integrator.diagnostics.invalid_inputs = voice
                    .integrator
                    .diagnostics
                    .invalid_inputs
                    .saturating_add(1);
                inputs[lane] = 0.0;
            }
            if !cvs[lane].is_finite() {
                voice.integrator.diagnostics.invalid_inputs = voice
                    .integrator
                    .diagnostics
                    .invalid_inputs
                    .saturating_add(1);
                cvs[lane] = 0.0;
            }
        }
        let (cutoff0, rate0, fixed0, scale0, feedback0) = (
            first.cutoff,
            first.sample_rate,
            first.fixed_g,
            first.input_scale,
            first.feedback,
        );
        let (cutoff1, rate1, fixed1, scale1, feedback1) = (
            second.cutoff,
            second.sample_rate,
            second.fixed_g,
            second.input_scale,
            second.feedback,
        );
        let integrator0 = &mut first.integrator;
        let integrator1 = &mut second.integrator;
        let last_cv0 = &mut first.last_cv;
        let last_cv1 = &mut second.last_cv;
        Oversampler::process_two(
            &mut first.oversampler,
            &mut second.oversampler,
            inputs,
            cvs,
            |signal, control| {
                *last_cv0 = control[0];
                *last_cv1 = control[1];
                [
                    integrator0.step(
                        signal[0],
                        scale0,
                        coefficient(cutoff0, rate0, fixed0, control[0]),
                        feedback0,
                    ),
                    integrator1.step(
                        signal[1],
                        scale1,
                        coefficient(cutoff1, rate1, fixed1, control[1]),
                        feedback1,
                    ),
                ]
            },
        )
        .map(|sample| sample as f32)
    }
    /// Four independent scalar circuits share FIR traversal. No solver state
    /// or constructor parameter is combined between voices.
    pub(crate) fn process_four(
        first: &mut Self,
        second: &mut Self,
        third: &mut Self,
        fourth: &mut Self,
        inputs: [f32; 4],
        cvs: [f32; 4],
    ) -> [f32; 4] {
        let mut inputs = inputs.map(f64::from);
        let mut cvs = cvs.map(f64::from);
        for (lane, voice) in [&mut *first, &mut *second, &mut *third, &mut *fourth]
            .into_iter()
            .enumerate()
        {
            if !inputs[lane].is_finite() {
                voice.integrator.diagnostics.invalid_inputs = voice
                    .integrator
                    .diagnostics
                    .invalid_inputs
                    .saturating_add(1);
                inputs[lane] = 0.0;
            }
            if !cvs[lane].is_finite() {
                voice.integrator.diagnostics.invalid_inputs = voice
                    .integrator
                    .diagnostics
                    .invalid_inputs
                    .saturating_add(1);
                cvs[lane] = 0.0;
            }
        }
        let parameters = [
            (
                first.cutoff,
                first.sample_rate,
                first.fixed_g,
                first.input_scale,
                first.feedback,
            ),
            (
                second.cutoff,
                second.sample_rate,
                second.fixed_g,
                second.input_scale,
                second.feedback,
            ),
            (
                third.cutoff,
                third.sample_rate,
                third.fixed_g,
                third.input_scale,
                third.feedback,
            ),
            (
                fourth.cutoff,
                fourth.sample_rate,
                fourth.fixed_g,
                fourth.input_scale,
                fourth.feedback,
            ),
        ];
        let mut integrators = [
            &mut first.integrator,
            &mut second.integrator,
            &mut third.integrator,
            &mut fourth.integrator,
        ];
        let mut last_cvs = [
            &mut first.last_cv,
            &mut second.last_cv,
            &mut third.last_cv,
            &mut fourth.last_cv,
        ];
        Oversampler::process_four(
            &mut first.oversampler,
            &mut second.oversampler,
            &mut third.oversampler,
            &mut fourth.oversampler,
            inputs,
            cvs,
            |signal, control| {
                let mut output = [0.0; 4];
                for lane in 0..4 {
                    *last_cvs[lane] = control[lane];
                    let (cutoff, rate, fixed, scale, feedback) = parameters[lane];
                    output[lane] = integrators[lane].step(
                        signal[lane],
                        scale,
                        coefficient(cutoff, rate, fixed, control[lane]),
                        feedback,
                    );
                }
                output
            },
        )
        .map(|sample| sample as f32)
    }
    /// Preserve capacitor voltages and both FIR histories across graph rebuilds;
    /// constructor parameters on `self` remain in force. Recompute the previous
    /// endpoint derivative for the new circuit parameters without resetting q.
    pub fn copy_state_from(&mut self, previous: &Self) {
        self.oversampler.copy_state_from(&previous.oversampler);
        self.integrator = previous.integrator.clone();
        self.last_cv = previous.last_cv;
        let g = self.g_for_cv(self.last_cv);
        self.integrator.previous_term = currents(
            self.integrator.q,
            self.integrator.previous_input * self.input_scale,
            self.feedback,
        )
        .0
        .map(|f| g * f);
    }
    pub fn diagnostics(&self) -> Diagnostics {
        self.integrator.diagnostics
    }
    pub fn half_bias_current_a(&self, cutoff_cv: f64) -> f64 {
        4.0 * THERMAL_VOLTAGE
            * CAPACITANCE_F
            * self.g_for_cv(cutoff_cv)
            * self.sample_rate
            * OVERSAMPLING as f64
    }
}

#[cfg(test)]
mod optimization_reference;

#[cfg(test)]
mod tests {
    use super::*;

    fn kernel_magnitude(host_frequency: f64) -> f64 {
        let phase = TAU * host_frequency / OVERSAMPLING as f64;
        let mut real = 0.0;
        let mut imaginary = 0.0;
        for (n, h) in KERNEL.iter().enumerate() {
            real += h * (phase * n as f64).cos();
            imaginary -= h * (phase * n as f64).sin();
        }
        real.hypot(imaginary)
    }
    #[test]
    fn fir_passband_stopband_and_latency_are_quantified() {
        let mut maximum_passband_error = 0.0_f64;
        let mut maximum_stopband = 0.0_f64;
        for n in 0..=400 {
            maximum_passband_error =
                maximum_passband_error.max((kernel_magnitude(n as f64 / 1000.0) - 1.0).abs());
        }
        for n in 500..=4000 {
            maximum_stopband = maximum_stopband.max(kernel_magnitude(n as f64 / 1000.0));
        }
        eprintln!(
            "FIR passband error={maximum_passband_error:.7}, stopband={} dB",
            20.0 * maximum_stopband.log10()
        );
        assert!(maximum_passband_error < 0.0002);
        assert!(maximum_stopband < 0.0002); // at least 74 dB at/above host Nyquist
        let mut resampler = Oversampler::new();
        let impulse: Vec<_> = (0..140)
            .map(|n| resampler.process(if n == 0 { 1.0 } else { 0.0 }, |x| x))
            .collect();
        let peak = impulse
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap()
            .0;
        assert_eq!(peak, LATENCY_SAMPLES);
        assert!((impulse.iter().sum::<f64>() - 1.0).abs() < 1e-7);
        for n in 0..64 {
            assert!((impulse[64 - n] - impulse[64 + n]).abs() < 1e-12);
        }
    }

    // Least-squares sinusoidal fit, valid for noninteger numbers of cycles.
    fn response(rate: f64, pole: f64, resonance: f64, frequency: f64) -> (f64, Diagnostics) {
        let mut ladder = Ladder::new(rate, pole, resonance, 1.0);
        let amplitude = 1e-4;
        let warmup = (rate * 0.10) as usize;
        let count = (rate * 0.25) as usize;
        let (mut ss, mut cc, mut sc, mut ys, mut yc) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for n in 0..warmup + count {
            let phase = TAU * frequency * n as f64 / rate;
            let y = ladder.process((amplitude * phase.sin()) as f32, 0.0) as f64;
            if n >= warmup {
                let (s, c) = phase.sin_cos();
                ss += s * s;
                cc += c * c;
                sc += s * c;
                ys += y * s;
                yc += y * c;
            }
        }
        let determinant = ss * cc - sc * sc;
        let sine = (ys * cc - yc * sc) / determinant;
        let cosine = (yc * ss - ys * sc) / determinant;
        (sine.hypot(cosine) / amplitude, ladder.diagnostics())
    }
    fn analog_magnitude(frequency_ratio: f64, k: f64) -> f64 {
        // (1+jr)^4+k = (1-6r²+r⁴+k) + j(4r-4r³).
        let r = frequency_ratio;
        1.0 / (1.0 - 6.0 * r * r + r.powi(4) + k).hypot(4.0 * r - 4.0 * r.powi(3))
    }
    #[test]
    fn small_signal_matches_continuous_circuit() {
        for resonance in [0.0, 0.65, 0.95] {
            for ratio in [0.1, 0.434_979_442, 1.0, 2.0] {
                let (actual, diagnostics) = response(48_000.0, 1_000.0, resonance, ratio * 1_000.0);
                let expected = analog_magnitude(ratio, 4.0 * resonance);
                let error = (actual / expected - 1.0).abs();
                eprintln!("linear resonance={resonance} f/fp={ratio} measured={actual:.8} expected={expected:.8} error={error:.6}");
                assert!(error < 0.0015);
                assert_eq!(diagnostics.solver_failures, 0);
            }
        }
        assert!((20.0 * analog_magnitude(1.0, 0.0).log10() + 12.041_199_826).abs() < 1e-7);
        assert!((analog_magnitude(0.434_979_442, 0.0) - 0.5_f64.sqrt()).abs() < 1e-8);
    }
    #[test]
    fn dc_transfer_and_bias_have_physical_units() {
        for resonance in [0.0, 0.6, 0.95] {
            let mut ladder = Ladder::new(48_000.0, 1_000.0, resonance, 1.0);
            let mut output = 0.0;
            for _ in 0..12_000 {
                output = ladder.process(0.7, 0.0);
            }
            assert!((output as f64 - 0.7 / (1.0 + 4.0 * resonance)).abs() < 1e-5);
            let expected = 4.0 * PI * THERMAL_VOLTAGE * CAPACITANCE_F * 1_000.0;
            assert!((ladder.half_bias_current_a(0.0) - expected).abs() < 1e-16);
            assert!((ladder.half_bias_current_a(1.0) / expected - 2.0).abs() < 1e-12);
            assert_eq!(ladder.diagnostics().solver_failures, 0);
        }
    }
    #[test]
    fn sample_rate_consistency() {
        for frequency in [300.0, 1_200.0, 4_000.0] {
            let (low, _) = response(44_100.0, 2_000.0, 0.8, frequency);
            let (high, _) = response(96_000.0, 2_000.0, 0.8, frequency);
            assert!((low / high - 1.0).abs() < 0.003);
        }
    }

    // Independent explicit RK4 reference: same reconstructed input and FIR
    // boundary conditions, but 16 fine steps per internal trapezoidal interval
    // (128× host rate). Input varies linearly between reconstructed endpoints.
    // Deliberately does not call production currents() or use its Jacobian.
    fn reference_derivative(
        q: [f64; 4],
        vin: f64,
        pole: f64,
        resonance: f64,
        drive: f64,
    ) -> [f64; 4] {
        let omega = TAU * pole;
        let u = INPUT_DIVIDER * drive * vin / (2.0 * THERMAL_VOLTAGE);
        [
            omega * ((u - 4.0 * resonance * q[3]).tanh() - q[0].tanh()),
            omega * (q[0].tanh() - q[1].tanh()),
            omega * (q[1].tanh() - q[2].tanh()),
            omega * (q[2].tanh() - q[3].tanh()),
        ]
    }
    fn reference_interval(
        q: &mut [f64; 4],
        start: f64,
        end: f64,
        rate: f64,
        poles: [f64; 2],
        resonance: f64,
        drive: f64,
    ) -> f64 {
        const SUBSTEPS: usize = 16;
        let [start_pole, end_pole] = poles;
        let dt = 1.0 / (rate * OVERSAMPLING as f64 * SUBSTEPS as f64);
        for sub in 0..SUBSTEPS {
            let v0 = start + (end - start) * sub as f64 / SUBSTEPS as f64;
            let v1 = start + (end - start) * (sub as f64 + 0.5) / SUBSTEPS as f64;
            let v2 = start + (end - start) * (sub as f64 + 1.0) / SUBSTEPS as f64;
            let p0 = start_pole + (end_pole - start_pole) * sub as f64 / SUBSTEPS as f64;
            let p1 = start_pole + (end_pole - start_pole) * (sub as f64 + 0.5) / SUBSTEPS as f64;
            let p2 = start_pole + (end_pole - start_pole) * (sub as f64 + 1.0) / SUBSTEPS as f64;
            let k1 = reference_derivative(*q, v0, p0, resonance, drive);
            let k2 = reference_derivative(
                std::array::from_fn(|i| q[i] + 0.5 * dt * k1[i]),
                v1,
                p1,
                resonance,
                drive,
            );
            let k3 = reference_derivative(
                std::array::from_fn(|i| q[i] + 0.5 * dt * k2[i]),
                v1,
                p1,
                resonance,
                drive,
            );
            let k4 = reference_derivative(
                std::array::from_fn(|i| q[i] + dt * k3[i]),
                v2,
                p2,
                resonance,
                drive,
            );
            for i in 0..4 {
                q[i] += dt * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]) / 6.0;
            }
        }
        OUTPUT_GAIN * 2.0 * THERMAL_VOLTAGE * q[3]
    }
    #[test]
    fn nonlinear_feedback_agrees_with_finely_stepped_reference() {
        let rate = 48_000.0;
        for (pole, resonance, drive) in [(1200.0, 0.85, 2.5), (7000.0, 0.9, 4.0)] {
            let mut ladder = Ladder::new(rate, pole, resonance, drive);
            let mut resampler = Oversampler::new();
            let mut q = [0.0; 4];
            let mut previous = 0.0;
            let (mut error_energy, mut reference_energy, mut max_error) = (0.0, 0.0, 0.0_f64);
            for n in 0..8_000 {
                let time = n as f64 / rate;
                let input =
                    (2.1 * (TAU * 731.0 * time).sin() + 0.9 * (TAU * 1937.0 * time).sin()) as f32;
                let actual = ladder.process(input, 0.0) as f64;
                let reference = resampler.process(input as f64, |next| {
                    let y = reference_interval(
                        &mut q,
                        previous,
                        next,
                        rate,
                        [pole, pole],
                        resonance,
                        drive,
                    );
                    previous = next;
                    y
                });
                if n > 1000 {
                    error_energy += (actual - reference).powi(2);
                    reference_energy += reference * reference;
                    max_error = max_error.max((actual - reference).abs());
                }
            }
            let relative_rms = (error_energy / reference_energy).sqrt();
            eprintln!("RK4 pole={pole}, resonance={resonance}, drive={drive}: relative RMS error={relative_rms:.7}, max volts error={max_error:.7}");
            assert!(relative_rms < 0.001);
            assert!(max_error < 0.005);
            assert_eq!(ladder.diagnostics().solver_failures, 0);
        }
    }
    #[test]
    fn nonlinear_audio_rate_cv_agrees_with_independent_reference() {
        let rate = 48_000.0;
        let mut ladder = Ladder::new(rate, 2300.0, 0.9, 2.0);
        let mut resampler = Oversampler::new();
        let mut q = [0.0; 4];
        let mut previous_input = 0.0;
        let mut previous_pole = 2300.0;
        let (mut error_energy, mut reference_energy) = (0.0, 0.0);
        for n in 0..8_000 {
            let time = n as f64 / rate;
            let input = (2.0 * (TAU * 721.0 * time).sin()) as f32;
            let cv = (0.75 * (TAU * 371.0 * time).sin()) as f32;
            let actual = ladder.process(input, cv) as f64;
            let reference = resampler.process_pair(input as f64, cv as f64, |next, control| {
                let pole = (2300.0 * control.exp2()).clamp(20.0, 18000.0);
                let y = reference_interval(
                    &mut q,
                    previous_input,
                    next,
                    rate,
                    [previous_pole, pole],
                    0.9,
                    2.0,
                );
                previous_input = next;
                previous_pole = pole;
                y
            });
            if n > 1000 {
                error_energy += (actual - reference).powi(2);
                reference_energy += reference * reference;
            }
        }
        let relative_rms = (error_energy / reference_energy).sqrt();
        eprintln!("371 Hz CV RK4 relative RMS error={relative_rms:.7}");
        assert!(relative_rms < 0.001);
        assert_eq!(ladder.diagnostics().solver_failures, 0);
    }
    #[test]
    fn self_oscillation_requires_excitation_and_saturates_through_the_circuit() {
        let mut ladder = Ladder::new(48_000.0, 1000.0, 1.1, 1.0);
        for _ in 0..100 {
            assert_eq!(ladder.process(0.0, 0.0), 0.0);
        }
        let mut energy = 0.0;
        let mut maximum = 0.0_f64;
        for n in 0..16_000 {
            let output = ladder.process(if n == 0 { 0.1 } else { 0.0 }, 0.0) as f64;
            if n > 12_000 {
                energy += output * output;
            }
            maximum = maximum.max(output.abs());
        }
        let rms = (energy / 3999.0).sqrt();
        eprintln!("self-oscillation RMS={rms:.6} V, peak={maximum:.6} V");
        assert!(rms > 0.1 && maximum < 10.0);
        assert_eq!(ladder.diagnostics().solver_failures, 0);
    }
    #[test]
    fn high_resonance_drive_and_cv_remain_finite_without_solver_failures() {
        for rate in [8_000.0, 44_100.0, 48_000.0, 192_000.0] {
            let mut ladder = Ladder::new(rate, 1000.0, 1.1, 4.0);
            let mut maximum = 0.0_f32;
            for n in 0..12_000 {
                let input = if n % 29 < 14 { 10.0 } else { -10.0 };
                let cv = if n % 137 < 50 {
                    20.0
                } else if n % 137 < 90 {
                    -20.0
                } else {
                    0.0
                };
                let y = ladder.process(input, cv);
                assert!(y.is_finite());
                maximum = maximum.max(y.abs());
            }
            eprintln!(
                "stress rate={rate}, max={maximum}, diagnostics={:?}",
                ladder.diagnostics()
            );
            assert!(maximum < 100.0);
            assert_eq!(ladder.diagnostics().solver_failures, 0);
            assert!(ladder.diagnostics().max_accepted_residual < 1e-10);
        }
    }
    #[test]
    fn copied_state_is_continuous_and_new_parameters_take_effect() {
        let mut old = Ladder::new(48_000.0, 1500.0, 0.8, 1.0);
        for n in 0..1000 {
            old.process((TAU * 331.0 * n as f64 / 48_000.0).sin() as f32, 0.3);
        }
        let mut same = Ladder::new(48_000.0, 1500.0, 0.8, 1.0);
        same.copy_state_from(&old);
        for n in 1000..1100 {
            let input = (TAU * 331.0 * n as f64 / 48_000.0).sin() as f32;
            assert_eq!(old.process(input, 0.3), same.process(input, 0.3));
        }
        let mut changed = Ladder::new(48_000.0, 4000.0, 0.2, 2.0);
        changed.copy_state_from(&old);
        assert_eq!(changed.integrator.q, old.integrator.q);
        assert_eq!(changed.feedback, 0.8);
        assert_ne!(
            changed.integrator.previous_term,
            old.integrator.previous_term
        );
    }
    #[test]
    fn solver_failure_retains_state_counts_event_and_recovers() {
        let mut integrator = Integrator::new();
        integrator.previous_term[0] = f64::INFINITY;
        assert_eq!(integrator.step(0.0, 1.0, 0.1, 3.6), 0.0);
        assert_eq!(integrator.q, [0.0; 4]);
        assert_eq!(integrator.diagnostics.solver_failures, 1);
        assert_eq!(integrator.step(0.0, 1.0, 0.1, 3.6), 0.0);
        assert_eq!(integrator.diagnostics.solver_failures, 1);
    }
    #[test]
    fn invalid_input_policy_is_counted() {
        let mut ladder = Ladder::new(48_000.0, 1000.0, 0.8, 1.0);
        assert!(ladder.process(f32::NAN, f32::INFINITY).is_finite());
        assert_eq!(ladder.diagnostics().invalid_inputs, 2);
    }
}

#[cfg(test)]
mod pair_tests;
