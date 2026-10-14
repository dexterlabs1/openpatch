//! Original four-stage 2x FIR resampler for the isolated SVF drive experiment.
//! MIT. The host-adjacent 129-tap Blackman sinc cuts off at .45 host rate;
//! higher stages use 33/17/33-tap exact-zero halfbands. Each stage's normalized
//! kernel is shared by interpolation and decimation. The complete phase-zero
//! cascade delays 76 host samples. Storage is fixed; processing and state
//! adoption allocate nothing. Numerical output is compared to direct FIRs.
use kernels::{H2, H4, H8, H16};
#[path = "svf_resampler_kernels.rs"]
mod kernels;

pub(crate) const LATENCY_SAMPLES: usize = 76;

/// A mirrored low-rate history permits contiguous polyphase reads. M is twice
/// ceil(N/2); coefficients and history share only the per-stage arithmetic.
#[derive(Clone, Debug)]
struct Interpolator<const N: usize, const M: usize, const HALF: bool> {
    history: [f64; M],
    head: usize,
}
impl<const N: usize, const M: usize, const HALF: bool> Interpolator<N, M, HALF> {
    fn new() -> Self { Self { history: [0.0; M], head: 0 } }
    #[inline]
    fn process(&mut self, input: f64, h: &[f64; N]) -> [f64; 2] {
        let count = M / 2;
        self.head += 1;
        if self.head == count { self.head = 0; }
        self.history[self.head] = input;
        self.history[self.head + count] = input;
        let history = &self.history[self.head + 1..self.head + 1 + count];
        let even = if HALF {
            h[N / 2] * history[count - 1 - N / 4]
        } else {
            let taps = N.div_ceil(2);
            let mut sum = 0.0;
            for tap in 0..taps / 2 {
                sum += h[2 * tap] * (history[count - 1 - tap] + history[count - taps + tap]);
            }
            if taps % 2 != 0 { sum += h[2 * (taps / 2)] * history[count - 1 - taps / 2]; }
            sum
        };
        let taps = N / 2;
        let mut odd = 0.0;
        for tap in 0..taps / 2 {
            odd += h[2 * tap + 1] * (history[count - 1 - tap] + history[count - taps + tap]);
        }
        if taps % 2 != 0 { odd += h[2 * (taps / 2) + 1] * history[count - 1 - taps / 2]; }
        [2.0 * even, 2.0 * odd]
    }
}

/// All high-rate outputs enter history; only the phase-zero FIR is evaluated.
#[derive(Clone, Debug)]
struct Decimator<const N: usize, const M: usize, const HALF: bool> {
    history: [f64; M],
    head: usize,
}
impl<const N: usize, const M: usize, const HALF: bool> Decimator<N, M, HALF> {
    fn new() -> Self { Self { history: [0.0; M], head: 0 } }
    #[inline]
    fn push(&mut self, input: f64) {
        self.head += 1;
        if self.head == N { self.head = 0; }
        self.history[self.head] = input;
        self.history[self.head + N] = input;
    }
    #[inline]
    fn process(&mut self, input: [f64; 2], h: &[f64; N]) -> f64 {
        self.push(input[0]);
        let history = &self.history[self.head + 1..self.head + 1 + N];
        let midpoint = N / 2;
        let mut sum = 0.0;
        if HALF {
            // Every other coefficient is mathematically zero, including the
            // outermost tap. No multiply is performed for these exact zeros.
            for tap in (1..midpoint).step_by(2) {
                sum += h[tap] * (history[N - 1 - tap] + history[tap]);
            }
        } else {
            for tap in 0..midpoint {
                sum += h[tap] * (history[N - 1 - tap] + history[tap]);
            }
        }
        sum += h[midpoint] * history[midpoint];
        self.push(input[1]);
        sum
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Oversampler {
    up2: Interpolator<129, 130, false>,
    up4: Interpolator<33, 34, true>,
    up8: Interpolator<17, 18, true>,
    up16: Interpolator<33, 34, true>,
    down16: Decimator<33, 66, true>,
    down8: Decimator<17, 34, true>,
    down4: Decimator<33, 66, true>,
    down2: Decimator<129, 258, false>,
}
impl Oversampler {
    pub(crate) fn new() -> Self {
        Self {
            up2: Interpolator::new(), up4: Interpolator::new(),
            up8: Interpolator::new(), up16: Interpolator::new(),
            down16: Decimator::new(), down8: Decimator::new(),
            down4: Decimator::new(), down2: Decimator::new(),
        }
    }
    pub(crate) fn copy_state_from(&mut self, old: &Self) { *self = old.clone(); }
    /// SVF supplies zero control here; cutoff CV is delayed by the complete
    /// integer group delay in DriveLane. Exactly sixteen chronological scalar
    /// callbacks preserve the original drive curve and nonlinear evaluation.
    #[inline]
    pub(crate) fn process_pair(&mut self, input: f64, control: f64,
        mut circuit: impl FnMut(f64, f64) -> f64) -> f64 {
        debug_assert_eq!(control, 0.0, "SVF control is delayed outside the drive resampler");
        let at2 = self.up2.process(input, &H2);
        let mut at4 = [0.0; 4];
        for (n, x) in at2.into_iter().enumerate() {
            at4[2*n..2*n+2].copy_from_slice(&self.up4.process(x, &H4));
        }
        let mut at8 = [0.0; 8];
        for (n, x) in at4.into_iter().enumerate() {
            at8[2*n..2*n+2].copy_from_slice(&self.up8.process(x, &H8));
        }
        let mut out8 = [0.0; 8];
        for (n, x) in at8.into_iter().enumerate() {
            let pair = self.up16.process(x, &H16);
            let a = circuit(pair[0], 0.0);
            let b = circuit(pair[1], 0.0);
            out8[n] = self.down16.process([a, b], &H16);
        }
        let mut out4 = [0.0; 4];
        for (n, pair) in out8.chunks_exact(2).enumerate() {
            out4[n] = self.down8.process([pair[0], pair[1]], &H8);
        }
        let mut out2 = [0.0; 2];
        for (n, pair) in out4.chunks_exact(2).enumerate() {
            out2[n] = self.down4.process([pair[0], pair[1]], &H4);
        }
        self.down2.process(out2, &H2)
    }
}

#[cfg(test)]
#[path = "svf_resampler_tests.rs"]
mod tests;
